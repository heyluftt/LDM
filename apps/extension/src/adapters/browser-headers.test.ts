import type { DetectedDownloadCandidate } from "@ldm/contracts";
import { afterEach, describe, expect, it, vi } from "vitest";
import { browserLikeHlsHeaders, browserLikeVideoHeaders } from "./browser-headers";

describe("browser-like media headers", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("adds safe HLS defaults without forwarding sensitive headers", () => {
    vi.stubGlobal("navigator", {
      languages: ["de-DE", "en-US"],
      language: "de-DE",
      userAgent: "Browser UA",
    });

    const headers = browserLikeHlsHeaders({
      id: "hls",
      url: "https://cdn.example.test/master.m3u8",
      source: "hls",
      pageUrl: "https://example.test/watch",
      kind: "video",
      adapterId: "hls",
      headers: {
        Cookie: "blocked=true",
        Authorization: "Bearer blocked",
        Range: "bytes=1-",
        Accept: "application/x-mpegurl",
      },
    } satisfies DetectedDownloadCandidate);

    expect(headers).toEqual({
      accept: "application/x-mpegurl",
      "accept-language": "de-DE,en-US",
      origin: "https://example.test",
      "user-agent": "Browser UA",
    });
  });

  it("keeps video headers minimal when browser globals are unavailable", () => {
    vi.stubGlobal("navigator", undefined);

    const headers = browserLikeVideoHeaders({
      id: "video",
      url: "https://cdn.example.test/video.mp4",
      source: "html5-video",
      kind: "video",
      adapterId: "generic-html5-video",
    } satisfies DetectedDownloadCandidate);

    expect(headers).toEqual({ accept: "*/*" });
  });
});
