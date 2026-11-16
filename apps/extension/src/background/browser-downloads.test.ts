import type { DesktopToExtensionMessage, ExtensionToDesktopMessage } from "@ldm/contracts";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  allowNextBrowserDownloadFallback,
  armBrowserStartupGuard,
  type BrowserDownloadItem,
  clearBrowserDownloadMetadata,
  createBrowserDownloadJob,
  getBrowserDownloadPreferences,
  maybeRouteBrowserDownload,
  rememberBrowserDownloadRequestHeaders,
  rememberBrowserDownloadResponse,
  setBrowserDownloadPreferencesForTests,
  setBrowserDownloadPromptForTests,
} from "./browser-downloads";

beforeEach(() => {
  vi.stubGlobal("chrome", {
    runtime: {
      lastError: undefined,
    },
  });
  setBrowserDownloadPreferencesForTests({
    mode: "auto",
    selectedKinds: defaultSelectedKinds(),
    domainRules: [],
    warnOnExecutable: true,
  });
});

afterEach(() => {
  clearBrowserDownloadMetadata();
  vi.unstubAllGlobals();
});

describe("browser download job creation", () => {
  it("builds direct jobs for program downloads without leaking local paths", () => {
    rememberBrowserDownloadRequestHeaders({
      url: "https://example.test/releases/installer.exe",
      requestHeaders: [
        { name: "User-Agent", value: "Browser UA" },
        { name: "Accept-Language", value: "de-DE" },
        { name: "Cookie", value: "blocked=true" },
      ],
    });
    rememberBrowserDownloadResponse({
      url: "https://example.test/releases/installer.exe",
      responseHeaders: [
        { name: "Content-Type", value: "application/x-msdownload" },
        { name: "Content-Length", value: "50000000" },
      ],
    });

    const job = createBrowserDownloadJob({
      id: 1,
      url: "https://example.test/releases/installer.exe",
      filename: "C:\\Downloads\\installer.exe",
      mime: "application/x-msdownload",
      fileSize: -1,
      totalBytes: -1,
      referrer: "https://example.test/releases",
    });

    expect(job).toMatchObject({
      adapterId: "browser-download",
      kind: "program",
      suggestedFilename: "installer.exe",
      contentLength: 50_000_000,
      pageUrl: "https://example.test/releases",
      preferRangeRequest: true,
      headers: {
        "accept-language": "de-DE",
        "user-agent": "Browser UA",
      },
    });
    expect(job?.suggestedFilename).not.toContain("Users");
  });

  it("skips browser-like html downloads without a file signal", () => {
    expect(
      createBrowserDownloadJob({
        id: 2,
        url: "https://example.test/page",
        filename: "page",
        mime: "text/html",
        fileSize: 2048,
        totalBytes: 2048,
      }),
    ).toBeNull();
  });

  it("uses query filenames for extensionless browser downloads", () => {
    const job = createBrowserDownloadJob({
      id: 3,
      url: "https://example.test/file-download/112?filename=setup.exe",
      filename: "",
      mime: "application/octet-stream",
      fileSize: 1024,
      totalBytes: 1024,
      startTime: new Date().toISOString(),
    });

    expect(job).toMatchObject({
      kind: "program",
      suggestedFilename: "setup.exe",
    });
  });

  it("uses the original GitHub release filename when the final asset URL has no extension", () => {
    const releaseUrl = "https://github.com/example/app/releases/download/v1.0.0/app_1.0.0_x64-setup.exe";
    const finalUrl =
      "https://objects.githubusercontent.com/github-production-release-asset-123/asset?X-Amz-Signature=redacted";
    rememberBrowserDownloadResponse({
      url: finalUrl,
      responseHeaders: [{ name: "Content-Type", value: "application/octet-stream" }],
    });

    const job = createBrowserDownloadJob({
      id: 4,
      url: releaseUrl,
      finalUrl,
      filename: "",
      mime: "application/octet-stream",
      fileSize: -1,
      totalBytes: -1,
      startTime: new Date().toISOString(),
    });

    expect(job).toMatchObject({
      sourceUrl: finalUrl,
      sourceFallbackUrls: [releaseUrl],
      kind: "program",
      suggestedFilename: "app_1.0.0_x64-setup.exe",
    });
  });

  it("keeps only the basename from content disposition filenames", () => {
    rememberBrowserDownloadResponse({
      url: "https://example.test/file-download/112",
      responseHeaders: [
        { name: "Content-Disposition", value: 'attachment; filename="C:\\Downloads\\setup.exe"' },
        { name: "Content-Type", value: "application/octet-stream" },
      ],
    });

    const job = createBrowserDownloadJob({
      id: 4,
      url: "https://example.test/file-download/112",
      filename: "",
      mime: "",
      fileSize: -1,
      totalBytes: -1,
      startTime: new Date().toISOString(),
    });

    expect(job).toMatchObject({
      kind: "program",
      suggestedFilename: "setup.exe",
    });
  });
});

describe("browser download routing", () => {
  it("pauses the browser download, hands it to LDM, and removes it only after acceptance", async () => {
    const calls: string[] = [];
    const sendNativeMessage = vi.fn(async (message: ExtensionToDesktopMessage): Promise<DesktopToExtensionMessage> => {
      expect(message).toMatchObject({
        type: "createJob",
        job: {
          sourceUrl: "https://example.test/file.zip",
          kind: "archive",
        },
      });
      calls.push("native");
      return { type: "jobAccepted", schemaVersion: 1, jobId: "job-1" };
    });

    await maybeRouteBrowserDownload(
      downloadItem("https://example.test/file.zip"),
      sendNativeMessage,
      recordingApi(calls),
    );

    expect(sendNativeMessage).toHaveBeenCalledTimes(1);
    expect(calls).toEqual(["pause:7", "native", "cancel:7", "erase:7"]);
  });

  it("resumes the browser download when LDM rejects the job", async () => {
    const calls: string[] = [];

    await maybeRouteBrowserDownload(
      downloadItem("https://example.test/manual.pdf"),
      async () => ({ type: "jobRejected", schemaVersion: 1, reason: "offline" }),
      recordingApi(calls),
    );

    expect(calls).toEqual(["pause:7", "resume:7"]);
  });

  it("resumes the browser download when native messaging throws", async () => {
    const calls: string[] = [];

    await maybeRouteBrowserDownload(
      downloadItem("https://example.test/manual.pdf"),
      async () => {
        throw new Error("native host closed");
      },
      recordingApi(calls),
    );

    expect(calls).toEqual(["pause:7", "resume:7"]);
  });

  it("resumes the browser download while first-run setup is open", async () => {
    const calls: string[] = [];

    await maybeRouteBrowserDownload(
      downloadItem("https://example.test/manual.pdf"),
      async () => ({
        type: "jobRejected",
        schemaVersion: 1,
        reason: "Open LDM and complete the first-run setup.",
      }),
      recordingApi(calls),
    );

    expect(calls).toEqual(["pause:7", "resume:7"]);
  });

  it("leaves a download alone when it can no longer be paused", async () => {
    const calls: string[] = [];
    const sendNativeMessage = vi.fn(
      async (): Promise<DesktopToExtensionMessage> => ({
        type: "jobAccepted",
        schemaVersion: 1,
        jobId: "job-1",
      }),
    );

    await maybeRouteBrowserDownload(downloadItem("https://example.test/sample.zip"), sendNativeMessage, {
      ...recordingApi(calls),
      pause(id, callback) {
        calls.push(`pause:${id}`);
        (chrome.runtime as { lastError?: chrome.runtime.LastError }).lastError = { message: "already complete" };
        callback?.();
        (chrome.runtime as { lastError?: chrome.runtime.LastError }).lastError = undefined;
      },
    });

    expect(sendNativeMessage).not.toHaveBeenCalled();
    expect(calls).toEqual(["pause:7"]);
  });

  it("keeps the browser copy when LDM accepted but the download cannot be canceled anymore", async () => {
    const calls: string[] = [];

    await maybeRouteBrowserDownload(
      downloadItem("https://example.test/sample.zip"),
      async () => ({ type: "jobAccepted", schemaVersion: 1, jobId: "job-1" }),
      {
        ...recordingApi(calls),
        cancel(id, callback) {
          calls.push(`cancel:${id}`);
          (chrome.runtime as { lastError?: chrome.runtime.LastError }).lastError = { message: "already complete" };
          callback?.();
          (chrome.runtime as { lastError?: chrome.runtime.LastError }).lastError = undefined;
        },
      },
    );

    expect(calls).toEqual(["pause:7", "cancel:7"]);
  });

  it("ignores browser downloads restored from an earlier session", async () => {
    const sendNativeMessage = vi.fn(
      async (): Promise<DesktopToExtensionMessage> => ({
        type: "jobAccepted",
        schemaVersion: 1,
        jobId: "job-1",
      }),
    );
    const item = downloadItem("https://example.test/old-file.zip");
    item.startTime = "2020-01-01T00:00:00.000Z";

    await maybeRouteBrowserDownload(item, sendNativeMessage, strictDownloadsApi());

    expect(sendNativeMessage).not.toHaveBeenCalled();
  });

  it("ignores automatic routing during browser startup", async () => {
    const sendNativeMessage = vi.fn(
      async (): Promise<DesktopToExtensionMessage> => ({
        type: "jobAccepted",
        schemaVersion: 1,
        jobId: "job-1",
      }),
    );
    armBrowserStartupGuard();

    await maybeRouteBrowserDownload(
      downloadItem("https://example.test/startup-file.zip"),
      sendNativeMessage,
      strictDownloadsApi(),
    );

    expect(sendNativeMessage).not.toHaveBeenCalled();
  });

  it("ignores browser downloads that already completed before routing", async () => {
    const sendNativeMessage = vi.fn(
      async (): Promise<DesktopToExtensionMessage> => ({
        type: "jobAccepted",
        schemaVersion: 1,
        jobId: "job-1",
      }),
    );
    const item = downloadItem("https://example.test/finished-file.zip");
    item.state = "complete";

    await maybeRouteBrowserDownload(item, sendNativeMessage, strictDownloadsApi());

    expect(sendNativeMessage).not.toHaveBeenCalled();
  });

  it("routes the same browser download id only once per session", async () => {
    const calls: string[] = [];
    const sendNativeMessage = vi.fn(
      async (): Promise<DesktopToExtensionMessage> => ({
        type: "jobAccepted",
        schemaVersion: 1,
        jobId: "job-1",
      }),
    );
    const api = recordingApi(calls);
    const item = downloadItem("https://example.test/one-file.zip");

    await maybeRouteBrowserDownload(item, sendNativeMessage, api);
    await maybeRouteBrowserDownload(item, sendNativeMessage, api);

    expect(sendNativeMessage).toHaveBeenCalledTimes(1);
    expect(calls).toEqual(["pause:7", "cancel:7", "erase:7"]);
  });

  it("does not route a resumed browser download again", async () => {
    const calls: string[] = [];
    const item = downloadItem("https://example.test/manual.pdf");
    await maybeRouteBrowserDownload(
      item,
      async () => ({ type: "jobRejected", schemaVersion: 1, reason: "offline" }),
      recordingApi(calls),
    );

    await maybeRouteBrowserDownload(
      item,
      async () => {
        throw new Error("resumed browser download should not route");
      },
      strictDownloadsApi(),
    );

    expect(calls).toEqual(["pause:7", "resume:7"]);
  });

  it("shares in-flight routing between browser download events", async () => {
    const calls: string[] = [];
    let finishNativeMessage: (() => void) | undefined;
    const sendNativeMessage = vi.fn(async (): Promise<DesktopToExtensionMessage> => {
      await new Promise<void>((resolve) => {
        finishNativeMessage = resolve;
      });
      return { type: "jobAccepted", schemaVersion: 1, jobId: "job-1" };
    });
    const api = recordingApi(calls);
    const item = downloadItem("https://example.test/one-file.zip");

    const firstRouting = maybeRouteBrowserDownload(item, sendNativeMessage, api);
    const secondRouting = maybeRouteBrowserDownload(item, sendNativeMessage, api);

    expect(secondRouting).toBe(firstRouting);
    await vi.waitFor(() => {
      expect(sendNativeMessage).toHaveBeenCalledTimes(1);
    });
    expect(calls).toEqual(["pause:7"]);

    finishNativeMessage?.();
    await firstRouting;
    expect(calls).toEqual(["pause:7", "cancel:7", "erase:7"]);
  });

  it("respects off browser handling without touching the download", async () => {
    setBrowserDownloadPreferencesForTests({
      mode: "off",
      selectedKinds: defaultSelectedKinds(),
      domainRules: [],
      warnOnExecutable: true,
    });
    const sendNativeMessage = vi.fn(
      async (): Promise<DesktopToExtensionMessage> => ({
        type: "jobAccepted",
        schemaVersion: 1,
        jobId: "job-1",
      }),
    );

    await maybeRouteBrowserDownload(
      downloadItem("https://example.test/file.zip"),
      sendNativeMessage,
      strictDownloadsApi(),
    );

    expect(sendNativeMessage).not.toHaveBeenCalled();
  });

  it("respects ask browser handling when the user keeps the browser download", async () => {
    setBrowserDownloadPreferencesForTests({
      mode: "ask",
      selectedKinds: defaultSelectedKinds(),
      domainRules: [],
      warnOnExecutable: true,
    });
    const sendNativeMessage = vi.fn(
      async (): Promise<DesktopToExtensionMessage> => ({
        type: "jobAccepted",
        schemaVersion: 1,
        jobId: "job-1",
      }),
    );
    const prompt = vi.fn(async () => false);
    setBrowserDownloadPromptForTests(prompt);
    const calls: string[] = [];
    const api = recordingApi(calls);

    await maybeRouteBrowserDownload(downloadItem("https://example.test/file.zip"), sendNativeMessage, api);
    await maybeRouteBrowserDownload(downloadItem("https://example.test/file.zip"), sendNativeMessage, api);

    expect(prompt).toHaveBeenCalledWith({
      filename: "file.zip",
      kind: "archive",
      reason: "ask",
    });
    expect(prompt).toHaveBeenCalledTimes(1);
    expect(sendNativeMessage).not.toHaveBeenCalled();
    expect(calls).toEqual(["pause:7", "resume:7"]);
  });

  it("routes ask browser handling after explicit confirmation", async () => {
    setBrowserDownloadPreferencesForTests({
      mode: "ask",
      selectedKinds: defaultSelectedKinds(),
      domainRules: [],
      warnOnExecutable: true,
    });
    setBrowserDownloadPromptForTests(async () => true);
    const calls: string[] = [];
    const sendNativeMessage = vi.fn(
      async (): Promise<DesktopToExtensionMessage> => ({
        type: "jobAccepted",
        schemaVersion: 1,
        jobId: "job-1",
      }),
    );

    await maybeRouteBrowserDownload(
      downloadItem("https://example.test/file.zip"),
      sendNativeMessage,
      recordingApi(calls),
    );

    expect(sendNativeMessage).toHaveBeenCalledTimes(1);
    expect(calls).toEqual(["pause:7", "cancel:7", "erase:7"]);
  });

  it("respects selected file types for automatic browser handling", async () => {
    setBrowserDownloadPreferencesForTests({
      mode: "auto",
      selectedKinds: { ...defaultSelectedKinds(), archive: false },
      domainRules: [],
      warnOnExecutable: true,
    });
    const sendNativeMessage = vi.fn(
      async (): Promise<DesktopToExtensionMessage> => ({
        type: "jobAccepted",
        schemaVersion: 1,
        jobId: "job-1",
      }),
    );

    await maybeRouteBrowserDownload(
      downloadItem("https://example.test/file.zip"),
      sendNativeMessage,
      strictDownloadsApi(),
    );

    expect(sendNativeMessage).not.toHaveBeenCalled();
  });

  it("uses the most specific domain rule for browser downloads", async () => {
    setBrowserDownloadPreferencesForTests({
      mode: "auto",
      selectedKinds: { ...defaultSelectedKinds(), archive: false },
      domainRules: [
        { host: "example.test", action: "off" },
        { host: "downloads.example.test", action: "auto" },
      ],
      warnOnExecutable: true,
    });
    const sendNativeMessage = vi.fn(
      async (): Promise<DesktopToExtensionMessage> => ({
        type: "jobAccepted",
        schemaVersion: 1,
        jobId: "job-1",
      }),
    );

    await maybeRouteBrowserDownload(
      downloadItem("https://downloads.example.test/file.zip"),
      sendNativeMessage,
      acceptingDownloadsApi(),
    );

    expect(sendNativeMessage).toHaveBeenCalledTimes(1);
  });

  it("applies page domain rules to browser downloads served from a different host", async () => {
    setBrowserDownloadPreferencesForTests({
      mode: "auto",
      selectedKinds: defaultSelectedKinds(),
      domainRules: [{ host: "example.test", action: "off" }],
      warnOnExecutable: true,
    });
    const sendNativeMessage = vi.fn(
      async (): Promise<DesktopToExtensionMessage> => ({
        type: "jobAccepted",
        schemaVersion: 1,
        jobId: "job-1",
      }),
    );

    await maybeRouteBrowserDownload(
      {
        ...downloadItem("https://cdn.example-cdn.test/file.zip"),
        referrer: "https://example.test/downloads",
      },
      sendNativeMessage,
      strictDownloadsApi(),
    );

    expect(sendNativeMessage).not.toHaveBeenCalled();
  });

  it("loads browser integration settings through native messaging and caches them", async () => {
    setBrowserDownloadPreferencesForTests(undefined);
    const calls: string[] = [];
    const sendNativeMessage = vi.fn(async (message: ExtensionToDesktopMessage): Promise<DesktopToExtensionMessage> => {
      calls.push(message.type);
      if (message.type === "getBrowserIntegrationPreferences") {
        return {
          type: "browserIntegrationPreferences",
          schemaVersion: 1,
          preferences: {
            mode: "auto",
            selectedKinds: defaultSelectedKinds(),
            domainRules: [],
            warnOnExecutable: true,
          },
        };
      }
      return { type: "jobAccepted", schemaVersion: 1, jobId: "job-1" };
    });

    await maybeRouteBrowserDownload(
      downloadItem("https://example.test/file.zip"),
      sendNativeMessage,
      acceptingDownloadsApi(),
    );
    await maybeRouteBrowserDownload(
      { ...downloadItem("https://example.test/second.zip"), id: 8 },
      sendNativeMessage,
      acceptingDownloadsApi(),
    );
    const cached = await getBrowserDownloadPreferences(sendNativeMessage);

    expect(calls).toEqual(["getBrowserIntegrationPreferences", "createJob", "createJob"]);
    expect(cached?.mode).toBe("auto");
  });

  it("leaves the browser download alone when the preferences cannot be loaded", async () => {
    setBrowserDownloadPreferencesForTests(undefined);
    const sendNativeMessage = vi.fn(
      async (): Promise<DesktopToExtensionMessage> => ({
        type: "jobRejected",
        schemaVersion: 1,
        reason: "Desktop app could not be reached.",
      }),
    );

    await maybeRouteBrowserDownload(
      downloadItem("https://example.test/file.zip"),
      sendNativeMessage,
      strictDownloadsApi(),
    );

    expect(sendNativeMessage).toHaveBeenCalledTimes(1);
  });

  it("lets an always-use domain rule override global file type selection", async () => {
    setBrowserDownloadPreferencesForTests({
      mode: "auto",
      selectedKinds: { ...defaultSelectedKinds(), archive: false },
      domainRules: [{ host: "example.test", action: "auto" }],
      warnOnExecutable: true,
    });
    const sendNativeMessage = vi.fn(
      async (): Promise<DesktopToExtensionMessage> => ({
        type: "jobAccepted",
        schemaVersion: 1,
        jobId: "job-1",
      }),
    );

    await maybeRouteBrowserDownload(
      downloadItem("https://downloads.example.test/file.zip"),
      sendNativeMessage,
      acceptingDownloadsApi(),
    );

    expect(sendNativeMessage).toHaveBeenCalledTimes(1);
  });

  it("shows one non-blocking warning while automatically routing executable downloads", async () => {
    const notificationCreate = vi.fn((_: string, __: unknown, callback?: () => void) => {
      callback?.();
    });
    vi.stubGlobal("chrome", {
      runtime: {
        lastError: undefined,
      },
      notifications: {
        create: notificationCreate,
      },
    });
    const prompt = vi.fn(async () => true);
    setBrowserDownloadPromptForTests(prompt);
    const sendNativeMessage = vi.fn(
      async (): Promise<DesktopToExtensionMessage> => ({
        type: "jobAccepted",
        schemaVersion: 1,
        jobId: "job-1",
      }),
    );
    const api = acceptingDownloadsApi();

    await maybeRouteBrowserDownload(downloadItem("https://example.test/first.exe"), sendNativeMessage, api);
    await maybeRouteBrowserDownload(
      { ...downloadItem("https://example.test/second.exe"), id: 8 },
      sendNativeMessage,
      api,
    );

    expect(prompt).not.toHaveBeenCalled();
    expect(notificationCreate).toHaveBeenCalledTimes(1);
    expect(sendNativeMessage).toHaveBeenCalledTimes(2);
  });

  it("lets explicit fallback downloads pass through once", async () => {
    const sendNativeMessage = vi.fn(
      async (): Promise<DesktopToExtensionMessage> => ({
        type: "jobAccepted",
        schemaVersion: 1,
        jobId: "job-1",
      }),
    );
    allowNextBrowserDownloadFallback("https://example.test/fallback.zip");

    await maybeRouteBrowserDownload(
      downloadItem("https://example.test/fallback.zip"),
      sendNativeMessage,
      strictDownloadsApi(),
    );
    await maybeRouteBrowserDownload(
      downloadItem("https://example.test/fallback.zip"),
      sendNativeMessage,
      strictDownloadsApi(),
    );

    expect(sendNativeMessage).not.toHaveBeenCalled();
  });
});

function downloadItem(url: string): BrowserDownloadItem {
  return {
    id: 7,
    url,
    filename: filenameFromUrl(url),
    mime: "",
    fileSize: 10_000,
    totalBytes: 10_000,
    startTime: new Date().toISOString(),
  };
}

function defaultSelectedKinds() {
  return {
    video: true,
    audio: true,
    program: true,
    document: true,
    archive: true,
    image: true,
    other: false,
  };
}

function recordingApi(calls: string[]) {
  return {
    cancel(id: number, callback?: () => void) {
      calls.push(`cancel:${id}`);
      callback?.();
    },
    erase(query: chrome.downloads.DownloadQuery, callback?: (ids: number[]) => void) {
      calls.push(`erase:${query.id}`);
      callback?.([query.id as number]);
    },
    pause(id: number, callback?: () => void) {
      calls.push(`pause:${id}`);
      callback?.();
    },
    resume(id: number, callback?: () => void) {
      calls.push(`resume:${id}`);
      callback?.();
    },
  };
}

function strictDownloadsApi() {
  return {
    cancel() {
      throw new Error("should not cancel");
    },
    erase() {
      throw new Error("should not erase");
    },
    pause() {
      throw new Error("should not pause");
    },
    resume() {
      throw new Error("should not resume");
    },
  };
}

function acceptingDownloadsApi() {
  return {
    cancel(_id: number, callback?: () => void) {
      callback?.();
    },
    erase(_query: chrome.downloads.DownloadQuery, callback?: (ids: number[]) => void) {
      callback?.([]);
    },
    pause(_id: number, callback?: () => void) {
      callback?.();
    },
    resume(_id: number, callback?: () => void) {
      callback?.();
    },
  };
}

function filenameFromUrl(value: string): string {
  return new URL(value).pathname.split("/").filter(Boolean).pop() ?? "download";
}
