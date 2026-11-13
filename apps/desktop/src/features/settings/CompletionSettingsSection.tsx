import type { CompletionPreferences } from "../../shared/completionPreferences";

export function CompletionSettingsSection({
  onChange,
  preferences,
}: {
  onChange: (preferences: CompletionPreferences) => void;
  preferences: CompletionPreferences;
}) {
  return (
    <section className="panel">
      <div className="panel-header">
        <div>
          <h2>Completion Window</h2>
        </div>
      </div>
      <div className="settings-list">
        <label className="setting-toggle">
          <input
            type="checkbox"
            checked={preferences.enabled}
            onChange={(event) =>
              onChange({
                ...preferences,
                enabled: event.currentTarget.checked,
              })
            }
          />
          <span>Show a small window when a download completes</span>
        </label>
        <label className="setting-toggle">
          <input
            type="checkbox"
            checked={preferences.showOpenFile}
            disabled={!preferences.enabled}
            onChange={(event) =>
              onChange({
                ...preferences,
                showOpenFile: event.currentTarget.checked,
              })
            }
          />
          <span>Show open file action</span>
        </label>
        <label className="setting-toggle">
          <input
            type="checkbox"
            checked={preferences.showShowInFolder}
            disabled={!preferences.enabled}
            onChange={(event) =>
              onChange({
                ...preferences,
                showShowInFolder: event.currentTarget.checked,
              })
            }
          />
          <span>Show open folder action</span>
        </label>
      </div>
    </section>
  );
}
