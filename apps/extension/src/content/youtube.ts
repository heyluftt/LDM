import type { DetectedDownloadCandidate } from "@ldm/contracts";
import { cleanPageTitle, firstText } from "./page-title";

/**
 * YouTube specifics: telling the main player apart from hover previews, and building a provider
 * candidate for a preview so its thumbnail button downloads the previewed video.
 */

export function isPrimaryYouTubeVideo(video: HTMLVideoElement, pageUrl: string): boolean {
  if (!youtubeVideoIdFromUrl(pageUrl)) {
    return false;
  }

  const path = pathnameForUrl(pageUrl);
  if (path?.startsWith("/shorts/")) {
    return Boolean(video.closest("ytd-reel-video-renderer"));
  }

  return video.classList.contains("html5-main-video") || Boolean(video.closest("#movie_player"));
}

export function youtubeProviderCandidateForPreviewVideo(
  video: HTMLVideoElement,
  pageTitle: string,
): DetectedDownloadCandidate | undefined {
  if (!isYouTubePage(window.location.href) || isPrimaryYouTubeVideo(video, window.location.href)) {
    return undefined;
  }

  const container = youtubeVideoContainer(video);
  const videoUrl = container ? youtubeVideoUrlFromContainer(container) : undefined;
  const videoId = videoUrl ? youtubeVideoIdFromUrl(videoUrl) : undefined;
  if (!videoUrl || !videoId) {
    return undefined;
  }

  const title = (container ? youtubeTitleFromContainer(container) : undefined) ?? pageTitle;
  return {
    id: `youtube-preview-${videoId}`,
    url: videoUrl,
    source: "provider",
    title,
    pageUrl: videoUrl,
    suggestedFilename: title,
    kind: "video",
    adapterId: "provider-youtube",
    providerId: "youtube",
    requiresUsageNotice: true,
  };
}

function youtubeVideoContainer(video: HTMLVideoElement): HTMLElement | null {
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

function youtubeVideoUrlFromContainer(container: HTMLElement): string | undefined {
  const anchors = [
    container.matches("a[href]") ? (container as HTMLAnchorElement) : null,
    ...Array.from(container.querySelectorAll<HTMLAnchorElement>("a[href]")),
  ].filter((anchor): anchor is HTMLAnchorElement => Boolean(anchor));

  for (const anchor of anchors) {
    const canonicalUrl = canonicalYouTubeVideoUrl(anchor.href);
    if (canonicalUrl) {
      return canonicalUrl;
    }
  }

  return undefined;
}

export function canonicalYouTubeVideoUrl(value: string): string | undefined {
  try {
    const url = new URL(value, window.location.href);
    const host = url.hostname.replace(/^www\./, "");
    const videoId = youtubeVideoIdFromUrl(url.href);
    if (!videoId || (host !== "youtube.com" && host !== "youtu.be")) {
      return undefined;
    }

    if (url.pathname.startsWith("/shorts/")) {
      return `https://www.youtube.com/shorts/${videoId}`;
    }

    return `https://www.youtube.com/watch?v=${videoId}`;
  } catch {
    return undefined;
  }
}

function youtubeTitleFromContainer(container: HTMLElement): string | undefined {
  const titleElement = container.querySelector<HTMLElement>(
    "#video-title, a#video-title, h3 a, h3, yt-formatted-string",
  );
  const title = firstText([
    titleElement?.getAttribute("title"),
    titleElement?.textContent,
    container.getAttribute("title"),
    container.getAttribute("aria-label"),
  ]);

  return cleanPageTitle(title) || undefined;
}

export function isYouTubePage(pageUrl: string): boolean {
  try {
    const host = new URL(pageUrl).hostname.replace(/^www\./, "");
    return host === "youtube.com" || host === "youtu.be";
  } catch {
    return false;
  }
}

export function youtubeVideoIdFromUrl(pageUrl: string): string | undefined {
  try {
    const url = new URL(pageUrl, window.location.href);
    const host = url.hostname.replace(/^www\./, "");
    if (host === "youtu.be") {
      return url.pathname.split("/").filter(Boolean)[0];
    }

    if (host !== "youtube.com") {
      return undefined;
    }

    if (url.pathname === "/watch") {
      return url.searchParams.get("v") ?? undefined;
    }

    const pathMatch = url.pathname.match(/^\/(?:shorts|embed)\/([^/?#]+)/u);
    return pathMatch?.[1];
  } catch {
    return undefined;
  }
}

function pathnameForUrl(pageUrl: string): string | undefined {
  try {
    return new URL(pageUrl, window.location.href).pathname;
  } catch {
    return undefined;
  }
}
