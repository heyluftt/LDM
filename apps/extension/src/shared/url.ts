/**
 * URL, filename and extension helpers shared by the background worker and the content scripts.
 *
 * Which extensions count as "media" differs per caller (the request sniffer only knows media
 * and stream extensions, the page detector also classifies archives, programs and documents),
 * so the extension helpers take an `isKnownExtension` predicate instead of a fixed list.
 */

export type ExtensionPredicate = (extension: string) => boolean;

/** Path extensions that are never media even when a query string hints otherwise. */
export const STATIC_ASSET_PATH_EXTENSIONS: ReadonlySet<string> = new Set([
  "gif",
  "svg",
  "avif",
  "ico",
  "css",
  "js",
  "json",
]);

/** Short, stable, non-cryptographic hash used to build candidate and overlay ids. */
export function hashString(value: string): string {
  let hash = 0;
  for (let index = 0; index < value.length; index += 1) {
    hash = Math.imul(31, hash) + value.charCodeAt(index);
  }
  return Math.abs(hash).toString(36);
}

/** The decoded last path segment of `url`, or undefined when the URL does not parse. */
export function filenameFromUrl(url: string, base?: string): string | undefined {
  try {
    const pathname = new URL(url, base).pathname;
    const filename = pathname.split("/").filter(Boolean).pop();
    return filename ? decodeURIComponent(filename) : undefined;
  } catch {
    return undefined;
  }
}

/** `value` lower-cased when `isKnownExtension` accepts it. */
export function normalizeMediaExtension(
  value: string | undefined,
  isKnownExtension: ExtensionPredicate,
): string | undefined {
  if (!value) {
    return undefined;
  }

  const extension = value.toLowerCase();
  return isKnownExtension(extension) ? extension : undefined;
}

export function hasStaticAssetPathExtension(value: string | undefined): boolean {
  return Boolean(value && STATIC_ASSET_PATH_EXTENSIONS.has(value.toLowerCase()));
}

/**
 * The media extension of a URL: from its path unless that is a static asset, otherwise from
 * MIME hints or extension-like tokens in the query string.
 */
export function extensionForUrl(url: URL, isKnownExtension: ExtensionPredicate): string | undefined {
  const match = url.pathname.match(/\.([a-z0-9]+)$/i);
  if (hasStaticAssetPathExtension(match?.[1])) {
    return undefined;
  }

  const pathnameExtension = normalizeMediaExtension(match?.[1], isKnownExtension);
  if (pathnameExtension) {
    return pathnameExtension;
  }

  return extensionFromQuery(url.searchParams, isKnownExtension);
}

/** A media extension implied by query parameters (`mime=video/mp4`, `format=m3u8`, ...). */
export function extensionFromQuery(params: URLSearchParams, isKnownExtension: ExtensionPredicate): string | undefined {
  for (const [key, value] of params) {
    const haystack = `${key}=${value}`.toLowerCase();
    if (haystack.includes("mpegurl") || haystack.includes("m3u8")) {
      return "m3u8";
    }
    if (haystack.includes("dash") || haystack.includes("mpd")) {
      return "mpd";
    }
    if (haystack.includes("video/mp4")) {
      return "mp4";
    }
    if (haystack.includes("video/webm")) {
      return "webm";
    }
    if (haystack.includes("audio/mp4")) {
      return "m4a";
    }
    for (const match of haystack.matchAll(/(?:^|[=/?&._-])([a-z0-9]{2,5})(?=$|[/?&._-])/gu)) {
      const extension = normalizeMediaExtension(match[1], isKnownExtension);
      if (extension) {
        return extension;
      }
    }
  }

  return undefined;
}

/** The MIME type without parameters, lower-cased; undefined when empty. */
export function normalizeContentType(value: string | undefined): string | undefined {
  return value?.split(";")[0]?.trim().toLowerCase() || undefined;
}
