import type { DownloadJob } from "@ldm/contracts";
import {
  displayProgressBytes,
  displaySpeedBytesPerSecond,
  displayTotalBytes,
  estimateEtaSeconds,
  formatPercent,
  progressDetailLine,
  progressPercent,
  progressStatusLine,
} from "./queueProgressModel";

export function QueueProgress({ job }: { job: DownloadJob }) {
  const total = displayTotalBytes(job) ?? 0;
  const percent = progressPercent(job, total);
  const progressBytes = displayProgressBytes(job);
  const displayPercent = percent === null ? 0 : Math.max(0, Math.min(100, percent));
  const displayPercentText = percent === null ? null : `${formatPercent(percent)}`;
  const currentSpeed = displaySpeedBytesPerSecond(job);
  const etaSeconds = job.status === "running" ? (job.etaSeconds ?? estimateEtaSeconds(job, currentSpeed, total)) : null;
  const hasProgress = progressBytes !== null || percent !== null;
  const hasFill = percent !== null && displayPercent > 0;
  const statusLine = progressStatusLine(job, total, displayPercentText, progressBytes);
  const detailLine = progressDetailLine(currentSpeed, etaSeconds);

  return (
    <div className="progress-cell">
      <div
        className={`progress-bar ${hasFill ? "filled" : hasProgress ? "unknown" : "empty"}`}
        aria-label={statusLine}
        aria-valuemax={100}
        aria-valuemin={0}
        aria-valuenow={Math.round(displayPercent)}
        role="progressbar"
      >
        <span style={{ width: `${displayPercent}%` }} />
      </div>
      <span className="progress-meta">
        <span className="progress-main">{statusLine}</span>
        {detailLine ? <span className="progress-sub">{detailLine}</span> : null}
      </span>
    </div>
  );
}
