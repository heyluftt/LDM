import { type KeyboardEvent, useEffect, useState } from "react";
import {
  type BrowserDownloadMode,
  type BrowserIntegrationPreferences,
  type NativeHostRegistration,
  normalizeExtensionId,
} from "../../shared/browserIntegrationPreferences";
import { browserModes, domainActionLabel, downloadKinds } from "./settingsData";

export function BrowserIntegrationSection({
  domainAction,
  domainHost,
  error,
  nativeHostRegistration,
  onAddDomainRule,
  onChange,
  onDomainActionChange,
  onDomainHostChange,
  preferences,
}: {
  domainAction: BrowserDownloadMode;
  domainHost: string;
  error: string | null;
  nativeHostRegistration: NativeHostRegistration | null;
  onAddDomainRule: () => void;
  onChange: (preferences: BrowserIntegrationPreferences) => void;
  onDomainActionChange: (action: BrowserDownloadMode) => void;
  onDomainHostChange: (host: string) => void;
  preferences: BrowserIntegrationPreferences;
}) {
  return (
    <section className="panel">
      <div className="panel-header">
        <div>
          <h2>Browser Integration</h2>
        </div>
      </div>
      <div className="settings-list">
        {/* biome-ignore lint/a11y/useSemanticElements: a fieldset would change the control's layout; role=group carries the same semantics */}
        <div className="segmented-control" role="group" aria-label="Browser download handling">
          {browserModes.map((mode) => (
            <button
              className={preferences.mode === mode.id ? "segment active" : "segment"}
              type="button"
              key={mode.id}
              aria-pressed={preferences.mode === mode.id}
              onClick={() => onChange({ ...preferences, mode: mode.id })}
            >
              {mode.label}
            </button>
          ))}
        </div>
        <div className="kind-grid">
          {downloadKinds.map((item) => (
            <label className="setting-toggle" key={item.kind}>
              <input
                type="checkbox"
                checked={preferences.selectedKinds[item.kind]}
                disabled={preferences.mode !== "auto"}
                onChange={(event) =>
                  onChange({
                    ...preferences,
                    selectedKinds: {
                      ...preferences.selectedKinds,
                      [item.kind]: event.currentTarget.checked,
                    },
                  })
                }
              />
              <span>{item.label}</span>
            </label>
          ))}
        </div>
        <label className="setting-toggle">
          <input
            type="checkbox"
            checked={preferences.warnOnExecutable}
            onChange={(event) =>
              onChange({
                ...preferences,
                warnOnExecutable: event.currentTarget.checked,
              })
            }
          />
          <span>Show a short warning for executable files</span>
        </label>
        <div className="domain-rule-box">
          <div className="domain-rule-form">
            <input
              type="text"
              placeholder="example.com"
              value={domainHost}
              onChange={(event) => onDomainHostChange(event.currentTarget.value)}
            />
            <select
              value={domainAction}
              onChange={(event) => onDomainActionChange(event.currentTarget.value as BrowserDownloadMode)}
            >
              <option value="auto">Always use LDM</option>
              <option value="ask">Ask</option>
              <option value="off">Ignore</option>
            </select>
            <button className="secondary-action" type="button" onClick={onAddDomainRule}>
              Add
            </button>
          </div>
          {preferences.domainRules.length > 0 ? (
            <div className="domain-rule-list">
              {preferences.domainRules.map((rule) => (
                <div className="domain-rule-row" key={rule.host}>
                  <strong>{rule.host}</strong>
                  <span>{domainActionLabel(rule.action)}</span>
                  <button
                    className="secondary-action"
                    type="button"
                    onClick={() =>
                      onChange({
                        ...preferences,
                        domainRules: preferences.domainRules.filter((item) => item.host !== rule.host),
                      })
                    }
                  >
                    Remove
                  </button>
                </div>
              ))}
            </div>
          ) : null}
        </div>
        <ExtensionIdField
          extensionId={preferences.extensionId ?? ""}
          registration={nativeHostRegistration}
          onCommit={(extensionId) => onChange({ ...preferences, extensionId })}
        />
        {error ? <div className="legal-error">{error}</div> : null}
      </div>
    </section>
  );
}

/**
 * One more extension id the native messaging host is registered for, next to the LDM extension's
 * own. Committed on Enter or blur so a half-typed id never triggers a registration; an invalid
 * id is shown as such and not saved.
 */
function ExtensionIdField({
  extensionId,
  onCommit,
  registration,
}: {
  extensionId: string;
  onCommit: (extensionId: string | undefined) => void;
  registration: NativeHostRegistration | null;
}) {
  const [draft, setDraft] = useState(extensionId);
  const [invalid, setInvalid] = useState(false);

  useEffect(() => {
    setDraft(extensionId);
    setInvalid(false);
  }, [extensionId]);

  const commit = () => {
    if (draft.trim() === "") {
      setInvalid(false);
      if (extensionId !== "") {
        onCommit(undefined);
      }
      return;
    }
    const normalized = normalizeExtensionId(draft);
    if (!normalized) {
      setInvalid(true);
      return;
    }
    setInvalid(false);
    setDraft(normalized);
    if (normalized !== extensionId) {
      onCommit(normalized);
    }
  };

  const onKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === "Enter") {
      event.currentTarget.blur();
    }
    if (event.key === "Escape") {
      setDraft(extensionId);
      setInvalid(false);
      event.currentTarget.blur();
    }
  };

  return (
    <div className="extension-id-box">
      <label className="extension-id-field">
        <span>
          <strong>Additional extension ID</strong>
          <small>
            LDM registers itself for the LDM extension in Chrome, Edge, Brave, and Brave Origin on its own. Enter an ID
            here only for an extension build that shows a different 32-letter ID on the browser's extensions page, such
            as a fork.
          </small>
        </span>
        <input
          aria-label="Additional extension ID"
          autoCapitalize="off"
          autoCorrect="off"
          onBlur={commit}
          onChange={(event) => setDraft(event.currentTarget.value)}
          onKeyDown={onKeyDown}
          placeholder="abcdefghijklmnopabcdefghijklmnop"
          spellCheck={false}
          type="text"
          value={draft}
        />
      </label>
      {invalid ? <div className="legal-error">An extension ID is 32 letters from a to p.</div> : null}
      <RegistrationStatus registration={registration} />
    </div>
  );
}

function RegistrationStatus({ registration }: { registration: NativeHostRegistration | null }) {
  if (!registration) {
    return null;
  }
  if (registration.error) {
    return <div className="legal-error">{registration.error}</div>;
  }
  if (!registration.registered) {
    return <div className="settings-note">Native host not registered yet.</div>;
  }
  return (
    <div className="settings-note" title={registration.manifestPath ?? undefined}>
      Native host registered for {registration.browsers.join(", ")}: {registration.hostPath}
    </div>
  );
}
