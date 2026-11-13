import { adapterKindOf, type DownloadJob, isDirectHttpAdapter, isPauseResumeAdapter } from "@ldm/contracts";

export function isQueueJob(job: DownloadJob): boolean {
  return job.status === "queued" || job.status === "running" || job.status === "paused";
}

export function isDirectHttpJob(job: DownloadJob): boolean {
  return isDirectHttpAdapter(job.adapterId);
}

export function isPauseResumeJob(job: DownloadJob): boolean {
  return isPauseResumeAdapter(job.adapterId);
}

export function kindLabel(kind: DownloadJob["kind"]): string {
  const labels: Record<DownloadJob["kind"], string> = {
    video: "Video",
    audio: "Audio",
    program: "Program",
    document: "Document",
    archive: "Archive",
    image: "Image",
    other: "Other",
  };

  return labels[kind];
}

export function sourceLabel(job: DownloadJob): string {
  if (job.providerId) {
    return job.providerId;
  }

  const kind = adapterKindOf(job.adapterId);
  switch (kind.type) {
    case "hls":
      return "HLS";
    case "dash":
      return "DASH";
    case "provider":
      return kind.provider;
    case "direct-http":
      break;
  }

  if (job.adapterId === "browser-download") {
    return "browser";
  }
  if (job.adapterId === "generic-download-links") {
    return "link";
  }
  if (job.adapterId === "generic-html5-video") {
    return "media";
  }

  return job.adapterId;
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

export function controlNote(job: DownloadJob): string | null {
  if (isPauseResumeJob(job) || job.status !== "running") {
    return null;
  }

  return "Cancel/retry only";
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
