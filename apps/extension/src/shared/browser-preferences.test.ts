import { describe, expect, it } from "vitest";
import { decideBrowserRouting, normalizeBrowserIntegrationPreferences } from "./browser-preferences";

function preferences(overrides: Partial<ReturnType<typeof normalizeBrowserIntegrationPreferences>> = {}) {
  return normalizeBrowserIntegrationPreferences({
    mode: "auto",
    selectedKinds: {
      video: true,
      audio: true,
      program: true,
      document: true,
      archive: true,
      image: true,
      other: false,
    },
    domainRules: [],
    warnOnExecutable: true,
    ...overrides,
  });
}

function subject(
  sourceUrl: string,
  kind: "archive" | "program" = "archive",
  pageUrl = "https://example.test/downloads",
) {
  return { sourceUrl, kind, pageUrl, suggestedFilename: sourceUrl.split("/").pop() };
}

describe("browser routing decisions", () => {
  it("keeps everything in the browser while handling is off", () => {
    expect(decideBrowserRouting(subject("https://example.test/file.zip"), preferences({ mode: "off" }))).toEqual({
      action: "browser",
    });
  });

  it("routes selected file types automatically", () => {
    expect(decideBrowserRouting(subject("https://example.test/file.zip"), preferences())).toEqual({ action: "ldm" });
  });

  it("keeps unselected file types in the browser", () => {
    const prefs = preferences({ selectedKinds: { ...preferences().selectedKinds, archive: false } });

    expect(decideBrowserRouting(subject("https://example.test/file.zip"), prefs)).toEqual({ action: "browser" });
  });

  it("asks before routing in ask mode and flags executables once", () => {
    const prefs = preferences({ mode: "ask" });

    expect(decideBrowserRouting(subject("https://example.test/file.zip"), prefs)).toEqual({
      action: "ask",
      prompt: { filename: "file.zip", kind: "archive", reason: "ask" },
    });
    expect(decideBrowserRouting(subject("https://example.test/setup.exe", "program"), prefs)).toMatchObject({
      action: "ask",
      prompt: { reason: "ask-executable" },
    });
    expect(
      decideBrowserRouting(subject("https://example.test/setup.exe", "program"), prefs, { warnedExecutable: true }),
    ).toMatchObject({ action: "ask", prompt: { reason: "ask" } });
  });

  it("lets an ignore domain rule keep downloads in the browser", () => {
    const prefs = preferences({ domainRules: [{ host: "example.test", action: "off" }] });

    expect(decideBrowserRouting(subject("https://downloads.example.test/file.zip"), prefs)).toEqual({
      action: "browser",
    });
  });

  it("applies page domain rules to downloads served from a different host", () => {
    const prefs = preferences({ domainRules: [{ host: "example.test", action: "off" }] });

    expect(decideBrowserRouting(subject("https://cdn.example-cdn.test/file.zip"), prefs)).toEqual({
      action: "browser",
    });
  });

  it("normalizes domain rules before matching", () => {
    const prefs = preferences({ domainRules: [{ host: " HTTPS://Example.TEST/releases/ ", action: "off" }] });

    expect(prefs.domainRules).toEqual([{ host: "example.test", action: "off" }]);
    expect(decideBrowserRouting(subject("https://downloads.example.test/file.zip"), prefs)).toEqual({
      action: "browser",
    });
  });

  it("uses the most specific domain rule", () => {
    const prefs = preferences({
      selectedKinds: { ...preferences().selectedKinds, archive: false },
      domainRules: [
        { host: "example.test", action: "off" },
        { host: "downloads.example.test", action: "auto" },
      ],
    });

    expect(decideBrowserRouting(subject("https://downloads.example.test/file.zip"), prefs)).toEqual({ action: "ldm" });
  });

  it("lets an always-use domain rule override the file type selection", () => {
    const prefs = preferences({
      selectedKinds: { ...preferences().selectedKinds, archive: false },
      domainRules: [{ host: "example.test", action: "auto" }],
    });

    expect(decideBrowserRouting(subject("https://downloads.example.test/file.zip"), prefs)).toEqual({ action: "ldm" });
  });

  it("falls back to defaults for malformed preferences", () => {
    expect(normalizeBrowserIntegrationPreferences(null)).toMatchObject({ mode: "auto", warnOnExecutable: true });
    expect(normalizeBrowserIntegrationPreferences({ mode: "nonsense", domainRules: "x" })).toMatchObject({
      mode: "auto",
      domainRules: [],
    });
  });
});
