import type { DownloadOption, ProviderAdapter } from "@ldm/contracts";
import { browserLikeHlsHeaders } from "./browser-headers";

export const hlsAdapter: ProviderAdapter = {
  id: "hls",
  displayName: "HLS stream",
  async detect() {
    return [];
  },
  async listOptions(candidate) {
    return [
      {
        id: "manifest",
        label: "HLS manifest",
        url: candidate.url,
        kind: "video",
        contentType: candidate.contentType ?? "application/vnd.apple.mpegurl",
        requiresExternalTool: true,
      } satisfies DownloadOption,
    ];
  },
  async createDownloadJob(candidate, option) {
    return {
      sourceUrl: option.url,
      sourceFallbackUrls: candidate.sourceFallbackUrls,
      pageUrl: candidate.pageUrl,
      pageTitle: candidate.title,
      referrer: candidate.pageUrl,
      suggestedFilename: streamSuggestedFilename(candidate.title, candidate.suggestedFilename),
      contentType: option.contentType,
      kind: "video",
      adapterId: "hls",
      selectedOptionId: option.id,
      headers: browserLikeHlsHeaders(candidate),
    };
  },
  explainUnsupported() {
    return "HLS manifests are detected now; stream selection and muxing are planned as separate download core capabilities.";
  },
};

function streamSuggestedFilename(title: string | undefined, fallback: string | undefined): string | undefined {
  const trimmedTitle = title?.trim();
  if (trimmedTitle) {
    return trimmedTitle;
  }

  return fallback;
}
