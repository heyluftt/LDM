import type { DownloadKind } from "@ldm/contracts";
import type { ToolStatus } from "../../shared/api/tools";
import type { BrowserDownloadMode } from "../../shared/browserIntegrationPreferences";

export const downloadKinds: Array<{ kind: DownloadKind; label: string }> = [
  { kind: "video", label: "Video" },
  { kind: "audio", label: "Audio" },
  { kind: "image", label: "Images" },
  { kind: "program", label: "Programs" },
  { kind: "archive", label: "Archives" },
  { kind: "document", label: "Documents" },
  { kind: "other", label: "Other" },
];

export const browserModes: Array<{ id: BrowserDownloadMode; label: string }> = [
  { id: "off", label: "Off" },
  { id: "ask", label: "Ask first" },
  { id: "auto", label: "Auto for selected file types" },
];

export function normalizeDomainRuleHost(value: string): string {
  const trimmed = value.trim().toLowerCase();
  if (!trimmed) {
    return "";
  }

  let host = trimmed;
  try {
    host = /^[a-z][a-z0-9+.-]*:\/\//u.test(trimmed) ? new URL(trimmed).hostname : trimmed.split(/[/?#]/u)[0];
  } catch {
    host = "";
  }

  return host.replace(/^\*\./u, "").replace(/^\.+|\.+$/gu, "");
}

export function toolSourceLabel(source: ToolStatus["source"]): string {
  if (source === "bundled") {
    return "Bundled";
  }
  if (source === "env") {
    return "Env";
  }
  return "PATH";
}

export function domainActionLabel(action: BrowserDownloadMode): string {
  if (action === "auto") {
    return "Always use LDM";
  }
  if (action === "off") {
    return "Ignore";
  }
  return "Ask";
}

export function formatAcceptedAt(value: string | undefined): string {
  const seconds = Number.parseInt(value ?? "", 10);
  if (!Number.isFinite(seconds) || seconds <= 0) {
    return "Not accepted";
  }
  return new Intl.DateTimeFormat(undefined, {
    dateStyle: "medium",
    timeStyle: "short",
  }).format(new Date(seconds * 1000));
}
