import type { DetectedDownloadCandidate } from "@ldm/contracts";
import { describe, expect, it } from "vitest";
import { chooseXNetworkCandidateForUrls, extractXMediaIdsFromUrl } from "./x-media-matcher";

describe("X media matching", () => {
  it("extracts media IDs from X thumbnail and video URLs", () => {
    expect(extractXMediaIdsFromUrl("https://pbs.twimg.com/ext_tw_video_thumb/1234567890/pu/img/example.jpg")).toEqual([
      "1234567890",
    ]);
    expect(extractXMediaIdsFromUrl("https://video.twimg.com/ext_tw_video/1234567890/pu/pl/playlist.m3u8")).toEqual([
      "1234567890",
    ]);
  });

  it("chooses the candidate matching the clicked video's media ID", () => {
    const chosen = chooseXNetworkCandidateForUrls(
      ["https://pbs.twimg.com/ext_tw_video_thumb/111/pu/img/poster.jpg"],
      [
        networkCandidate("222", "https://video.twimg.com/ext_tw_video/222/pu/vid/avc1/wrong.mp4"),
        networkCandidate("111", "https://video.twimg.com/ext_tw_video/111/pu/vid/avc1/right.mp4"),
      ],
    );

    expect(chosen?.id).toBe("111");
  });

  it("prefers a matching manifest over direct media so video and audio can be merged", () => {
    const chosen = chooseXNetworkCandidateForUrls(
      ["https://pbs.twimg.com/ext_tw_video_thumb/111/pu/img/poster.jpg"],
      [
        {
          ...networkCandidate("manifest", "https://video.twimg.com/ext_tw_video/111/pu/pl/master.m3u8"),
          source: "hls",
          adapterId: "hls",
        },
        networkCandidate("direct", "https://video.twimg.com/ext_tw_video/111/pu/vid/avc1/video.mp4"),
      ],
    );

    expect(chosen?.id).toBe("manifest");
  });

  it("does not choose audio-only candidates for an X video button", () => {
    const chosen = chooseXNetworkCandidateForUrls(
      ["https://pbs.twimg.com/ext_tw_video_thumb/111/pu/img/poster.jpg"],
      [
        {
          ...networkCandidate("audio", "https://video.twimg.com/ext_tw_video/111/pu/audio/audio.mp4"),
          kind: "audio",
        },
        {
          ...networkCandidate("manifest", "https://video.twimg.com/ext_tw_video/111/pu/pl/master.m3u8"),
          source: "hls",
          adapterId: "hls",
        },
      ],
    );

    expect(chosen?.id).toBe("manifest");
  });
});

function networkCandidate(id: string, url: string): DetectedDownloadCandidate {
  return {
    id,
    url,
    source: "html5-video",
    title: "Post",
    pageUrl: "https://x.com/example/status/1",
    suggestedFilename: "Post",
    kind: "video",
    adapterId: "generic-html5-video",
    contentLength: 2_000_000,
  };
}
