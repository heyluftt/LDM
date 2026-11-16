import type { DownloadJob } from "@ldm/contracts";
import { Trash2 } from "lucide-react";
import { useMemo, useState } from "react";
import { clearHistory, removeHistoryJob, retryJob } from "../../shared/api/downloads";
import { useJobsStore } from "../../shared/jobsStore";
import { HistoryActions } from "./HistoryActions";
import { actionErrorMessage, formatTimestamp, historySummary, statusLabel, terminalStatuses } from "./historyJobMeta";

export function HistoryView() {
  const allJobs = useJobsStore((state) => state.jobs);
  const [busyJobId, setBusyJobId] = useState<string | null>(null);
  const [copiedJobId, setCopiedJobId] = useState<string | null>(null);
  const [actionErrors, setActionErrors] = useState<Record<string, string>>({});
  const sortedJobs = useMemo(
    () =>
      allJobs.filter((job) => terminalStatuses.has(job.status)).sort((left, right) => right.updatedAt - left.updatedAt),
    [allJobs],
  );

  return (
    <section className="panel compact-panel history-panel">
      <div className="panel-header">
        <div>
          <h2>History</h2>
          <p className="panel-subtitle">{historySummary(sortedJobs)}</p>
        </div>
        <button
          className="secondary-action"
          type="button"
          disabled={sortedJobs.length === 0 || busyJobId !== null}
          onClick={() => {
            void clearEntries(sortedJobs, setBusyJobId);
          }}
        >
          <Trash2 size={15} />
          <span>Clear</span>
        </button>
      </div>

      <div className="compact-list history-table">
        {sortedJobs.length === 0 ? <div className="empty-state">No history yet</div> : null}
        {sortedJobs.map((job) => (
          <HistoryRow
            actionError={actionErrors[job.id]}
            busy={busyJobId === job.id}
            copied={copiedJobId === job.id}
            job={job}
            key={job.id}
            onCopy={(jobId) => {
              setCopiedJobId(jobId);
              window.setTimeout(() => setCopiedJobId(null), 1400);
            }}
            onRemove={(nextJob) =>
              void removeEntry({
                job: nextJob,
                setActionErrors,
                setBusyJobId,
              })
            }
            onRetry={(nextJob) =>
              void retryEntry({
                job: nextJob,
                setActionErrors,
                setBusyJobId,
              })
            }
          />
        ))}
      </div>
    </section>
  );
}

function HistoryRow({
  actionError,
  busy,
  copied,
  job,
  onCopy,
  onRemove,
  onRetry,
}: {
  actionError?: string;
  busy: boolean;
  copied: boolean;
  job: DownloadJob;
  onCopy: (jobId: string) => void;
  onRemove: (job: DownloadJob) => void;
  onRetry: (job: DownloadJob) => void;
}) {
  return (
    <article className={`compact-download-card history-row ${job.status}`}>
      <div className="compact-download-main">
        <div className="compact-download-title-row">
          <span className="truncate queue-file-name" title={job.filename}>
            {job.filename}
          </span>
          <span className={`status-badge ${job.status}`} title={job.error ?? job.status}>
            {statusLabel(job.status)}
          </span>
        </div>
        <span className="queue-file-meta">
          <span>{job.kind}</span>
          <span>{formatTimestamp(job.updatedAt)}</span>
          <span className="truncate" title={job.targetDirectory}>
            {job.targetDirectory}
          </span>
        </span>
        {job.error ? <small className="status-line error">{job.error}</small> : null}
        {actionError ? <small className="status-line action-error">{actionError}</small> : null}
      </div>
      <HistoryActions busy={busy} copied={copied} job={job} onCopy={onCopy} onRemove={onRemove} onRetry={onRetry} />
    </article>
  );
}

async function removeEntry({ job, setActionErrors, setBusyJobId }: HistoryActionContext) {
  setBusyJobId(job.id);
  clearActionError(job.id, setActionErrors);

  try {
    await removeHistoryJob(job.id);
    useJobsStore.getState().removeJobs([job.id]);
  } catch (error) {
    setActionErrors((currentErrors) => ({
      ...currentErrors,
      [job.id]: actionErrorMessage(error),
    }));
  } finally {
    setBusyJobId(null);
  }
}

async function retryEntry({ job, setActionErrors, setBusyJobId }: HistoryActionContext) {
  setBusyJobId(job.id);
  clearActionError(job.id, setActionErrors);

  try {
    const updatedJob = await retryJob(job.id);
    useJobsStore.getState().upsertJob(updatedJob);
  } catch (error) {
    setActionErrors((currentErrors) => ({
      ...currentErrors,
      [job.id]: actionErrorMessage(error),
    }));
  } finally {
    setBusyJobId(null);
  }
}

async function clearEntries(
  historyJobs: DownloadJob[],
  setBusyJobId: React.Dispatch<React.SetStateAction<string | null>>,
) {
  setBusyJobId("clear-history");
  try {
    await clearHistory();
    useJobsStore.getState().removeJobs(historyJobs.map((job) => job.id));
  } finally {
    setBusyJobId(null);
  }
}

function clearActionError(
  jobId: string,
  setActionErrors: React.Dispatch<React.SetStateAction<Record<string, string>>>,
) {
  setActionErrors((currentErrors) => {
    const { [jobId]: _removed, ...nextErrors } = currentErrors;
    return nextErrors;
  });
}

type HistoryActionContext = {
  job: DownloadJob;
  setActionErrors: React.Dispatch<React.SetStateAction<Record<string, string>>>;
  setBusyJobId: React.Dispatch<React.SetStateAction<string | null>>;
};
