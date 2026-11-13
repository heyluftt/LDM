/** Request/response header helpers shared by the background listeners and the adapters. */

export type Header = {
  name: string;
  value?: string;
};

/**
 * The only request headers LDM forwards to the desktop app. Cookies, authorization and other
 * credentials never leave the browser.
 */
export const FORWARDED_REQUEST_HEADERS: ReadonlySet<string> = new Set([
  "accept",
  "accept-language",
  "origin",
  "referer",
  "user-agent",
]);

/** The value of the first header named `name` (compared lower-case). */
export function headerValue(headers: Header[] | undefined, name: string): string | undefined {
  const header = headers?.find((item) => item.name.toLowerCase() === name);
  return header?.value;
}

/** `headerValue` parsed as an integer; undefined when missing or not a number. */
export function numberHeaderValue(headers: Header[] | undefined, name: string): number | undefined {
  const value = headerValue(headers, name);
  if (!value) {
    return undefined;
  }

  const parsed = Number.parseInt(value, 10);
  return Number.isFinite(parsed) ? parsed : undefined;
}

/** Keeps only `FORWARDED_REQUEST_HEADERS`, lower-cased and trimmed, dropping empty values. */
export function sanitizeRequestHeaders(headers: Header[] | undefined): Record<string, string> {
  const safeHeaders: Record<string, string> = {};
  for (const header of headers ?? []) {
    const name = header.name.trim().toLowerCase();
    const value = header.value?.trim();
    if (!value || !FORWARDED_REQUEST_HEADERS.has(name)) {
      continue;
    }

    safeHeaders[name] = value;
  }

  return safeHeaders;
}

/** `sanitizeRequestHeaders` for headers already stored as a record (candidate headers). */
export function sanitizeHeaderRecord(headers: Record<string, string> | undefined): Record<string, string> {
  return sanitizeRequestHeaders(Object.entries(headers ?? {}).map(([name, value]) => ({ name, value })));
}
