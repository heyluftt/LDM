import type { DetectedDownloadCandidate } from "@ldm/contracts";
import {
  isMasterManifestUrlValue,
  QUALITY_HEIGHT_UNIT,
  qualityHeightForUrl,
  STREAM_PIXELS_PER_POINT,
  scoreCandidateQuality,
  scoreStreamManifestUrl,
} from "../shared/stream-quality";
import { collectCandidates, collectUsableVideoElements } from "./media-detector";
import { currentPageTitle } from "./page-title";
import { clearNetworkCandidates, getNetworkCandidates } from "./runtime-messaging";
import { chooseXNetworkCandidateForUrls } from "./x-media-matcher";
import { isPrimaryYouTubeVideo, youtubeProviderCandidateForPreviewVideo } from "./youtube";

/**
 * Picks the candidate a video's download button should use, and refreshes it right before the
 * download so expiring or stale stream URLs are replaced by the current ones.
 *
 * Candidate ids encode where a candidate came from (`candidateOrigin`), and the origin decides
 * how far a candidate is trusted: a request the browser was seen making ranks above the DOM,
 * which ranks above URLs read out of player config text.
 */

const STREAM_PREPARE_TIMEOUT_MS = 6500;
const STREAM_PREPARE_POLL_MS = 350;
/** The desktop app tries these in order after the main URL; more than a handful only delays the error. */
const MAX_STREAM_FALLBACK_URLS = 5;

/**
 * Where a candidate came from, read from the id prefix its detector minted: `network-` (the
 * background's webRequest listener saw the browser request it), `resource-` (a
 * PerformanceResourceTiming entry), `video-` (`<video>`/`<source>` attributes), `embedded-`
 * (a URL found in player config text), `page-url` (the page itself).
 */
type CandidateOrigin = "network" | "resource" | "video" | "embedded" | "page" | "other";

function candidateOrigin(candidate: DetectedDownloadCandidate): CandidateOrigin {
  if (candidate.id.startsWith("network-")) {
    return "network";
  }
  if (candidate.id.startsWith("resource-")) {
    return "resource";
  }
  if (candidate.id.startsWith("video-")) {
    return "video";
  }
  if (candidate.id.startsWith("embedded-")) {
    return "embedded";
  }
  if (candidate.id === "page-url") {
    return "page";
  }
  return "other";
}

/** The browser requested this URL during the current page view, so it is what the player uses now. */
function isRequestOrigin(candidate: DetectedDownloadCandidate): boolean {
  const origin = candidateOrigin(candidate);
  return origin === "network" || origin === "resource";
}

/**
 * How current a stream URL is: 2 for a request the browser made during this page view, 1 for a
 * URL read from config text that may belong to an earlier or different player state, 0 for the
 * rest. `chooseBestStreamCandidate` only considers anything below 2 when nothing fresher exists.
 */
const STREAM_FRESHNESS: Record<CandidateOrigin, number> = {
  network: 2,
  resource: 2,
  embedded: 1,
  video: 0,
  page: 0,
  other: 0,
};

/** When two detectors report the same URL: a seen request beats the DOM, which beats config text. */
const MERGE_RANK: Record<CandidateOrigin, number> = {
  network: 3,
  resource: 3,
  video: 2,
  embedded: 1,
  page: 0,
  other: 0,
};

/*
 * Scores only order candidates against each other, so the absolute numbers carry no meaning;
 * what matters is how the terms trade off. Quality enters as points per pixel of height, so the
 * comments state each trade-off in pixels.
 */

/** Ranking of HLS/DASH manifests in `scoreGenericNetworkCandidate`. */
const STREAM_SCORE = {
  /** Above `DIRECT_SCORE.videoBase`: a manifest carries the whole quality ladder, a file one rendition. */
  base: 140,
  /** Per freshness step: a seen request beats config text by 120 points, the same as 360p. */
  freshnessWeight: 120,
  /**
   * Explicit height (from the manifest or the player) is reduced to the scale of
   * `scoreStreamManifestUrl` and capped at 1380p, so taller renditions stop gaining there and
   * freshness plus the master-manifest preference decide between them.
   */
  explicitQualityMax: 460,
};

/** Ranking of direct video files in `scoreGenericNetworkCandidate`. */
const DIRECT_SCORE = {
  videoBase: 100,
  otherBase: 80,
  /** Per freshness step: 80 points, the same as 640p, so a seen request almost always wins. */
  freshnessWeight: 80,
  /** URL height guesses count one point per 8 pixels, capped at 1280p. */
  pixelsPerPoint: 8,
  qualityMax: 160,
  /** One point per MB, capped: size separates otherwise equal files and nothing more. */
  sizeMaxMb: 40,
};

/**
 * Ranking of the candidates found while a download is being prepared, `preparedCandidateScore`.
 * The origin tier sits on top of the generic score and is spaced so that a seen request
 * (network) beats a timing entry (resource) unless that one is 450p taller, and either beats
 * config text (embedded) unless it is about 1260p taller.
 */
const PREPARED_SCORE = {
  tier: { network: 800, resource: 650, embedded: 350, video: 0, page: 0, other: 0 } satisfies Record<
    CandidateOrigin,
    number
  >,
  /** Evidence that the URL works as it is: the request's own headers, a content type, fallback URLs. */
  headers: 80,
  contentType: 40,
  fallbackUrls: 30,
};

export type DownloadRequestContext = {
  video?: HTMLVideoElement;
  setStatus?: (message: string) => void;
  debugCandidates?: DetectedDownloadCandidate[];
};

export function chooseGenericVideoCandidate(
  video: HTMLVideoElement,
  domVideoCandidates: DetectedDownloadCandidate[],
  networkCandidates: DetectedDownloadCandidate[],
  providerCandidates: DetectedDownloadCandidate[],
  usableVideoCount: number,
  pageTitle: string,
): DetectedDownloadCandidate | undefined {
  const platformCandidate = choosePlatformProviderCandidate(providerCandidates);
  if (platformCandidate && shouldUsePlatformProviderForVideo(video, platformCandidate)) {
    return platformCandidate;
  }

  const previewProviderCandidate = youtubeProviderCandidateForPreviewVideo(video, pageTitle);
  if (previewProviderCandidate) {
    return previewProviderCandidate;
  }

  const contextUrls = collectGenericVideoContextUrls(video);
  const matchingNetworkCandidate = chooseNetworkCandidateForUrls(contextUrls, networkCandidates);
  const bestStreamCandidate = chooseBestStreamCandidate(networkCandidates);
  if (
    bestStreamCandidate &&
    matchingNetworkCandidate &&
    shouldPreferStreamOverDirectCandidate(matchingNetworkCandidate, bestStreamCandidate, usableVideoCount)
  ) {
    return withStreamFallbackUrls(bestStreamCandidate, networkCandidates);
  }

  if (matchingNetworkCandidate) {
    return matchingNetworkCandidate;
  }

  if (bestStreamCandidate && usableVideoCount <= 1) {
    return withStreamFallbackUrls(bestStreamCandidate, networkCandidates);
  }

  const directDomCandidate = domVideoCandidates[0];
  if (directDomCandidate) {
    return directDomCandidate;
  }

  const downloadableNetworkCandidates = networkCandidates
    .filter(isDownloadableVideoCandidate)
    .filter((candidate) => isSafeGenericVideoFallbackCandidate(candidate, video))
    .sort(compareGenericNetworkCandidates);
  if (downloadableNetworkCandidates.length === 1 || usableVideoCount <= 1) {
    return downloadableNetworkCandidates[0];
  }

  return undefined;
}

function shouldUsePlatformProviderForVideo(video: HTMLVideoElement, candidate: DetectedDownloadCandidate): boolean {
  if (candidate.providerId === "youtube") {
    return isPrimaryYouTubeVideo(video, candidate.url);
  }

  return candidate.providerId === "tiktok";
}

export function mergeRequestCandidates(
  networkCandidates: DetectedDownloadCandidate[],
  documentCandidates: DetectedDownloadCandidate[],
): DetectedDownloadCandidate[] {
  const byUrl = new Map<string, DetectedDownloadCandidate>();
  for (const candidate of [...networkCandidates, ...documentCandidates.filter(isDocumentMediaCandidate)]) {
    const existing = byUrl.get(candidate.url);
    if (!existing || candidateMergeRank(candidate) >= candidateMergeRank(existing)) {
      byUrl.set(candidate.url, candidate);
    }
  }
  return Array.from(byUrl.values());
}

function candidateMergeRank(candidate: DetectedDownloadCandidate): number {
  return MERGE_RANK[candidateOrigin(candidate)];
}

function isDocumentMediaCandidate(candidate: DetectedDownloadCandidate): boolean {
  const origin = candidateOrigin(candidate);
  return (origin === "resource" || origin === "embedded") && isDownloadableVideoCandidate(candidate);
}

function choosePlatformProviderCandidate(
  providerCandidates: DetectedDownloadCandidate[],
): DetectedDownloadCandidate | undefined {
  return providerCandidates.find(
    (candidate) => candidate.providerId === "youtube" || candidate.providerId === "tiktok",
  );
}

export function chooseXVideoCandidate(
  video: HTMLVideoElement,
  domVideoCandidates: DetectedDownloadCandidate[],
  networkCandidates: DetectedDownloadCandidate[],
  providerCandidates: DetectedDownloadCandidate[],
): DetectedDownloadCandidate | undefined {
  return (
    chooseXNetworkCandidateForUrls(collectXVideoContextUrls(video), networkCandidates) ??
    domVideoCandidates[0] ??
    providerCandidateForXVideo(video, providerCandidates)
  );
}

function collectXVideoContextUrls(video: HTMLVideoElement): string[] {
  const urls = new Set<string>();
  addUrlCandidate(urls, video.poster);
  addUrlCandidate(urls, video.currentSrc);
  addUrlCandidate(urls, video.src);

  const container =
    video.closest<HTMLElement>("article") ??
    video.closest<HTMLElement>('[data-testid="cellInnerDiv"]') ??
    video.parentElement;

  container?.querySelectorAll<HTMLImageElement>("img").forEach((image) => {
    addUrlCandidate(urls, image.currentSrc);
    addUrlCandidate(urls, image.src);
    addSrcsetUrls(urls, image.srcset);
  });
  container?.querySelectorAll<HTMLVideoElement>("video").forEach((containerVideo) => {
    addUrlCandidate(urls, containerVideo.poster);
    addUrlCandidate(urls, containerVideo.currentSrc);
    addUrlCandidate(urls, containerVideo.src);
  });
  container?.querySelectorAll<HTMLSourceElement>("source").forEach((source) => {
    addUrlCandidate(urls, source.src);
    addSrcsetUrls(urls, source.srcset);
  });

  return Array.from(urls);
}

function collectGenericVideoContextUrls(video: HTMLVideoElement): string[] {
  const urls = new Set<string>();
  addUrlCandidate(urls, video.currentSrc);
  addUrlCandidate(urls, video.src);
  addUrlCandidate(urls, video.poster);
  video.querySelectorAll<HTMLSourceElement>("source").forEach((source) => {
    addUrlCandidate(urls, source.src);
    addSrcsetUrls(urls, source.srcset);
  });

  const container =
    video.closest<HTMLElement>("article") ??
    video.closest<HTMLElement>("figure") ??
    video.closest<HTMLElement>('[role="group"]') ??
    video.parentElement;
  container?.querySelectorAll<HTMLVideoElement>("video").forEach((containerVideo) => {
    addUrlCandidate(urls, containerVideo.currentSrc);
    addUrlCandidate(urls, containerVideo.src);
    addUrlCandidate(urls, containerVideo.poster);
  });
  container?.querySelectorAll<HTMLSourceElement>("source").forEach((source) => {
    addUrlCandidate(urls, source.src);
    addSrcsetUrls(urls, source.srcset);
  });

  return Array.from(urls);
}

function chooseNetworkCandidateForUrls(
  contextUrls: string[],
  networkCandidates: DetectedDownloadCandidate[],
): DetectedDownloadCandidate | undefined {
  const comparableUrls = new Set(contextUrls.map(normalizeComparableUrl).filter(Boolean));
  if (comparableUrls.size === 0) {
    return undefined;
  }

  return networkCandidates
    .filter(isDownloadableVideoCandidate)
    .filter((candidate) => comparableUrls.has(normalizeComparableUrl(candidate.url)))
    .sort(compareGenericNetworkCandidates)[0];
}

function isDownloadableVideoCandidate(candidate: DetectedDownloadCandidate): boolean {
  return candidate.kind === "video" && ["html5-video", "hls", "dash"].includes(candidate.source);
}

function isSafeGenericVideoFallbackCandidate(candidate: DetectedDownloadCandidate, video: HTMLVideoElement): boolean {
  if (candidate.source === "hls" || candidate.source === "dash") {
    return true;
  }

  if (candidate.source !== "html5-video") {
    return true;
  }

  if (candidateMatchesVideoContext(candidate, video)) {
    return true;
  }

  const origin = candidateOrigin(candidate);
  if (origin === "video") {
    return true;
  }

  if (origin === "embedded") {
    return false;
  }

  return !hasActivePlaybackEvidence(video);
}

export function compareGenericNetworkCandidates(
  left: DetectedDownloadCandidate,
  right: DetectedDownloadCandidate,
): number {
  return scoreGenericNetworkCandidate(right) - scoreGenericNetworkCandidate(left);
}

function scoreGenericNetworkCandidate(candidate: DetectedDownloadCandidate): number {
  if (candidate.source === "hls" || candidate.source === "dash") {
    return (
      STREAM_SCORE.base +
      streamFreshnessScore(candidate) * STREAM_SCORE.freshnessWeight +
      compactQualityScore(candidate)
    );
  }

  const sourceScore = candidate.source === "html5-video" ? DIRECT_SCORE.videoBase : DIRECT_SCORE.otherBase;
  const sizeScore = Math.min(Math.floor((candidate.contentLength ?? 0) / 1_000_000), DIRECT_SCORE.sizeMaxMb);
  const qualityScore = Math.min(
    DIRECT_SCORE.qualityMax,
    Math.round((qualityHeightForUrl(candidate.url, window.location.href) ?? 0) / DIRECT_SCORE.pixelsPerPoint),
  );
  return sourceScore + streamFreshnessScore(candidate) * DIRECT_SCORE.freshnessWeight + qualityScore + sizeScore;
}

/** Explicit quality on the scale of `scoreStreamManifestUrl`, which decides when nothing explicit is known. */
function compactQualityScore(candidate: DetectedDownloadCandidate): number {
  const explicitScore = scoreCandidateQuality(candidate, window.location.href);
  if (explicitScore > 0) {
    return Math.min(
      STREAM_SCORE.explicitQualityMax,
      Math.round(explicitScore / QUALITY_HEIGHT_UNIT / STREAM_PIXELS_PER_POINT),
    );
  }

  return scoreStreamManifestUrl(candidate.url, window.location.href);
}

export function chooseBestStreamCandidate(
  candidates: DetectedDownloadCandidate[],
): DetectedDownloadCandidate | undefined {
  const streamCandidates = candidates.filter(
    (candidate) => candidate.kind === "video" && (candidate.source === "hls" || candidate.source === "dash"),
  );
  const freshStreamCandidates = streamCandidates.filter(isRequestOrigin);
  const rankedCandidates = (freshStreamCandidates.length > 0 ? freshStreamCandidates : streamCandidates).sort(
    compareGenericNetworkCandidates,
  );
  const bestCandidate = rankedCandidates[0];
  if (!bestCandidate) {
    return undefined;
  }

  return choosePreferredMasterStreamCandidate(bestCandidate, rankedCandidates) ?? bestCandidate;
}

function choosePreferredStreamCandidateForCurrent(
  currentCandidate: DetectedDownloadCandidate,
  candidates: DetectedDownloadCandidate[],
  includeSingleVideoStreams: boolean,
  includeCurrentCandidate = true,
): DetectedDownloadCandidate {
  const relatedCandidates = relatedStreamCandidatesForCandidate(
    currentCandidate,
    candidates,
    includeSingleVideoStreams,
    includeCurrentCandidate,
  );
  const preferredMaster = choosePreferredMasterStreamCandidate(currentCandidate, relatedCandidates);
  if (preferredMaster) {
    return preferredMaster;
  }

  return chooseBestStreamCandidate(relatedCandidates) ?? currentCandidate;
}

function relatedStreamCandidatesForCandidate(
  currentCandidate: DetectedDownloadCandidate,
  candidates: DetectedDownloadCandidate[],
  includeSingleVideoStreams: boolean,
  includeCurrentCandidate = true,
): DetectedDownloadCandidate[] {
  const fallbackUrls = new Set(currentCandidate.sourceFallbackUrls ?? []);
  const relatedCandidates = candidates.filter((candidate) => {
    if (candidate.kind !== "video" || candidate.source !== currentCandidate.source) {
      return false;
    }

    return (
      candidate.url === currentCandidate.url ||
      fallbackUrls.has(candidate.url) ||
      areLikelySameStreamFamily(currentCandidate.url, candidate.url) ||
      includeSingleVideoStreams
    );
  });

  if (relatedCandidates.some((candidate) => candidate.url === currentCandidate.url)) {
    return relatedCandidates;
  }

  return includeCurrentCandidate ? [currentCandidate, ...relatedCandidates] : relatedCandidates;
}

function choosePreferredMasterStreamCandidate(
  currentCandidate: DetectedDownloadCandidate,
  candidates: DetectedDownloadCandidate[],
): DetectedDownloadCandidate | undefined {
  const currentHeight = candidateQualityHeight(currentCandidate);
  const masters = candidates.filter((candidate) => {
    if (
      candidate.source !== currentCandidate.source ||
      !isMasterManifestUrlValue(candidate.url, window.location.href)
    ) {
      return false;
    }

    const masterHeight = candidateQualityHeight(candidate);
    return currentHeight === 0 || masterHeight === 0 || masterHeight >= currentHeight;
  });

  return masters.sort(compareGenericNetworkCandidates)[0];
}

function candidateQualityHeight(candidate: DetectedDownloadCandidate): number {
  return candidate.qualityHeight ?? qualityHeightForUrl(candidate.url, window.location.href) ?? 0;
}

function areLikelySameStreamFamily(left: string, right: string): boolean {
  try {
    const leftUrl = new URL(left, window.location.href);
    const rightUrl = new URL(right, window.location.href);
    if (leftUrl.origin !== rightUrl.origin) {
      return false;
    }

    return streamDirectory(leftUrl) === streamDirectory(rightUrl);
  } catch {
    return false;
  }
}

function streamDirectory(url: URL): string {
  const parts = decodeURIComponent(url.pathname).split("/");
  parts.pop();
  return parts.join("/");
}

function shouldPreferStreamOverDirectCandidate(
  directCandidate: DetectedDownloadCandidate,
  streamCandidate: DetectedDownloadCandidate,
  usableVideoCount: number,
): boolean {
  if (usableVideoCount > 1 || directCandidate.source !== "html5-video") {
    return false;
  }

  if (streamCandidate.source !== "hls" && streamCandidate.source !== "dash") {
    return false;
  }

  if (!isReliableStreamCandidate(streamCandidate)) {
    return false;
  }

  return isLikelyExpiringBrowserCandidate(directCandidate);
}

function isReliableStreamCandidate(candidate: DetectedDownloadCandidate): boolean {
  if (isRequestOrigin(candidate)) {
    return true;
  }

  const hasForwardedHeaders = Boolean(candidate.headers && Object.keys(candidate.headers).length > 0);
  if (hasForwardedHeaders || candidate.contentType || candidate.qualityHeight || candidate.bitrate) {
    return true;
  }

  try {
    return new URL(candidate.url, window.location.href).search.length === 0;
  } catch {
    return false;
  }
}

function isLikelyExpiringBrowserCandidate(candidate: DetectedDownloadCandidate): boolean {
  if (!isRequestOrigin(candidate)) {
    return false;
  }

  const hasForwardedHeaders = Boolean(candidate.headers && Object.keys(candidate.headers).length > 0);
  if (hasForwardedHeaders || candidate.contentType || candidate.contentLength) {
    return false;
  }

  try {
    return new URL(candidate.url, window.location.href).search.length > 0;
  } catch {
    return false;
  }
}

export function withStreamFallbackUrls(
  candidate: DetectedDownloadCandidate,
  candidates: DetectedDownloadCandidate[],
): DetectedDownloadCandidate {
  if (candidate.source !== "hls" && candidate.source !== "dash") {
    return candidate;
  }

  const fallbackUrls = candidates
    .filter((item) => item.source === candidate.source && item.url !== candidate.url)
    .sort(compareStreamFallbackCandidates)
    .map((item) => item.url)
    .filter((url, index, urls) => urls.indexOf(url) === index)
    .slice(0, MAX_STREAM_FALLBACK_URLS);

  return fallbackUrls.length > 0 ? { ...candidate, sourceFallbackUrls: fallbackUrls } : candidate;
}

function compareStreamFallbackCandidates(left: DetectedDownloadCandidate, right: DetectedDownloadCandidate): number {
  return streamFreshnessScore(right) - streamFreshnessScore(left) || compareGenericNetworkCandidates(left, right);
}

export function streamFreshnessScore(candidate: DetectedDownloadCandidate): number {
  return STREAM_FRESHNESS[candidateOrigin(candidate)];
}

function normalizeComparableUrl(value: string): string {
  try {
    const url = new URL(value, window.location.href);
    url.hash = "";
    return url.href;
  } catch {
    return "";
  }
}

export function countUsableVideos(documentRef: Document): number {
  return collectUsableVideoElements(documentRef).length;
}

function providerCandidateForXVideo(
  video: HTMLVideoElement,
  providerCandidates: DetectedDownloadCandidate[],
): DetectedDownloadCandidate | undefined {
  const providerCandidate = providerCandidates.find((candidate) => candidate.providerId === "x");
  const providerStatusId = providerCandidate?.pageUrl ? xStatusId(providerCandidate.pageUrl) : undefined;
  if (!providerCandidate || !providerStatusId) {
    return undefined;
  }

  const container =
    video.closest<HTMLElement>("article") ??
    video.closest<HTMLElement>('[data-testid="cellInnerDiv"]') ??
    video.parentElement;
  const statusIds = new Set<string>();
  container?.querySelectorAll<HTMLAnchorElement>("a[href]").forEach((anchor) => {
    const href = anchor.getAttribute("href");
    if (!href) {
      return;
    }

    const statusId = xStatusId(new URL(href, window.location.href).href);
    if (statusId) {
      statusIds.add(statusId);
    }
  });

  return statusIds.has(providerStatusId) ? providerCandidate : undefined;
}

export function diagnosticCandidatesForVideo(
  video: HTMLVideoElement,
  selectedCandidate: DetectedDownloadCandidate,
  requestCandidates: DetectedDownloadCandidate[],
  providerCandidates: DetectedDownloadCandidate[],
  isX: boolean,
  usableVideoCount: number,
): DetectedDownloadCandidate[] {
  const contextUrls = isX ? collectXVideoContextUrls(video) : collectGenericVideoContextUrls(video);
  const comparableUrls = new Set(contextUrls.map(normalizeComparableUrl).filter(Boolean));
  const fallbackUrls = new Set(selectedCandidate.sourceFallbackUrls ?? []);
  const includeBroadStreamContext = usableVideoCount <= 1;
  const candidates = new Map<string, DetectedDownloadCandidate>();
  candidates.set(selectedCandidate.url, selectedCandidate);

  for (const candidate of requestCandidates) {
    const matchesSelected = candidate.url === selectedCandidate.url;
    const matchesFallback = fallbackUrls.has(candidate.url);
    const matchesContext = comparableUrls.has(normalizeComparableUrl(candidate.url));
    const matchesSingleVideoStream =
      includeBroadStreamContext && (candidate.source === "hls" || candidate.source === "dash");

    if (matchesSelected || matchesFallback || matchesContext || matchesSingleVideoStream) {
      candidates.set(candidate.url, candidate.url === selectedCandidate.url ? selectedCandidate : candidate);
    }
  }

  for (const candidate of providerCandidates) {
    if (candidate.url === selectedCandidate.url || candidate.providerId === selectedCandidate.providerId) {
      candidates.set(candidate.url, candidate.url === selectedCandidate.url ? selectedCandidate : candidate);
    }
  }

  return Array.from(candidates.values());
}

export async function prepareCandidateBeforeDownload(
  candidate: DetectedDownloadCandidate,
  context: DownloadRequestContext,
): Promise<DetectedDownloadCandidate> {
  const requiresFreshDiscovery = requiresFreshDiscoveryBeforeDownload(candidate, context.video);
  const includeCurrentCandidate = shouldIncludeCurrentCandidateDuringPrepare(
    candidate,
    context.video,
    requiresFreshDiscovery,
  );
  if (!requiresFreshDiscovery && !shouldPrepareMediaCandidate(candidate)) {
    return candidate;
  }

  context.setStatus?.(requiresFreshDiscovery ? "Finding current video stream..." : "Preparing stream...");
  if (requiresFreshDiscovery) {
    await beginFreshMediaDiscovery();
  }

  let bestCandidate = await refreshMediaCandidateBeforeDownload(candidate, {
    includeCurrentCandidate,
    video: context.video,
  });
  if (bestCandidate && isReadyPreparedCandidate(candidate, bestCandidate)) {
    return bestCandidate;
  }

  const deadline = Date.now() + STREAM_PREPARE_TIMEOUT_MS;
  while (Date.now() < deadline) {
    await sleep(STREAM_PREPARE_POLL_MS);
    const nextCandidate = await refreshMediaCandidateBeforeDownload(bestCandidate ?? candidate, {
      includeCurrentCandidate: includeCurrentCandidate || Boolean(bestCandidate),
      video: context.video,
    });
    if (
      nextCandidate &&
      (!bestCandidate || preparedCandidateScore(nextCandidate) > preparedCandidateScore(bestCandidate))
    ) {
      bestCandidate = nextCandidate;
    }
    if (bestCandidate && isReadyPreparedCandidate(candidate, bestCandidate)) {
      return bestCandidate;
    }
  }

  if (requiresFreshDiscovery) {
    if (hasActivePlaybackEvidence(context.video)) {
      throw new Error("Current video stream could not be captured yet. Keep the video playing, then click LDM again.");
    }
    throw new Error("Current video stream is not ready yet. Start playback once, then click LDM again.");
  }

  if (!includeCurrentCandidate && !bestCandidate) {
    throw new Error(
      hasActivePlaybackEvidence(context.video)
        ? "Current video stream could not be captured yet. Keep the video playing, then click LDM again."
        : "Current video stream is not ready yet. Start playback once, then click LDM again.",
    );
  }

  return bestCandidate ?? candidate;
}

function shouldIncludeCurrentCandidateDuringPrepare(
  candidate: DetectedDownloadCandidate,
  video: HTMLVideoElement | undefined,
  requiresFreshDiscovery: boolean,
): boolean {
  if (requiresFreshDiscovery) {
    return false;
  }

  if (isUnboundEmbeddedDirectVideoCandidate(candidate) && !candidateMatchesVideoContext(candidate, video)) {
    return false;
  }

  return true;
}

function shouldPrepareMediaCandidate(candidate: DetectedDownloadCandidate): boolean {
  if (candidate.kind !== "video") {
    return false;
  }

  return (
    candidate.source === "hls" ||
    candidate.source === "dash" ||
    isUnboundEmbeddedDirectVideoCandidate(candidate) ||
    isLikelyExpiringBrowserCandidate(candidate)
  );
}

function isUnboundEmbeddedDirectVideoCandidate(candidate: DetectedDownloadCandidate): boolean {
  if (candidate.source !== "html5-video" || candidateOrigin(candidate) !== "embedded") {
    return false;
  }

  try {
    const candidateUrl = new URL(candidate.url, window.location.href);
    const pageUrl = new URL(window.location.href);
    return candidateUrl.origin !== pageUrl.origin && !candidateMatchesAnyUsableVideoContext(candidate);
  } catch {
    return true;
  }
}

function candidateMatchesAnyUsableVideoContext(candidate: DetectedDownloadCandidate): boolean {
  return collectUsableVideoElements(document).some((video) => candidateMatchesVideoContext(candidate, video));
}

async function refreshMediaCandidateBeforeDownload(
  candidate: DetectedDownloadCandidate,
  options: { includeCurrentCandidate?: boolean; video?: HTMLVideoElement } = {},
): Promise<DetectedDownloadCandidate | undefined> {
  if (candidate.source === "hls" || candidate.source === "dash") {
    return refreshStreamCandidateBeforeDownload(candidate, options);
  }

  if (candidate.kind !== "video") {
    return candidate;
  }

  const pageUrl = window.location.href;
  const pageTitle = currentPageTitle();
  const documentCandidates = collectCandidates(document, pageUrl, pageTitle, {
    minResourceStartTime: currentResourceCandidateStartTime(),
  });
  const networkCandidates = await getNetworkCandidates(pageTitle);
  const requestCandidates = mergeRequestCandidates(networkCandidates, documentCandidates);
  const videoStreamCandidate = options.video
    ? chooseNetworkCandidateForUrls(collectGenericVideoContextUrls(options.video), requestCandidates)
    : undefined;
  if (videoStreamCandidate && shouldPreferPreparedVideoCandidate(candidate, videoStreamCandidate, options.video)) {
    return withStreamFallbackUrls(
      {
        ...videoStreamCandidate,
        title: candidate.title ?? videoStreamCandidate.title,
        pageUrl: candidate.pageUrl ?? videoStreamCandidate.pageUrl,
        suggestedFilename: candidate.suggestedFilename ?? videoStreamCandidate.suggestedFilename,
      },
      [videoStreamCandidate, ...requestCandidates],
    );
  }

  const bestStreamCandidate = chooseBestStreamCandidate(requestCandidates);
  if (bestStreamCandidate && shouldPreferPreparedStreamCandidate(candidate, bestStreamCandidate)) {
    return withStreamFallbackUrls(
      {
        ...bestStreamCandidate,
        title: candidate.title ?? bestStreamCandidate.title,
        pageUrl: candidate.pageUrl ?? bestStreamCandidate.pageUrl,
        suggestedFilename: candidate.suggestedFilename ?? bestStreamCandidate.suggestedFilename,
      },
      [bestStreamCandidate, ...requestCandidates],
    );
  }

  return options.includeCurrentCandidate === false ? undefined : candidate;
}

async function refreshStreamCandidateBeforeDownload(
  candidate: DetectedDownloadCandidate,
  options: { includeCurrentCandidate?: boolean; video?: HTMLVideoElement } = {},
): Promise<DetectedDownloadCandidate | undefined> {
  if (candidate.source !== "hls" && candidate.source !== "dash") {
    return candidate;
  }

  const pageUrl = window.location.href;
  const pageTitle = currentPageTitle();
  const documentCandidates = collectCandidates(document, pageUrl, pageTitle, {
    minResourceStartTime: currentResourceCandidateStartTime(),
  });
  const networkCandidates = await getNetworkCandidates(pageTitle);
  const requestCandidates = mergeRequestCandidates(networkCandidates, documentCandidates);
  const includeSingleVideoStreams = countUsableVideos(document) <= 1;
  const relatedCandidates = relatedStreamCandidatesForCandidate(
    candidate,
    requestCandidates,
    includeSingleVideoStreams,
    options.includeCurrentCandidate !== false,
  );
  if (relatedCandidates.length === 0) {
    return undefined;
  }
  const refreshedCandidate = choosePreferredStreamCandidateForCurrent(
    candidate,
    relatedCandidates,
    includeSingleVideoStreams,
    options.includeCurrentCandidate !== false,
  );

  return withStreamFallbackUrls(
    {
      ...refreshedCandidate,
      title: candidate.title ?? refreshedCandidate.title,
      pageUrl: candidate.pageUrl ?? refreshedCandidate.pageUrl,
      suggestedFilename: candidate.suggestedFilename ?? refreshedCandidate.suggestedFilename,
    },
    [refreshedCandidate, candidate, ...relatedCandidates],
  );
}

export function currentResourceCandidateStartTime(): number {
  return Number.parseFloat(document.documentElement.dataset.ldmResourceCandidateStartTime ?? "0") || 0;
}

export function setResourceCandidateStartTime(value: number): void {
  if (document.documentElement) {
    document.documentElement.dataset.ldmResourceCandidateStartTime = value.toString();
  }
}

export function currentPerformanceTime(): number {
  return typeof performance === "undefined" ? 0 : performance.now();
}

async function beginFreshMediaDiscovery(): Promise<void> {
  setResourceCandidateStartTime(currentPerformanceTime());
  // Keep discovery passive. Calling video.load() can reset custom players into a loading state.
  await clearNetworkCandidates();
}

function requiresFreshDiscoveryBeforeDownload(
  candidate: DetectedDownloadCandidate,
  video: HTMLVideoElement | undefined,
): boolean {
  if (!video || candidate.source === "provider") {
    return false;
  }

  if (hasActivePlaybackEvidence(video)) {
    return false;
  }

  if (isUnboundEmbeddedDirectVideoCandidate(candidate)) {
    return true;
  }

  const origin = candidateOrigin(candidate);
  if (origin === "video" || origin === "page") {
    return false;
  }

  if (candidateMatchesVideoContext(candidate, video)) {
    return false;
  }

  if (origin === "embedded") {
    return candidate.source === "html5-video";
  }

  return isRequestOrigin(candidate);
}

function candidateMatchesVideoContext(
  candidate: DetectedDownloadCandidate,
  video: HTMLVideoElement | undefined,
): boolean {
  if (!video) {
    return false;
  }

  const contextUrls = new Set(collectGenericVideoContextUrls(video).map(normalizeComparableUrl).filter(Boolean));
  if (contextUrls.size === 0) {
    return false;
  }

  return [candidate.url, ...(candidate.sourceFallbackUrls ?? [])]
    .map(normalizeComparableUrl)
    .some((url) => url && contextUrls.has(url));
}

function hasActivePlaybackEvidence(video: HTMLVideoElement | undefined): boolean {
  if (!video) {
    return false;
  }

  return (
    video.readyState > HTMLMediaElement.HAVE_NOTHING ||
    video.currentTime > 0 ||
    !video.paused ||
    video.networkState === HTMLMediaElement.NETWORK_LOADING
  );
}

function shouldPreferPreparedStreamCandidate(
  currentCandidate: DetectedDownloadCandidate,
  streamCandidate: DetectedDownloadCandidate,
): boolean {
  if (streamCandidate.source !== "hls" && streamCandidate.source !== "dash") {
    return false;
  }

  if (currentCandidate.source === "hls" || currentCandidate.source === "dash") {
    return preparedCandidateScore(streamCandidate) > preparedCandidateScore(currentCandidate);
  }

  return isLikelyExpiringBrowserCandidate(currentCandidate) || isReadyStreamCandidate(streamCandidate);
}

function shouldPreferPreparedVideoCandidate(
  currentCandidate: DetectedDownloadCandidate,
  nextCandidate: DetectedDownloadCandidate,
  video: HTMLVideoElement | undefined,
): boolean {
  if (nextCandidate.source === "hls" || nextCandidate.source === "dash") {
    return shouldPreferPreparedStreamCandidate(currentCandidate, nextCandidate);
  }

  if (nextCandidate.source !== "html5-video" || !candidateMatchesVideoContext(nextCandidate, video)) {
    return false;
  }

  return isUnboundEmbeddedDirectVideoCandidate(currentCandidate) || isLikelyExpiringBrowserCandidate(currentCandidate);
}

function isReadyPreparedCandidate(
  originalCandidate: DetectedDownloadCandidate,
  candidate: DetectedDownloadCandidate,
): boolean {
  if (candidate.source === "hls" || candidate.source === "dash") {
    return isReadyStreamCandidate(candidate);
  }

  if (originalCandidate.source === "hls" || originalCandidate.source === "dash") {
    return false;
  }

  return !isLikelyExpiringBrowserCandidate(candidate);
}

function isReadyStreamCandidate(candidate: DetectedDownloadCandidate): boolean {
  if (isRequestOrigin(candidate)) {
    return Boolean(
      candidate.qualityHeight ||
        candidate.bitrate ||
        candidate.contentType ||
        candidate.headers ||
        (candidate.sourceFallbackUrls?.length ?? 0) > 0,
    );
  }

  return Boolean(
    candidate.qualityHeight ||
      candidate.bitrate ||
      candidate.contentType ||
      (candidate.sourceFallbackUrls?.length ?? 0) > 0,
  );
}

export function preparedCandidateScore(candidate: DetectedDownloadCandidate): number {
  const tierScore = PREPARED_SCORE.tier[candidateOrigin(candidate)];
  const evidenceScore =
    (candidate.headers ? PREPARED_SCORE.headers : 0) +
    (candidate.contentType ? PREPARED_SCORE.contentType : 0) +
    ((candidate.sourceFallbackUrls?.length ?? 0) > 0 ? PREPARED_SCORE.fallbackUrls : 0);

  return tierScore + evidenceScore + scoreGenericNetworkCandidate(candidate);
}

function sleep(delayMs: number): Promise<void> {
  return new Promise((resolve) => window.setTimeout(resolve, delayMs));
}

export function isXPage(pageUrl: string): boolean {
  try {
    const host = new URL(pageUrl).hostname.replace(/^www\./, "");
    return host === "x.com" || host === "twitter.com";
  } catch {
    return false;
  }
}

function xStatusId(pageUrl: string): string | undefined {
  try {
    const url = new URL(pageUrl, window.location.href);
    const host = url.hostname.replace(/^www\./, "");
    if (host !== "x.com" && host !== "twitter.com") {
      return undefined;
    }

    return url.pathname.match(/\/status\/(\d+)/u)?.[1];
  } catch {
    return undefined;
  }
}

function addUrlCandidate(urls: Set<string>, value: string | undefined): void {
  if (!value) {
    return;
  }

  try {
    urls.add(new URL(value, window.location.href).href);
  } catch {
    // Ignore non-URL DOM values.
  }
}

function addSrcsetUrls(urls: Set<string>, srcset: string | undefined): void {
  if (!srcset) {
    return;
  }

  for (const entry of srcset.split(",")) {
    const url = entry.trim().split(/\s+/u)[0];
    addUrlCandidate(urls, url);
  }
}
