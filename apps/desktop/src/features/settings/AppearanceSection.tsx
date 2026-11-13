import type { AppColorMode, AppTheme } from "../../shared/appearancePreferences";

export function AppearanceSection({
  colorMode,
  onColorModeChange,
  onThemeChange,
  theme,
}: {
  colorMode: AppColorMode;
  onColorModeChange: (colorMode: AppColorMode) => void;
  onThemeChange: (theme: AppTheme) => void;
  theme: AppTheme;
}) {
  return (
    <section className="panel">
      <div className="panel-header">
        <div>
          <h2>Appearance</h2>
        </div>
      </div>
      <div className="settings-list">
        {/* biome-ignore lint/a11y/useSemanticElements: a fieldset would change the control's layout; role=group carries the same semantics */}
        <div className="theme-switch" role="group" aria-label="Theme">
          <button
            className={theme === "modern" ? "theme-option active" : "theme-option"}
            type="button"
            aria-pressed={theme === "modern"}
            onClick={() => onThemeChange("modern")}
          >
            <strong>Modern</strong>
            <span>Clean utility UI</span>
          </button>
          <button
            className={theme === "classic" ? "theme-option active" : "theme-option"}
            type="button"
            aria-pressed={theme === "classic"}
            onClick={() => onThemeChange("classic")}
          >
            <strong>Classic UI</strong>
            <span>Win32-style desktop feel</span>
          </button>
        </div>
        <div className="color-mode-row">
          <div>
            <strong>Dark mode</strong>
            <span>Use darker surfaces for the selected theme</span>
          </div>
          <button
            className={colorMode === "dark" ? "switch-toggle active" : "switch-toggle"}
            type="button"
            aria-pressed={colorMode === "dark"}
            onClick={() => onColorModeChange(colorMode === "dark" ? "light" : "dark")}
          >
            {colorMode === "dark" ? "On" : "Off"}
          </button>
        </div>
      </div>
    </section>
  );
}
