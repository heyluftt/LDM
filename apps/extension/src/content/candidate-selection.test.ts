import type { DetectedDownloadCandidate } from "@ldm/contracts";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  chooseBestStreamCandidate,
  chooseGenericVideoCandidate,
  mergeRequestCandidates,
  preparedCandidateScore,
  streamFreshnessScore,
  withStreamFallbackUrls,
} from "./candidate-selection";

const PAGE_URL = "https://example.test/watch/owned";

beforeEach(() => {
  vi.stubGlobal("window", { location: { href: PAGE_URL } });
});

afterEach(() => {
  vi.unstubAllGlobals();
});

/** A detached `<video>` stand-in: no sources, no container. */
function fakeVideo(): HTMLVideoElement {
  return {
    getAttribute: () => null,
    currentSrc: "",
    src: "",
    poster: "",
    classList: { contains: () => false },
    closest: () => null,
    querySelectorAll: () => [],
    parentElement: null,
  } as unknown as HTMLVideoElement;
}

function streamCandidate(
  id: string,
  url: string,
  extra: Partial<DetectedDownloadCandidate> = {},
): DetectedDownloadCandidate {
  return {
    id,
    url,
    source: "hls",
    kind: "video",
    adapterId: "hls",
    pageUrl: PAGE_URL,
    ...extra,
  };
}

describe("candidate selection", () => {
  it("ranks a seen request above a stream read from player config text", () => {
    const embedded = streamCandidate("embedded-config-1", "https://cdn.example.test/v/720p.m3u8", {
      qualityHeight: 720,
    });
    const network = streamCandidate("network-hls-1", "https://cdn.example.test/v/master.m3u8", {
      qualityHeight: 1080,
    });

    const chosen = chooseGenericVideoCandidate(fakeVideo(), [], [embedded, network], [], 1, "Owned");
    expect(chosen?.url).toBe(network.url);
    expect(streamFreshnessScore(network)).toBeGreaterThan(streamFreshnessScore(embedded));
    expect(preparedCandidateScore(network)).toBeGreaterThan(preparedCandidateScore(embedded));
  });

  it("keeps the network candidate when an embedded candidate reports the same url", () => {
    const url = "https://cdn.example.test/v/master.m3u8";
    const merged = mergeRequestCandidates(
      [streamCandidate("network-hls-1", url)],
      [streamCandidate("embedded-config-1", url)],
    );

    expect(merged.map((candidate) => candidate.id)).toEqual(["network-hls-1"]);
  });

  it("prefers a master manifest over a single rendition", () => {
    const rendition = streamCandidate("network-hls-1", "https://cdn.example.test/v/720p.m3u8", { qualityHeight: 720 });
    const master = streamCandidate("network-hls-2", "https://cdn.example.test/v/master.m3u8");

    expect(chooseBestStreamCandidate([rendition, master])?.id).toBe(master.id);
    expect(chooseBestStreamCandidate([master, rendition])?.id).toBe(master.id);
  });

  it("limits stream fallback urls to five, freshest first, without duplicates", () => {
    const master = streamCandidate("network-hls-master", "https://cdn.example.test/v/master.m3u8");
    const others = Array.from({ length: 8 }, (_, index) =>
      streamCandidate(`network-hls-${index}`, `https://cdn.example.test/v/${index}p.m3u8`),
    );
    const duplicate = streamCandidate("resource-hls-dup", others[0]?.url ?? "");
    const embedded = streamCandidate("embedded-config-1", "https://cdn.example.test/v/embedded.m3u8");

    const withFallbacks = withStreamFallbackUrls(master, [embedded, ...others, duplicate, master]);

    expect(withFallbacks.sourceFallbackUrls).toHaveLength(5);
    expect(withFallbacks.sourceFallbackUrls).not.toContain(master.url);
    expect(withFallbacks.sourceFallbackUrls).not.toContain(embedded.url);
    expect(new Set(withFallbacks.sourceFallbackUrls).size).toBe(5);
  });

  it("leaves direct video candidates without stream fallbacks", () => {
    const direct = streamCandidate("network-video-1", "https://cdn.example.test/v/clip.mp4", {
      source: "html5-video",
      adapterId: "generic-html5-video",
    });

    expect(
      withStreamFallbackUrls(direct, [streamCandidate("network-hls-1", "https://cdn.example.test/v/a.m3u8")]),
    ).toBe(direct);
  });
});
