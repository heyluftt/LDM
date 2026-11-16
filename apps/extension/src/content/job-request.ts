import type { CreateDownloadJobRequest, DetectedDownloadCandidate, RejectionCode } from "@ldm/contracts";
import { adapters } from "../adapters";
import { isMasterManifestUrlValue, qualityHintForCandidate } from "../shared/stream-quality";
import { buildCandidateDiagnostics, buildVideoStateDiagnostic, safeDisplayUrl } from "./candidate-diagnostics";
import { type DownloadRequestContext, prepareCandidateBeforeDownload } from "./candidate-selection";
import { isAccepted, isRejected, sendRuntimeMessage } from "./runtime-messaging";

/** Turns a chosen candidate into a job request and hands it to the desktop app. */

export async function createJobFromCandidate(candidate: DetectedDownloadCandidate): Promise<CreateDownloadJobRequest> {
  const adapter = adapters.find((item) => item.id === candidate.adapterId);
  if (!adapter) {
    throw new Error(`No adapter found for ${candidate.adapterId}`);
  }

  const options = await adapter.listOptions(candidate);
  const option = options[0];
  if (!option) {
    const reason = adapter.explainUnsupported(candidate) ?? "No download option is available.";
    throw new Error(reason);
  }

  return adapter.createDownloadJob(candidate, option);
}

export async function requestDownload(
  candidate: DetectedDownloadCandidate,
  context: DownloadRequestContext = {},
): Promise<{ ok: true } | { ok: false; message: string; code?: RejectionCode }> {
  try {
    const refreshedCandidate = await prepareCandidateBeforeDownload(candidate, context);
    context.setStatus?.("Sending to LDM...");
    const job = await createJobFromCandidate(refreshedCandidate);
    job.qualityReport = buildQualityReport(candidate, refreshedCandidate, context.debugCandidates, context.video);
    const response = await sendRuntimeMessage({ type: "ldm:createJob", job });
    if (isAccepted(response)) {
      return { ok: true };
    }

    if (isRejected(response)) {
      return { ok: false, message: response.reason, code: response.code };
    }
    return { ok: false, message: "LDM did not accept the download." };
  } catch (error) {
    return { ok: false, message: error instanceof Error ? error.message : "Download request failed." };
  }
}

function buildQualityReport(
  originalCandidate: DetectedDownloadCandidate,
  preparedCandidate: DetectedDownloadCandidate,
  debugCandidates: DetectedDownloadCandidate[] = [],
  video?: HTMLVideoElement,
): string {
  const candidates = [originalCandidate, preparedCandidate, ...debugCandidates];
  return JSON.stringify(
    {
      version: 1,
      selectedBeforePrepare: qualityCandidateSummary(originalCandidate),
      selectedAfterPrepare: qualityCandidateSummary(preparedCandidate),
      selectionChanged: originalCandidate.url !== preparedCandidate.url,
      player: buildVideoStateDiagnostic(video),
      candidates: buildCandidateDiagnostics(preparedCandidate, candidates),
      note: "Quality report omits query strings, header values, cookies, authorization, and local file paths.",
    },
    null,
    2,
  );
}

function qualityCandidateSummary(candidate: DetectedDownloadCandidate) {
  return {
    id: candidate.id,
    source: candidate.source,
    adapterId: candidate.adapterId,
    quality: qualityHintForCandidate(candidate),
    qualityHeight: candidate.qualityHeight ?? null,
    bitrate: candidate.bitrate ?? null,
    isMasterManifest: isMasterManifestUrlValue(candidate.url, window.location.href),
    url: safeDisplayUrl(candidate.url),
    hasQuery: candidateUrlHasQuery(candidate.url),
    fallbackCount: candidate.sourceFallbackUrls?.length ?? 0,
    contentType: candidate.contentType ?? null,
    contentLength: candidate.contentLength ?? null,
    hasHeaders: Boolean(candidate.headers && Object.keys(candidate.headers).length > 0),
  };
}

function candidateUrlHasQuery(value: string): boolean {
  try {
    return new URL(value, window.location.href).search.length > 0;
  } catch {
    return value.includes("?");
  }
}
