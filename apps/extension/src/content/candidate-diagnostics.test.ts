import type { DetectedDownloadCandidate } from "@ldm/contracts";
import { describe, expect, it } from "vitest";
import { buildCandidateDiagnostics, qualityHintForUrl, safeDisplayUrl } from "./candidate-diagnostics";

describe("candidate diagnostics", () => {
  it("shows quality hints without exposing signed query strings", () => {
    expect(qualityHintForUrl("https://cdn.example.test/video/1920x1080/index-v1-a1.m3u8?sig=hidden")).toBe("1080p");
    expect(safeDisplayUrl("https://cdn.example.test/video/1920x1080/index-v1-a1.m3u8?sig=hidden")).toBe(
      "cdn.example.test/video/1920x1080/index-v1-a1.m3u8",
    );
    expect(qualityHintForUrl("https://cdn.example.test/video/720P_4000K_1.mp4/master.m3u8?sig=hidden")).toBe(
      "720p master",
    );
  });

  it("keeps the selected stream first in diagnostics", () => {
    const selected = candidate("embedded-master", "https://cdn.example.test/video/master.m3u8?sig=hidden", "hls");
    const diagnostics = buildCandidateDiagnostics(selected, [
      candidate("network-720", "https://cdn.example.test/video/1280x720/index.m3u8", "hls"),
      selected,
    ]);

    expect(diagnostics[0]).toMatchObject({
      selected: true,
      origin: "embedded",
      quality: "master",
      hasQuery: true,
      hasHeaders: false,
      contentType: null,
      contentLength: null,
    });
    expect(diagnostics[0]?.url).not.toContain("sig");
  });

  it("shows safe request metadata for candidate debugging", () => {
    const diagnostics = buildCandidateDiagnostics({
      ...candidate("network-mp4", "https://cdn.example.test/video.mp4?sig=hidden", "html5-video"),
      headers: { accept: "*/*", "user-agent": "Browser" },
      contentType: "video/mp4",
      contentLength: 1024,
    });

    expect(diagnostics[0]).toMatchObject({
      hasQuery: true,
      hasHeaders: true,
      contentType: "video/mp4",
      contentLength: 1024,
    });
    expect(diagnostics[0]?.url).not.toContain("hidden");
  });
});

function candidate(id: string, url: string, source: DetectedDownloadCandidate["source"]): DetectedDownloadCandidate {
  return {
    id,
    url,
    source,
    title: "Owned video",
    pageUrl: "https://example.test/watch",
    kind: "video",
    adapterId: source,
  };
}
