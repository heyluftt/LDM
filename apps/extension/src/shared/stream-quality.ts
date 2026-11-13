import type { DetectedDownloadCandidate } from "@ldm/contracts";

/**
 * `scoreCandidateQuality` encodes a height of `h` pixels as `h * QUALITY_HEIGHT_UNIT` and keeps
 * the units below for the bitrate, so height always wins and bitrate breaks ties.
 */
export const QUALITY_HEIGHT_UNIT = 1_000_000;
/**
 * A master manifest without a height in its URL counts as 900p: above the common single
 * renditions, below an explicit 1080p, which is a sure thing where the master is a promise.
 */
const MASTER_MANIFEST_HEIGHT = 900;

/** URL-guessed stream quality (`scoreStreamManifestUrl`) counts one point per this many pixels of height. */
export const STREAM_PIXELS_PER_POINT = 3;
/** Cap for the URL height guess: 1080p. */
const STREAM_URL_QUALITY_MAX = 360;
/** Half the cap: a master outranks a single rendition unless that one is 540p taller. */
const MASTER_MANIFEST_BONUS = 180;
/** A master without a height in its URL ranks just below a 1080p rendition and above 720p (240). */
const MASTER_MANIFEST_WITHOUT_HEIGHT_SCORE = 320;
/** Keeps a recognised stream URL without any quality hint above zero. */
const UNKNOWN_STREAM_URL_SCORE = 20;

export function qualityHintForCandidate(candidate: DetectedDownloadCandidate): string {
  if (candidate.qualityLabel) {
    return candidate.qualityLabel;
  }
  if (candidate.qualityHeight && candidate.bitrate) {
    return `${candidate.qualityHeight}p ${Math.round(candidate.bitrate / 1000)}k`;
  }
  if (candidate.qualityHeight) {
    return `${candidate.qualityHeight}p`;
  }
  return qualityHintForUrl(candidate.url);
}

export function qualityHintForUrl(value: string, baseUrl?: string): string {
  const parsed = parseUrl(value, baseUrl);
  const haystack = qualityHaystack(value, baseUrl);
  const isMaster = parsed ? isMasterManifestUrl(parsed) : false;
  const height = qualityHeightFromText(haystack);

  if (height) {
    return isMaster ? `${height}p master` : `${height}p`;
  }
  if (isMaster) {
    return "master";
  }
  if (haystack.includes(".m3u8")) {
    return "hls";
  }
  if (haystack.includes(".mpd")) {
    return "dash";
  }
  return "unknown";
}

export function qualityHeightForUrl(value: string, baseUrl?: string): number | undefined {
  return qualityHeightFromText(qualityHaystack(value, baseUrl));
}

export function qualityHeightFromText(value: string): number | undefined {
  const resolutionMatch = value.match(/(?:^|[^\d])\d{3,5}x(\d{3,5})(?:[^\d]|$)/u);
  if (resolutionMatch?.[1]) {
    return Number.parseInt(resolutionMatch[1], 10);
  }

  const heightMatch =
    value.match(/(?:^|[^\d])(\d{3,5})p(?:[^\d]|$)/u) ??
    value.match(/(?:height|res|quality|rendition)[=:_-](\d{3,5})(?:[^\d]|$)/u);
  if (heightMatch?.[1]) {
    return Number.parseInt(heightMatch[1], 10);
  }

  return undefined;
}

export function scoreStreamManifestUrl(value: string, baseUrl?: string): number {
  const parsed = parseUrl(value, baseUrl);
  if (!parsed) {
    return 0;
  }

  const height = qualityHeightForUrl(parsed.href);
  if (height) {
    const masterBonus = isMasterManifestUrl(parsed) ? MASTER_MANIFEST_BONUS : 0;
    return masterBonus + Math.min(STREAM_URL_QUALITY_MAX, Math.round(height / STREAM_PIXELS_PER_POINT));
  }

  if (isMasterManifestUrl(parsed)) {
    return MASTER_MANIFEST_WITHOUT_HEIGHT_SCORE;
  }

  return UNKNOWN_STREAM_URL_SCORE;
}

export function scoreCandidateQuality(candidate: DetectedDownloadCandidate, baseUrl?: string): number {
  const explicitQuality =
    (candidate.qualityHeight ?? 0) * QUALITY_HEIGHT_UNIT + Math.min(candidate.bitrate ?? 0, QUALITY_HEIGHT_UNIT - 1);

  if (explicitQuality > 0) {
    return explicitQuality;
  }

  const height = qualityHeightForUrl(candidate.url, baseUrl);
  if (height) {
    return height * QUALITY_HEIGHT_UNIT;
  }

  if ((candidate.source === "hls" || candidate.source === "dash") && isMasterManifestUrlValue(candidate.url, baseUrl)) {
    return MASTER_MANIFEST_HEIGHT * QUALITY_HEIGHT_UNIT;
  }

  return 0;
}

export function isMasterManifestUrlValue(value: string, baseUrl?: string): boolean {
  const parsed = parseUrl(value, baseUrl);
  return parsed ? isMasterManifestUrl(parsed) : false;
}

export function isMasterManifestUrl(url: URL): boolean {
  const filename = decodeURIComponent(url.pathname).split("/").filter(Boolean).pop()?.toLowerCase();

  return /^(?:master|playlist|manifest|index)\.(?:m3u8|mpd)$/u.test(filename ?? "");
}

function qualityHaystack(value: string, baseUrl?: string): string {
  const parsed = parseUrl(value, baseUrl);
  if (!parsed) {
    return value.toLowerCase();
  }

  return decodeURIComponent(`${parsed.pathname}?${parsed.searchParams}`).toLowerCase();
}

function parseUrl(value: string, baseUrl?: string): URL | null {
  try {
    return new URL(value, baseUrl);
  } catch {
    return null;
  }
}
