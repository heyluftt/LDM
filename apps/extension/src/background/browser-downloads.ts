import type {
  BrowserIntegrationPreferences,
  CreateDownloadJobRequest,
  DesktopToExtensionMessage,
  DownloadKind,
  ExtensionToDesktopMessage,
} from "@ldm/contracts";
import { BRAND_MARK_DATA_URL } from "../shared/brand-mark";
import {
  BROWSER_PREFERENCES_STORAGE_KEY,
  type BrowserDownloadPrompt,
  decideBrowserRouting,
  normalizeBrowserIntegrationPreferences,
} from "../shared/browser-preferences";
import { type Header, headerValue, numberHeaderValue, sanitizeRequestHeaders } from "../shared/headers";
import { filenameFromUrl, normalizeContentType } from "../shared/url";

export type { BrowserDownloadPrompt } from "../shared/browser-preferences";

type BrowserDownloadRequestDetails = {
  url: string;
  requestHeaders?: Header[];
};

type BrowserDownloadResponseDetails = {
  url: string;
  responseHeaders?: Header[];
};

export type BrowserDownloadItem = Pick<
  chrome.downloads.DownloadItem,
  "id" | "url" | "filename" | "mime" | "fileSize" | "totalBytes"
> &
  Partial<Pick<chrome.downloads.DownloadItem, "byExtensionId" | "finalUrl" | "referrer" | "startTime" | "state">>;

type StoredRequestMetadata = {
  detectedAt: number;
  headers: Record<string, string>;
};

type StoredResponseMetadata = {
  detectedAt: number;
  contentDisposition?: string;
  contentLength?: number;
  contentType?: string;
};

type BrowserDownloadApi = {
  cancel(downloadId: number, callback?: () => void): void;
  erase(query: chrome.downloads.DownloadQuery, callback?: (erasedIds: number[]) => void): void;
  pause(downloadId: number, callback?: () => void): void;
  resume(downloadId: number, callback?: () => void): void;
};

type NativeSender = (message: ExtensionToDesktopMessage) => Promise<DesktopToExtensionMessage>;

export type BrowserDownloadPreferences = BrowserIntegrationPreferences;

const MAX_METADATA_AGE_MS = 5 * 60 * 1000;
const MAX_METADATA_ENTRIES = 300;
const MAX_HANDLED_DOWNLOAD_IDS = 500;
const FRESH_DOWNLOAD_TOLERANCE_MS = 30 * 1000;
const BROWSER_STARTUP_GUARD_MS = 15 * 1000;
const BROWSER_PROMPT_TIMEOUT_MS = 30 * 1000;
const BROWSER_FALLBACK_TTL_MS = 60 * 1000;
/** How long a fetched preferences copy is reused before the native host is asked again. */
const PREFERENCES_CACHE_TTL_MS = 10 * 1000;
const NOTIFICATION_ICON_URL = BRAND_MARK_DATA_URL;
const backgroundStartedAt = Date.now();
const routingDownloadPromises = new Map<number, Promise<void>>();
const handledDownloadIds = new Set<number>();
const browserFallbackUrls = new Map<string, number>();
const requestMetadataByUrl = new Map<string, StoredRequestMetadata>();
const responseMetadataByUrl = new Map<string, StoredResponseMetadata>();
let cachedPreferences: { value: BrowserDownloadPreferences; fetchedAt: number } | undefined;
let preferencesRequest: Promise<BrowserDownloadPreferences | undefined> | undefined;
let browserPreferencesOverride: BrowserDownloadPreferences | undefined;
let browserDownloadPromptOverride: ((prompt: BrowserDownloadPrompt) => Promise<boolean>) | undefined;
let browserStartupGuardUntil = 0;
let warnedExecutableThisSession = false;
let notificationListenersRegistered = false;
const pendingBrowserDownloadPrompts = new Map<
  string,
  {
    resolve: (accepted: boolean) => void;
    timeoutId: ReturnType<typeof setTimeout>;
  }
>();

const EXTENSION_KINDS = new Map<string, DownloadKind>([
  ["mp4", "video"],
  ["m4v", "video"],
  ["webm", "video"],
  ["mov", "video"],
  ["mkv", "video"],
  ["avi", "video"],
  ["wmv", "video"],
  ["flv", "video"],
  ["mp3", "audio"],
  ["m4a", "audio"],
  ["aac", "audio"],
  ["wav", "audio"],
  ["flac", "audio"],
  ["ogg", "audio"],
  ["opus", "audio"],
  ["exe", "program"],
  ["msi", "program"],
  ["msix", "program"],
  ["appx", "program"],
  ["bat", "program"],
  ["cmd", "program"],
  ["ps1", "program"],
  ["scr", "program"],
  ["com", "program"],
  ["zip", "archive"],
  ["7z", "archive"],
  ["rar", "archive"],
  ["tar", "archive"],
  ["gz", "archive"],
  ["bz2", "archive"],
  ["xz", "archive"],
  ["iso", "archive"],
  ["pdf", "document"],
  ["doc", "document"],
  ["docx", "document"],
  ["xls", "document"],
  ["xlsx", "document"],
  ["ppt", "document"],
  ["pptx", "document"],
  ["txt", "document"],
  ["csv", "document"],
  ["rtf", "document"],
  ["odt", "document"],
  ["ods", "document"],
  ["odp", "document"],
  ["epub", "document"],
  ["json", "document"],
  ["png", "image"],
  ["jpg", "image"],
  ["jpeg", "image"],
  ["webp", "image"],
  ["gif", "image"],
  ["bmp", "image"],
  ["tif", "image"],
  ["tiff", "image"],
  ["svg", "image"],
  ["avif", "image"],
  ["ico", "image"],
]);

const OTHER_DOWNLOAD_EXTENSIONS = new Set(["bin", "dmg", "pkg", "torrent"]);
const SKIPPED_CONTENT_TYPES = new Set(["application/xhtml+xml", "text/html", "text/javascript", "text/css"]);

// webRequest listeners may answer with a BlockingResponse; `undefined` says these never block.
export function rememberBrowserDownloadRequestHeaders(details: BrowserDownloadRequestDetails): undefined {
  const headers = sanitizeRequestHeaders(details.requestHeaders);
  if (Object.keys(headers).length === 0) {
    return;
  }

  pruneMetadataMaps();
  requestMetadataByUrl.set(details.url, {
    detectedAt: Date.now(),
    headers,
  });
  trimMetadataMap(requestMetadataByUrl);
}

export function rememberBrowserDownloadResponse(details: BrowserDownloadResponseDetails): undefined {
  const contentDisposition = headerValue(details.responseHeaders, "content-disposition");
  const contentType = normalizeContentType(headerValue(details.responseHeaders, "content-type"));
  const contentLength = positiveNumberHeaderValue(details.responseHeaders, "content-length");

  if (!contentDisposition && !contentType && contentLength === undefined) {
    return;
  }

  pruneMetadataMaps();
  responseMetadataByUrl.set(details.url, {
    detectedAt: Date.now(),
    contentDisposition,
    contentLength,
    contentType,
  });
  trimMetadataMap(responseMetadataByUrl);
}

export function clearBrowserDownloadMetadata(): void {
  requestMetadataByUrl.clear();
  responseMetadataByUrl.clear();
  routingDownloadPromises.clear();
  handledDownloadIds.clear();
  browserFallbackUrls.clear();
  cachedPreferences = undefined;
  preferencesRequest = undefined;
  browserPreferencesOverride = undefined;
  browserDownloadPromptOverride = undefined;
  browserStartupGuardUntil = 0;
  warnedExecutableThisSession = false;
  for (const [notificationId, prompt] of pendingBrowserDownloadPrompts) {
    clearTimeout(prompt.timeoutId);
    prompt.resolve(false);
    void clearNotification(notificationId);
  }
  pendingBrowserDownloadPrompts.clear();
}

export function allowNextBrowserDownloadFallback(url: string): void {
  const key = urlKey(url);
  if (key) {
    browserFallbackUrls.set(key, Date.now());
  }
}

export function armBrowserStartupGuard(now = Date.now()): void {
  browserStartupGuardUntil = now + BROWSER_STARTUP_GUARD_MS;
}

export function maybeRouteBrowserDownload(
  item: BrowserDownloadItem,
  sendNativeMessage: NativeSender,
  downloadsApi: BrowserDownloadApi = chrome.downloads,
): Promise<void> {
  if (handledDownloadIds.has(item.id)) {
    return Promise.resolve();
  }

  const existing = routingDownloadPromises.get(item.id);
  if (existing) {
    return existing;
  }

  const routing = routeBrowserDownload(item, sendNativeMessage, downloadsApi).finally(() => {
    routingDownloadPromises.delete(item.id);
  });
  routingDownloadPromises.set(item.id, routing);
  return routing;
}

/**
 * Hands a browser download to LDM without ever losing it.
 *
 * The browser download is only paused while LDM is asked; it is canceled and erased after LDM
 * has accepted the job, and resumed whenever LDM declines, is unreachable, or the user keeps
 * the browser download. The old flow canceled first and tried to re-download on failure,
 * which lost one-time links.
 */
async function routeBrowserDownload(
  item: BrowserDownloadItem,
  sendNativeMessage: NativeSender,
  downloadsApi: BrowserDownloadApi,
): Promise<void> {
  if (shouldAllowBrowserDownloadFallback(item)) {
    rememberHandledDownload(item.id);
    return;
  }
  if (!isFreshBrowserDownload(item)) {
    return;
  }

  const job = createBrowserDownloadJob(item);
  if (!job) {
    return;
  }
  const preferences = await getBrowserDownloadPreferences(sendNativeMessage);
  if (!preferences) {
    rememberHandledDownload(item.id);
    return;
  }
  const decision = decideBrowserRouting(job, preferences, { warnedExecutable: warnedExecutableThisSession });
  if (decision.action === "browser") {
    rememberHandledDownload(item.id);
    return;
  }

  const paused = await downloadApiCall((done) => downloadsApi.pause(item.id, done));
  if (!paused) {
    // Already finished or otherwise out of reach: let the browser keep it.
    rememberHandledDownload(item.id);
    return;
  }

  if (decision.action === "ask") {
    const accepted = await confirmBrowserDownload(decision.prompt);
    if (!accepted) {
      rememberHandledDownload(item.id);
      await downloadApiCall((done) => downloadsApi.resume(item.id, done));
      return;
    }
  }

  markExecutableWarningShown(job, preferences, {
    alreadyShownInPrompt: decision.action === "ask" && decision.prompt.reason === "ask-executable",
  });

  let accepted = false;
  try {
    const response = await sendNativeMessage({
      type: "createJob",
      schemaVersion: 1,
      job,
    });
    accepted = response.type === "jobAccepted";
  } catch {
    accepted = false;
  }
  rememberHandledDownload(item.id);

  if (!accepted) {
    await downloadApiCall((done) => downloadsApi.resume(item.id, done));
    return;
  }

  const canceled = await downloadApiCall((done) => downloadsApi.cancel(item.id, done));
  if (canceled) {
    await eraseBrowserDownload(item.id, downloadsApi);
  }
}

/**
 * Shows the one-time executable warning for a job that a content script routes to LDM
 * directly (link clicks skip the browser download flow above).
 */
export function noteExecutableJob(
  job: CreateDownloadJobRequest,
  preferences: BrowserDownloadPreferences | undefined,
): void {
  if (!preferences) {
    return;
  }
  markExecutableWarningShown(job, preferences);
}

export function setBrowserDownloadPreferencesForTests(preferences: BrowserDownloadPreferences | undefined): void {
  browserPreferencesOverride = preferences ? normalizeBrowserIntegrationPreferences(preferences) : undefined;
}

export function setBrowserDownloadPromptForTests(
  prompt: ((details: BrowserDownloadPrompt) => Promise<boolean>) | undefined,
): void {
  browserDownloadPromptOverride = prompt;
}

export function createBrowserDownloadJob(item: BrowserDownloadItem): CreateDownloadJobRequest | null {
  if (item.byExtensionId) {
    return null;
  }

  const sourceUrl = item.finalUrl || item.url;
  if (!isHttpUrl(sourceUrl)) {
    return null;
  }

  const sourceUrls = uniqueUrls([sourceUrl, item.url, item.finalUrl]);
  const requestMetadata = requestMetadataForItem(item);
  const responseMetadata = responseMetadataForItem(item);
  const contentDisposition = responseMetadata?.contentDisposition;
  const suggestedFilename =
    baseFilename(item.filename) ||
    baseFilename(filenameFromContentDisposition(contentDisposition)) ||
    firstFilenameFromUrls(sourceUrls);
  const contentType = normalizeContentType(item.mime) ?? responseMetadata?.contentType;
  const contentLength =
    positiveNumber(item.totalBytes) ?? positiveNumber(item.fileSize) ?? responseMetadata?.contentLength;
  const kind = classifyBrowserDownload({
    contentDisposition,
    contentType,
    filename: suggestedFilename,
    urls: sourceUrls,
  });

  if (!kind) {
    return null;
  }

  return {
    sourceUrl,
    sourceFallbackUrls: item.finalUrl && item.finalUrl !== item.url && isHttpUrl(item.url) ? [item.url] : [],
    pageUrl: isHttpUrl(item.referrer) ? item.referrer : undefined,
    referrer: isHttpUrl(item.referrer) ? item.referrer : undefined,
    suggestedFilename,
    contentType,
    contentLength,
    kind,
    adapterId: "browser-download",
    selectedOptionId: "browser-download",
    headers: requestMetadata?.headers,
    preferRangeRequest: true,
  };
}

function classifyBrowserDownload(input: {
  contentDisposition?: string;
  contentType?: string;
  filename?: string;
  urls: string[];
}): DownloadKind | null {
  const extension =
    extensionForFilename(input.filename) ??
    input.urls.map(extensionForUrl).find((value): value is string => Boolean(value));
  const kindFromExtension = extension ? EXTENSION_KINDS.get(extension) : undefined;
  if (kindFromExtension) {
    return kindFromExtension;
  }

  if (extension && OTHER_DOWNLOAD_EXTENSIONS.has(extension)) {
    return "other";
  }

  const normalizedContentType = normalizeContentType(input.contentType);
  if (normalizedContentType) {
    if (SKIPPED_CONTENT_TYPES.has(normalizedContentType)) {
      return null;
    }
    if (normalizedContentType.startsWith("video/")) {
      return "video";
    }
    if (normalizedContentType.startsWith("audio/")) {
      return "audio";
    }
    if (normalizedContentType.startsWith("image/")) {
      return "image";
    }

    const kindFromContentType = kindForContentType(normalizedContentType);
    if (kindFromContentType) {
      return kindFromContentType;
    }
  }

  if (isAttachment(input.contentDisposition)) {
    return "other";
  }

  if (normalizedContentType === "application/octet-stream") {
    return "other";
  }

  return null;
}

function isFreshBrowserDownload(item: BrowserDownloadItem): boolean {
  if (Date.now() < browserStartupGuardUntil) {
    return false;
  }
  if (item.state === "complete") {
    return false;
  }
  if (!item.startTime) {
    return false;
  }

  const startedAt = Date.parse(item.startTime);
  if (!Number.isFinite(startedAt)) {
    return false;
  }

  return startedAt >= backgroundStartedAt - FRESH_DOWNLOAD_TOLERANCE_MS;
}

function rememberHandledDownload(downloadId: number): void {
  handledDownloadIds.add(downloadId);
  while (handledDownloadIds.size > MAX_HANDLED_DOWNLOAD_IDS) {
    const firstId = handledDownloadIds.values().next().value as number | undefined;
    if (firstId === undefined) {
      return;
    }
    handledDownloadIds.delete(firstId);
  }
}

/**
 * The current preferences, fetched through the native host at most every few seconds and
 * mirrored into `chrome.storage.session` so content scripts can decide link clicks without
 * a round trip. `undefined` means the host could not answer.
 */
export async function getBrowserDownloadPreferences(
  sendNativeMessage: NativeSender,
): Promise<BrowserDownloadPreferences | undefined> {
  if (browserPreferencesOverride) {
    return browserPreferencesOverride;
  }
  if (cachedPreferences && Date.now() - cachedPreferences.fetchedAt < PREFERENCES_CACHE_TTL_MS) {
    return cachedPreferences.value;
  }
  if (preferencesRequest) {
    return preferencesRequest;
  }

  preferencesRequest = fetchBrowserDownloadPreferences(sendNativeMessage).finally(() => {
    preferencesRequest = undefined;
  });
  return preferencesRequest;
}

async function fetchBrowserDownloadPreferences(
  sendNativeMessage: NativeSender,
): Promise<BrowserDownloadPreferences | undefined> {
  let response: DesktopToExtensionMessage;
  try {
    response = await sendNativeMessage({
      type: "getBrowserIntegrationPreferences",
      schemaVersion: 1,
    });
  } catch {
    return cachedPreferences?.value;
  }
  if (response.type !== "browserIntegrationPreferences") {
    return cachedPreferences?.value;
  }

  const preferences = normalizeBrowserIntegrationPreferences(response.preferences);
  cachedPreferences = { value: preferences, fetchedAt: Date.now() };
  publishPreferencesToContentScripts(preferences);
  return preferences;
}

function publishPreferencesToContentScripts(preferences: BrowserDownloadPreferences): void {
  try {
    chrome.storage?.session?.set({ [BROWSER_PREFERENCES_STORAGE_KEY]: preferences }, () => {
      // Best effort: without session storage, content scripts fall back to asking the background.
      void chrome.runtime.lastError;
    });
  } catch {
    // Session storage is unavailable here; see above.
  }
}

function markExecutableWarningShown(
  job: CreateDownloadJobRequest,
  preferences: BrowserDownloadPreferences,
  options: { alreadyShownInPrompt?: boolean } = {},
): void {
  if (job.kind !== "program" || !preferences.warnOnExecutable || warnedExecutableThisSession) {
    return;
  }

  warnedExecutableThisSession = true;
  if (!options.alreadyShownInPrompt) {
    showExecutableNotice(job);
  }
}

function showExecutableNotice(job: CreateDownloadJobRequest): void {
  if (typeof chrome === "undefined" || !chrome.notifications) {
    return;
  }

  chrome.notifications.create(
    `ldm-executable-notice-${Date.now()}-${Math.random().toString(36).slice(2)}`,
    {
      type: "basic",
      iconUrl: NOTIFICATION_ICON_URL,
      title: "Executable download",
      message: `Executable files can be unsafe. LDM saves ${
        job.suggestedFilename ?? "this file"
      } but will not run it automatically.`,
      priority: 0,
    },
    () => {
      // Notification delivery is best-effort and must not block routing.
    },
  );
}

function confirmBrowserDownload(prompt: BrowserDownloadPrompt): Promise<boolean> {
  if (browserDownloadPromptOverride) {
    return browserDownloadPromptOverride(prompt);
  }
  if (typeof chrome === "undefined" || !chrome.notifications) {
    return Promise.resolve(false);
  }

  registerNotificationListeners();
  const notificationId = `ldm-browser-download-${Date.now()}-${Math.random().toString(36).slice(2)}`;
  return new Promise((resolve) => {
    const timeoutId = setTimeout(() => {
      settleBrowserDownloadPrompt(notificationId, false);
    }, BROWSER_PROMPT_TIMEOUT_MS);
    pendingBrowserDownloadPrompts.set(notificationId, { resolve, timeoutId });
    chrome.notifications.create(
      notificationId,
      {
        type: "basic",
        iconUrl: NOTIFICATION_ICON_URL,
        title: prompt.reason === "executable" ? "Executable download" : "Use LDM for this download?",
        message: browserDownloadPromptMessage(prompt),
        buttons: [{ title: "Use LDM" }, { title: "Keep browser download" }],
        priority: 1,
      },
      () => {
        if (chrome.runtime.lastError) {
          settleBrowserDownloadPrompt(notificationId, false);
        }
      },
    );
  });
}

function browserDownloadPromptMessage(prompt: BrowserDownloadPrompt): string {
  const filename = prompt.filename.slice(0, 160);
  if (prompt.reason === "ask-executable") {
    return `${filename}\nExecutable files can be unsafe. LDM saves the file but will not run it automatically.`;
  }
  if (prompt.reason === "executable") {
    return `Executable files can be unsafe. LDM saves ${filename} but will not run it automatically.`;
  }
  return filename;
}

function registerNotificationListeners(): void {
  if (notificationListenersRegistered) {
    return;
  }
  chrome.notifications.onButtonClicked.addListener((notificationId, buttonIndex) => {
    settleBrowserDownloadPrompt(notificationId, buttonIndex === 0);
  });
  chrome.notifications.onClosed.addListener((notificationId) => {
    settleBrowserDownloadPrompt(notificationId, false);
  });
  notificationListenersRegistered = true;
}

function settleBrowserDownloadPrompt(notificationId: string, accepted: boolean): void {
  const prompt = pendingBrowserDownloadPrompts.get(notificationId);
  if (!prompt) {
    return;
  }
  pendingBrowserDownloadPrompts.delete(notificationId);
  clearTimeout(prompt.timeoutId);
  prompt.resolve(accepted);
  void clearNotification(notificationId);
}

function clearNotification(notificationId: string): Promise<void> {
  if (typeof chrome === "undefined" || !chrome.notifications) {
    return Promise.resolve();
  }
  return new Promise((resolve) => {
    chrome.notifications.clear(notificationId, () => resolve());
  });
}

function kindForContentType(contentType: string): DownloadKind | null {
  if (
    contentType === "application/pdf" ||
    contentType === "application/msword" ||
    contentType.startsWith("application/vnd.openxmlformats-officedocument.") ||
    contentType.startsWith("application/vnd.oasis.opendocument.") ||
    contentType === "text/plain" ||
    contentType === "text/csv"
  ) {
    return "document";
  }

  if (
    contentType === "application/zip" ||
    contentType === "application/x-7z-compressed" ||
    contentType === "application/vnd.rar" ||
    contentType === "application/x-rar-compressed" ||
    contentType === "application/x-tar" ||
    contentType === "application/gzip" ||
    contentType === "application/x-bzip2" ||
    contentType === "application/x-xz" ||
    contentType === "application/x-iso9660-image"
  ) {
    return "archive";
  }

  if (
    contentType === "application/x-msdownload" ||
    contentType === "application/vnd.microsoft.portable-executable" ||
    contentType === "application/x-msi" ||
    contentType === "application/msix"
  ) {
    return "program";
  }

  return null;
}

function requestMetadataForItem(item: BrowserDownloadItem): StoredRequestMetadata | undefined {
  return metadataForUrls(requestMetadataByUrl, candidateUrls(item));
}

function responseMetadataForItem(item: BrowserDownloadItem): StoredResponseMetadata | undefined {
  return metadataForUrls(responseMetadataByUrl, candidateUrls(item));
}

function metadataForUrls<T>(metadata: Map<string, T>, urls: string[]): T | undefined {
  pruneMetadataMaps();
  for (const url of urls) {
    const value = metadata.get(url);
    if (value) {
      return value;
    }
  }

  return undefined;
}

function candidateUrls(item: BrowserDownloadItem): string[] {
  return [item.finalUrl, item.url].filter((url): url is string => Boolean(url));
}

function uniqueUrls(urls: Array<string | undefined>): string[] {
  const seen = new Set<string>();
  const unique: string[] = [];
  for (const url of urls) {
    if (!url || seen.has(url)) {
      continue;
    }
    seen.add(url);
    unique.push(url);
  }

  return unique;
}

function pruneMetadataMaps(): void {
  pruneMetadataMap(requestMetadataByUrl);
  pruneMetadataMap(responseMetadataByUrl);
  const now = Date.now();
  for (const [key, detectedAt] of browserFallbackUrls) {
    if (now - detectedAt > BROWSER_FALLBACK_TTL_MS) {
      browserFallbackUrls.delete(key);
    }
  }
}

function pruneMetadataMap<T extends { detectedAt: number }>(metadata: Map<string, T>): void {
  const now = Date.now();
  for (const [url, stored] of metadata) {
    if (now - stored.detectedAt > MAX_METADATA_AGE_MS) {
      metadata.delete(url);
    }
  }
}

function trimMetadataMap<T>(metadata: Map<string, T>): void {
  while (metadata.size > MAX_METADATA_ENTRIES) {
    const firstKey = metadata.keys().next().value as string | undefined;
    if (!firstKey) {
      return;
    }
    metadata.delete(firstKey);
  }
}

function eraseBrowserDownload(downloadId: number, downloadsApi: BrowserDownloadApi): Promise<boolean> {
  return downloadApiCall((done) => downloadsApi.erase({ id: downloadId }, done));
}

function shouldAllowBrowserDownloadFallback(item: BrowserDownloadItem): boolean {
  pruneMetadataMaps();
  for (const url of candidateUrls(item)) {
    const key = urlKey(url);
    if (key && browserFallbackUrls.delete(key)) {
      return true;
    }
  }

  return false;
}

function downloadApiCall(invoke: (done: () => void) => void): Promise<boolean> {
  return new Promise((resolve) => {
    try {
      invoke(() => {
        resolve(!chrome.runtime.lastError);
      });
    } catch {
      resolve(false);
    }
  });
}

function isHttpUrl(value: string | undefined): value is string {
  if (!value) {
    return false;
  }

  try {
    const url = new URL(value);
    return url.protocol === "http:" || url.protocol === "https:";
  } catch {
    return false;
  }
}

function urlKey(value: string | undefined): string | undefined {
  if (!isHttpUrl(value)) {
    return undefined;
  }

  try {
    const url = new URL(value);
    url.hash = "";
    return url.href;
  } catch {
    return undefined;
  }
}

function baseFilename(value: string | undefined): string | undefined {
  const filename = value?.split(/[\\/]/u).filter(Boolean).pop()?.trim();
  return filename || undefined;
}

/** The path filename when it has a known extension, else a `filename=`-style query value. */
function filenameFromUrlOrQuery(value: string): string | undefined {
  try {
    const url = new URL(value);
    const decodedPathFilename = filenameFromUrl(value);
    const queryFilename = filenameFromQuery(url);
    return hasKnownFilenameExtension(decodedPathFilename)
      ? decodedPathFilename
      : (queryFilename ?? decodedPathFilename);
  } catch {
    return undefined;
  }
}

function firstFilenameFromUrls(urls: string[]): string | undefined {
  const filenames = urls.flatMap((url) => {
    const filename = filenameFromUrlOrQuery(url);
    return filename ? [filename] : [];
  });
  const filenameWithKnownExtension = filenames.find(hasKnownFilenameExtension);
  if (filenameWithKnownExtension) {
    return filenameWithKnownExtension;
  }

  for (const url of urls) {
    const filename = filenameFromUrlOrQuery(url);
    if (filename) {
      return filename;
    }
  }

  return undefined;
}

function filenameFromQuery(url: URL): string | undefined {
  for (const key of ["filename", "file", "download", "name"]) {
    const value = url.searchParams.get(key)?.trim();
    const filename = value?.split(/[\\/]/u).filter(Boolean).pop();
    if (hasKnownFilenameExtension(filename)) {
      return filename;
    }
  }

  return undefined;
}

function hasKnownFilenameExtension(value: string | undefined): boolean {
  const extension = extensionForFilename(value);
  return Boolean(extension && (EXTENSION_KINDS.has(extension) || OTHER_DOWNLOAD_EXTENSIONS.has(extension)));
}

function filenameFromContentDisposition(value: string | undefined): string | undefined {
  if (!value) {
    return undefined;
  }

  const starMatch = value.match(/filename\*=UTF-8''([^;]+)/iu);
  if (starMatch?.[1]) {
    return decodeHeaderFilename(starMatch[1]);
  }

  const quotedMatch = value.match(/filename="([^"]+)"/iu);
  if (quotedMatch?.[1]) {
    return quotedMatch[1].trim();
  }

  const filenameMatch = value.match(/filename=([^;]+)/iu);
  return filenameMatch?.[1]?.trim();
}

function decodeHeaderFilename(value: string): string | undefined {
  try {
    return decodeURIComponent(value.trim());
  } catch {
    return value.trim() || undefined;
  }
}

function extensionForFilename(value: string | undefined): string | undefined {
  const filename = baseFilename(value);
  const match = filename?.match(/\.([a-z0-9]{1,8})$/iu);
  return match?.[1]?.toLowerCase();
}

function extensionForUrl(value: string): string | undefined {
  try {
    const pathname = new URL(value).pathname;
    return extensionForFilename(pathname);
  } catch {
    return undefined;
  }
}

function positiveNumberHeaderValue(headers: Header[] | undefined, name: string): number | undefined {
  return positiveNumber(numberHeaderValue(headers, name));
}

function positiveNumber(value: number | undefined): number | undefined {
  return typeof value === "number" && Number.isFinite(value) && value > 0 ? value : undefined;
}

function isAttachment(value: string | undefined): boolean {
  return value?.toLowerCase().includes("attachment") ?? false;
}
