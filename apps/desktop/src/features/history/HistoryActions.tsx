import type { DownloadJob } from "@ldm/contracts";
import { Clipboard, RotateCcw, Trash2 } from "lucide-react";
import { canRetry } from "./historyJobMeta";
import { buildHistoryReport } from "./historyReport";

export function HistoryActions({
  busy,
  copied,
  job,
  onCopy,
  onRemove,
  onRetry,
}: {
  busy: boolean;
  copied: boolean;
  job: DownloadJob;
  onCopy: (jobId: string) => void;
  onRemove: (job: DownloadJob) => void;
  onRetry: (job: DownloadJob) => void;
}) {
  return (
    <span className="job-actions" role="toolbar" aria-label={`History actions for ${job.filename}`}>
      <button
        type="button"
        title={copied ? "Report copied" : "Copy report"}
        disabled={busy}
        onClick={() => {
          void navigator.clipboard.writeText(buildHistoryReport(job));
          onCopy(job.id);
        }}
      >
        <Clipboard size={14} />
      </button>
      <button type="button" title="Retry" disabled={busy || !canRetry(job)} onClick={() => onRetry(job)}>
        <RotateCcw size={14} />
      </button>
      <button type="button" title="Remove from history" disabled={busy} onClick={() => onRemove(job)}>
        <Trash2 size={14} />
      </button>
    </span>
  );
}
