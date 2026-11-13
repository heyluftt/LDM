import { describe, expect, it } from "vitest";
import type { ToolStatus } from "../../shared/api/tools";
import { isSetupComplete, setupSteps } from "./gettingStartedModel";

function tool(kind: ToolStatus["kind"], found: boolean): ToolStatus {
  return { kind, name: kind, envVar: "", required: kind !== "deno", found, message: "" };
}

const contact = { at: 1_763_060_000, origin: "chrome-extension://dpfifoelihelgibggmccbedplhjpfmdc/" };

describe("setupSteps", () => {
  it("lists what a fresh installation is missing, with what to do about each", () => {
    const steps = setupSteps(null, [tool("ytDlp", false), tool("ffmpeg", false), tool("deno", false)]);

    expect(steps.map((step) => [step.id, step.done])).toEqual([
      ["extension", false],
      ["ytDlp", false],
      ["ffmpeg", false],
    ]);
    expect(steps[0].hint).toContain("Load unpacked");
    expect(steps[1].command).toBe("winget install yt-dlp.yt-dlp");
    expect(steps[2].command).toBe("winget install Gyan.FFmpeg");
    expect(isSetupComplete(steps)).toBe(false);
  });

  it("marks a step done without a hint or a command once it is there", () => {
    const steps = setupSteps(contact, [tool("ytDlp", true), tool("ffmpeg", false)]);

    expect(steps[0]).toEqual({ id: "extension", title: "Browser extension", done: true, status: "Connected." });
    expect(steps[1]).toEqual({ id: "ytDlp", title: "yt-dlp", done: true, status: "Found." });
    expect(steps[2].done).toBe(false);
    expect(isSetupComplete(steps)).toBe(false);
  });

  it("is complete with the extension, yt-dlp, and FFmpeg; Deno and FFprobe are not asked for", () => {
    const steps = setupSteps(contact, [
      tool("ytDlp", true),
      tool("ffmpeg", true),
      tool("ffprobe", false),
      tool("deno", false),
    ]);

    expect(isSetupComplete(steps)).toBe(true);
  });

  it("treats a tool the backend did not report as missing", () => {
    expect(setupSteps(contact, []).map((step) => step.done)).toEqual([true, false, false]);
  });
});
