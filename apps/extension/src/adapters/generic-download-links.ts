import type { DetectedDownloadCandidate, DownloadOption, ProviderAdapter } from "@ldm/contracts";
import { browserLikeVideoHeaders } from "./browser-headers";

export const genericDownloadLinksAdapter: ProviderAdapter = {
  id: "generic-download-links",
  displayName: "Download links",
  async detect() {
    return [];
  },
  async listOptions(candidate) {
    return [
      {
        id: "direct",
        label: "Direct download",
        url: candidate.url,
        kind: candidate.kind,
        contentType: candidate.contentType,
        estimatedBytes: candidate.contentLength,
      } satisfies DownloadOption,
    ];
  },
  async createDownloadJob(candidate, option) {
    return {
      sourceUrl: option.url,
      pageUrl: candidate.pageUrl,
      pageTitle: candidate.title,
      referrer: candidate.pageUrl,
      suggestedFilename: suggestedDownloadFilename(candidate, option),
      contentType: option.contentType,
      contentLength: option.estimatedBytes,
      kind: option.kind,
      adapterId: "generic-download-links",
      selectedOptionId: option.id,
      headers: option.kind === "video" ? browserLikeVideoHeaders(candidate) : candidate.headers,
      preferRangeRequest: option.kind === "video",
    };
  },
  explainUnsupported() {
    return null;
  },
};

function suggestedDownloadFilename(candidate: DetectedDownloadCandidate, option: DownloadOption): string | undefined {
  const suggestedFilename = candidate.suggestedFilename?.trim();
  if (option.kind === "video") {
    const title = candidate.title?.trim();
    if (title && isMeaningfulFilenameTitle(title, suggestedFilename)) {
      const extension = extensionFromUrl(candidate.url);
      return extension && !title.toLowerCase().endsWith(`.${extension}`) ? `${title}.${extension}` : title;
    }
  }

  return suggestedFilename || undefined;
}

function isMeaningfulFilenameTitle(title: string, suggestedFilename: string | undefined): boolean {
  if (title.length < 3 || !/[a-z0-9]/iu.test(title)) {
    return false;
  }

  return !suggestedFilename || title !== suggestedFilename;
}

function extensionFromUrl(value: string): string | undefined {
  try {
    const extension = new URL(value).pathname.match(/\.([a-z0-9]+)$/iu)?.[1]?.toLowerCase();
    return extension && extension.length <= 5 ? extension : undefined;
  } catch {
    return undefined;
  }
}
