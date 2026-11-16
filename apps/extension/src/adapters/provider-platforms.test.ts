import { describe, expect, it } from "vitest";
import { dashAdapter } from "./dash";
import { hlsAdapter } from "./hls";
import { tiktokAdapter } from "./provider-tiktok";
import { xAdapter } from "./provider-x";
import { youtubeAdapter } from "./provider-youtube";

describe("platform provider adapters", () => {
  it("detects YouTube pages and creates a provider job", async () => {
    const candidates = await youtubeAdapter.detect({
      pageUrl: "https://www.youtube.com/watch?v=owned-content",
      pageTitle: "Owned video",
    });

    expect(candidates).toHaveLength(1);
    expect(candidates[0]?.requiresUsageNotice).toBe(true);
    const options = await youtubeAdapter.listOptions(candidates[0]!);
    expect(options).toHaveLength(1);
    expect(options[0]?.requiresExternalTool).toBe(true);

    const job = await youtubeAdapter.createDownloadJob(candidates[0]!, options[0]!);
    expect(job).toMatchObject({
      adapterId: "provider-youtube",
      providerId: "youtube",
      sourceUrl: "https://www.youtube.com/watch?v=owned-content",
      suggestedFilename: "Owned video",
    });
    expect(youtubeAdapter.explainUnsupported(candidates[0]!)).toBeNull();
  });

  it("does not create a broad YouTube provider candidate on non-video pages", async () => {
    const candidates = await youtubeAdapter.detect({
      pageUrl: "https://www.youtube.com/",
      pageTitle: "Home",
    });

    expect(candidates).toEqual([]);
  });

  it("detects YouTube Shorts as provider jobs", async () => {
    const candidates = await youtubeAdapter.detect({
      pageUrl: "https://www.youtube.com/shorts/owned-short",
      pageTitle: "Owned short",
    });

    expect(candidates[0]?.id).toBe("youtube-owned-short");
  });

  it("detects X pages and creates a provider job", async () => {
    const candidates = await xAdapter.detect({
      pageUrl: "https://x.com/example/status/1",
      pageTitle: "Owned post",
    });

    expect(candidates).toHaveLength(1);
    expect(candidates[0]?.providerId).toBe("x");
    const options = await xAdapter.listOptions(candidates[0]!);
    expect(options).toHaveLength(1);
    const job = await xAdapter.createDownloadJob(candidates[0]!, options[0]!);
    expect(job.adapterId).toBe("provider-x");
    expect(job.providerId).toBe("x");
  });

  it("does not send generic X pages to the provider backend", async () => {
    const candidates = await xAdapter.detect({
      pageUrl: "https://x.com/home",
      pageTitle: "Home",
    });

    expect(candidates).toEqual([]);
  });

  it("detects TikTok pages and creates a provider job", async () => {
    const candidates = await tiktokAdapter.detect({
      pageUrl: "https://www.tiktok.com/@example/video/1",
      pageTitle: "Owned clip",
    });

    expect(candidates).toHaveLength(1);
    expect(candidates[0]?.providerId).toBe("tiktok");
    const options = await tiktokAdapter.listOptions(candidates[0]!);
    expect(options).toHaveLength(1);
    const job = await tiktokAdapter.createDownloadJob(candidates[0]!, options[0]!);
    expect(job.adapterId).toBe("provider-tiktok");
    expect(job.providerId).toBe("tiktok");
  });

  it("uses page titles instead of manifest filenames for stream jobs", async () => {
    const hlsOptions = await hlsAdapter.listOptions({
      id: "hls",
      url: "https://cdn.example.test/index-v1-a1.m3u8",
      source: "hls",
      title: "Owned stream title",
      pageUrl: "https://example.test/watch",
      suggestedFilename: "index-v1-a1.m3u8",
      kind: "video",
      adapterId: "hls",
    });
    const hlsJob = await hlsAdapter.createDownloadJob(
      {
        id: "hls",
        url: "https://cdn.example.test/index-v1-a1.m3u8",
        source: "hls",
        title: "Owned stream title",
        pageUrl: "https://example.test/watch",
        suggestedFilename: "index-v1-a1.m3u8",
        kind: "video",
        adapterId: "hls",
      },
      hlsOptions[0]!,
    );

    expect(hlsJob.suggestedFilename).toBe("Owned stream title");
    expect(hlsJob.sourceFallbackUrls).toEqual(undefined);

    const dashOptions = await dashAdapter.listOptions({
      id: "dash",
      url: "https://cdn.example.test/manifest.mpd",
      source: "dash",
      title: "Owned dash title",
      pageUrl: "https://example.test/watch",
      suggestedFilename: "manifest.mpd",
      kind: "video",
      adapterId: "dash",
    });
    const dashJob = await dashAdapter.createDownloadJob(
      {
        id: "dash",
        url: "https://cdn.example.test/manifest.mpd",
        source: "dash",
        title: "Owned dash title",
        pageUrl: "https://example.test/watch",
        suggestedFilename: "manifest.mpd",
        kind: "video",
        adapterId: "dash",
      },
      dashOptions[0]!,
    );

    expect(dashJob.suggestedFilename).toBe("Owned dash title");
  });

  it("passes stream fallback URLs through to download jobs", async () => {
    const options = await hlsAdapter.listOptions({
      id: "hls",
      url: "https://cdn.example.test/master.m3u8",
      sourceFallbackUrls: ["https://cdn.example.test/720/index.m3u8"],
      source: "hls",
      title: "Owned stream title",
      pageUrl: "https://example.test/watch",
      kind: "video",
      adapterId: "hls",
    });
    const job = await hlsAdapter.createDownloadJob(
      {
        id: "hls",
        url: "https://cdn.example.test/master.m3u8",
        sourceFallbackUrls: ["https://cdn.example.test/720/index.m3u8"],
        source: "hls",
        title: "Owned stream title",
        pageUrl: "https://example.test/watch",
        kind: "video",
        adapterId: "hls",
      },
      options[0]!,
    );

    expect(job.sourceFallbackUrls).toEqual(["https://cdn.example.test/720/index.m3u8"]);
  });
});
