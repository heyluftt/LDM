import type { DetectedDownloadCandidate } from "@ldm/contracts";
import { defineContentScript } from "wxt/utils/define-content-script";
import { adapters } from "../adapters";
import { installBrowserPreferencesCache } from "../content/browser-preferences-cache";
import {
  chooseGenericVideoCandidate,
  chooseXVideoCandidate,
  countUsableVideos,
  currentPerformanceTime,
  currentResourceCandidateStartTime,
  diagnosticCandidatesForVideo,
  isXPage,
  mergeRequestCandidates,
  setResourceCandidateStartTime,
} from "../content/candidate-selection";
import { requestDownload } from "../content/job-request";
import { installDownloadLinkInterceptor } from "../content/link-interceptor";
import { collectCandidates, collectVideoDownloadTargets } from "../content/media-detector";
import { ensureVideoOverlayButtons, repositionVideoOverlays, resetDismissedOverlays } from "../content/overlay-button";
import { currentPageTitle } from "../content/page-title";
import { clearNetworkCandidates, getNetworkCandidates, sendRuntimeMessage } from "../content/runtime-messaging";

export default defineContentScript({
  matches: ["<all_urls>"],
  runAt: "document_start",
  allFrames: true,
  matchAboutBlank: true,
  matchOriginAsFallback: true,
  main() {
    installBrowserPreferencesCache(() => sendRuntimeMessage({ type: "ldm:getBrowserPreferences" }));
    installDownloadLinkInterceptor();
    let lastPageUrl = window.location.href;
    let renderTimer: number | undefined;
    let renderInFlight = false;
    let renderPending = false;
    let repositionFrame: number | undefined;
    setResourceCandidateStartTime(currentPerformanceTime());

    const clearVideoOverlays = () => {
      ensureVideoOverlayButtons({
        targets: [],
        onDownload: async () => ({ ok: false, message: "Page changed." }),
      });
    };

    const clearPageScopedCandidates = async () => {
      setResourceCandidateStartTime(currentPerformanceTime());
      await clearNetworkCandidates();
    };

    const scheduleRender = (delayMs = 80) => {
      if (renderTimer !== undefined) {
        window.clearTimeout(renderTimer);
      }

      renderTimer = window.setTimeout(() => {
        renderTimer = undefined;
        void render();
      }, delayMs);
    };

    // Buttons follow their video on the next frame; the full candidate pass stays debounced.
    const requestReposition = () => {
      if (repositionFrame !== undefined) {
        return;
      }
      repositionFrame = window.requestAnimationFrame(() => {
        repositionFrame = undefined;
        repositionVideoOverlays();
      });
    };

    const renderOnce = async () => {
      const pageUrl = window.location.href;
      if (pageUrl !== lastPageUrl) {
        lastPageUrl = pageUrl;
        await clearPageScopedCandidates();
        clearVideoOverlays();
        resetDismissedOverlays();
      }

      const pageTitle = currentPageTitle();
      const candidates = collectCandidates(document, pageUrl, pageTitle, {
        minResourceStartTime: currentResourceCandidateStartTime(),
      });
      const [networkCandidates, providerCandidates] = await Promise.all([
        getNetworkCandidates(pageTitle),
        detectProviderCandidates(pageTitle),
      ]);
      if (window.location.href !== pageUrl) {
        renderPending = true;
        return;
      }

      const requestCandidates = mergeRequestCandidates(networkCandidates, candidates);
      const isX = isXPage(pageUrl);
      const usableVideoCount = countUsableVideos(document);
      const videoTargets = collectVideoDownloadTargets(
        document,
        pageUrl,
        pageTitle,
        undefined,
        (video, domVideoCandidates) =>
          isX
            ? chooseXVideoCandidate(video, domVideoCandidates, requestCandidates, providerCandidates)
            : chooseGenericVideoCandidate(
                video,
                domVideoCandidates,
                requestCandidates,
                providerCandidates,
                usableVideoCount,
                pageTitle,
              ),
      ).map((target) => ({
        ...target,
        debugCandidates: diagnosticCandidatesForVideo(
          target.video,
          target.candidate,
          requestCandidates,
          providerCandidates,
          isX,
          usableVideoCount,
        ),
      }));

      ensureVideoOverlayButtons({
        targets: videoTargets,
        onDownload: async (candidate, context) => {
          const result = await requestDownload(candidate, context);
          return result.ok ? { ok: true } : { ok: false, message: result.message, code: result.code };
        },
      });
    };

    // Renders never abort each other. A request that arrives mid-render runs once more after
    // it, so buttons are brought up to date even while the mouse keeps moving over the page.
    const render = async () => {
      if (renderInFlight) {
        renderPending = true;
        return;
      }
      renderInFlight = true;
      try {
        do {
          renderPending = false;
          await renderOnce();
        } while (renderPending);
      } catch {
        // A failed pass leaves the previous buttons in place; the next trigger retries.
      } finally {
        renderInFlight = false;
      }
    };

    installLocationChangeWatcher(() => {
      clearVideoOverlays();
      resetDismissedOverlays();
      scheduleRender(40);
    });

    void render();
    const observer = new MutationObserver(() => {
      scheduleRender();
    });
    if (document.documentElement) {
      observer.observe(document.documentElement, { childList: true, subtree: true });
    }
    document.addEventListener("DOMContentLoaded", () => scheduleRender(0), { once: true });
    document.addEventListener("pointerover", () => scheduleRender(40), { passive: true, capture: true });
    document.addEventListener("pointerout", () => scheduleRender(80), { passive: true, capture: true });
    window.addEventListener(
      "scroll",
      () => {
        requestReposition();
        scheduleRender(150);
      },
      { passive: true, capture: true },
    );
    window.addEventListener(
      "resize",
      () => {
        requestReposition();
        scheduleRender(150);
      },
      { passive: true },
    );
    const intervalId = window.setInterval(() => {
      scheduleRender();
    }, 1500);
    window.addEventListener(
      "pagehide",
      () => {
        window.clearInterval(intervalId);
        if (renderTimer !== undefined) {
          window.clearTimeout(renderTimer);
        }
        if (repositionFrame !== undefined) {
          window.cancelAnimationFrame(repositionFrame);
        }
        clearVideoOverlays();
      },
      { once: true },
    );
  },
});

function installLocationChangeWatcher(onChange: () => void): void {
  const notify = () => {
    window.setTimeout(onChange, 0);
  };
  const originalPushState = history.pushState.bind(history);
  const originalReplaceState = history.replaceState.bind(history);

  history.pushState = (data: unknown, unused: string, url?: string | URL | null) => {
    originalPushState(data, unused, url);
    notify();
  };
  history.replaceState = (data: unknown, unused: string, url?: string | URL | null) => {
    originalReplaceState(data, unused, url);
    notify();
  };
  window.addEventListener("popstate", notify);
  window.addEventListener("hashchange", notify);
}

async function detectProviderCandidates(pageTitle: string): Promise<DetectedDownloadCandidate[]> {
  const context = {
    pageUrl: window.location.href,
    pageTitle,
    referrer: document.referrer || undefined,
    documentContentType: document.contentType,
  };

  const results = await Promise.all(adapters.map((adapter) => adapter.detect(context)));
  return results.flat();
}
