import type { DetectedDownloadCandidate } from "@ldm/contracts";
import { sanitizeHeaderRecord } from "../shared/headers";

export function browserLikeVideoHeaders(candidate: DetectedDownloadCandidate): Record<string, string> | undefined {
  return browserLikeMediaHeaders(candidate, "*/*");
}

export function browserLikeHlsHeaders(candidate: DetectedDownloadCandidate): Record<string, string> | undefined {
  const headers = browserLikeMediaHeaders(candidate, "application/vnd.apple.mpegurl,application/x-mpegurl,*/*");
  if (!headers) {
    return undefined;
  }

  const origin = pageOrigin(candidate.pageUrl);
  if (origin && !headers.origin) {
    headers.origin = origin;
  }

  return headers;
}

function browserLikeMediaHeaders(
  candidate: DetectedDownloadCandidate,
  accept: string,
): Record<string, string> | undefined {
  const headers: Record<string, string> = {
    accept,
    ...sanitizeHeaderRecord(candidate.headers),
  };

  const language = browserAcceptLanguage();
  if (language && !headers["accept-language"]) {
    headers["accept-language"] = language;
  }

  const userAgent = browserUserAgent();
  if (userAgent && !headers["user-agent"]) {
    headers["user-agent"] = userAgent;
  }

  return Object.keys(headers).length > 0 ? headers : undefined;
}

function browserAcceptLanguage(): string | undefined {
  if (typeof navigator === "undefined") {
    return undefined;
  }

  return navigator.languages?.length ? navigator.languages.join(",") : navigator.language || undefined;
}

function browserUserAgent(): string | undefined {
  if (typeof navigator === "undefined") {
    return undefined;
  }

  return navigator.userAgent || undefined;
}

function pageOrigin(pageUrl: string | undefined): string | undefined {
  if (!pageUrl) {
    return undefined;
  }

  try {
    return new URL(pageUrl).origin;
  } catch {
    return undefined;
  }
}
