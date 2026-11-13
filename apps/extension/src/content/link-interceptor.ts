import type { CreateDownloadJobRequest, DetectedDownloadCandidate } from "@ldm/contracts";
import {
  type BrowserDownloadPrompt,
  type BrowserRoutingDecision,
  decideBrowserRouting,
} from "../shared/browser-preferences";
import { cachedBrowserPreferences } from "./browser-preferences-cache";
import { candidateFromDownloadLink } from "./download-link-detector";
import { createJobFromCandidate } from "./job-request";
import { isAccepted, sendRuntimeMessage } from "./runtime-messaging";

/**
 * Routes clicks on download links to LDM. The decision is made synchronously from the cached
 * browser preferences so the browser's own download is only blocked when LDM takes the link;
 * if LDM then rejects it, the click is replayed as a plain browser download.
 */

const browserFallbackAnchors = new WeakSet<HTMLAnchorElement>();
const activeDownloadLinkAnchors = new WeakSet<HTMLAnchorElement>();
const recentDownloadLinkActivations = new Map<string, number>();
let downloadLinkPromptQueue = Promise.resolve();
const DOWNLOAD_LINK_DEDUPE_MS = 1500;

export function installDownloadLinkInterceptor(): void {
  // Capture on the window only: it runs before any page handler, and registering the same
  // handler on the document as well only doubled every event.
  window.addEventListener("pointerdown", handleDownloadLinkActivation, true);
  window.addEventListener("mousedown", handleDownloadLinkActivation, true);
  window.addEventListener("click", handleDownloadLinkActivation, true);
}

/**
 * Decides synchronously, from the cached preferences, whether a link click belongs to LDM.
 * Only then is the browser's default blocked. With no preferences known yet, or while
 * handling is off, the click is left alone; a download the browser starts anyway is still
 * picked up by the background routing.
 */
function handleDownloadLinkActivation(event: MouseEvent): void {
  if (!shouldInterceptDownloadClick(event)) {
    return;
  }

  const target = event.target;
  if (!(target instanceof Element)) {
    return;
  }

  const anchor = target.closest<HTMLAnchorElement>("a[href]");
  if (!anchor) {
    return;
  }
  if (browserFallbackAnchors.delete(anchor)) {
    return;
  }

  if (activeDownloadLinkAnchors.has(anchor)) {
    blockBrowserDownloadEvent(event);
    return;
  }

  const candidate = candidateFromDownloadLink(anchor, window.location.href);
  if (!candidate) {
    return;
  }
  const preferences = cachedBrowserPreferences();
  if (!preferences) {
    return;
  }
  const decision = decideBrowserRouting(
    {
      kind: candidate.kind,
      sourceUrl: candidate.url,
      pageUrl: candidate.pageUrl,
      suggestedFilename: candidate.suggestedFilename,
    },
    preferences,
  );
  if (decision.action === "browser") {
    return;
  }

  const activationKey = downloadLinkActivationKey(candidate);
  if (isRecentDownloadLinkActivation(activationKey)) {
    blockBrowserDownloadEvent(event);
    return;
  }

  activeDownloadLinkAnchors.add(anchor);
  rememberDownloadLinkActivation(activationKey);
  blockBrowserDownloadEvent(event);

  void requestDownloadLink(candidate, anchor, decision).finally(() => {
    activeDownloadLinkAnchors.delete(anchor);
  });
}

function downloadLinkActivationKey(candidate: DetectedDownloadCandidate): string {
  return `${candidate.pageUrl ?? window.location.href}\n${candidate.url}`;
}

function isRecentDownloadLinkActivation(key: string): boolean {
  pruneRecentDownloadLinkActivations();
  const lastActivatedAt = recentDownloadLinkActivations.get(key);
  return lastActivatedAt !== undefined && Date.now() - lastActivatedAt < DOWNLOAD_LINK_DEDUPE_MS;
}

function rememberDownloadLinkActivation(key: string): void {
  pruneRecentDownloadLinkActivations();
  recentDownloadLinkActivations.set(key, Date.now());
}

function pruneRecentDownloadLinkActivations(): void {
  const now = Date.now();
  for (const [key, activatedAt] of recentDownloadLinkActivations) {
    if (now - activatedAt > DOWNLOAD_LINK_DEDUPE_MS) {
      recentDownloadLinkActivations.delete(key);
    }
  }
}

function blockBrowserDownloadEvent(event: MouseEvent): void {
  event.preventDefault();
  event.stopPropagation();
  event.stopImmediatePropagation();
}

function shouldInterceptDownloadClick(event: MouseEvent): boolean {
  return (
    event.button === 0 &&
    !event.defaultPrevented &&
    !event.altKey &&
    !event.ctrlKey &&
    !event.metaKey &&
    !event.shiftKey
  );
}

async function requestDownloadLink(
  candidate: DetectedDownloadCandidate,
  anchor: HTMLAnchorElement,
  decision: Exclude<BrowserRoutingDecision, { action: "browser" }>,
): Promise<void> {
  let job: CreateDownloadJobRequest | undefined;
  try {
    if (decision.action === "ask") {
      const confirmed = await confirmDownloadLinkRouting(decision.prompt);
      if (!confirmed) {
        openBrowserDownloadFallback(candidate.url, anchor);
        return;
      }
    }

    job = await createJobFromCandidate(candidate);
    const response = await sendRuntimeMessage({ type: "ldm:createJob", job });
    if (!isAccepted(response)) {
      openBrowserDownloadFallback(candidate.url, anchor, job);
    }
  } catch {
    openBrowserDownloadFallback(candidate.url, anchor, job);
  }
}

function confirmDownloadLinkRouting(prompt: BrowserDownloadPrompt): Promise<boolean> {
  const result = downloadLinkPromptQueue.then(() => showDownloadLinkPrompt(prompt));
  downloadLinkPromptQueue = result.then(
    () => undefined,
    () => undefined,
  );
  return result;
}

function showDownloadLinkPrompt(prompt: BrowserDownloadPrompt): Promise<boolean> {
  return new Promise((resolve) => {
    const host = document.createElement("div");
    host.setAttribute("data-ldm-download-prompt", "");
    const shadow = host.attachShadow({ mode: "closed" });
    const style = document.createElement("style");
    style.textContent = `
      :host { all: initial; }
      .prompt {
        position: fixed;
        right: 18px;
        bottom: 18px;
        z-index: 2147483647;
        box-sizing: border-box;
        width: min(360px, calc(100vw - 36px));
        padding: 14px;
        border: 1px solid #334155;
        border-radius: 8px;
        background: #0b1525;
        color: #f8fafc;
        box-shadow: 0 14px 38px rgba(0, 0, 0, .34);
        font: 13px/1.42 system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
      }
      .title { margin: 0 0 4px; font-size: 14px; font-weight: 700; }
      .filename {
        overflow: hidden;
        margin: 0;
        color: #cbd5e1;
        text-overflow: ellipsis;
        white-space: nowrap;
      }
      .warning { margin: 8px 0 0; color: #fde68a; }
      .actions { display: flex; justify-content: flex-end; gap: 8px; margin-top: 12px; }
      button {
        box-sizing: border-box;
        min-height: 32px;
        padding: 0 11px;
        border: 1px solid #475569;
        border-radius: 5px;
        background: #172033;
        color: #f8fafc;
        cursor: pointer;
        font: 600 12px/1 system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
      }
      button:hover { background: #26344d; }
      .primary { border-color: #23d160; background: #15803d; }
      .primary:hover { background: #16a34a; }
    `;
    const card = document.createElement("div");
    card.className = "prompt";
    card.setAttribute("role", "dialog");
    card.setAttribute("aria-modal", "true");
    card.setAttribute("aria-label", "Use LDM for this download?");

    const title = document.createElement("p");
    title.className = "title";
    title.textContent = "Use LDM for this download?";
    const filename = document.createElement("p");
    filename.className = "filename";
    filename.title = prompt.filename;
    filename.textContent = prompt.filename;
    card.append(title, filename);

    if (prompt.reason === "executable" || prompt.reason === "ask-executable") {
      const warning = document.createElement("p");
      warning.className = "warning";
      warning.textContent = "Executable files can be unsafe. LDM saves the file but will not run it automatically.";
      card.append(warning);
    }

    const actions = document.createElement("div");
    actions.className = "actions";
    const browserButton = document.createElement("button");
    browserButton.type = "button";
    browserButton.textContent = "Keep browser download";
    const ldmButton = document.createElement("button");
    ldmButton.type = "button";
    ldmButton.className = "primary";
    ldmButton.textContent = "Use LDM";
    actions.append(browserButton, ldmButton);
    card.append(actions);
    shadow.append(style, card);

    const settle = (accepted: boolean) => {
      document.removeEventListener("keydown", onKeyDown, true);
      host.remove();
      resolve(accepted);
    };
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        settle(false);
      }
    };
    browserButton.addEventListener("click", () => settle(false), { once: true });
    ldmButton.addEventListener("click", () => settle(true), { once: true });
    document.addEventListener("keydown", onKeyDown, true);
    (document.documentElement ?? document.body).append(host);
    ldmButton.focus();
  });
}

function openBrowserDownloadFallback(url: string, anchor?: HTMLAnchorElement, job?: CreateDownloadJobRequest): void {
  const urls = uniqueUrls([url, job?.sourceUrl, ...(job?.sourceFallbackUrls ?? [])]);
  void Promise.all(
    urls.map((fallbackUrl) => sendRuntimeMessage({ type: "ldm:allowBrowserDownloadFallback", url: fallbackUrl })),
  ).finally(() => {
    if (anchor?.isConnected) {
      browserFallbackAnchors.add(anchor);
      anchor.click();
      return;
    }
    window.location.href = url;
  });
}

function uniqueUrls(urls: Array<string | undefined>): string[] {
  return Array.from(new Set(urls.filter((value): value is string => Boolean(value))));
}
