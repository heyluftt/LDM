import { afterEach, describe, expect, it, vi } from "vitest";
import { collectCandidates, extractMediaUrlsFromText } from "./media-detector";

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("embedded media detection", () => {
  it("extracts escaped HLS master URLs from player config text", () => {
    const urls = extractMediaUrlsFromText(
      '{"sources":[{"src":"https:\\/\\/cdn.example.test\\/video\\/master.m3u8?sig=abc\\u0026expires=123"}]}',
      "https://example.test/watch/owned",
    );

    expect(urls).toEqual(["https://cdn.example.test/video/master.m3u8?sig=abc&expires=123"]);
  });

  it("resolves root-relative stream manifests against the page URL", () => {
    const urls = extractMediaUrlsFromText(
      '{"hls":"/media/owned/playlist.m3u8?quality=auto"}',
      "https://example.test/watch/owned",
    );

    expect(urls).toEqual(["https://example.test/media/owned/playlist.m3u8?quality=auto"]);
  });

  it("ignores static GIF player resources from performance entries", () => {
    vi.stubGlobal("window", {
      location: { href: "https://example.test/watch/owned" },
    });
    vi.stubGlobal("performance", {
      now: () => 20,
      getEntriesByType: () => [
        {
          name: "https://example.test/engine/jwplayer/player.gif?quality=720p&mime=video%2Fmp4",
          startTime: 1,
          responseEnd: 2,
        },
      ],
    });

    const documentRef = {
      querySelectorAll: () => [],
    } as unknown as Document;

    expect(collectCandidates(documentRef, "https://example.test/watch/owned", "Owned video")).toEqual([]);
  });
});
