// The types of jobs, settings, and messages are generated from `crates/ldm-types` (ts-rs) into
// `./generated` whenever `cargo test` runs. To change one, change the Rust type and run the
// tests; what is written by hand in this file is only what exists on the TypeScript side.
import type { CreateDownloadJobRequest } from "./generated/CreateDownloadJobRequest";
import type { DetectedDownloadCandidate } from "./generated/DetectedDownloadCandidate";
import type { DetectionContext } from "./generated/DetectionContext";
import type { DownloadOption } from "./generated/DownloadOption";

export type { BrowserDownloadDomainRule } from "./generated/BrowserDownloadDomainRule";
export type { BrowserDownloadKinds } from "./generated/BrowserDownloadKinds";
export type { BrowserDownloadMode } from "./generated/BrowserDownloadMode";
export type { BrowserIntegrationPreferences } from "./generated/BrowserIntegrationPreferences";
export type { CreateDownloadJobRequest } from "./generated/CreateDownloadJobRequest";
export type { DesktopToExtensionMessage } from "./generated/DesktopToExtensionMessage";
export type { DetectedDownloadCandidate } from "./generated/DetectedDownloadCandidate";
export type { DetectionContext } from "./generated/DetectionContext";
export type { DetectionSource } from "./generated/DetectionSource";
export type { DownloadJob } from "./generated/DownloadJob";
export type { DownloadKind } from "./generated/DownloadKind";
export type { DownloadOption } from "./generated/DownloadOption";
export type { DownloadStatus } from "./generated/DownloadStatus";
export type { ExtensionContact } from "./generated/ExtensionContact";
export type { ExtensionToDesktopMessage } from "./generated/ExtensionToDesktopMessage";
export type { RejectionCode } from "./generated/RejectionCode";

export type ProviderAdapter = {
  id: string;
  displayName: string;
  detect(context: DetectionContext): Promise<DetectedDownloadCandidate[]>;
  listOptions(candidate: DetectedDownloadCandidate): Promise<DownloadOption[]>;
  createDownloadJob(candidate: DetectedDownloadCandidate, option: DownloadOption): Promise<CreateDownloadJobRequest>;
  explainUnsupported(candidate: DetectedDownloadCandidate): string | null;
};

export const SCHEMA_VERSION = 1 as const;

export const PLATFORM_USAGE_NOTICE =
  "You are responsible for making sure you are allowed to download this content. LDM does not get around DRM, paywalls, login requirements, or access controls.";

export function isPlatformProvider(providerId: string | undefined): boolean {
  return providerId === "youtube" || providerId === "x" || providerId === "tiktok";
}

/**
 * Classification of an adapter id, mirroring `ldm_types::adapter::AdapterKind`. Everything that
 * is not an HLS/DASH manifest or a `provider-*` page is a plain HTTP download run by the core.
 */
export type AdapterKind =
  | { type: "direct-http" }
  | { type: "hls" }
  | { type: "dash" }
  | { type: "provider"; provider: string };

export const HLS_ADAPTER_ID = "hls" as const;
export const DASH_ADAPTER_ID = "dash" as const;
export const PROVIDER_ADAPTER_ID_PREFIX = "provider-" as const;

export function adapterKindOf(adapterId: string): AdapterKind {
  if (adapterId === HLS_ADAPTER_ID) {
    return { type: "hls" };
  }
  if (adapterId === DASH_ADAPTER_ID) {
    return { type: "dash" };
  }
  if (adapterId.startsWith(PROVIDER_ADAPTER_ID_PREFIX)) {
    return { type: "provider", provider: adapterId.slice(PROVIDER_ADAPTER_ID_PREFIX.length) };
  }
  return { type: "direct-http" };
}

/** Plain HTTP download run by the core itself (`generic-*`, `browser-download`, unknown ids). */
export function isDirectHttpAdapter(adapterId: string): boolean {
  return adapterKindOf(adapterId).type === "direct-http";
}

/** Streams and provider pages go through the external backend (yt-dlp) or native HLS. */
export function usesExternalBackendAdapter(adapterId: string): boolean {
  return !isDirectHttpAdapter(adapterId);
}

/** Direct downloads and native HLS can be paused and resumed; DASH and provider jobs cannot. */
export function isPauseResumeAdapter(adapterId: string): boolean {
  const kind = adapterKindOf(adapterId).type;
  return kind === "direct-http" || kind === "hls";
}

/** HLS or DASH manifest. */
export function isStreamAdapter(adapterId: string): boolean {
  const kind = adapterKindOf(adapterId).type;
  return kind === "hls" || kind === "dash";
}
