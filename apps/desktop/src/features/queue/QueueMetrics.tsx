import type { DownloadJob } from "@ldm/contracts";
import { displaySpeedBytesPerSecond, formatSpeed } from "./queueProgressModel";

type MetricTone = "good" | "muted" | "neutral" | "wait";

export function QueueMetrics({ jobs }: { jobs: DownloadJob[] }) {
  const metrics = queueMetrics(jobs);

  return (
    <section className="metric-strip" aria-label="Queue status">
      <MetricPill label="Running" value={metrics.running.toString()} tone={metrics.running > 0 ? "good" : "neutral"} />
      <MetricPill label="Queued" value={metrics.queued.toString()} tone={metrics.queued > 0 ? "wait" : "neutral"} />
      <MetricPill label="Paused" value={metrics.paused.toString()} tone={metrics.paused > 0 ? "muted" : "neutral"} />
      <MetricPill label="Speed" value={metrics.speedLabel} tone={metrics.totalSpeed > 0 ? "good" : "neutral"} />
    </section>
  );
}

export function queueSummary(jobs: DownloadJob[]): string {
  if (jobs.length === 0) {
    return "No active downloads";
  }

  const running = jobs.filter((job) => job.status === "running").length;
  const paused = jobs.filter((job) => job.status === "paused").length;
  const queued = jobs.filter((job) => job.status === "queued").length;
  const parts = [
    running ? `${running} running` : null,
    paused ? `${paused} paused` : null,
    queued ? `${queued} queued` : null,
  ].filter(Boolean);

  return parts.length > 0 ? parts.join(" - ") : `${jobs.length} active`;
}

function MetricPill({ label, value, tone }: { label: string; value: string; tone: MetricTone }) {
  return (
    <span className={`metric-pill ${tone}`}>
      <span>{label}</span>
      <strong>{value}</strong>
    </span>
  );
}

function queueMetrics(jobs: DownloadJob[]) {
  const running = jobs.filter((job) => job.status === "running").length;
  const paused = jobs.filter((job) => job.status === "paused").length;
  const queued = jobs.filter((job) => job.status === "queued").length;
  const totalSpeed = jobs
    .filter((job) => job.status === "running")
    .reduce((sum, job) => sum + (displaySpeedBytesPerSecond(job) ?? 0), 0);

  return {
    running,
    paused,
    queued,
    totalSpeed,
    speedLabel: totalSpeed > 0 ? formatSpeed(totalSpeed) : "0 B/s",
  };
}
