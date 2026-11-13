export type AppTheme = "modern" | "classic";
export type AppColorMode = "light" | "dark";

const THEME_STORAGE_KEY = "ldm.appearance.theme";
const COLOR_MODE_STORAGE_KEY = "ldm.appearance.colorMode";
const THEME_CHANGE_EVENT = "ldm-appearance-theme-change";
const COLOR_MODE_CHANGE_EVENT = "ldm-appearance-color-mode-change";
const DEFAULT_THEME: AppTheme = "modern";
const DEFAULT_COLOR_MODE: AppColorMode = "light";

export function getAppTheme(): AppTheme {
  const stored = window.localStorage.getItem(THEME_STORAGE_KEY);
  return isAppTheme(stored) ? stored : DEFAULT_THEME;
}

export function getAppColorMode(): AppColorMode {
  const stored = window.localStorage.getItem(COLOR_MODE_STORAGE_KEY);
  return isAppColorMode(stored) ? stored : DEFAULT_COLOR_MODE;
}

export function setAppTheme(theme: AppTheme): void {
  window.localStorage.setItem(THEME_STORAGE_KEY, theme);
  applyAppTheme(theme);
  window.dispatchEvent(new CustomEvent(THEME_CHANGE_EVENT, { detail: theme }));
}

export function setAppColorMode(colorMode: AppColorMode): void {
  window.localStorage.setItem(COLOR_MODE_STORAGE_KEY, colorMode);
  applyAppColorMode(colorMode);
  window.dispatchEvent(new CustomEvent(COLOR_MODE_CHANGE_EVENT, { detail: colorMode }));
}

export function applyAppTheme(theme = getAppTheme(), colorMode = getAppColorMode()): void {
  document.documentElement.dataset.theme = theme;
  applyAppColorMode(colorMode);
}

export function applyAppColorMode(colorMode = getAppColorMode()): void {
  document.documentElement.dataset.colorMode = colorMode;
}

export function subscribeToAppTheme(listener: (theme: AppTheme) => void): () => void {
  const onThemeChange = (event: Event) => {
    if (event instanceof CustomEvent && isAppTheme(event.detail)) {
      listener(event.detail);
    }
  };

  window.addEventListener(THEME_CHANGE_EVENT, onThemeChange);
  return () => window.removeEventListener(THEME_CHANGE_EVENT, onThemeChange);
}

export function subscribeToAppColorMode(listener: (colorMode: AppColorMode) => void): () => void {
  const onColorModeChange = (event: Event) => {
    if (event instanceof CustomEvent && isAppColorMode(event.detail)) {
      listener(event.detail);
    }
  };

  window.addEventListener(COLOR_MODE_CHANGE_EVENT, onColorModeChange);
  return () => window.removeEventListener(COLOR_MODE_CHANGE_EVENT, onColorModeChange);
}

function isAppTheme(value: unknown): value is AppTheme {
  return value === "modern" || value === "classic";
}

function isAppColorMode(value: unknown): value is AppColorMode {
  return value === "light" || value === "dark";
}
