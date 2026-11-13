import type { DownloadJob } from "@ldm/contracts";
import { Clipboard } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { getStartupStatus, type StartupStatus, setStartupEnabled } from "../../shared/api/startup";
import { useJobsStore } from "../../shared/jobsStore";
import { GettingStarted } from "./GettingStarted";
import { QueueActions, type QueueJobAction } from "./QueueActions";
import { QueueMetrics, queueSummary } from "./QueueMetrics";
import { QueueProgress } from "./QueueProgress";
import { buildJobDiagnostics } from "./queueDiagnostics";
import { actionErrorMessage, controlNote, isQueueJob, kindLabel, sourceLabel, statusLabel } from "./queueJobMeta";
import { displayTotalBytes, formatBytes } from "./queueProgressModel";

export function QueueView() {
  const allJobs = useJobsStore((state) => state.jobs);
  const jobs = useMemo(() => allJobs.filter(isQueueJob), [allJobs]);
  const [startupStatus, setStartupStatus] = useState<StartupStatus | null>(null);
  const [startupPromptDismissed, setStartupPromptDismissed] = useState(hasDismissedStartupPrompt);
  const [startupPromptError, setStartupPromptError] = useState<string | null>(null);
  const [copiedReportId, setCopiedReportId] = useState<string | null>(null);
  const [busyJobId, setBusyJobId] = useState<string | null>(null);
  const [actionErrors, setActionErrors] = useState<Record<string, string>>({});

  useEffect(() => {
    let active = true;
    void getStartupStatus()
      .then((status) => {
        if (active) {
          setStartupStatus(status);
        }
      })
      .catch(() => {
        if (active) {
          setStartupStatus(null);
        }
      });
    return () => {
      active = false;
    };
  }, []);

  const showStartupPrompt =
    jobs.some((job) => job.status === "queued" || job.status === "running") &&
    startupStatus?.available === true &&
    startupStatus.enabled === false &&
    !startupPromptDismissed;

  return (
    <section className="panel compact-panel queue-panel">
      <div className="panel-header">
        <div>
          <h1>Downloads</h1>
          <p className="panel-subtitle">{queueSummary(jobs)}</p>
        </div>
        <QueueMetrics jobs={jobs} />
      </div>

      {showStartupPrompt ? (
        <StartupPrompt
          error={startupPromptError}
          onDismiss={() => {
            rememberStartupPromptDismissed();
            setStartupPromptDismissed(true);
          }}
          onEnable={() => {
            setStartupPromptError(null);
            void setStartupEnabled(true)
              .then((status) => {
                setStartupStatus(status);
                rememberStartupPromptDismissed();
                setStartupPromptDismissed(true);
              })
              .catch((error: unknown) => {
                setStartupPromptError(error instanceof Error ? error.message : "Could not enable startup.");
              });
          }}
        />
      ) : null}

      <div className="compact-list queue-table">
        {jobs.length === 0 ? <GettingStarted fallback={<div className="empty-state">No downloads</div>} /> : null}
        {jobs.map((job) => (
          <QueueRow
            key={job.id}
            job={job}
            copiedReportId={copiedReportId}
            busy={busyJobId === job.id}
            actionError={actionErrors[job.id]}
            onCopiedReport={setCopiedReportId}
            onAction={(actionJob, action) =>
              runJobAction({
                job: actionJob,
                action,
                setBusyJobId,
                setActionErrors,
              })
            }
          />
        ))}
      </div>
    </section>
  );
}

function StartupPrompt({
  error,
  onDismiss,
  onEnable,
}: {
  error: string | null;
  onDismiss: () => void;
  onEnable: () => void;
}) {
  return (
    <div className="startup-prompt">
      <div className="startup-prompt-copy">
        <strong>Start LDM with Windows?</strong>
        <span>This keeps LDM ready in the tray so browser downloads can hand off faster.</span>
      </div>
      <div className="startup-prompt-actions">
        <button className="secondary-action" type="button" onClick={onEnable}>
          Enable
        </button>
        <button className="secondary-action" type="button" onClick={onDismiss}>
          Not now
        </button>
      </div>
      {error ? <small className="status-line action-error">{error}</small> : null}
    </div>
  );
}

const STARTUP_PROMPT_DISMISSED_KEY = "ldm.startupPromptDismissed";

function hasDismissedStartupPrompt(): boolean {
  return window.localStorage.getItem(STARTUP_PROMPT_DISMISSED_KEY) === "1";
}

function rememberStartupPromptDismissed(): void {
  window.localStorage.setItem(STARTUP_PROMPT_DISMISSED_KEY, "1");
}

function QueueRow({
  actionError,
  busy,
  copiedReportId,
  job,
  onAction,
  onCopiedReport,
}: {
  actionError?: string;
  busy: boolean;
  copiedReportId: string | null;
  job: DownloadJob;
  onAction: (job: DownloadJob, action: QueueJobAction) => Promise<void>;
  onCopiedReport: (jobId: string | null) => void;
}) {
  const totalBytes = displayTotalBytes(job);

  return (
    <article className={`compact-download-card queue-row ${job.status}`}>
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
          <span>{kindLabel(job.kind)}</span>
          <span>{sourceLabel(job)}</span>
          {totalBytes ? <span>{formatBytes(totalBytes)}</span> : null}
          <span className="truncate" title={job.targetDirectory}>
            {job.targetDirectory}
          </span>
        </span>
        <QueueProgress job={job} />
        {job.error ? <small className="status-line error">{job.error}</small> : null}
        {actionError ? <small className="status-line action-error">{actionError}</small> : null}
        {controlNote(job) ? <small className="status-line status-note">{controlNote(job)}</small> : null}
      </div>
      <div className="compact-download-actions">
        <QueueActions busy={busy} job={job} onAction={onAction} />
        <button
          className="diagnostic-button"
          type="button"
          title="Copy job diagnostics"
          onClick={() => {
            void navigator.clipboard.writeText(buildJobDiagnostics(job));
            onCopiedReport(job.id);
            window.setTimeout(() => onCopiedReport(null), 1400);
          }}
        >
          <Clipboard size={13} />
          <span>{copiedReportId === job.id ? "Copied" : "Report"}</span>
        </button>
      </div>
    </article>
  );
}

async function runJobAction({
  action,
  job,
  setActionErrors,
  setBusyJobId,
}: {
  action: QueueJobAction;
  job: DownloadJob;
  setActionErrors: React.Dispatch<React.SetStateAction<Record<string, string>>>;
  setBusyJobId: React.Dispatch<React.SetStateAction<string | null>>;
}) {
  setBusyJobId(job.id);
  setActionErrors((currentErrors) => {
    const { [job.id]: _removed, ...nextErrors } = currentErrors;
    return nextErrors;
  });

  try {
    const updatedJob = await action(job.id);
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
