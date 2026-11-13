import type { DownloadJob } from "@ldm/contracts";
import { Pause, Play, X } from "lucide-react";
import { cancelJob, pauseJob, resumeJob } from "../../shared/api/downloads";
import { isPauseResumeJob } from "./queueJobMeta";

export type QueueJobAction = (jobId: string) => Promise<DownloadJob>;

export function QueueActions({
  busy,
  job,
  onAction,
}: {
  busy: boolean;
  job: DownloadJob;
  onAction: (job: DownloadJob, action: QueueJobAction) => Promise<void>;
}) {
  const canPauseResumeThisJob = isPauseResumeJob(job);
  const isRunning = job.status === "running";
  const isPaused = job.status === "paused";
  const canResume = canPauseResumeThisJob && (isPaused || job.status === "failed");
  const canCancel = job.status === "queued" || job.status === "running" || job.status === "paused";
  const pauseTitle = canPauseResumeThisJob ? "Pause" : "Provider jobs can be canceled, but not paused safely yet";
  const resumeTitle = canPauseResumeThisJob ? "Resume" : "Provider jobs can be retried from History if they fail";

  return (
    <span className="job-actions" role="toolbar" aria-label={`Actions for ${job.filename}`}>
      <button
        type="button"
        title={resumeTitle}
        disabled={busy || !canResume}
        onClick={() => void onAction(job, resumeJob)}
      >
        <Play size={14} />
      </button>
      <button
        type="button"
        title={pauseTitle}
        disabled={busy || !canPauseResumeThisJob || !isRunning}
        onClick={() => void onAction(job, pauseJob)}
      >
        <Pause size={14} />
      </button>
      <button type="button" title="Cancel" disabled={busy || !canCancel} onClick={() => void onAction(job, cancelJob)}>
        <X size={14} />
      </button>
    </span>
  );
}
