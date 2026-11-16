import { describe, expect, it } from "vitest";
import {
  adapterKindOf,
  isDirectHttpAdapter,
  isPauseResumeAdapter,
  isPlatformProvider,
  isStreamAdapter,
  PLATFORM_USAGE_NOTICE,
  usesExternalBackendAdapter,
} from "./index";

describe("platform provider contracts", () => {
  it("identifies guarded platform providers", () => {
    expect(isPlatformProvider("youtube")).toBe(true);
    expect(isPlatformProvider("x")).toBe(true);
    expect(isPlatformProvider("tiktok")).toBe(true);
    expect(isPlatformProvider("generic")).toBe(false);
  });

  it("keeps the usage notice explicit", () => {
    expect(PLATFORM_USAGE_NOTICE).toContain("allowed to download");
    expect(PLATFORM_USAGE_NOTICE).toContain("DRM");
  });
});

describe("adapter classification", () => {
  it("parses known adapter ids", () => {
    expect(adapterKindOf("hls")).toEqual({ type: "hls" });
    expect(adapterKindOf("dash")).toEqual({ type: "dash" });
    expect(adapterKindOf("provider-youtube")).toEqual({ type: "provider", provider: "youtube" });
    expect(adapterKindOf("provider-x")).toEqual({ type: "provider", provider: "x" });
  });

  it("treats direct and unknown adapter ids as direct http", () => {
    for (const id of [
      "generic-html5-video",
      "generic-download-links",
      "browser-download",
      "something-new",
      "",
      "HLS",
    ]) {
      expect(adapterKindOf(id), id).toEqual({ type: "direct-http" });
      expect(isDirectHttpAdapter(id), id).toBe(true);
      expect(usesExternalBackendAdapter(id), id).toBe(false);
      expect(isPauseResumeAdapter(id), id).toBe(true);
      expect(isStreamAdapter(id), id).toBe(false);
    }
  });

  it("classifies streams and providers", () => {
    expect(isDirectHttpAdapter("hls")).toBe(false);
    expect(isDirectHttpAdapter("dash")).toBe(false);
    expect(isDirectHttpAdapter("provider-tiktok")).toBe(false);

    expect(usesExternalBackendAdapter("hls")).toBe(true);
    expect(usesExternalBackendAdapter("dash")).toBe(true);
    expect(usesExternalBackendAdapter("provider-tiktok")).toBe(true);

    expect(isPauseResumeAdapter("hls")).toBe(true);
    expect(isPauseResumeAdapter("dash")).toBe(false);
    expect(isPauseResumeAdapter("provider-tiktok")).toBe(false);

    expect(isStreamAdapter("hls")).toBe(true);
    expect(isStreamAdapter("dash")).toBe(true);
    expect(isStreamAdapter("provider-tiktok")).toBe(false);
  });
});
