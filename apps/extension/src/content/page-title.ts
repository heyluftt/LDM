/** The title shown for downloads from the current page. */

export function currentPageTitle(): string {
  const visibleTitle = firstText(
    ["h1.ytd-watch-metadata yt-formatted-string", "h1.ytd-watch-metadata", "#title h1 yt-formatted-string", "h1"]
      .map((selector) => document.querySelector(selector)?.textContent)
      .filter(Boolean),
  );
  const metadataTitle = firstText([
    document.querySelector<HTMLMetaElement>('meta[property="og:title"]')?.content,
    document.querySelector<HTMLMetaElement>('meta[name="twitter:title"]')?.content,
  ]);

  return cleanPageTitle(visibleTitle || metadataTitle || document.title || "Video");
}

export function firstText(values: Array<string | null | undefined>): string {
  return values.map((value) => value?.trim()).find((value): value is string => Boolean(value)) ?? "";
}

export function cleanPageTitle(value: string): string {
  return value.replace(/\s+-\s+YouTube$/u, "").trim() || "Video";
}
