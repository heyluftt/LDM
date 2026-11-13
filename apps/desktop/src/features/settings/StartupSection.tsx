import type { StartupStatus } from "../../shared/api/startup";

export function StartupSection({
  error,
  onToggle,
  status,
}: {
  error: string | null;
  onToggle: (enabled: boolean) => void;
  status: StartupStatus | null;
}) {
  const unavailable = status?.available === false;

  return (
    <section className="panel">
      <div className="panel-header">
        <div>
          <h2>Startup</h2>
        </div>
      </div>
      <div className="settings-list">
        <label className="setting-toggle startup-toggle">
          <input
            type="checkbox"
            checked={status?.enabled === true}
            disabled={!status || unavailable}
            onChange={(event) => onToggle(event.currentTarget.checked)}
          />
          <span>
            <strong>Start LDM with Windows</strong>
            <small>Keep LDM ready in the tray for browser downloads.</small>
          </span>
        </label>
        {unavailable && status?.reason ? <div className="settings-note">{status.reason}</div> : null}
        {error ? <div className="legal-error">{error}</div> : null}
      </div>
    </section>
  );
}
