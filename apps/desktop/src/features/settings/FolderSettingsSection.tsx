import type { DownloadKind } from "@ldm/contracts";
import { FolderOpen, RotateCcw } from "lucide-react";
import { type KeyboardEvent, useEffect, useState } from "react";
import type { TargetFolder } from "../../shared/api/settings";
import { downloadKinds } from "./settingsData";

export function FolderSettingsSection({
  busyKind,
  error,
  folders,
  onEdit,
  onPick,
  onReset,
}: {
  busyKind: DownloadKind | null;
  error: string | null;
  folders: TargetFolder[];
  onEdit: (kind: DownloadKind, path: string) => void;
  onPick: (kind: DownloadKind) => void;
  onReset: (kind: DownloadKind) => void;
}) {
  const byKind = new Map(folders.map((folder) => [folder.kind, folder]));

  return (
    <section className="panel">
      <div className="panel-header">
        <div>
          <h2>Download folders</h2>
          <p className="panel-subtitle">Where each file type is saved. Type a path or pick a folder.</p>
        </div>
      </div>
      <div className="folder-grid">
        {downloadKinds.map((item) => {
          const folder = byKind.get(item.kind);
          return (
            <FolderRow
              busy={busyKind === item.kind}
              folder={folder}
              key={item.kind}
              kind={item.kind}
              label={item.label}
              onEdit={onEdit}
              onPick={onPick}
              onReset={onReset}
            />
          );
        })}
        {folders.length === 0 ? <div className="empty-state">Folder settings unavailable</div> : null}
      </div>
      {error ? <div className="legal-error">{error}</div> : null}
    </section>
  );
}

function FolderRow({
  busy,
  folder,
  kind,
  label,
  onEdit,
  onPick,
  onReset,
}: {
  busy: boolean;
  folder: TargetFolder | undefined;
  kind: DownloadKind;
  label: string;
  onEdit: (kind: DownloadKind, path: string) => void;
  onPick: (kind: DownloadKind) => void;
  onReset: (kind: DownloadKind) => void;
}) {
  const savedPath = folder?.path ?? "";
  const [draft, setDraft] = useState(savedPath);

  // Keep the field in sync when the backend reports a new folder (picker, reset, other window).
  useEffect(() => {
    setDraft(savedPath);
  }, [savedPath]);

  const commit = () => {
    const next = draft.trim();
    if (next && next !== savedPath) {
      onEdit(kind, next);
    } else if (!next) {
      setDraft(savedPath);
    }
  };

  const onKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === "Enter") {
      event.currentTarget.blur();
    }
    if (event.key === "Escape") {
      setDraft(savedPath);
      event.currentTarget.blur();
    }
  };

  return (
    <div className={folder?.custom ? "folder-row custom" : "folder-row"}>
      <span className="folder-label">{label}</span>
      <input
        aria-label={`${label} folder`}
        disabled={busy || !folder}
        onBlur={commit}
        onChange={(event) => setDraft(event.currentTarget.value)}
        onKeyDown={onKeyDown}
        spellCheck={false}
        title={folder?.custom ? `Custom folder. Default: ${folder.defaultPath}` : "Default folder"}
        type="text"
        value={draft}
      />
      <span className="folder-actions">
        <button
          className="secondary-action icon-action"
          disabled={busy || !folder}
          onClick={() => onPick(kind)}
          title="Choose folder"
          type="button"
        >
          <FolderOpen size={14} />
        </button>
        <button
          className="secondary-action icon-action"
          disabled={busy || !folder?.custom}
          onClick={() => onReset(kind)}
          title={folder ? `Reset to ${folder.defaultPath}` : "Reset to default"}
          type="button"
        >
          <RotateCcw size={14} />
        </button>
      </span>
    </div>
  );
}
