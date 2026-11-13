import { Minus, Plus } from "lucide-react";
import { type KeyboardEvent, useEffect, useState } from "react";
import type { ConcurrencySettings } from "../../shared/api/settings";

export function ParallelDownloadsSection({
  error,
  onChange,
  settings,
}: {
  error: string | null;
  onChange: (value: number) => void;
  settings: ConcurrencySettings | null;
}) {
  const saved = settings?.maxConcurrentDownloads ?? null;
  const [draft, setDraft] = useState(saved === null ? "" : String(saved));

  useEffect(() => {
    setDraft(saved === null ? "" : String(saved));
  }, [saved]);

  /** Stores `value`, held to the allowed range; anything that is not a number restores the saved one. */
  const commit = (value: number) => {
    if (!settings) {
      return;
    }
    if (!Number.isFinite(value)) {
      setDraft(String(settings.maxConcurrentDownloads));
      return;
    }
    const clamped = Math.min(settings.max, Math.max(settings.min, value));
    setDraft(String(clamped));
    if (clamped !== settings.maxConcurrentDownloads) {
      onChange(clamped);
    }
  };

  const typed = Number.parseInt(draft, 10);
  const current = Number.isFinite(typed) ? typed : (saved ?? 0);
  const step = (delta: number) => commit(current + delta);

  const onKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === "Enter") {
      event.currentTarget.blur();
    }
    if (event.key === "Escape" && settings) {
      setDraft(String(settings.maxConcurrentDownloads));
      event.currentTarget.blur();
    }
    if (event.key === "ArrowUp" || event.key === "ArrowDown") {
      event.preventDefault();
      step(event.key === "ArrowUp" ? 1 : -1);
    }
  };

  return (
    <section className="panel">
      <div className="panel-header">
        <div>
          <h2>Parallel downloads</h2>
          <p className="panel-subtitle">Downloads beyond this limit wait in the queue until a slot is free.</p>
        </div>
      </div>
      <div className="settings-list">
        <div className="setting-field">
          <span>
            <strong>Downloads at the same time</strong>
            <small>{settings ? `${settings.min} to ${settings.max}` : "Unavailable"}</small>
          </span>
          {/* A text field between two buttons instead of a number field: the arrows a number
              field brings along are drawn by Windows and ignore the app's themes. */}
          <fieldset className="stepper" disabled={!settings}>
            <legend className="visually-hidden">Downloads at the same time</legend>
            <button
              aria-label="One fewer"
              className="secondary-action stepper-button"
              disabled={!settings || current <= settings.min}
              onClick={() => step(-1)}
              type="button"
            >
              <Minus size={13} />
            </button>
            <input
              aria-label="Downloads at the same time"
              autoComplete="off"
              inputMode="numeric"
              onBlur={() => commit(Number.parseInt(draft, 10))}
              onChange={(event) => setDraft(event.currentTarget.value.replace(/\D/gu, ""))}
              onKeyDown={onKeyDown}
              type="text"
              value={draft}
            />
            <button
              aria-label="One more"
              className="secondary-action stepper-button"
              disabled={!settings || current >= settings.max}
              onClick={() => step(1)}
              type="button"
            >
              <Plus size={13} />
            </button>
          </fieldset>
        </div>
        {error ? <div className="legal-error">{error}</div> : null}
      </div>
    </section>
  );
}
