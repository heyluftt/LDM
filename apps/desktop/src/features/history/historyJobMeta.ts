import type { DownloadJob } from "@ldm/contracts";

export const terminalStatuses = new Set(["completed", "failed", "canceled"]);

export function historySummary(jobs: DownloadJob[]): string {
  if (jobs.length === 0) {
    return "No finished downloads";
  }

  const completed = jobs.filter((job) => job.status === "completed").length;
  const failed = jobs.filter((job) => job.status === "failed").length;
  const canceled = jobs.filter((job) => job.status === "canceled").length;
  const parts = [
    `${jobs.length} entries`,
    completed ? `${completed} completed` : null,
    failed ? `${failed} failed` : null,
    canceled ? `${canceled} canceled` : null,
  ].filter(Boolean);

  return parts.join(" - ");
}

export function statusLabel(status: DownloadJob["status"]): string {
  const labels: Record<DownloadJob["status"], string> = {
    queued: "Queued",
    running: "Running",
    paused: "Paused",
    completed: "Completed",
    failed: "Failed",
    canceled: "Canceled",
  };

  return labels[status];
}

export function canRetry(job: DownloadJob): boolean {
  return job.status === "failed" || job.status === "canceled";
}

export function actionErrorMessage(error: unknown): string {
  if (error instanceof Error) {
    return error.message;
  }
  if (typeof error === "string") {
    return error;
  }
  return "Action failed.";
}

export function formatTimestamp(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds <= 0) {
    return "Unknown";
  }

  return new Intl.DateTimeFormat(undefined, {
    dateStyle: "short",
    timeStyle: "short",
  }).format(new Date(seconds * 1000));
}
