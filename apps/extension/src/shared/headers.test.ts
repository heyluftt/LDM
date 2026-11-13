import { describe, expect, it } from "vitest";
import { headerValue, numberHeaderValue, sanitizeHeaderRecord, sanitizeRequestHeaders } from "./headers";

describe("shared header helpers", () => {
  const headers = [
    { name: "Content-Type", value: "video/mp4" },
    { name: "Content-Length", value: "1024" },
    { name: "X-Empty", value: "" },
  ];

  it("finds headers case-insensitively and parses numbers", () => {
    expect(headerValue(headers, "content-type")).toBe("video/mp4");
    expect(headerValue(headers, "missing")).toBeUndefined();
    expect(numberHeaderValue(headers, "content-length")).toBe(1024);
    expect(numberHeaderValue(headers, "content-type")).toBeUndefined();
    expect(numberHeaderValue(headers, "x-empty")).toBeUndefined();
    expect(numberHeaderValue(undefined, "content-length")).toBeUndefined();
  });

  it("forwards only the allowlisted request headers", () => {
    const safe = sanitizeRequestHeaders([
      { name: "Cookie", value: "session=secret" },
      { name: "Authorization", value: "Bearer token" },
      { name: " User-Agent ", value: " Browser/1 " },
      { name: "Referer", value: "https://example.invalid/page" },
      { name: "Accept", value: "   " },
    ]);

    expect(safe).toEqual({ "user-agent": "Browser/1", referer: "https://example.invalid/page" });
    expect(sanitizeRequestHeaders(undefined)).toEqual({});
  });

  it("sanitizes header records the same way", () => {
    expect(sanitizeHeaderRecord({ Cookie: "secret", Origin: " https://example.invalid " })).toEqual({
      origin: "https://example.invalid",
    });
  });
});
