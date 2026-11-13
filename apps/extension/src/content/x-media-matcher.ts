import type { DetectedDownloadCandidate } from "@ldm/contracts";
import { scoreStreamManifestUrl } from "../shared/stream-quality";

const X_MEDIA_ID_PATTERN =
  /\/(?:ext_tw_video|ext_tw_video_thumb|amplify_video|amplify_video_thumb|tweet_video|tweet_video_thumb)\/(\d+)/giu;

export function chooseXNetworkCandidateForUrls(
  contextUrls: string[],
  networkCandidates: DetectedDownloadCandidate[],
): DetectedDownloadCandidate | undefined {
  const contextMediaIds = mediaIdsFromUrls(contextUrls);
  if (contextMediaIds.size === 0) {
    return undefined;
  }

  return networkCandidates
    .filter((candidate) => candidate.kind === "video")
    .filter((candidate) => intersects(contextMediaIds, extractXMediaIdsFromUrl(candidate.url)))
    .sort(compareXNetworkCandidates)[0];
}

export function extractXMediaIdsFromUrl(value: string): string[] {
  let url: URL;
  try {
    url = new URL(value, currentPageUrl());
  } catch {
    return [];
  }

  if (!isXMediaHost(url.hostname)) {
    return [];
  }

  const mediaIds = new Set<string>();
  const haystack = decodeURIComponent(url.pathname);
  for (const match of haystack.matchAll(X_MEDIA_ID_PATTERN)) {
    if (match[1]) {
      mediaIds.add(match[1]);
    }
  }

  return Array.from(mediaIds);
}

function currentPageUrl(): string {
  return typeof window === "undefined" ? "https://x.com/" : window.location.href;
}

function mediaIdsFromUrls(urls: string[]): Set<string> {
  const ids = new Set<string>();
  for (const url of urls) {
    for (const mediaId of extractXMediaIdsFromUrl(url)) {
      ids.add(mediaId);
    }
  }
  return ids;
}

function isXMediaHost(hostname: string): boolean {
  const host = hostname.replace(/^www\./u, "");
  return host === "video.twimg.com" || host === "pbs.twimg.com";
}

function intersects(left: Set<string>, right: string[]): boolean {
  return right.some((value) => left.has(value));
}

function compareXNetworkCandidates(left: DetectedDownloadCandidate, right: DetectedDownloadCandidate): number {
  return scoreXNetworkCandidate(right) - scoreXNetworkCandidate(left);
}

function scoreXNetworkCandidate(candidate: DetectedDownloadCandidate): number {
  if (candidate.source === "hls" || candidate.source === "dash") {
    return 100 + scoreStreamManifestUrl(candidate.url, currentPageUrl());
  }
  if (candidate.source === "html5-video") {
    return 50 + Math.min(Math.floor((candidate.contentLength ?? 0) / 1_000_000), 20);
  }
  return 0;
}
