import type { DetectedDownloadCandidate, DownloadKind } from "@ldm/contracts";
import { filenameFromUrl } from "../shared/url";

/**
 * Link clicks are intercepted only on explicit file signals: a known file extension in the
 * URL path or in a filename query parameter, or a `download` attribute. Image extensions are
 * the exception: a browser displays a linked image instead of downloading it, and galleries
 * open their large view from such links, so an image link counts only with a `download`
 * attribute (its kind stays `image` so the routing rules for images apply). Everything else is
 * left to the browser; if the browser then starts a download, the background routing picks it
 * up. Text heuristics ("Download", "Installer") used to live here and blocked ordinary links.
 */
const LINK_KINDS = new Map<string, DownloadKind>([
  ["mp4", "video"],
  ["webm", "video"],
  ["mkv", "video"],
  ["m3u8", "video"],
  ["mpd", "video"],
  ["mp3", "audio"],
  ["m4a", "audio"],
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
  ["pdf", "document"],
  ["png", "image"],
  ["jpg", "image"],
  ["jpeg", "image"],
  ["webp", "image"],
]);

export function candidateFromDownloadLink(
  anchor: HTMLAnchorElement,
  pageUrl: string,
  id = "link-click",
): DetectedDownloadCandidate | null {
  const href = anchor.getAttribute("href");
  if (!href) {
    return null;
  }

  let url: string;
  try {
    url = new URL(href, pageUrl).href;
  } catch {
    return null;
  }
  if (!url.startsWith("http:") && !url.startsWith("https:")) {
    return null;
  }

  const urlExtension = extensionForUrl(url);
  const queryFilename = filenameFromQuery(url);
  const queryExtension = extensionForFilename(queryFilename);
  const extension = urlExtension ?? queryExtension;
  const kind: DownloadKind = extension ? (LINK_KINDS.get(extension) ?? "other") : "other";
  const downloadAttribute = cleanAttribute(anchor.getAttribute("download"));
  const hasDownloadAttribute = anchor.hasAttribute("download");
  // An image extension alone is no file signal: the browser shows the image instead of
  // downloading it, and a gallery thumbnail links to the full image without wanting a download.
  const hasFileSignal = extension !== undefined && kind !== "image";
  if (!hasFileSignal && !hasDownloadAttribute) {
    return null;
  }

  const downloadName =
    downloadAttribute ||
    (urlExtension ? filenameFromUrl(url) : undefined) ||
    queryFilename ||
    (extension ? `download.${extension}` : undefined);
  const source = extension === "m3u8" ? "hls" : extension === "mpd" ? "dash" : "download-link";
  const adapterId = extension === "m3u8" ? "hls" : extension === "mpd" ? "dash" : "generic-download-links";

  return {
    id,
    url,
    source,
    pageUrl,
    title: meaningfulLinkText(anchor) || downloadName,
    suggestedFilename: downloadName,
    adapterId,
    kind,
  };
}

function meaningfulLinkText(anchor: HTMLAnchorElement): string | undefined {
  const text = anchor.textContent?.replace(/\s+/gu, " ").trim();
  if (!text || text.length < 3 || !/[a-z0-9]/iu.test(text)) {
    return undefined;
  }

  return text;
}

function cleanAttribute(value: string | null | undefined): string | undefined {
  const text = value?.replace(/\s+/gu, " ").trim();
  return text || undefined;
}

function extensionForUrl(url: string): string | undefined {
  try {
    return extensionForFilename(new URL(url).pathname);
  } catch {
    return undefined;
  }
}

function extensionForFilename(value: string | undefined): string | undefined {
  const match = value?.match(/\.([a-z0-9]{1,8})$/iu);
  const extension = match?.[1]?.toLowerCase();
  return extension && LINK_KINDS.has(extension) ? extension : undefined;
}

function filenameFromQuery(url: string): string | undefined {
  try {
    const searchParams = new URL(url).searchParams;
    for (const key of ["filename", "file", "download", "name"]) {
      const value = cleanAttribute(searchParams.get(key));
      const filename = value?.split(/[\\/]/u).filter(Boolean).pop();
      if (extensionForFilename(filename)) {
        return filename;
      }
    }
  } catch {
    return undefined;
  }

  return undefined;
}
