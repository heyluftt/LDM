import type { DownloadJob } from "@ldm/contracts";

export function buildHistoryReport(job: DownloadJob): string {
  return JSON.stringify(
    {
      version: 1,
      status: job.status,
      kind: job.kind,
      adapterId: job.adapterId,
      providerId: job.providerId ?? null,
      source: summarizeUrl(job.sourceUrl),
      finalUrl: job.finalUrl ? summarizeUrl(job.finalUrl) : null,
      page: job.pageUrl ? summarizeUrl(job.pageUrl) : null,
      filename: {
        nameLength: job.filename.length,
        extension: job.filename.includes(".") ? (job.filename.split(".").pop()?.toLowerCase() ?? "") : "",
      },
      progressBytes: job.progressBytes,
      totalBytes: job.totalBytes ?? null,
      supportsResume: job.supportsResume,
      retryCount: job.retryCount,
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
