import type {
  BrowserIntegrationPreferences,
  CreateDownloadJobRequest,
  DesktopToExtensionMessage,
  DetectedDownloadCandidate,
} from "@ldm/contracts";
import { defineBackground } from "wxt/utils/define-background";
import {
  allowNextBrowserDownloadFallback,
  armBrowserStartupGuard,
  getBrowserDownloadPreferences,
  maybeRouteBrowserDownload,
  noteExecutableJob,
  rememberBrowserDownloadRequestHeaders,
  rememberBrowserDownloadResponse,
} from "../background/browser-downloads";
import {
  clearTabMediaRequests,
  getMediaCandidatesForTab,
  rememberMediaRequest,
  rememberMediaRequestHeaders,
} from "../background/media-requests";
import { prewarmNativeMessaging, sendNativeMessage } from "../background/native-messaging";

type ContentMessage =
  | { type: "ldm:allowBrowserDownloadFallback"; url: string }
  | { type: "ldm:clearNetworkCandidates" }
  | { type: "ldm:createJob"; job: CreateDownloadJobRequest }
  | { type: "ldm:getBrowserPreferences" }
  | { type: "ldm:getNetworkCandidates"; pageUrl: string; pageTitle?: string }
  | { type: "ldm:ping" };

type BackgroundResponse =
  | DesktopToExtensionMessage
  | { type: "browserDownloadFallbackAllowed" }
  | { type: "browserPreferences"; preferences: BrowserIntegrationPreferences | null }
  | { type: "networkCandidates"; candidates: DetectedDownloadCandidate[] };

const CONTENT_MESSAGE_TYPES = new Set<ContentMessage["type"]>([
  "ldm:allowBrowserDownloadFallback",
  "ldm:clearNetworkCandidates",
  "ldm:createJob",
  "ldm:getBrowserPreferences",
  "ldm:getNetworkCandidates",
  "ldm:ping",
]);

export default defineBackground(() => {
  prewarmNativeMessaging();
  allowContentScriptsToReadSessionStorage();
  chrome.runtime.onStartup.addListener(armBrowserStartupGuard);
  chrome.webRequest.onBeforeSendHeaders.addListener(rememberMediaRequestHeaders, { urls: ["<all_urls>"] }, [
    "requestHeaders",
    "extraHeaders",
  ]);
  chrome.webRequest.onBeforeSendHeaders.addListener(rememberBrowserDownloadRequestHeaders, { urls: ["<all_urls>"] }, [
    "requestHeaders",
    "extraHeaders",
  ]);
  chrome.webRequest.onHeadersReceived.addListener(rememberMediaRequest, { urls: ["<all_urls>"] }, ["responseHeaders"]);
  chrome.webRequest.onHeadersReceived.addListener(rememberBrowserDownloadResponse, { urls: ["<all_urls>"] }, [
    "responseHeaders",
  ]);
  // onDeterminingFilename fires once per new download with the browser's filename. The
  // download is never held back here: routing pauses it itself while LDM is asked.
  chrome.downloads.onDeterminingFilename.addListener((item, suggest) => {
    suggest();
    void maybeRouteBrowserDownload(item, sendNativeMessage);
  });
  chrome.tabs.onRemoved.addListener(clearTabMediaRequests);
  chrome.tabs.onUpdated.addListener((tabId, changeInfo) => {
    if (changeInfo.url) {
      clearTabMediaRequests(tabId);
    }
  });

  chrome.runtime.onMessage.addListener(
    (message: unknown, sender: chrome.runtime.MessageSender, sendResponse: (response?: BackgroundResponse) => void) => {
      if (!isContentMessage(message)) {
        return false;
      }

      switch (message.type) {
        case "ldm:allowBrowserDownloadFallback": {
          allowNextBrowserDownloadFallback(message.url);
          sendResponse({ type: "browserDownloadFallbackAllowed" });
          return false;
        }
        case "ldm:clearNetworkCandidates": {
          const tabId = sender.tab?.id;
          if (typeof tabId === "number") {
            clearTabMediaRequests(tabId);
          }
          sendResponse({ type: "networkCandidates", candidates: [] });
          return false;
        }
        case "ldm:getNetworkCandidates": {
          const tabId = sender.tab?.id;
          const candidates =
            typeof tabId === "number" ? getMediaCandidatesForTab(tabId, message.pageUrl, message.pageTitle) : [];
          sendResponse({ type: "networkCandidates", candidates });
          return false;
        }
        case "ldm:getBrowserPreferences": {
          void getBrowserDownloadPreferences(sendNativeMessage).then((preferences) => {
            sendResponse({ type: "browserPreferences", preferences: preferences ?? null });
          });
          return true;
        }
        case "ldm:ping": {
          void sendNativeMessage({ type: "ping", schemaVersion: 1 }).then(sendResponse);
          return true;
        }
        case "ldm:createJob": {
          void getBrowserDownloadPreferences(sendNativeMessage)
            .then((preferences) => {
              noteExecutableJob(message.job, preferences);
              return sendNativeMessage({ type: "createJob", schemaVersion: 1, job: message.job });
            })
            .then(sendResponse);
          return true;
        }
      }
    },
  );
});

function isContentMessage(value: unknown): value is ContentMessage {
  return (
    typeof value === "object" &&
    value !== null &&
    "type" in value &&
    typeof value.type === "string" &&
    CONTENT_MESSAGE_TYPES.has(value.type as ContentMessage["type"])
  );
}

/** Lets content scripts read the preferences copy the background keeps in session storage. */
function allowContentScriptsToReadSessionStorage(): void {
  try {
    void chrome.storage.session.setAccessLevel({ accessLevel: "TRUSTED_AND_UNTRUSTED_CONTEXTS" });
  } catch {
    // Older browsers without session storage access levels: content scripts ask the background instead.
  }
}
