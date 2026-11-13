import type { DetectedDownloadCandidate, RejectionCode } from "@ldm/contracts";

/** Messages between the content script and the background worker, with their reply guards. */

export function sendRuntimeMessage(message: unknown): Promise<unknown> {
  return new Promise((resolve) => {
    chrome.runtime.sendMessage(message, (response) => {
      const runtimeError = chrome.runtime.lastError;
      if (runtimeError) {
        resolve({ type: "jobRejected", reason: runtimeError.message ?? "Extension messaging failed." });
        return;
      }

      resolve(response);
    });
  });
}

export function isAccepted(value: unknown): value is { type: "jobAccepted"; jobId: string } {
  return typeof value === "object" && value !== null && "type" in value && value.type === "jobAccepted";
}

export function isRejected(value: unknown): value is { type: "jobRejected"; reason: string; code?: RejectionCode } {
  return (
    typeof value === "object" &&
    value !== null &&
    "type" in value &&
    value.type === "jobRejected" &&
    "reason" in value &&
    typeof value.reason === "string"
  );
}

function isNetworkCandidatesResponse(value: unknown): value is {
  type: "networkCandidates";
  candidates: DetectedDownloadCandidate[];
} {
  return (
    typeof value === "object" &&
    value !== null &&
    "type" in value &&
    value.type === "networkCandidates" &&
    "candidates" in value &&
    Array.isArray(value.candidates)
  );
}

export async function getNetworkCandidates(pageTitle: string): Promise<DetectedDownloadCandidate[]> {
  const response = await sendRuntimeMessage({
    type: "ldm:getNetworkCandidates",
    pageUrl: window.location.href,
    pageTitle,
  });

  if (isNetworkCandidatesResponse(response)) {
    return response.candidates;
  }

  return [];
}

/** Forgets the network candidates the background collected for this tab. */
export function clearNetworkCandidates(): Promise<unknown> {
  return sendRuntimeMessage({ type: "ldm:clearNetworkCandidates" });
}
