import { useState } from "react";
import { openReleasePage } from "../../shared/api/updates";
import { shouldAnnounceUpdate } from "../../shared/updateModel";
import { useUpdateStore } from "../../shared/updateStore";

/**
 * Says that a newer LDM has been released. LDM does not install it: the button opens the
 * release page, where the installer and the extension are, and "Later" keeps the notice away
 * until the version after this one.
 */
export function UpdateBanner() {
  const status = useUpdateStore((state) => state.status);
  const dismissedVersion = useUpdateStore((state) => state.dismissedVersion);
  const dismiss = useUpdateStore((state) => state.dismiss);
  const [error, setError] = useState<string | null>(null);

  if (!status || !shouldAnnounceUpdate(status, dismissedVersion)) {
    return null;
  }

  return (
    <div className="update-banner" role="status">
      <div className="update-banner-copy">
        <strong>LDM {status.latestVersion} is available</strong>
        <span>
          You have {status.currentVersion}. The release page has the new installer and the extension; installing over
          this version keeps your settings and history.
        </span>
        {error ? <small className="status-line action-error">{error}</small> : null}
      </div>
      <div className="update-banner-actions">
        <button
          className="primary-action"
          type="button"
          onClick={() => {
            setError(null);
            void openReleasePage().catch((reason: unknown) => {
              setError(reason instanceof Error ? reason.message : "Could not open the release page.");
            });
          }}
        >
          Open release page
        </button>
        <button className="secondary-action" type="button" onClick={() => dismiss(status.latestVersion)}>
          Later
        </button>
      </div>
    </div>
  );
}
