import type { DetectedDownloadCandidate, DownloadOption, ProviderAdapter } from "@ldm/contracts";
import { browserLikeVideoHeaders } from "./browser-headers";

export const genericHtml5VideoAdapter: ProviderAdapter = {
  id: "generic-html5-video",
  displayName: "HTML5 video",
  async detect() {
    return [];
  },
  async listOptions(candidate) {
    return [optionFromCandidate(candidate, "Original video")];
  },
  async createDownloadJob(candidate, option) {
    return {
      sourceUrl: option.url,
      pageUrl: candidate.pageUrl,
      pageTitle: candidate.title,
      referrer: candidate.pageUrl,
      suggestedFilename: suggestedVideoFilename(candidate),
      contentType: candidate.contentType,
      contentLength: candidate.contentLength,
      kind: option.kind,
      adapterId: "generic-html5-video",
      selectedOptionId: option.id,
      headers: browserLikeVideoHeaders(candidate),
      preferRangeRequest: true,
    };
  },
  explainUnsupported() {
    return null;
  },
};

function optionFromCandidate(candidate: DetectedDownloadCandidate, label: string): DownloadOption {
  return {
    id: "original",
    label,
    url: candidate.url,
    kind: "video",
    contentType: candidate.contentType,
    estimatedBytes: candidate.contentLength,
  };
}

function suggestedVideoFilename(candidate: DetectedDownloadCandidate): string | undefined {
  const title = candidate.title?.trim();
  if (!title) {
    return candidate.suggestedFilename;
  }

  const extension = extensionFromUrl(candidate.url);
  return extension && !title.toLowerCase().endsWith(`.${extension}`) ? `${title}.${extension}` : title;
}

function extensionFromUrl(value: string): string | undefined {
  try {
    const extension = new URL(value).pathname.match(/\.([a-z0-9]+)$/iu)?.[1]?.toLowerCase();
    return extension && extension.length <= 5 ? extension : undefined;
  } catch {
    return undefined;
  }
}
