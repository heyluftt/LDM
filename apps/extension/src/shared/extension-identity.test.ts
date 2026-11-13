import { describe, expect, it } from "vitest";
import { EXTENSION_ID, EXTENSION_PUBLIC_KEY } from "./extension-identity";

/** How Chromium turns a public key into an ID: the first 16 bytes of its SHA-256, in letters a to p. */
async function extensionIdFor(publicKey: string): Promise<string> {
  const der = Uint8Array.from(atob(publicKey), (character) => character.charCodeAt(0));
  const digest = new Uint8Array(await crypto.subtle.digest("SHA-256", der));
  return Array.from(digest.slice(0, 16), (byte) =>
    [byte >> 4, byte & 0x0f].map((nibble) => String.fromCharCode(97 + nibble)).join(""),
  ).join("");
}

describe("the extension identity", () => {
  it("names the ID a browser derives from the manifest key", async () => {
    expect(await extensionIdFor(EXTENSION_PUBLIC_KEY)).toBe(EXTENSION_ID);
  });

  it("is a well-formed extension ID", () => {
    expect(EXTENSION_ID).toMatch(/^[a-p]{32}$/u);
  });
});
