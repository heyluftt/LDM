import { createProviderAdapter } from "./provider";

export const xAdapter = createProviderAdapter({
  id: "provider-x",
  providerId: "x",
  displayName: "X",
  matchPage(pageUrl) {
    const statusUrl = xStatusUrl(pageUrl);
    return statusUrl ? { url: statusUrl } : null;
  },
});

function xStatusUrl(pageUrl: string): string | null {
  try {
    const url = new URL(pageUrl);
    const host = url.hostname.replace(/^www\./, "");
    const isXHost = host === "x.com" || host === "twitter.com";
    const isStatusPath = /^\/(?:[^/]+\/status|i\/web\/status)\/\d+/u.test(url.pathname);
    return isXHost && isStatusPath ? url.href : null;
  } catch {
    return null;
  }
}
