export type CompletionPreferences = {
  enabled: boolean;
  showOpenFile: boolean;
  showShowInFolder: boolean;
};

const STORAGE_KEY = "ldm.completionPreferences";
const CHANGE_EVENT = "ldm:completion-preferences-changed";

const DEFAULT_PREFERENCES: CompletionPreferences = {
  enabled: true,
  showOpenFile: true,
  showShowInFolder: true,
};

export function getCompletionPreferences(): CompletionPreferences {
  try {
    const parsed = JSON.parse(window.localStorage.getItem(STORAGE_KEY) ?? "{}") as Partial<CompletionPreferences>;
    return {
      enabled: typeof parsed.enabled === "boolean" ? parsed.enabled : DEFAULT_PREFERENCES.enabled,
      showOpenFile: typeof parsed.showOpenFile === "boolean" ? parsed.showOpenFile : DEFAULT_PREFERENCES.showOpenFile,
      showShowInFolder:
        typeof parsed.showShowInFolder === "boolean" ? parsed.showShowInFolder : DEFAULT_PREFERENCES.showShowInFolder,
    };
  } catch {
    return DEFAULT_PREFERENCES;
  }
}

export function setCompletionPreferences(preferences: CompletionPreferences): void {
  window.localStorage.setItem(STORAGE_KEY, JSON.stringify(preferences));
  window.dispatchEvent(new CustomEvent(CHANGE_EVENT));
}

export function subscribeToCompletionPreferences(callback: () => void): () => void {
  window.addEventListener(CHANGE_EVENT, callback);
  return () => window.removeEventListener(CHANGE_EVENT, callback);
}
