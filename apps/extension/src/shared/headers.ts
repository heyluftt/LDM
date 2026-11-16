export type Header = {
  name: string;
  value?: string;
};

/** The only request headers forwarded to the desktop app; no cookies or credentials. */
export const FORWARDED_REQUEST_HEADERS: ReadonlySet<string> = new Set([
  "accept",
  "accept-language",
  "origin",
  "referer",
  "user-agent",
]);

/** First header value named `name`, compared lower-case. */
export function headerValue(headers: Header[] | undefined, name: string): string | undefined {
  const header = headers?.find((item) => item.name.toLowerCase() === name);
  return header?.value;
}

/** `headerValue` as an integer, or undefined. */
export function numberHeaderValue(headers: Header[] | undefined, name: string): number | undefined {
  const value = headerValue(headers, name);
  if (!value) {
    return undefined;
  }

  const parsed = Number.parseInt(value, 10);
  return Number.isFinite(parsed) ? parsed : undefined;
}

/** Keeps only `FORWARDED_REQUEST_HEADERS`, lower-cased and trimmed. */
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

/** `sanitizeRequestHeaders` for a header record. */
export function sanitizeHeaderRecord(headers: Record<string, string> | undefined): Record<string, string> {
  return sanitizeRequestHeaders(Object.entries(headers ?? {}).map(([name, value]) => ({ name, value })));
}
