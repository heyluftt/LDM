import type { DetectedDownloadCandidate } from "@ldm/contracts";
import { describe, expect, it } from "vitest";
import { genericDownloadLinksAdapter } from "./generic-download-links";

describe("generic download link adapter", () => {
  it("does not use decorative link text as a video filename", async () => {
    const job = await genericDownloadLinksAdapter.createDownloadJob(
      {
        id: "link-click",
        url: "https://example.test/videos/sample-5s.mp4",
        source: "download-link",
        title: "↓",
        suggestedFilename: "sample-5s.mp4",
        kind: "video",
        adapterId: "generic-download-links",
      } satisfies DetectedDownloadCandidate,
      {
        id: "direct",
        label: "Direct download",
        url: "https://example.test/videos/sample-5s.mp4",
        kind: "video",
      },
    );

    expect(job.suggestedFilename).toBe("sample-5s.mp4");
  });
});
