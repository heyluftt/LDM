import { createProviderAdapter } from "./provider";

export const tiktokAdapter = createProviderAdapter({
  id: "provider-tiktok",
  providerId: "tiktok",
  displayName: "TikTok",
  matchPage(pageUrl) {
    return isTikTokUrl(pageUrl) ? { url: pageUrl } : null;
  },
});

function isTikTokUrl(pageUrl: string): boolean {
  try {
    const host = new URL(pageUrl).hostname.replace(/^www\./, "");
    return host === "tiktok.com" || host.endsWith(".tiktok.com");
  } catch {
    return false;
  }
}
