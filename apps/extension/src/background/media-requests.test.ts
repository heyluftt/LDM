import { afterEach, describe, expect, it, vi } from "vitest";
import {
  candidateFromNetworkRequest,
  clearTabMediaRequests,
  getMediaCandidatesForTab,
  rememberMediaRequest,
  rememberMediaRequestHeaders,
} from "./media-requests";

afterEach(() => {
  vi.useRealTimers();
  clearTabMediaRequests(41);
  clearTabMediaRequests(42);
});

describe("network media request detection", () => {
  it("detects direct X video URLs as generic video downloads", () => {
    const candidate = candidateFromNetworkRequest({
      url: "https://video.twimg.com/ext_tw_video/1/pu/vid/avc1/1280x720/test.mp4?tag=12",
      pageUrl: "https://x.com/example/status/1",
      pageTitle: "Owned post",
      contentType: "video/mp4",
    });

    expect(candidate).toMatchObject({
      source: "html5-video",
      adapterId: "generic-html5-video",
      kind: "video",
      suggestedFilename: "test.mp4",
    });
  });

  it("detects HLS and DASH manifests without accepting media segments", () => {
    expect(
      candidateFromNetworkRequest({
        url: "https://example.test/master.m3u8",
        contentType: "application/vnd.apple.mpegurl",
      }),
    ).toMatchObject({ source: "hls", adapterId: "hls" });

    expect(
      candidateFromNetworkRequest({
        url: "https://example.test/manifest.mpd",
        contentType: "application/dash+xml",
      }),
    ).toMatchObject({ source: "dash", adapterId: "dash" });

    expect(candidateFromNetworkRequest({ url: "https://example.test/chunk-1.m4s" })).toBeNull();
  });

  it("rejects tiny direct video responses that are likely fragments or probes", () => {
    expect(
      candidateFromNetworkRequest({
        url: "https://video.twimg.com/ext_tw_video/1/pu/vid/avc1/1280x720/init.mp4",
        contentType: "video/mp4",
        contentLength: 1536,
      }),
    ).toBeNull();

    expect(
      candidateFromNetworkRequest({
        url: "https://video.twimg.com/ext_tw_video/1/pu/vid/avc1/1280x720/full.mp4",
        contentType: "video/mp4",
        contentLength: 2_000_000,
      }),
    ).toMatchObject({ source: "html5-video", kind: "video" });
  });

  it("classifies audio-only MP4 responses as audio even when the extension is mp4", () => {
    expect(
      candidateFromNetworkRequest({
        url: "https://video.twimg.com/amplify_video/1/pu/audio/128000/audio.mp4",
        contentType: "video/mp4",
        contentLength: 2_000_000,
      }),
    ).toMatchObject({ source: "html5-video", kind: "audio" });

    expect(
      candidateFromNetworkRequest({
        url: "https://video.twimg.com/amplify_video/1/pu/vid/mp4a/128000/audio.mp4",
        contentType: "video/mp4",
        contentLength: 2_000_000,
      }),
    ).toMatchObject({ source: "html5-video", kind: "audio" });

    expect(
      candidateFromNetworkRequest({
        url: "https://video.twimg.com/ext_tw_video/1/pu/vid/avc1/1280x720/video.mp4",
        contentType: "audio/mp4",
        contentLength: 2_000_000,
      }),
    ).toMatchObject({ source: "html5-video", kind: "audio" });
  });

  it("detects media hints from query parameters when paths are opaque", () => {
    expect(
      candidateFromNetworkRequest({
        url: "https://cdn.example.test/playback?id=1&mime=video%2Fmp4",
        contentType: "application/octet-stream",
        contentLength: 2_000_000,
      }),
    ).toMatchObject({ source: "html5-video", kind: "video" });

    expect(
      candidateFromNetworkRequest({
        url: "https://cdn.example.test/stream?id=1&type=application%2Fvnd.apple.mpegurl",
        contentType: "application/octet-stream",
      }),
    ).toMatchObject({ source: "hls", adapterId: "hls" });
  });

  it("rejects static GIF player resources even when query parameters look like video", () => {
    expect(
      candidateFromNetworkRequest({
        url: "https://example.test/engine/jwplayer/player.gif?quality=720p&mime=video%2Fmp4",
        contentType: "application/octet-stream",
        contentLength: 10,
      }),
    ).toBeNull();
  });

  it("attaches only safe browser request headers to network candidates", () => {
    rememberMediaRequestHeaders({
      tabId: 41,
      url: "https://cdn.example.test/video.mp4",
      requestHeaders: [
        { name: "User-Agent", value: "Browser UA" },
        { name: "Accept-Language", value: "de-DE" },
        { name: "Referer", value: "https://example.test/watch/owned" },
        { name: "Cookie", value: "blocked=true" },
        { name: "Authorization", value: "Bearer blocked" },
        { name: "Range", value: "bytes=100-" },
      ],
    });

    rememberMediaRequest({
      tabId: 41,
      url: "https://cdn.example.test/video.mp4",
      documentUrl: "https://example.test/watch/owned",
      responseHeaders: [
        { name: "content-type", value: "video/mp4" },
        { name: "content-length", value: "2000000" },
      ],
    });

    expect(getMediaCandidatesForTab(41, "https://example.test/watch/owned")[0]?.headers).toEqual({
      "accept-language": "de-DE",
      referer: "https://example.test/watch/owned",
      "user-agent": "Browser UA",
    });
  });

  it("expires stream manifests quickly because signed URLs can go stale", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-01-01T00:00:00Z"));

    rememberMediaRequest({
      tabId: 41,
      url: "https://cdn.example.test/master.m3u8",
      documentUrl: "https://example.test/watch/owned",
      responseHeaders: [{ name: "content-type", value: "application/vnd.apple.mpegurl" }],
    });

    expect(getMediaCandidatesForTab(41, "https://example.test/watch/owned")).toHaveLength(1);

    vi.setSystemTime(new Date("2026-01-01T00:02:00Z"));

    expect(getMediaCandidatesForTab(41, "https://example.test/watch/owned")).toEqual([]);
  });

  it("does not reuse candidates from another document URL on the same origin", () => {
    rememberMediaRequest({
      tabId: 42,
      url: "https://cdn.example.test/master.m3u8",
      documentUrl: "https://example.test/watch/old",
      responseHeaders: [{ name: "content-type", value: "application/vnd.apple.mpegurl" }],
    });

    expect(getMediaCandidatesForTab(42, "https://example.test/watch/new")).toEqual([]);
  });

  it("expires origin-only candidates quickly so old player streams are not reused", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-01-01T00:00:00Z"));

    rememberMediaRequest({
      tabId: 42,
      url: "https://cdn.example.test/master.m3u8",
      initiator: "https://example.test",
      responseHeaders: [{ name: "content-type", value: "application/vnd.apple.mpegurl" }],
    });

    expect(getMediaCandidatesForTab(42, "https://example.test/watch/current")).toHaveLength(1);

    vi.setSystemTime(new Date("2026-01-01T00:00:16Z"));

    expect(getMediaCandidatesForTab(42, "https://example.test/watch/current")).toEqual([]);
  });

  it("does not reuse exact initiator candidates on a different same-origin page", () => {
    rememberMediaRequest({
      tabId: 42,
      url: "https://cdn.example.test/master.m3u8",
      initiator: "https://example.test/watch/old",
      responseHeaders: [{ name: "content-type", value: "application/vnd.apple.mpegurl" }],
    });

    expect(getMediaCandidatesForTab(42, "https://example.test/watch/new")).toEqual([]);
  });

  it("prefers master stream manifests over recently loaded rendition manifests", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-01-01T00:00:00Z"));

    rememberMediaRequest({
      tabId: 41,
      url: "https://cdn.example.test/video/master.m3u8",
      documentUrl: "https://example.test/watch/owned",
      responseHeaders: [{ name: "content-type", value: "application/vnd.apple.mpegurl" }],
    });

    vi.setSystemTime(new Date("2026-01-01T00:00:10Z"));

    rememberMediaRequest({
      tabId: 41,
      url: "https://cdn.example.test/video/1280x720/index-v1-a1.m3u8",
      documentUrl: "https://example.test/watch/owned",
      responseHeaders: [{ name: "content-type", value: "application/vnd.apple.mpegurl" }],
    });

    expect(getMediaCandidatesForTab(41, "https://example.test/watch/owned")[0]?.url).toBe(
      "https://cdn.example.test/video/master.m3u8",
    );
  });

  it("prefers the highest visible stream rendition when no master is known", () => {
    rememberMediaRequest({
      tabId: 42,
      url: "https://cdn.example.test/video/1280x720/index-v1-a1.m3u8",
      documentUrl: "https://example.test/watch/owned",
      responseHeaders: [{ name: "content-type", value: "application/vnd.apple.mpegurl" }],
    });
    rememberMediaRequest({
      tabId: 42,
      url: "https://cdn.example.test/video/1920x1080/index-v1-a1.m3u8",
      documentUrl: "https://example.test/watch/owned",
      responseHeaders: [{ name: "content-type", value: "application/vnd.apple.mpegurl" }],
    });

    expect(getMediaCandidatesForTab(42, "https://example.test/watch/owned")[0]?.url).toBe(
      "https://cdn.example.test/video/1920x1080/index-v1-a1.m3u8",
    );
  });
});
