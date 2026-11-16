import type { DownloadOption, ProviderAdapter } from "@ldm/contracts";

export const dashAdapter: ProviderAdapter = {
  id: "dash",
  displayName: "DASH stream",
  async detect() {
    return [];
  },
  async listOptions(candidate) {
    return [
      {
        id: "manifest",
        label: "DASH manifest",
        url: candidate.url,
        kind: "video",
        contentType: candidate.contentType ?? "application/dash+xml",
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
      adapterId: "dash",
      selectedOptionId: option.id,
      headers: candidate.headers,
    };
  },
  explainUnsupported() {
    return "DASH manifests are detected now; stream selection and muxing are planned as separate download core capabilities.";
  },
};

function streamSuggestedFilename(title: string | undefined, fallback: string | undefined): string | undefined {
  const trimmedTitle = title?.trim();
  if (trimmedTitle) {
    return trimmedTitle;
  }

  return fallback;
}
