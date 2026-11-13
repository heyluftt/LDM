import type { DetectedDownloadCandidate } from "@ldm/contracts";
import { isMasterManifestUrlValue, qualityHeightForUrl, qualityHintForCandidate } from "../shared/stream-quality";

export { qualityHintForUrl } from "../shared/stream-quality";

export type CandidateDiagnostic = {
  selected: boolean;
  origin: string;
  source: string;
  adapterId: string;
  kind: string;
  quality: string;
  qualityHeight: number | null;
  bitrate: number | null;
  isMasterManifest: boolean;
  url: string;
  hasQuery: boolean;
  hasHeaders: boolean;
  contentType: string | null;
  contentLength: number | null;
  fallbackCount: number;
  diagnosticScore: number;
};

export type VideoStateDiagnostic = {
  readyState: { value: number; label: string };
  networkState: { value: number; label: string };
  paused: boolean;
  ended: boolean;
  seeking: boolean;
  muted: boolean;
  autoplay: boolean;
  controls: boolean;
  preload: string;
  currentTime: number;
  duration: number | null;
  playbackRate: number;
  videoWidth: number;
  videoHeight: number;
  elementWidth: number;
  elementHeight: number;
  visible: boolean;
  error: { code: number; label: string; message: string } | null;
  sources: Array<{ kind: string; url: string; hasQuery: boolean }>;
  buffered: Array<{ start: number; end: number }>;
  seekable: Array<{ start: number; end: number }>;
};

export function buildCandidateDiagnostics(
  selectedCandidate: DetectedDownloadCandidate,
  candidates: DetectedDownloadCandidate[] = [],
): CandidateDiagnostic[] {
  const byUrl = new Map<string, DetectedDownloadCandidate>();
  byUrl.set(selectedCandidate.url, selectedCandidate);
  for (const candidate of candidates) {
    byUrl.set(candidate.url, candidate.url === selectedCandidate.url ? selectedCandidate : candidate);
  }

  return Array.from(byUrl.values())
    .sort((left, right) => diagnosticSortScore(right, selectedCandidate) - diagnosticSortScore(left, selectedCandidate))
    .slice(0, 12)
    .map((candidate) => {
      const score = diagnosticSortScore(candidate, selectedCandidate);
      return {
        selected: candidate.url === selectedCandidate.url,
        origin: originForCandidate(candidate),
        source: candidate.source,
        adapterId: candidate.adapterId,
        kind: candidate.kind,
        quality: qualityHintForCandidate(candidate),
        qualityHeight: candidate.qualityHeight ?? qualityHeightForUrl(candidate.url) ?? null,
        bitrate: candidate.bitrate ?? null,
        isMasterManifest: isMasterManifestUrlValue(candidate.url),
        url: safeDisplayUrl(candidate.url),
        hasQuery: urlHasQuery(candidate.url),
        hasHeaders: Boolean(candidate.headers && Object.keys(candidate.headers).length > 0),
        contentType: candidate.contentType ?? null,
        contentLength: candidate.contentLength ?? null,
        fallbackCount: candidate.sourceFallbackUrls?.length ?? 0,
        diagnosticScore: score,
      };
    });
}

export function buildCandidateDebugReport(
  selectedCandidate: DetectedDownloadCandidate,
  candidates: DetectedDownloadCandidate[] = [],
  video?: HTMLVideoElement,
): string {
  return JSON.stringify(
    {
      selected: {
        origin: originForCandidate(selectedCandidate),
        source: selectedCandidate.source,
        adapterId: selectedCandidate.adapterId,
        kind: selectedCandidate.kind,
        quality: qualityHintForCandidate(selectedCandidate),
        qualityHeight: selectedCandidate.qualityHeight ?? qualityHeightForUrl(selectedCandidate.url) ?? null,
        bitrate: selectedCandidate.bitrate ?? null,
        isMasterManifest: isMasterManifestUrlValue(selectedCandidate.url),
        url: safeDisplayUrl(selectedCandidate.url),
        hasQuery: urlHasQuery(selectedCandidate.url),
        hasHeaders: Boolean(selectedCandidate.headers && Object.keys(selectedCandidate.headers).length > 0),
        contentType: selectedCandidate.contentType ?? null,
        contentLength: selectedCandidate.contentLength ?? null,
        fallbackCount: selectedCandidate.sourceFallbackUrls?.length ?? 0,
      },
      player: buildVideoStateDiagnostic(video),
      candidates: buildCandidateDiagnostics(selectedCandidate, candidates),
      note: "Report omits query strings, header values, cookies, authorization, and local file paths.",
    },
    null,
    2,
  );
}

export function buildVideoStateDiagnostic(video?: HTMLVideoElement): VideoStateDiagnostic | null {
  if (!video) {
    return null;
  }

  const rect = video.getBoundingClientRect();
  return {
    readyState: {
      value: video.readyState,
      label: readyStateLabel(video.readyState),
    },
    networkState: {
      value: video.networkState,
      label: networkStateLabel(video.networkState),
    },
    paused: video.paused,
    ended: video.ended,
    seeking: video.seeking,
    muted: video.muted,
    autoplay: video.autoplay,
    controls: video.controls,
    preload: video.preload,
    currentTime: roundedNumber(video.currentTime),
    duration: Number.isFinite(video.duration) ? roundedNumber(video.duration) : null,
    playbackRate: roundedNumber(video.playbackRate),
    videoWidth: video.videoWidth,
    videoHeight: video.videoHeight,
    elementWidth: Math.round(rect.width),
    elementHeight: Math.round(rect.height),
    visible: isVideoVisible(video, rect),
    error: video.error
      ? {
          code: video.error.code,
          label: mediaErrorLabel(video.error.code),
          message: video.error.message || "",
        }
      : null,
    sources: videoSources(video),
    buffered: timeRanges(video.buffered),
    seekable: timeRanges(video.seekable),
  };
}

function urlHasQuery(value: string): boolean {
  try {
    return new URL(value).search.length > 0;
  } catch {
    return value.includes("?");
  }
}

function videoSources(video: HTMLVideoElement): Array<{ kind: string; url: string; hasQuery: boolean }> {
  const sources: Array<{ kind: string; value: string }> = [
    { kind: "currentSrc", value: video.currentSrc },
    { kind: "src", value: video.src },
  ];
  video.querySelectorAll<HTMLSourceElement>("source").forEach((source, index) => {
    sources.push({ kind: `source-${index}`, value: source.src || source.getAttribute("src") || "" });
  });

  const seen = new Set<string>();
  return sources
    .filter((source) => source.value)
    .map((source) => ({
      kind: source.kind,
      url: safeDisplayUrl(source.value),
      hasQuery: urlHasQuery(source.value),
    }))
    .filter((source) => {
      const key = `${source.kind}:${source.url}`;
      if (seen.has(key)) {
        return false;
      }
      seen.add(key);
      return true;
    });
}

function timeRanges(ranges: TimeRanges): Array<{ start: number; end: number }> {
  const values: Array<{ start: number; end: number }> = [];
  for (let index = 0; index < ranges.length && values.length < 6; index += 1) {
    values.push({
      start: roundedNumber(ranges.start(index)),
      end: roundedNumber(ranges.end(index)),
    });
  }
  return values;
}

function roundedNumber(value: number): number {
  return Math.round(value * 100) / 100;
}

function isVideoVisible(video: HTMLVideoElement, rect: DOMRect): boolean {
  const style = video.ownerDocument.defaultView?.getComputedStyle(video);
  return Boolean(
    video.isConnected &&
      rect.width > 0 &&
      rect.height > 0 &&
      style &&
      style.display !== "none" &&
      style.visibility !== "hidden" &&
      style.opacity !== "0",
  );
}

function readyStateLabel(value: number): string {
  switch (value) {
    case HTMLMediaElement.HAVE_NOTHING:
      return "have-nothing";
    case HTMLMediaElement.HAVE_METADATA:
      return "have-metadata";
    case HTMLMediaElement.HAVE_CURRENT_DATA:
      return "have-current-data";
    case HTMLMediaElement.HAVE_FUTURE_DATA:
      return "have-future-data";
    case HTMLMediaElement.HAVE_ENOUGH_DATA:
      return "have-enough-data";
    default:
      return "unknown";
  }
}

function networkStateLabel(value: number): string {
  switch (value) {
    case HTMLMediaElement.NETWORK_EMPTY:
      return "empty";
    case HTMLMediaElement.NETWORK_IDLE:
      return "idle";
    case HTMLMediaElement.NETWORK_LOADING:
      return "loading";
    case HTMLMediaElement.NETWORK_NO_SOURCE:
      return "no-source";
    default:
      return "unknown";
  }
}

function mediaErrorLabel(value: number): string {
  switch (value) {
    case MediaError.MEDIA_ERR_ABORTED:
      return "aborted";
    case MediaError.MEDIA_ERR_NETWORK:
      return "network";
    case MediaError.MEDIA_ERR_DECODE:
      return "decode";
    case MediaError.MEDIA_ERR_SRC_NOT_SUPPORTED:
      return "source-not-supported";
    default:
      return "unknown";
  }
}

export function safeDisplayUrl(value: string): string {
  try {
    const url = new URL(value);
    return `${url.hostname}${decodeURIComponent(url.pathname)}`;
  } catch {
    return value.split("?")[0]?.slice(0, 160) ?? "";
  }
}

function diagnosticSortScore(
  candidate: DetectedDownloadCandidate,
  selectedCandidate: DetectedDownloadCandidate,
): number {
  let score = candidate.url === selectedCandidate.url ? 10_000 : 0;
  if (candidate.source === "hls" || candidate.source === "dash") {
    score += 1_000;
  }
  if (candidate.id.startsWith("network-") || candidate.id.startsWith("resource-")) {
    score += 300;
  }
  if (candidate.id.startsWith("embedded-")) {
    score += 150;
  }

  const quality = Number.parseInt(qualityHintForCandidate(candidate), 10);
  if (Number.isFinite(quality)) {
    score += quality;
  } else if (qualityHintForCandidate(candidate) === "master") {
    score += 800;
  }

  return score;
}

function originForCandidate(candidate: DetectedDownloadCandidate): string {
  if (candidate.id.startsWith("network-")) {
    return "network";
  }
  if (candidate.id.startsWith("resource-")) {
    return "performance";
  }
  if (candidate.id.startsWith("embedded-")) {
    return "embedded";
  }
  if (candidate.id.startsWith("video-")) {
    return "dom";
  }
  if (candidate.source === "provider") {
    return "provider";
  }

  return "unknown";
}
