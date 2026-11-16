import { describe, expect, it } from "vitest";
import { manifestVersion, workspaceVersion } from "../../workspace-version.js";

describe("the extension version", () => {
  it("is the version of the Cargo workspace", () => {
    expect(workspaceVersion()).toMatch(/^\d+\.\d+\.\d+/u);
    expect(manifestVersion().version).toBe(workspaceVersion().split(/[-+]/u)[0]);
  });

  it("goes into the manifest as it is when it is plain numbers", () => {
    expect(manifestVersion("0.2.0")).toEqual({ version: "0.2.0" });
  });

  it("keeps a pre-release suffix as the displayed name only", () => {
    expect(manifestVersion("0.2.0-beta.1")).toEqual({ version: "0.2.0", version_name: "0.2.0-beta.1" });
  });

  it("refuses a version no browser would load", () => {
    expect(() => manifestVersion("next")).toThrow(/does not start with a version/u);
  });
});
