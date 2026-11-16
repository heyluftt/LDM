import { describe, expect, it } from "vitest";
import { candidateFromDownloadLink } from "./download-link-detector";

describe("download link detection", () => {
  it("detects links with a known file extension", () => {
    const anchor = anchorFixture({ href: "https://example.test/files/sample-1.zip", text: "sample-1.zip" });

    expect(candidateFromDownloadLink(anchor, "https://example.test/")).toMatchObject({
      url: "https://example.test/files/sample-1.zip",
      kind: "archive",
      adapterId: "generic-download-links",
      suggestedFilename: "sample-1.zip",
      title: "sample-1.zip",
    });
  });

  it("detects explicit query filenames on extensionless download URLs", () => {
    const anchor = anchorFixture({
      href: "https://example.test/file-download/112?filename=setup.exe",
      text: "Download",
    });

    expect(candidateFromDownloadLink(anchor, "https://example.test/tools")).toMatchObject({
      kind: "program",
      suggestedFilename: "setup.exe",
    });
  });

  it("honors the download attribute even without a known extension", () => {
    const anchor = anchorFixture({ href: "https://example.test/export/42", text: "Export", download: "report.bin" });

    expect(candidateFromDownloadLink(anchor, "https://example.test/")).toMatchObject({
      kind: "other",
      suggestedFilename: "report.bin",
    });
  });

  it("leaves image links without a download attribute to the browser", () => {
    const anchor = anchorFixture({
      href: "https://cdn.shop.example/files/BeeHoodieFinalArtboard2_120x.jpg?v=1700000000",
      text: "Bee Hoodie",
    });

    expect(candidateFromDownloadLink(anchor, "https://shop.example/products/bee-hoodie")).toBeNull();
  });

  it("leaves image links with an image filename query parameter to the browser", () => {
    const anchor = anchorFixture({ href: "https://example.test/media/42?filename=photo.png", text: "Photo" });

    expect(candidateFromDownloadLink(anchor, "https://example.test/")).toBeNull();
  });

  it("detects image links with a download attribute as image downloads", () => {
    const anchor = anchorFixture({
      href: "https://cdn.shop.example/files/BeeHoodieFinalArtboard2.jpg",
      text: "Save image",
      download: "",
    });

    expect(candidateFromDownloadLink(anchor, "https://shop.example/products/bee-hoodie")).toMatchObject({
      url: "https://cdn.shop.example/files/BeeHoodieFinalArtboard2.jpg",
      kind: "image",
      adapterId: "generic-download-links",
      suggestedFilename: "BeeHoodieFinalArtboard2.jpg",
    });
  });

  it("still detects archive links without a download attribute", () => {
    const anchor = anchorFixture({ href: "https://example.test/files/release.zip", text: "Get it" });

    expect(candidateFromDownloadLink(anchor, "https://example.test/")).toMatchObject({
      kind: "archive",
      suggestedFilename: "release.zip",
    });
  });

  it("routes manifests to the stream adapters", () => {
    const anchor = anchorFixture({ href: "https://cdn.example.test/live/master.m3u8", text: "Stream" });

    expect(candidateFromDownloadLink(anchor, "https://example.test/")).toMatchObject({
      source: "hls",
      adapterId: "hls",
      kind: "video",
    });
  });

  it("leaves download-looking routes without a file signal to the browser", () => {
    const anchor = anchorFixture({
      href: "https://www.examplefile.com/file-download/112",
      text: "Download",
      parentText: "1 MB Exe File Size : 1 MB Download",
    });

    expect(candidateFromDownloadLink(anchor, "https://www.examplefile.com/general/exe/1-mb-exe-file")).toBeNull();
  });

  it("leaves installer redirect links to the browser download routing", () => {
    const anchor = anchorFixture({
      href: "https://installers.example.test/product/windows/download?id=5517",
      text: "Windows - FL Studio 26.1.0.5517",
    });

    expect(candidateFromDownloadLink(anchor, "https://forum.example.test/viewtopic.php")).toBeNull();
  });

  it("ignores non-http links", () => {
    const anchor = anchorFixture({ href: "mailto:someone@example.test?subject=file.zip", text: "Mail" });

    expect(candidateFromDownloadLink(anchor, "https://example.test/")).toBeNull();
  });
});

function anchorFixture(input: {
  href: string;
  text: string;
  parentText?: string;
  download?: string;
}): HTMLAnchorElement {
  const attributes = new Map<string, string>([["href", input.href]]);
  if (input.download !== undefined) {
    attributes.set("download", input.download);
  }

  const parent = {
    textContent: input.parentText ?? input.text,
    parentElement: null,
  } as HTMLElement;

  return {
    textContent: input.text,
    parentElement: parent,
    getAttribute(name: string) {
      return attributes.get(name) ?? null;
    },
    hasAttribute(name: string) {
      return attributes.has(name);
    },
  } as unknown as HTMLAnchorElement;
}
