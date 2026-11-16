import type { BrowserIntegrationPreferences } from "@ldm/contracts";
import { BROWSER_PREFERENCES_STORAGE_KEY, normalizeBrowserIntegrationPreferences } from "../shared/browser-preferences";

let cached: BrowserIntegrationPreferences | undefined;

/** Content script copy of the preferences, read synchronously by click handlers; `undefined` until known. */
export function cachedBrowserPreferences(): BrowserIntegrationPreferences | undefined {
  return cached;
}

export function installBrowserPreferencesCache(requestFromBackground: () => Promise<unknown>): void {
  const apply = (value: unknown) => {
    if (value && typeof value === "object") {
      cached = normalizeBrowserIntegrationPreferences(value);
    }
  };

  try {
    chrome.storage?.session?.get(BROWSER_PREFERENCES_STORAGE_KEY, (items) => {
      if (!chrome.runtime.lastError) {
        apply(items?.[BROWSER_PREFERENCES_STORAGE_KEY]);
      }
    });
  } catch {
    // No session storage in this frame.
  }

  try {
    chrome.storage?.onChanged?.addListener((changes, areaName) => {
      if (areaName === "session" && changes[BROWSER_PREFERENCES_STORAGE_KEY]) {
        apply(changes[BROWSER_PREFERENCES_STORAGE_KEY].newValue);
      }
    });
  } catch {
    // Without change events, the next refresh updates it.
  }

  const refresh = () => {
    void requestFromBackground().then((response) => {
      if (isPreferencesResponse(response)) {
        apply(response.preferences);
      }
    });
  };

  refresh();
  document.addEventListener("visibilitychange", () => {
    if (document.visibilityState === "visible") {
      refresh();
    }
  });
}

function isPreferencesResponse(value: unknown): value is { type: "browserPreferences"; preferences: unknown } {
  return typeof value === "object" && value !== null && "type" in value && value.type === "browserPreferences";
}
