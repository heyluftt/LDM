import { useEffect, useState } from "react";
import { acceptLegalTerms, getLegalStatus, type LegalStatus } from "../../shared/api/legal";
import { checkForUpdateIfAllowed } from "../../shared/updateStore";
import { quitApp } from "../../shared/windowControls";

const checks = [
  "I will only download content I own, have permission to download, or am legally allowed to save.",
  "I understand that some websites do not allow downloading in their terms.",
  "I will not use LDM to get around DRM, paywalls, login requirements, or access controls.",
  "I understand that LDM does not automatically read browser cookies, sign-in data, or account data.",
  "I accept the Terms of Use and Privacy Notice.",
];

export function FirstRunNotice() {
  const [status, setStatus] = useState<LegalStatus | null>(null);
  const [checked, setChecked] = useState<boolean[]>(() => checks.map(() => false));
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    let active = true;
    void getLegalStatus()
      .then((nextStatus) => {
        if (active) {
          setStatus(nextStatus);
        }
      })
      .catch(() => {
        if (active) {
          setError("First-run setup is unavailable.");
        }
      });

    return () => {
      active = false;
    };
  }, []);

  if (!status || status.accepted) {
    return null;
  }

  const allChecked = checked.every(Boolean);

  const accept = async () => {
    if (!allChecked || saving) {
      return;
    }

    setSaving(true);
    setError(null);
    try {
      setStatus(await acceptLegalTerms());
      // The check at start was skipped while the notice was open.
      void checkForUpdateIfAllowed();
    } catch {
      setError("Could not save first-run setup.");
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="legal-backdrop" role="presentation">
      <section className="legal-dialog" role="dialog" aria-modal="true" aria-labelledby="legal-title">
        <div className="legal-dialog-header">
          <h2 id="legal-title">Before using LDM</h2>
          <span>Version {status.legalTermsVersion}</span>
        </div>
        <p>LDM is a local-first download manager. Use it only for files and media you are allowed to download.</p>
        <div className="legal-checklist">
          {checks.map((label, index) => (
            <label className="legal-check" key={label}>
              <input
                type="checkbox"
                checked={checked[index]}
                onChange={(event) => {
                  const next = [...checked];
                  next[index] = event.currentTarget.checked;
                  setChecked(next);
                }}
              />
              <span>{label}</span>
            </label>
          ))}
        </div>
        {error ? <div className="legal-error">{error}</div> : null}
        <div className="legal-actions">
          <button className="secondary-action" type="button" onClick={() => void quitApp()}>
            Quit
          </button>
          <button
            className="primary-action"
            type="button"
            disabled={!allChecked || saving}
            onClick={() => void accept()}
          >
            Accept and continue
          </button>
        </div>
      </section>
    </div>
  );
}
