import type { BrowserDownloadMode, BrowserIntegrationPreferences } from "@ldm/contracts";
import { invoke } from "./api/invoke";

export type { BrowserDownloadMode, BrowserIntegrationPreferences };

export const defaultBrowserIntegrationPreferences: BrowserIntegrationPreferences = {
  mode: "auto",
  selectedKinds: {
    video: true,
    audio: true,
    program: true,
    document: true,
    archive: true,
    image: true,
    other: false,
  },
  domainRules: [],
  warnOnExecutable: true,
};

export function getBrowserIntegrationPreferences(): Promise<BrowserIntegrationPreferences> {
  return invoke<BrowserIntegrationPreferences>("browser_integration_preferences");
}

export function setBrowserIntegrationPreferences(
  preferences: BrowserIntegrationPreferences,
): Promise<BrowserIntegrationPreferences> {
  return invoke<BrowserIntegrationPreferences>("set_browser_integration_preferences", {
    preferences,
  });
}

export type NativeHostRegistration = {
  registered: boolean;
  extensionIds: string[];
  hostPath?: string | null;
  manifestPath?: string | null;
  browsers: string[];
  error?: string | null;
};

export function getNativeHostRegistration(): Promise<NativeHostRegistration> {
  return invoke<NativeHostRegistration>("native_host_registration");
}

/** Mirrors `normalize_extension_id` in ldm-types. */
export function normalizeExtensionId(value: string): string | null {
  const trimmed = value.trim().toLowerCase();
  const id = trimmed.replace(/^chrome-extension:\/\//u, "").replace(/\/+$/u, "");
  return /^[a-p]{32}$/u.test(id) ? id : null;
}
