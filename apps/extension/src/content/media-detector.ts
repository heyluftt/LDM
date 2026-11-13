import type { DetectedDownloadCandidate, DownloadKind } from "@ldm/contracts";
import { qualityHeightForUrl, qualityHintForUrl } from "../shared/stream-quality";
import { filenameFromUrl, hashString, extensionForUrl as mediaExtensionForUrl } from "../shared/url";

const MEDIA_EXTENSIONS = new Map<string, DownloadKind>([
  ["mp4", "video"],
  ["m4v", "video"],
  ["webm", "video"],
  ["mkv", "video"],
  ["mov", "video"],
  ["m3u8", "video"],
  ["mpd", "video"],
  ["mp3", "audio"],
  ["m4a", "audio"],
  ["aac", "audio"],
  ["wav", "audio"],
  ["flac", "audio"],
  ["zip", "archive"],
  ["7z", "archive"],
  ["rar", "archive"],
  ["exe", "program"],
  ["msi", "program"],
  ["msix", "program"],
  ["appx", "program"],
  ["bat", "program"],
  ["cmd", "program"],
  ["ps1", "program"],
  ["scr", "program"],
  ["com", "program"],
  ["pdf", "document"],
  ["doc", "document"],
  ["docx", "document"],
  ["png", "image"],
  ["jpg", "image"],
  ["jpeg", "image"],
  ["webp", "image"],
]);

const MAX_RESOURCE_CANDIDATE_AGE_MS = 10 * 60 * 1000;
const MAX_STREAM_RESOURCE_CANDIDATE_AGE_MS = 90 * 1000;
const MAX_EMBEDDED_CONFIG_CHARS = 1_000_000;
const MAX_EMBEDDED_CANDIDATES = 20;
const MIN_VIDEO_WIDTH = 180;
const MIN_VIDEO_HEIGHT = 100;
const MIN_VISIBLE_VIDEO_WIDTH = 140;
const MIN_VISIBLE_VIDEO_HEIGHT = 56;

const isKnownMediaExtension = (extension: string): boolean => MEDIA_EXTENSIONS.has(extension);
const videoElementIds = new WeakMap<HTMLVideoElement, string>();
let nextVideoElementId = 0;

export function collectCandidates(
  documentRef: Document,
  pageUrl: string,
  pageTitle: string,
  options: { minResourceStartTime?: number } = {},
): DetectedDownloadCandidate[] {
  const pageCandidate = candidateFromUrl(pageUrl, "page-url", pageUrl, pageTitle);
  const videos = Array.from(documentRef.querySelectorAll("video"));
  const videoCandidates = videos.flatMap((video, index) =>
    collectVideoSources(video).map((url, sourceIndex) => ({
      id: `video-${index}-${sourceIndex}`,
      url,
      source: sourceForUrl(url),
      title: video.getAttribute("title") ?? pageTitle,
      pageUrl,
      suggestedFilename: suggestedFilenameForCandidate(url, pageTitle),
      adapterId: adapterForUrl(url, "generic-html5-video"),
      kind: kindForUrl(url),
    })),
  );

  return [
    ...(pageCandidate ? [pageCandidate] : []),
    ...videoCandidates,
    ...collectEmbeddedMediaCandidates(documentRef, pageUrl, pageTitle),
    ...collectPerformanceMediaCandidates(pageUrl, pageTitle, options.minResourceStartTime),
  ];
}

export type VideoDownloadTarget = {
  id: string;
  video: HTMLVideoElement;
  candidate: DetectedDownloadCandidate;
};

export type VideoCandidateResolver = (
  video: HTMLVideoElement,
  candidates: DetectedDownloadCandidate[],
  index: number,
) => DetectedDownloadCandidate | undefined;

export function collectVideoDownloadTargets(
  documentRef: Document,
  pageUrl: string,
  pageTitle: string,
  fallbackCandidate?: DetectedDownloadCandidate,
  resolveCandidate?: VideoCandidateResolver,
): VideoDownloadTarget[] {
  const videos = collectUsableVideoElements(documentRef);

  return videos.flatMap((video, index) => {
    const candidates = collectVideoSources(video).map((url, sourceIndex) => ({
      id: `video-${index}-${sourceIndex}`,
      url,
      source: sourceForUrl(url),
      title: video.getAttribute("title") ?? pageTitle,
      pageUrl,
      suggestedFilename: suggestedFilenameForCandidate(url, pageTitle),
      adapterId: adapterForUrl(url, "generic-html5-video"),
      kind: kindForUrl(url),
    }));

    const resolvedCandidate = resolveCandidate?.(video, candidates, index);
    const candidate = resolvedCandidate ?? candidates[0] ?? fallbackCandidate;
    if (!candidate) {
      return [];
    }

    return [{ id: `ldm-video-target-${videoElementId(video)}`, video, candidate }];
  });
}

export function collectUsableVideoElements(documentRef: Document): HTMLVideoElement[] {
  return dedupeOverlappingVideos(Array.from(documentRef.querySelectorAll("video")).filter(isUsableVideo));
}

function collectVideoSources(video: HTMLVideoElement): string[] {
  const urls = new Set<string>();
  if (video.currentSrc) {
    addHttpUrl(urls, video.currentSrc);
  }
  if (video.src) {
    addHttpUrl(urls, video.src);
  }
  video.querySelectorAll("source").forEach((source) => {
    const src = source.getAttribute("src");
    if (src) {
      addHttpUrl(urls, new URL(src, window.location.href).href);
    }
  });
  return Array.from(urls);
}

function addHttpUrl(urls: Set<string>, value: string): void {
  try {
    const url = new URL(value, window.location.href);
    if (url.protocol === "http:" || url.protocol === "https:") {
      urls.add(url.href);
    }
  } catch {
    // Ignore DOM media sources that cannot be handled by the desktop downloader.
  }
}

function isUsableVideo(video: HTMLVideoElement): boolean {
  if (!video.isConnected) {
    return false;
  }

  if (video.ownerDocument !== document) {
    return false;
  }

  const style = window.getComputedStyle(video);
  if (style.display === "none" || style.visibility === "hidden" || style.opacity === "0") {
    return false;
  }

  if (video.getClientRects().length === 0 || hasHiddenAncestor(video)) {
    return false;
  }

  const rect = video.getBoundingClientRect();
  if (rect.width < MIN_VIDEO_WIDTH || rect.height < MIN_VIDEO_HEIGHT) {
    return false;
  }

  if (isTransientPreviewVideo(video) && !isPreviewHovered(video)) {
    return false;
  }

  const visibleRect = visibleViewportRect(rect);
  return Boolean(
    visibleRect && visibleRect.width >= MIN_VISIBLE_VIDEO_WIDTH && visibleRect.height >= MIN_VISIBLE_VIDEO_HEIGHT,
  );
}

function videoElementId(video: HTMLVideoElement): string {
  const existing = videoElementIds.get(video);
  if (existing) {
    return existing;
  }

  nextVideoElementId += 1;
  const id = nextVideoElementId.toString(36);
  videoElementIds.set(video, id);
  return id;
}

function hasHiddenAncestor(video: HTMLVideoElement): boolean {
  for (let element: HTMLElement | null = video; element; element = element.parentElement) {
    if (element.hidden || element.inert || element.getAttribute("aria-hidden") === "true") {
      return true;
    }

    const style = window.getComputedStyle(element);
    if (style.display === "none" || style.visibility === "hidden" || style.opacity === "0") {
      return true;
    }

    if (element === document.body) {
      break;
    }
  }

  return false;
}

function isTransientPreviewVideo(video: HTMLVideoElement): boolean {
  return Boolean(youtubePreviewContainer(video));
}

function isPreviewHovered(video: HTMLVideoElement): boolean {
  const container = youtubePreviewContainer(video);
  return Boolean(video.matches(":hover") || container?.matches(":hover"));
}

function youtubePreviewContainer(video: HTMLVideoElement): HTMLElement | null {
  if (!isYouTubeHost(window.location.href)) {
    return null;
  }

  if (video.closest("#movie_player, ytd-player, ytd-reel-video-renderer")) {
    return null;
  }

  return video.closest<HTMLElement>(
    [
      "ytd-thumbnail",
      "a#thumbnail",
      "ytd-rich-grid-media",
      "ytd-rich-item-renderer",
      "ytd-video-renderer",
      "ytd-compact-video-renderer",
      "ytd-grid-video-renderer",
      "ytd-playlist-video-renderer",
    ].join(","),
  );
}

function isYouTubeHost(value: string): boolean {
  try {
    const host = new URL(value).hostname.replace(/^www\./, "");
    return host === "youtube.com" || host === "youtu.be";
  } catch {
    return false;
  }
}

function dedupeOverlappingVideos(videos: HTMLVideoElement[]): HTMLVideoElement[] {
  const selected: HTMLVideoElement[] = [];
  const ranked = videos
    .map((video, documentIndex) => ({
      video,
      documentIndex,
      rect: video.getBoundingClientRect(),
    }))
    .sort((left, right) => videoRectArea(right.rect) - videoRectArea(left.rect));

  for (const item of ranked) {
    if (selected.some((video) => areSameVisualVideo(video.getBoundingClientRect(), item.rect))) {
      continue;
    }

    selected.push(item.video);
  }

  return selected.sort((left, right) => videos.indexOf(left) - videos.indexOf(right));
}

function areSameVisualVideo(left: DOMRect, right: DOMRect): boolean {
  const intersection = rectIntersection(left, right);
  if (!intersection) {
    return false;
  }

  const smallerArea = Math.min(videoRectArea(left), videoRectArea(right));
  if (smallerArea <= 0) {
    return false;
  }

  return videoRectArea(intersection) / smallerArea >= 0.72;
}

function visibleViewportRect(rect: DOMRect): DOMRect | null {
  const viewportWidth = window.innerWidth || document.documentElement.clientWidth;
  const viewportHeight = window.innerHeight || document.documentElement.clientHeight;
  const left = Math.max(0, rect.left);
  const top = Math.max(0, rect.top);
  const right = Math.min(viewportWidth, rect.right);
  const bottom = Math.min(viewportHeight, rect.bottom);

  if (right <= left || bottom <= top) {
    return null;
  }

  return DOMRect.fromRect({
    x: left,
    y: top,
    width: right - left,
    height: bottom - top,
  });
}

function rectIntersection(left: DOMRect, right: DOMRect): DOMRect | null {
  const x = Math.max(left.left, right.left);
  const y = Math.max(left.top, right.top);
  const maxX = Math.min(left.right, right.right);
  const maxY = Math.min(left.bottom, right.bottom);

  if (maxX <= x || maxY <= y) {
    return null;
  }

  return DOMRect.fromRect({
    x,
    y,
    width: maxX - x,
    height: maxY - y,
  });
}

function videoRectArea(rect: DOMRect): number {
  return Math.max(0, rect.width) * Math.max(0, rect.height);
}

function sourceForUrl(url: string): "html5-video" | "hls" | "dash" {
  const extension = extensionForUrl(url);
  if (extension === "m3u8") {
    return "hls";
  }
  if (extension === "mpd") {
    return "dash";
  }
  return "html5-video";
}

function adapterForUrl(url: string, fallback: string): string {
  const extension = extensionForUrl(url);
  if (extension === "m3u8") {
    return "hls";
  }
  if (extension === "mpd") {
    return "dash";
  }
  return fallback;
}

function kindForUrl(url: string): DownloadKind {
  const extension = extensionForUrl(url);
  if (!extension) {
    return "video";
  }
  return MEDIA_EXTENSIONS.get(extension) ?? "video";
}

function extensionForUrl(url: string): string | undefined {
  try {
    return mediaExtensionForUrl(new URL(url, window.location.href), isKnownMediaExtension);
  } catch {
    return undefined;
  }
}

function candidateFromUrl(
  url: string,
  id: string,
  pageUrl: string,
  pageTitle: string,
): DetectedDownloadCandidate | null {
  const extension = extensionForUrl(url);
  if (!extension || !MEDIA_EXTENSIONS.has(extension)) {
    return null;
  }
  const source = sourceForUrl(url);
  const qualityLabel = qualityHintForUrl(url, pageUrl);

  return {
    id,
    url,
    source,
    title: pageTitle,
    pageUrl,
    suggestedFilename: suggestedFilenameForCandidate(url, pageTitle),
    adapterId: adapterForUrl(url, "generic-download-links"),
    kind: kindForUrl(url),
    qualityHeight: qualityHeightForUrl(url, pageUrl),
    qualityLabel: qualityLabel === "unknown" ? undefined : qualityLabel,
  };
}

function suggestedFilenameForCandidate(url: string, pageTitle: string): string | undefined {
  const source = sourceForUrl(url);
  if (source === "hls" || source === "dash") {
    return pageTitle || filenameFromUrl(url, window.location.href);
  }

  return filenameFromUrl(url, window.location.href);
}

function collectPerformanceMediaCandidates(
  pageUrl: string,
  pageTitle: string,
  minStartTime = 0,
): DetectedDownloadCandidate[] {
  if (typeof performance === "undefined") {
    return [];
  }

  const candidates = new Map<string, DetectedDownloadCandidate>();
  const now = performance.now();
  for (const entry of performance.getEntriesByType("resource")) {
    if (entry.startTime < minStartTime) {
      continue;
    }

    const candidate = candidateFromUrl(entry.name, `resource-${hashString(entry.name)}`, pageUrl, pageTitle);
    if (candidate && isFreshResourceCandidate(entry, candidate, now)) {
      candidates.set(candidate.url, candidate);
    }
  }

  return Array.from(candidates.values());
}

function collectEmbeddedMediaCandidates(
  documentRef: Document,
  pageUrl: string,
  pageTitle: string,
): DetectedDownloadCandidate[] {
  const candidates = new Map<string, DetectedDownloadCandidate>();
  let scannedChars = 0;

  const addUrlsFromText = (value: string | null | undefined, sourceId: string) => {
    if (!value || scannedChars >= MAX_EMBEDDED_CONFIG_CHARS || candidates.size >= MAX_EMBEDDED_CANDIDATES) {
      return;
    }

    const remainingChars = MAX_EMBEDDED_CONFIG_CHARS - scannedChars;
    const text = value.slice(0, remainingChars);
    scannedChars += text.length;

    for (const url of extractMediaUrlsFromText(text, pageUrl)) {
      const candidate = candidateFromUrl(url, `embedded-${sourceId}-${hashString(url)}`, pageUrl, pageTitle);
      if (candidate) {
        candidates.set(candidate.url, candidate);
        if (candidates.size >= MAX_EMBEDDED_CANDIDATES) {
          return;
        }
      }
    }
  };

  documentRef.querySelectorAll<HTMLScriptElement>("script:not([src])").forEach((script, index) => {
    const type = script.type.trim().toLowerCase();
    if (!type || type.includes("json") || type.includes("javascript") || type.includes("ld+json")) {
      addUrlsFromText(script.textContent, `script-${index}`);
    }
  });

  documentRef
    .querySelectorAll<HTMLElement>("[data-setup],[data-config],[data-video],[data-src],[data-url]")
    .forEach((element, index) => {
      for (const attributeName of ["data-setup", "data-config", "data-video", "data-src", "data-url"]) {
        addUrlsFromText(element.getAttribute(attributeName), `attr-${index}-${attributeName}`);
      }
    });

  return Array.from(candidates.values()).slice(0, MAX_EMBEDDED_CANDIDATES);
}

export function extractMediaUrlsFromText(text: string, baseUrl: string): string[] {
  const normalizedText = decodeCommonUrlEscapes(text);
  const urls = new Set<string>();
  const mediaUrlPattern = /(?:https?:\/\/|\/\/|\/)[^\s"'<>]+?\.(?:m3u8|mpd|mp4|m4v|webm)(?:\?[^\s"'<>]*)?/giu;

  for (const match of normalizedText.matchAll(mediaUrlPattern)) {
    const rawUrl = trimTrailingUrlPunctuation(match[0]);
    try {
      const resolvedUrl = new URL(rawUrl, baseUrl);
      if (resolvedUrl.protocol === "http:" || resolvedUrl.protocol === "https:") {
        urls.add(resolvedUrl.href);
      }
    } catch {
      // Ignore malformed config snippets.
    }
  }

  return Array.from(urls);
}

function decodeCommonUrlEscapes(value: string): string {
  return value
    .replace(/\\u002[fF]/gu, "/")
    .replace(/\\\//gu, "/")
    .replace(/&amp;/gu, "&")
    .replace(/\\u0026/gu, "&")
    .replace(/\\u003[dD]/gu, "=");
}

function trimTrailingUrlPunctuation(value: string): string {
  return value.replace(/[),.;\]}]+$/u, "");
}

function isFreshResourceCandidate(entry: PerformanceEntry, candidate: DetectedDownloadCandidate, now: number): boolean {
  const maxAge =
    candidate.source === "hls" || candidate.source === "dash"
      ? MAX_STREAM_RESOURCE_CANDIDATE_AGE_MS
      : MAX_RESOURCE_CANDIDATE_AGE_MS;
  const finishedAt =
    "responseEnd" in entry && typeof entry.responseEnd === "number" ? entry.responseEnd : entry.startTime;
  return now - finishedAt <= maxAge;
}
