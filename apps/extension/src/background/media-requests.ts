import type { DetectedDownloadCandidate, DetectionSource, DownloadKind } from "@ldm/contracts";
import { type Header, headerValue, numberHeaderValue, sanitizeRequestHeaders } from "../shared/headers";
import { qualityHeightForUrl, qualityHintForUrl, scoreStreamManifestUrl } from "../shared/stream-quality";
import { extensionForUrl, filenameFromUrl, hashString, normalizeContentType } from "../shared/url";

type MediaRequestDetails = {
  tabId: number;
  url: string;
  initiator?: string;
  documentUrl?: string;
  responseHeaders?: Header[];
};

type MediaRequestHeaderDetails = {
  tabId: number;
  url: string;
  initiator?: string;
  documentUrl?: string;
  requestHeaders?: Header[];
};

type StoredCandidate = DetectedDownloadCandidate & {
  detectedAt: number;
  initiator?: string;
  score: number;
};

type RequestCandidateInput = {
  url: string;
  contentType?: string;
  contentLength?: number;
  pageUrl?: string;
  pageTitle?: string;
  headers?: Record<string, string>;
  detectedAt?: number;
};

type StoredRequestHeaders = {
  detectedAt: number;
  headers: Record<string, string>;
};

const MAX_CANDIDATES_PER_TAB = 30;
const MAX_CANDIDATE_AGE_MS = 10 * 60 * 1000;
const MAX_STREAM_CANDIDATE_AGE_MS = 90 * 1000;
const MAX_INITIATOR_ONLY_CANDIDATE_AGE_MS = 15 * 1000;
const MAX_REQUEST_HEADER_AGE_MS = 2 * 60 * 1000;
const MIN_DIRECT_VIDEO_BYTES = 512 * 1024;

const tabCandidates = new Map<number, StoredCandidate[]>();
const tabRequestHeaders = new Map<number, Map<string, StoredRequestHeaders>>();

const DIRECT_MEDIA_EXTENSIONS = new Map<string, DownloadKind>([
  ["mp4", "video"],
  ["m4v", "video"],
  ["webm", "video"],
  ["mov", "video"],
  ["mkv", "video"],
  ["mp3", "audio"],
  ["m4a", "audio"],
  ["aac", "audio"],
  ["wav", "audio"],
  ["flac", "audio"],
]);

const STREAM_EXTENSIONS = new Map<string, DetectionSource>([
  ["m3u8", "hls"],
  ["mpd", "dash"],
]);

const SEGMENT_EXTENSIONS = new Set(["ts", "m4s", "cmfv", "cmfa"]);

const isKnownMediaExtension = (extension: string): boolean =>
  DIRECT_MEDIA_EXTENSIONS.has(extension) || STREAM_EXTENSIONS.has(extension) || SEGMENT_EXTENSIONS.has(extension);

const VIDEO_CONTENT_TYPES = new Set(["video/mp4", "video/webm", "video/quicktime", "video/x-matroska"]);

const AUDIO_CONTENT_TYPES = new Set(["audio/mp4", "audio/mpeg", "audio/aac", "audio/wav", "audio/flac", "audio/webm"]);

const HLS_CONTENT_TYPES = new Set([
  "application/vnd.apple.mpegurl",
  "application/x-mpegurl",
  "audio/mpegurl",
  "audio/x-mpegurl",
]);

const DASH_CONTENT_TYPES = new Set(["application/dash+xml"]);

// webRequest listeners may answer with a BlockingResponse; `undefined` says these never block.
export function rememberMediaRequestHeaders(details: MediaRequestHeaderDetails): undefined {
  if (details.tabId < 0) {
    return;
  }

  const headers = sanitizeRequestHeaders(details.requestHeaders);
  if (Object.keys(headers).length === 0) {
    return;
  }

  const storedHeaders = pruneOldRequestHeaders(tabRequestHeaders.get(details.tabId) ?? new Map());
  storedHeaders.set(details.url, {
    detectedAt: Date.now(),
    headers,
  });
  tabRequestHeaders.set(details.tabId, storedHeaders);
}

export function rememberMediaRequest(details: MediaRequestDetails): undefined {
  if (details.tabId < 0) {
    return;
  }

  const requestHeaders = pruneOldRequestHeaders(tabRequestHeaders.get(details.tabId) ?? new Map());
  tabRequestHeaders.set(details.tabId, requestHeaders);
  const headerContentType = headerValue(details.responseHeaders, "content-type");
  const headerContentLength = numberHeaderValue(details.responseHeaders, "content-length");
  const candidate = candidateFromNetworkRequest({
    url: details.url,
    contentType: headerContentType,
    contentLength: headerContentLength,
    pageUrl: details.documentUrl,
    headers: requestHeaders.get(details.url)?.headers,
  });

  if (!candidate) {
    return;
  }

  const initiator = details.initiator ?? details.documentUrl;
  const stored: StoredCandidate = {
    ...candidate,
    initiator,
    detectedAt: Date.now(),
    score: scoreCandidate(candidate),
  };

  const candidates = pruneOldCandidates(tabCandidates.get(details.tabId) ?? []);
  const existingIndex = candidates.findIndex((item) => item.url === stored.url);
  if (existingIndex >= 0) {
    candidates[existingIndex] = {
      ...candidates[existingIndex],
      ...stored,
      detectedAt: stored.detectedAt,
      contentLength: stored.contentLength ?? candidates[existingIndex]?.contentLength,
      contentType: stored.contentType ?? candidates[existingIndex]?.contentType,
    };
  } else {
    candidates.push(stored);
  }

  tabCandidates.set(details.tabId, candidates.sort(compareCandidates).slice(0, MAX_CANDIDATES_PER_TAB));
}

export function clearTabMediaRequests(tabId: number): void {
  tabCandidates.delete(tabId);
  tabRequestHeaders.delete(tabId);
}

export function getMediaCandidatesForTab(
  tabId: number,
  pageUrl: string,
  pageTitle?: string,
): DetectedDownloadCandidate[] {
  const candidates = pruneOldCandidates(tabCandidates.get(tabId) ?? []);
  tabCandidates.set(tabId, candidates);

  return candidates
    .filter((candidate) => isCandidateForPage(candidate, pageUrl))
    .sort(compareCandidates)
    .map(({ detectedAt: _detectedAt, initiator: _initiator, score: _score, ...candidate }) => ({
      ...candidate,
      pageUrl,
      title: candidate.title ?? pageTitle,
      suggestedFilename: suggestedFilenameForCandidate(candidate, pageTitle),
    }));
}

export function candidateFromNetworkRequest(input: RequestCandidateInput): DetectedDownloadCandidate | null {
  let parsedUrl: URL;
  try {
    parsedUrl = new URL(input.url);
  } catch {
    return null;
  }

  if (parsedUrl.protocol !== "http:" && parsedUrl.protocol !== "https:") {
    return null;
  }

  const extension = extensionForUrl(parsedUrl, isKnownMediaExtension);
  if (extension && (SEGMENT_EXTENSIONS.has(extension) || isLikelyMediaSegment(parsedUrl, extension))) {
    return null;
  }

  const normalizedContentType = normalizeContentType(input.contentType);
  const source = sourceForRequest(extension, normalizedContentType);
  if (!source) {
    return null;
  }

  const kind = kindForRequest(parsedUrl, extension, normalizedContentType);
  if (!isUsableNetworkMedia(source, kind, input.contentLength)) {
    return null;
  }

  const adapterId = adapterForSource(source);

  return {
    id: `network-${source}-${hashString(parsedUrl.href)}`,
    url: parsedUrl.href,
    source,
    headers: input.headers,
    title: input.pageTitle,
    pageUrl: input.pageUrl,
    contentType: normalizedContentType,
    contentLength: input.contentLength,
    suggestedFilename: filenameFromUrl(parsedUrl.href),
    kind,
    adapterId,
    qualityHeight: qualityHeightForUrl(parsedUrl.href),
    qualityLabel: qualityHintForUrl(parsedUrl.href) === "unknown" ? undefined : qualityHintForUrl(parsedUrl.href),
  };
}

function sourceForRequest(extension: string | undefined, contentType: string | undefined): DetectionSource | null {
  if (extension) {
    const streamSource = STREAM_EXTENSIONS.get(extension);
    if (streamSource) {
      return streamSource;
    }
    if (DIRECT_MEDIA_EXTENSIONS.has(extension)) {
      return "html5-video";
    }
  }

  if (!contentType) {
    return null;
  }

  if (HLS_CONTENT_TYPES.has(contentType)) {
    return "hls";
  }
  if (DASH_CONTENT_TYPES.has(contentType)) {
    return "dash";
  }
  if (contentType.startsWith("video/") || VIDEO_CONTENT_TYPES.has(contentType)) {
    return "html5-video";
  }
  if (contentType.startsWith("audio/") || AUDIO_CONTENT_TYPES.has(contentType)) {
    return "html5-video";
  }

  return null;
}

function kindForRequest(url: URL, extension: string | undefined, contentType: string | undefined): DownloadKind {
  if (contentType?.startsWith("audio/") || isLikelyAudioMediaUrl(url)) {
    return "audio";
  }

  if (extension) {
    const directKind = DIRECT_MEDIA_EXTENSIONS.get(extension);
    if (directKind) {
      return directKind;
    }
  }

  return "video";
}

function adapterForSource(source: DetectionSource): string {
  if (source === "hls" || source === "dash") {
    return source;
  }
  return "generic-html5-video";
}

function scoreCandidate(candidate: DetectedDownloadCandidate): number {
  if (candidate.source === "hls" || candidate.source === "dash") {
    return 140 + scoreStreamManifestUrl(candidate.url);
  }
  if (candidate.source === "html5-video" && candidate.kind === "video") {
    return 100;
  }
  if (candidate.kind === "audio") {
    return 60;
  }
  return 40;
}

function isUsableNetworkMedia(source: DetectionSource, kind: DownloadKind, contentLength: number | undefined): boolean {
  if (source !== "html5-video" || kind !== "video" || contentLength === undefined) {
    return true;
  }

  return contentLength >= MIN_DIRECT_VIDEO_BYTES;
}

function isLikelyMediaSegment(url: URL, extension: string): boolean {
  const filename = url.pathname.split("/").filter(Boolean).pop()?.toLowerCase() ?? "";
  if (!filename) {
    return false;
  }

  if (extension === "mp4") {
    return /^(?:init|segment|seg|frag|fragment|chunk)[-_]?\d*\.mp4$/u.test(filename);
  }

  return false;
}

function isLikelyAudioMediaUrl(url: URL): boolean {
  const pathname = decodeURIComponent(url.pathname).toLowerCase();
  return /(?:^|\/)(?:audio|mp4a)(?:\/|$)/u.test(pathname);
}

function suggestedFilenameForCandidate(
  candidate: DetectedDownloadCandidate,
  pageTitle: string | undefined,
): string | undefined {
  if (candidate.source === "hls" || candidate.source === "dash") {
    return pageTitle ?? filenameFromUrl(candidate.url);
  }

  return candidate.suggestedFilename ?? filenameFromUrl(candidate.url) ?? pageTitle;
}

function compareCandidates(left: StoredCandidate, right: StoredCandidate): number {
  return right.score - left.score || right.detectedAt - left.detectedAt;
}

function pruneOldCandidates(candidates: StoredCandidate[]): StoredCandidate[] {
  return candidates.filter((candidate) => {
    const maxAge = isStreamCandidate(candidate) ? MAX_STREAM_CANDIDATE_AGE_MS : MAX_CANDIDATE_AGE_MS;
    return Date.now() - candidate.detectedAt <= maxAge;
  });
}

function pruneOldRequestHeaders(headers: Map<string, StoredRequestHeaders>): Map<string, StoredRequestHeaders> {
  const now = Date.now();
  for (const [url, stored] of headers) {
    if (now - stored.detectedAt > MAX_REQUEST_HEADER_AGE_MS) {
      headers.delete(url);
    }
  }

  return headers;
}

function isCandidateForPage(candidate: StoredCandidate, pageUrl: string): boolean {
  if (candidate.pageUrl && !sameDocumentUrl(candidate.pageUrl, pageUrl)) {
    return false;
  }

  if (!candidate.initiator) {
    return Boolean(candidate.pageUrl);
  }

  try {
    const page = new URL(pageUrl);
    const initiator = new URL(candidate.initiator);
    if (initiator.origin !== page.origin) {
      return false;
    }

    if (candidate.pageUrl) {
      return true;
    }

    if (isOriginOnlyUrl(initiator)) {
      return Date.now() - candidate.detectedAt <= MAX_INITIATOR_ONLY_CANDIDATE_AGE_MS;
    }

    return sameDocumentUrl(initiator.href, page.href);
  } catch {
    return false;
  }
}

function isOriginOnlyUrl(url: URL): boolean {
  return url.pathname === "/" && !url.search && !url.hash;
}

function isStreamCandidate(candidate: DetectedDownloadCandidate): boolean {
  return candidate.source === "hls" || candidate.source === "dash";
}

function sameDocumentUrl(left: string, right: string): boolean {
  try {
    const leftUrl = new URL(left);
    const rightUrl = new URL(right);
    leftUrl.hash = "";
    rightUrl.hash = "";
    return leftUrl.href === rightUrl.href;
  } catch {
    return left === right;
  }
}
