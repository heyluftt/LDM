import { describe, expect, it } from "vitest";
import { overlayErrorMessage } from "./overlay-messages";

describe("overlayErrorMessage", () => {
  it("goes by the code of a rejection, whatever its wording", () => {
    expect(overlayErrorMessage("Bitte zuerst die Einrichtung abschließen.", "firstRunSetupRequired")).toBe(
      "Open LDM and complete the first-run setup.",
    );
    expect(
      overlayErrorMessage(
        "Desktop app could not be reached. Set LDM_DESKTOP_EXE to the installed app. pipe not found",
        "desktopUnavailable",
      ),
    ).toBe("Desktop app could not be reached. Start or reopen LDM.");
  });

  it("does not guess a kind from the words of a rejection without a code", () => {
    expect(overlayErrorMessage("The first-run setup file could not be written.")).toBe(
      "The first-run setup file could not be written.",
    );
  });

  it("explains what to do when the page has no stream yet", () => {
    expect(overlayErrorMessage("Current video stream is not ready.")).toBe(
      "Start playback once, then click LDM again.",
    );
    expect(overlayErrorMessage("Current video stream could not be captured.")).toBe(
      "Keep the video playing, then click LDM again.",
    );
  });

  it("shows any other reason on one line and cut to the notice's length", () => {
    expect(overlayErrorMessage("invalid url:\n  ftp://example")).toBe("invalid url: ftp://example");
    expect(overlayErrorMessage("")).toBe("LDM could not start this download.");

    const long = overlayErrorMessage("x".repeat(400));
    expect(long).toHaveLength(180);
    expect(long.endsWith("...")).toBe(true);
  });
});
