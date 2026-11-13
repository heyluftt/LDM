import type { DownloadJob } from "@ldm/contracts";
import {
  averageSpeedBytesPerSecond,
  displayProgressBytes,
  displaySpeedBytesPerSecond,
  displayTotalBytes,
  knownSeconds,
  progressPercent,
} from "./queueProgressModel";

export function buildJobDiagnostics(job: DownloadJob): string {
  const nowSeconds = Math.floor(Date.now() / 1000);
  const createdAt = knownSeconds(job.createdAt);
  const updatedAt = knownSeconds(job.updatedAt);
  const rawTotalBytes = job.totalBytes ?? null;
  const shownTotalBytes = displayTotalBytes(job);
  const shownProgressBytes = displayProgressBytes(job);
  const elapsedSeconds = createdAt === null ? null : Math.max(0, nowSeconds - createdAt);
  const updatedAgeSeconds = updatedAt === null ? null : Math.max(0, nowSeconds - updatedAt);
  const averageSpeed = averageSpeedBytesPerSecond(job, nowSeconds);
  const activeSpeed = displaySpeedBytesPerSecond(job, nowSeconds);
  const activeEtaSeconds = job.status === "running" ? (job.etaSeconds ?? null) : null;
  const reportProgressPercent = progressPercent(job);

  return JSON.stringify(
    {
      version: 1,
      status: job.status,
      kind: job.kind,
      adapterId: job.adapterId ?? null,
      providerId: job.providerId ?? null,
      selectedOptionId: job.selectedOptionId ?? null,
      source: summarizeUrl(job.sourceUrl),
      finalUrl: job.finalUrl ? summarizeUrl(job.finalUrl) : null,
      page: job.pageUrl ? summarizeUrl(job.pageUrl) : null,
      filename: summarizeFilename(job.filename),
      requestContentType: job.requestContentType ?? null,
      requestContentLength: job.requestContentLength ?? null,
      sourceFallbackCount: job.sourceFallbackCount ?? 0,
      forwardedHeaderNames: [...(job.forwardedHeaderNames ?? [])].sort(),
      preferRangeRequest: job.preferRangeRequest ?? false,
      referrerPresent: job.referrerPresent ?? false,
      progressBytes: shownProgressBytes ?? job.progressBytes,
      rawProgressBytes: job.progressBytes,
      totalBytes: shownTotalBytes,
      rawTotalBytes,
      ignoredDisplayTotalBytes: rawTotalBytes !== null && shownTotalBytes === null,
      progressPercent: reportProgressPercent === null ? null : Math.round(reportProgressPercent * 10) / 10,
      averageSpeedBytesPerSecond: averageSpeed,
      instantSpeedBytesPerSecond: activeSpeed,
      etaSeconds: activeEtaSeconds,
      supportsResume: job.supportsResume,
      retryCount: job.retryCount,
      elapsedSeconds,
      updatedAgeSeconds,
      error: job.error ?? null,
      errorReport: parseJsonReport(job.errorReport),
      qualityReport: parseJsonReport(job.qualityReport),
      note: "Report omits query strings, local file paths, cookies, authorization, and header values.",
    },
    null,
    2,
  );
}

function summarizeUrl(value: string) {
  try {
    const url = new URL(value);
    const filename = url.pathname.split("/").filter(Boolean).pop() ?? "";
    return {
      host: url.hostname,
      filename: decodeURIComponent(filename),
      hasQuery: url.search.length > 0,
    };
  } catch {
    return {
      host: "",
      filename: "",
      hasQuery: value.includes("?"),
    };
  }
}

function summarizeFilename(value: string) {
  const extension = value.includes(".") ? (value.split(".").pop()?.toLowerCase() ?? "") : "";
  return {
    nameLength: value.length,
    extension,
  };
}

function parseJsonReport(value: string | undefined) {
  if (!value) {
    return null;
  }

  try {
    return JSON.parse(value) as unknown;
  } catch {
    return { parseError: true };
  }
}
