import type { BrowserDownloadMode, BrowserIntegrationPreferences, DownloadKind } from "@ldm/contracts";

/** `chrome.storage.session` key the background uses to share the current preferences with content scripts. */
export const BROWSER_PREFERENCES_STORAGE_KEY = "ldm.browserPreferences";

export type BrowserDownloadPrompt = {
  filename: string;
  kind: DownloadKind;
  reason: "ask" | "executable" | "ask-executable";
};

/** What a routing decision looks at: enough of a job or candidate to apply mode, kinds, and domain rules. */
export type RoutingSubject = {
  kind: DownloadKind;
  sourceUrl: string;
  pageUrl?: string;
  referrer?: string;
  suggestedFilename?: string;
};

export type BrowserRoutingDecision =
  | { action: "browser" }
  | { action: "ldm" }
  | { action: "ask"; prompt: BrowserDownloadPrompt };

const DEFAULT_SELECTED_KINDS: Record<DownloadKind, boolean> = {
  video: true,
  audio: true,
  program: true,
  document: true,
  archive: true,
  image: true,
  other: false,
};

export const DEFAULT_BROWSER_PREFERENCES: BrowserIntegrationPreferences = {
  mode: "auto",
  selectedKinds: DEFAULT_SELECTED_KINDS,
  domainRules: [],
  warnOnExecutable: true,
};

/**
 * Decides whether a download goes to LDM, stays in the browser, or needs the user's answer.
 *
 * Domain rules win over the global mode; an "auto" rule also overrides the file-type selection.
 * The same function runs in the background (for browser downloads) and in content scripts
 * (for link clicks, synchronously, before the click is blocked).
 */
export function decideBrowserRouting(
  subject: RoutingSubject,
  preferences: BrowserIntegrationPreferences,
  options: { warnedExecutable?: boolean } = {},
): BrowserRoutingDecision {
  const rule = matchingDomainRuleForSubject(subject, preferences.domainRules);
  const mode = rule?.action ?? preferences.mode;
  if (mode === "off") {
    return { action: "browser" };
  }
  if (mode === "auto") {
    if (!rule && preferences.selectedKinds[subject.kind] !== true) {
      return { action: "browser" };
    }
    return { action: "ldm" };
  }

  const warnsAboutExecutable = subject.kind === "program" && preferences.warnOnExecutable && !options.warnedExecutable;
  return {
    action: "ask",
    prompt: {
      filename: subject.suggestedFilename ?? "download",
      kind: subject.kind,
      reason: warnsAboutExecutable ? "ask-executable" : "ask",
    },
  };
}

export function normalizeBrowserIntegrationPreferences(value: unknown): BrowserIntegrationPreferences {
  if (!value || typeof value !== "object") {
    return DEFAULT_BROWSER_PREFERENCES;
  }
  const input = value as Partial<BrowserIntegrationPreferences>;
  const domainRules = Array.isArray(input.domainRules)
    ? input.domainRules.flatMap((rule) => {
        const host = normalizeDomainRuleHost(rule?.host);
        return host && isBrowserDownloadMode(rule.action) ? [{ host, action: rule.action }] : [];
      })
    : [];

  return {
    mode: isBrowserDownloadMode(input.mode) ? input.mode : DEFAULT_BROWSER_PREFERENCES.mode,
    selectedKinds: normalizeSelectedKinds(input.selectedKinds),
    domainRules: dedupeDomainRules(domainRules),
    warnOnExecutable: typeof input.warnOnExecutable === "boolean" ? input.warnOnExecutable : true,
  };
}

export function isBrowserDownloadMode(value: unknown): value is BrowserDownloadMode {
  return value === "off" || value === "ask" || value === "auto";
}

function normalizeSelectedKinds(value: unknown): Record<DownloadKind, boolean> {
  const selectedKinds = { ...DEFAULT_SELECTED_KINDS };
  if (!value || typeof value !== "object") {
    return selectedKinds;
  }

  for (const kind of Object.keys(selectedKinds) as DownloadKind[]) {
    const nextValue = (value as Partial<Record<DownloadKind, unknown>>)[kind];
    if (typeof nextValue === "boolean") {
      selectedKinds[kind] = nextValue;
    }
  }

  return selectedKinds;
}

export function normalizeDomainRuleHost(value: unknown): string | undefined {
  if (typeof value !== "string") {
    return undefined;
  }

  const trimmed = value.trim().toLowerCase();
  if (!trimmed) {
    return undefined;
  }

  const hostSource = /^[a-z][a-z0-9+.-]*:\/\//u.test(trimmed) ? hostForUrl(trimmed) : trimmed.split(/[/?#]/u)[0];
  const host = hostSource
    ?.replace(/^\*\./u, "")
    .trim()
    .replace(/^\.+|\.+$/gu, "");
  if (!host || !isValidDomainRuleHost(host)) {
    return undefined;
  }

  return host;
}

function isValidDomainRuleHost(host: string): boolean {
  return (
    host.length > 0 &&
    host.length <= 253 &&
    /^[a-z0-9.-]+$/u.test(host) &&
    !host.includes("..") &&
    !host.split(".").some((part) => part.length === 0 || part.startsWith("-") || part.endsWith("-"))
  );
}

type DomainRule = BrowserIntegrationPreferences["domainRules"][number];

function dedupeDomainRules(rules: DomainRule[]): DomainRule[] {
  const byHost = new Map<string, DomainRule>();
  for (const rule of rules) {
    byHost.set(rule.host, rule);
  }

  return Array.from(byHost.values()).sort((left, right) => right.host.length - left.host.length);
}

function matchingDomainRule(host: string, rules: DomainRule[]): DomainRule | undefined {
  const normalizedHost = normalizeDomainRuleHost(host);
  if (!normalizedHost) {
    return undefined;
  }

  return rules.find((rule) => normalizedHost === rule.host || normalizedHost.endsWith(`.${rule.host}`));
}

function matchingDomainRuleForSubject(subject: RoutingSubject, rules: DomainRule[]): DomainRule | undefined {
  const hosts = [hostForUrl(subject.sourceUrl), hostForUrl(subject.pageUrl), hostForUrl(subject.referrer)].filter(
    (host): host is string => Boolean(host),
  );
  const matches = hosts
    .flatMap((host) => {
      const rule = matchingDomainRule(host, rules);
      return rule ? [rule] : [];
    })
    .sort((left, right) => right.host.length - left.host.length);

  return matches[0];
}

function hostForUrl(value: string | undefined): string | undefined {
  if (!value) {
    return undefined;
  }

  try {
    return new URL(value).hostname.toLowerCase();
  } catch {
    return undefined;
  }
}
