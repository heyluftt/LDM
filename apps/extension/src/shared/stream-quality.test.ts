import { describe, expect, it } from "vitest";
import { qualityHintForUrl, scoreCandidateQuality, scoreStreamManifestUrl } from "./stream-quality";

describe("stream quality helpers", () => {
  it("detects visible rendition quality from stream URLs", () => {
    expect(qualityHintForUrl("https://cdn.example.test/hls/video/1080P_8000K_file.mp4/master.m3u8")).toBe(
      "1080p master",
    );
    expect(qualityHintForUrl("https://cdn.example.test/hls/video/720P_4000K_file.mp4/index-v1-a1.m3u8")).toBe("720p");
  });

  it("scores master and higher quality streams above lower renditions", () => {
    const rendition720 = "https://cdn.example.test/hls/video/720P_4000K_file.mp4/index-v1-a1.m3u8";
    const master1080 = "https://cdn.example.test/hls/video/1080P_8000K_file.mp4/master.m3u8";

    expect(scoreStreamManifestUrl(master1080)).toBeGreaterThan(scoreStreamManifestUrl(rendition720));
  });

  it("uses explicit candidate quality before URL guesses", () => {
    expect(
      scoreCandidateQuality({
        id: "network-hls-test",
        url: "https://cdn.example.test/hls/video/720P_4000K_file.mp4/index-v1-a1.m3u8",
        source: "hls",
        title: "Owned video",
        pageUrl: "https://example.test/watch",
        kind: "video",
        adapterId: "hls",
        qualityHeight: 1080,
        bitrate: 8_000_000,
      }),
    ).toBeGreaterThan(1_080_000_000);
  });
});
