import type { DownloadJob } from "@ldm/contracts";
import { isDirectHttpJob } from "./queueJobMeta";

const MANIFEST_TOTAL_MAX_BYTES = 1024 * 1024;
const LEGACY_SYNTHETIC_PROVIDER_TOTAL_BYTES = 10_000;

export function formatBytes(bytes: number): string {
  const safeBytes = Math.max(0, bytes);
  const units = ["B", "KB", "MB", "GB", "TB"] as const;
  let value = safeBytes;
  let unitIndex = 0;

  while (value >= 1024 && unitIndex < units.length - 1) {
    value /= 1024;
    unitIndex += 1;
  }

  if (unitIndex === 0) {
    return `${Math.round(value)} ${units[unitIndex]}`;
  }

  const decimals = value >= 100 ? 0 : 1;
  return `${value.toFixed(decimals)} ${units[unitIndex]}`;
}

export function formatSpeed(bytesPerSecond: number): string {
  return `${formatBytes(bytesPerSecond)}/s`;
}

export function formatDuration(seconds: number): string {
  const safeSeconds = Math.max(0, Math.round(seconds));
  if (safeSeconds < 60) {
    return `${safeSeconds}s`;
  }

  const minutes = Math.floor(safeSeconds / 60);
  const remainingSeconds = safeSeconds % 60;
  if (minutes < 60) {
    return `${minutes}:${remainingSeconds.toString().padStart(2, "0")}`;
  }

  const hours = Math.floor(minutes / 60);
  const remainingMinutes = minutes % 60;
  return `${hours}:${remainingMinutes.toString().padStart(2, "0")}:${remainingSeconds.toString().padStart(2, "0")}`;
}

export function displayTotalBytes(job: DownloadJob): number | null {
  const total = job.totalBytes ?? 0;
  if (total <= 0) {
    return null;
  }

  if (isManifestOrSyntheticTotal(job, total)) {
    return null;
  }

  return total;
}

export function displayProgressBytes(job: DownloadJob): number | null {
  if (isLegacySyntheticProviderTotal(job)) {
    return null;
  }

  return job.progressBytes > 0 ? job.progressBytes : null;
}

export function progressPercent(job: DownloadJob, displayTotal = displayTotalBytes(job) ?? 0): number | null {
  if (typeof job.progressPercent === "number" && Number.isFinite(job.progressPercent)) {
    return Math.max(0, Math.min(100, job.progressPercent));
  }

  if (isManifestOrSyntheticTotal(job, job.totalBytes ?? 0)) {
    return estimatedProgressPercentFromEta(job);
  }

  if (displayTotal <= 0) {
    return estimatedProgressPercentFromEta(job);
  }

  return Math.max(0, Math.min(100, (job.progressBytes / displayTotal) * 100));
}

export function progressStatusLine(
  job: DownloadJob,
  total: number,
  percentText: string | null,
  progressBytes: number | null,
): string {
  if (job.status === "queued") {
    return "Waiting";
  }

  if (progressBytes === null && percentText === null) {
    return job.status === "running" ? "Starting..." : "0 B";
  }

  if (total > 0) {
    const byteText = `${formatBytes(progressBytes ?? 0)} of ${formatBytes(total)}`;
    return percentText ? `${percentText} - ${byteText}` : byteText;
  }

  if (percentText && progressBytes === null) {
    return percentText;
  }

  return percentText
    ? `${percentText} - ${formatBytes(progressBytes ?? 0)}`
    : `${formatBytes(progressBytes ?? 0)} downloaded`;
}

export function progressDetailLine(speedBytesPerSecond: number | null, etaSeconds: number | null): string | null {
  const parts = [];
  if (speedBytesPerSecond && speedBytesPerSecond > 0) {
    parts.push(formatSpeed(speedBytesPerSecond));
  }
  if (etaSeconds !== null && etaSeconds >= 0) {
    parts.push(`${formatDuration(etaSeconds)} left`);
  }

  return parts.length > 0 ? parts.join(" - ") : null;
}

export function displaySpeedBytesPerSecond(
  job: DownloadJob,
  nowSeconds = Math.floor(Date.now() / 1000),
): number | null {
  if (job.status !== "running") {
    return null;
  }

  const speed = job.speedBytesPerSecond ?? averageSpeedBytesPerSecond(job, nowSeconds);
  return speed && speed > 0 ? speed : null;
}

export function formatPercent(percent: number): string {
  const clamped = Math.max(0, Math.min(100, percent));
  if (clamped === 0 || clamped === 100 || clamped >= 10) {
    return `${Math.round(clamped)}%`;
  }

  return `${clamped.toFixed(1)}%`;
}

export function estimateEtaSeconds(
  job: DownloadJob,
  speedBytesPerSecond: number | null,
  displayTotal = displayTotalBytes(job) ?? 0,
): number | null {
  if (!speedBytesPerSecond || speedBytesPerSecond <= 0) {
    return null;
  }

  if (displayTotal <= 0 || job.progressBytes >= displayTotal) {
    return null;
  }

  return Math.ceil((displayTotal - job.progressBytes) / speedBytesPerSecond);
}

export function averageSpeedBytesPerSecond(
  job: DownloadJob,
  nowSeconds = Math.floor(Date.now() / 1000),
): number | null {
  if (job.status !== "running") {
    return null;
  }

  const createdAt = knownSeconds(job.createdAt);
  if (createdAt === null || job.progressBytes <= 0) {
    return null;
  }

  const elapsedSeconds = Math.max(1, nowSeconds - createdAt);
  return Math.round(job.progressBytes / elapsedSeconds);
}

/** A Unix-second timestamp, or null when the backend did not know the time (`0`). */
export function knownSeconds(value: number): number | null {
  return Number.isFinite(value) && value > 0 ? value : null;
}

function isManifestOrSyntheticTotal(job: DownloadJob, total: number): boolean {
  if (isDirectHttpJob(job)) {
    return false;
  }

  if (isLegacySyntheticProviderTotal(job, total)) {
    return true;
  }

  if (total > MANIFEST_TOTAL_MAX_BYTES) {
    return false;
  }

  const contentType = job.requestContentType?.toLowerCase() ?? "";
  const urls = [job.sourceUrl, job.finalUrl ?? ""].map((value) => value.toLowerCase());
  return (
    contentType.includes("mpegurl") ||
    contentType.includes("mp2t") ||
    contentType.includes("dash") ||
    contentType.includes("mpd") ||
    urls.some((url) => url.includes(".m3u8") || url.includes(".mpd"))
  );
}

function isLegacySyntheticProviderTotal(job: DownloadJob, total = job.totalBytes ?? 0): boolean {
  return !isDirectHttpJob(job) && total === LEGACY_SYNTHETIC_PROVIDER_TOTAL_BYTES;
}

function estimatedProgressPercentFromEta(job: DownloadJob): number | null {
  if (job.status !== "running" || job.progressBytes <= 0) {
    return null;
  }

  const speed = job.speedBytesPerSecond ?? null;
  const etaSeconds = job.etaSeconds ?? null;
  if (!speed || speed <= 0 || etaSeconds === null || etaSeconds <= 0) {
    return null;
  }

  const estimatedTotal = job.progressBytes + speed * etaSeconds;
  if (estimatedTotal <= job.progressBytes) {
    return null;
  }

  return Math.max(0, Math.min(99.9, (job.progressBytes / estimatedTotal) * 100));
}
