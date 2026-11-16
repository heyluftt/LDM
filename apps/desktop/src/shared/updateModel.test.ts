import { describe, expect, it } from "vitest";
import { shouldAnnounceUpdate, updateSummary } from "./updateModel";

const upToDate = { currentVersion: "0.1.0", latestVersion: "0.1.0", updateAvailable: false };
const newer = { currentVersion: "0.1.0", latestVersion: "0.2.0", updateAvailable: true };

describe("shouldAnnounceUpdate", () => {
  it("announces a newer version", () => {
    expect(shouldAnnounceUpdate(newer, null)).toBe(true);
  });

  it("stays quiet without a check and when nothing is newer", () => {
    expect(shouldAnnounceUpdate(null, null)).toBe(false);
    expect(shouldAnnounceUpdate(upToDate, null)).toBe(false);
  });

  it("respects 'later' for that version and speaks up again for the next one", () => {
    expect(shouldAnnounceUpdate(newer, "0.2.0")).toBe(false);
    expect(shouldAnnounceUpdate({ ...newer, latestVersion: "0.3.0" }, "0.2.0")).toBe(true);
  });
});

describe("updateSummary", () => {
  it("says what is known", () => {
    expect(updateSummary(null, false, null)).toBe("Not checked yet.");
    expect(updateSummary(null, true, null)).toBe("Checking…");
    expect(updateSummary(upToDate, false, null)).toBe("This is the latest version.");
    expect(updateSummary(newer, false, null)).toBe("Version 0.2.0 is available.");
  });

  it("shows why a check failed instead of an old result", () => {
    expect(updateSummary(upToDate, false, "Could not reach GitHub: timed out")).toBe(
      "Could not check: Could not reach GitHub: timed out",
    );
  });
});
