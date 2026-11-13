import { getVersion } from "@tauri-apps/api/app";
import { useEffect, useState } from "react";
import { openReleasePage } from "../../shared/api/updates";
import { updateSummary } from "../../shared/updateModel";
import { checkForUpdateNow, useUpdateStore } from "../../shared/updateStore";

/** The running version, whether a newer one exists, and whether LDM looks for one on its own. */
export function UpdatesSection() {
  const status = useUpdateStore((state) => state.status);
  const checking = useUpdateStore((state) => state.checking);
  const error = useUpdateStore((state) => state.error);
  const checkAutomatically = useUpdateStore((state) => state.checkAutomatically);
  const setCheckAutomatically = useUpdateStore((state) => state.setCheckAutomatically);
  const [installedVersion, setInstalledVersion] = useState<string | null>(null);
  const [openError, setOpenError] = useState<string | null>(null);

  useEffect(() => {
    let active = true;
    // Known without asking anyone, so it shows even when checks are switched off.
    void getVersion()
      .then((version) => {
        if (active) {
          setInstalledVersion(version);
        }
      })
      .catch(() => undefined);
    return () => {
      active = false;
    };
  }, []);

  const version = status?.currentVersion ?? installedVersion;

  return (
    <section className="panel">
      <div className="panel-header">
        <div>
          <h2>Updates</h2>
          <p className="panel-subtitle">LDM tells you about a new version; you install it from the release page.</p>
        </div>
      </div>
      <div className="settings-list">
        <div className="setting-field">
          <span>
            <strong>{version ? `LDM ${version}` : "LDM"}</strong>
            <small>{updateSummary(status, checking, error)}</small>
          </span>
          <div className="setting-actions">
            {status?.updateAvailable ? (
              <button
                className="primary-action"
                type="button"
                onClick={() => {
                  setOpenError(null);
                  void openReleasePage().catch((reason: unknown) => {
                    setOpenError(reason instanceof Error ? reason.message : "Could not open the release page.");
                  });
                }}
              >
                Open release page
              </button>
            ) : null}
            <button
              className="secondary-action"
              disabled={checking}
              type="button"
              onClick={() => void checkForUpdateNow()}
            >
              Check now
            </button>
          </div>
        </div>
        {openError ? <div className="legal-error">{openError}</div> : null}
        <label className="setting-toggle startup-toggle">
          <input
            type="checkbox"
            checked={checkAutomatically}
            onChange={(event) => setCheckAutomatically(event.currentTarget.checked)}
          />
          <span>
            <strong>Check for updates automatically</strong>
            <small>Once at start and once a day, LDM asks GitHub which version is the latest.</small>
          </span>
        </label>
      </div>
    </section>
  );
}
