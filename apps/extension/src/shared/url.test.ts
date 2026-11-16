import { describe, expect, it } from "vitest";
import { extensionForUrl, extensionFromQuery, filenameFromUrl, hashString, normalizeContentType } from "./url";

const isMedia = (extension: string) => ["mp4", "m3u8", "mpd", "webm", "m4a"].includes(extension);

describe("shared url helpers", () => {
  it("hashes strings into short stable ids", () => {
    expect(hashString("https://cdn.example.invalid/a.mp4")).toBe(hashString("https://cdn.example.invalid/a.mp4"));
    expect(hashString("a")).not.toBe(hashString("b"));
    expect(hashString("")).toBe("0");
    expect(hashString("video")).toMatch(/^[0-9a-z]+$/u);
  });

  it("takes the decoded last path segment as filename", () => {
    expect(filenameFromUrl("https://example.invalid/media/My%20Clip.mp4?token=1")).toBe("My Clip.mp4");
    expect(filenameFromUrl("https://example.invalid/")).toBeUndefined();
    expect(filenameFromUrl("not a url")).toBeUndefined();
    expect(filenameFromUrl("/relative/clip.webm", "https://example.invalid/page")).toBe("clip.webm");
  });

  it("prefers the path extension and ignores static assets", () => {
    expect(extensionForUrl(new URL("https://example.invalid/clip.MP4"), isMedia)).toBe("mp4");
    expect(extensionForUrl(new URL("https://example.invalid/icon.svg?mime=video/mp4"), isMedia)).toBeUndefined();
    expect(extensionForUrl(new URL("https://example.invalid/stream?mime=video%2Fmp4"), isMedia)).toBe("mp4");
    expect(extensionForUrl(new URL("https://example.invalid/stream?format=m3u8"), isMedia)).toBe("m3u8");
    expect(extensionForUrl(new URL("https://example.invalid/stream?x=1"), isMedia)).toBeUndefined();
  });

  it("reads MIME hints and extension-like tokens from the query", () => {
    expect(extensionFromQuery(new URLSearchParams("type=application/dash+xml"), isMedia)).toBe("mpd");
    expect(extensionFromQuery(new URLSearchParams("file=intro.webm"), isMedia)).toBe("webm");
    expect(extensionFromQuery(new URLSearchParams("file=intro.txt"), isMedia)).toBeUndefined();
  });

  it("strips content type parameters", () => {
    expect(normalizeContentType("Video/MP4; codecs=avc1")).toBe("video/mp4");
    expect(normalizeContentType("")).toBeUndefined();
    expect(normalizeContentType(undefined)).toBeUndefined();
  });
});
