import type { DetectedDownloadCandidate, DownloadOption, ProviderAdapter } from "@ldm/contracts";

/** What a provider's `matchPage` returns for a page it handles. */
export type ProviderPageMatch = {
  /** The URL the desktop app's provider backend should fetch; usually the page URL itself. */
  url: string;
  /** Stable id of the media on that page, when the URL reveals one; part of the candidate id. */
  mediaId?: string;
};

/**
 * Builds the adapter for a provider page (YouTube, X, TikTok): one candidate per matching page,
 * one option, and a job the desktop app hands to its external provider backend.
 */
export function createProviderAdapter({
  displayName,
  id,
  matchPage,
  providerId,
}: {
  id: string;
  providerId: string;
  displayName: string;
  matchPage: (pageUrl: string) => ProviderPageMatch | null;
}): ProviderAdapter {
  const optionLabel = `${displayName} video`;

  return {
    id,
    displayName,
    async detect(context) {
      const match = matchPage(context.pageUrl);
      if (!match) {
        return [];
      }

      const candidate: DetectedDownloadCandidate = {
        id: `${providerId}-${match.mediaId ?? "page"}`,
        url: match.url,
        source: "provider",
        title: context.pageTitle,
        pageUrl: match.url,
        kind: "video",
        adapterId: id,
        providerId,
        requiresUsageNotice: true,
      };
      return [candidate];
    },
    async listOptions(candidate) {
      const option: DownloadOption = {
        id: "provider-default",
        label: optionLabel,
        url: candidate.url,
        kind: "video",
        requiresExternalTool: true,
      };
      return [option];
    },
    async createDownloadJob(candidate, option) {
      return {
        sourceUrl: option.url,
        pageUrl: candidate.pageUrl,
        pageTitle: candidate.title,
        referrer: candidate.pageUrl,
        suggestedFilename: candidate.title ?? optionLabel,
        kind: option.kind,
        adapterId: candidate.adapterId,
        providerId: candidate.providerId,
        selectedOptionId: option.id,
      };
    },
    explainUnsupported() {
      return null;
    },
  };
}
