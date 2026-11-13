import { createProviderAdapter } from "./provider";

export const youtubeAdapter = createProviderAdapter({
  id: "provider-youtube",
  providerId: "youtube",
  displayName: "YouTube",
  matchPage(pageUrl) {
    const videoId = youtubeVideoId(pageUrl);
    return videoId ? { url: pageUrl, mediaId: videoId } : null;
  },
});

function youtubeVideoId(pageUrl: string): string | undefined {
  try {
    const url = new URL(pageUrl);
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
    return url.pathname.match(/^\/(?:shorts|embed)\/([^/?#]+)/u)?.[1];
  } catch {
    return undefined;
  }
}
