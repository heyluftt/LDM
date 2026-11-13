import { type DetectedDownloadCandidate, PLATFORM_USAGE_NOTICE, type RejectionCode } from "@ldm/contracts";
import { createBrandMarkElement } from "../shared/brand-mark";
import { hashString } from "../shared/url";
import { buildCandidateDebugReport, buildCandidateDiagnostics } from "./candidate-diagnostics";
import { overlayErrorMessage } from "./overlay-messages";

type OverlayResult = { ok: true } | { ok: false; message: string; code?: RejectionCode };
type OverlayDownloadContext = {
  video: HTMLVideoElement;
  setStatus: (message: string) => void;
  debugCandidates?: DetectedDownloadCandidate[];
};

type VideoOverlayTarget = {
  id: string;
  video: HTMLVideoElement;
  candidate: DetectedDownloadCandidate;
  debugCandidates?: DetectedDownloadCandidate[];
};

type VideoOverlayOptions = {
  targets: VideoOverlayTarget[];
  onDownload: (candidate: DetectedDownloadCandidate, context: OverlayDownloadContext) => Promise<OverlayResult>;
};

const ROOT_ATTR = "data-ldm-target-id";
const OVERLAY_INSET = 10;
const MIN_VISIBLE_VIDEO_WIDTH = 140;
const MIN_VISIBLE_VIDEO_HEIGHT = 56;

/** Videos whose button the user closed; they stay hidden until the page URL changes. */
let dismissedVideos = new WeakSet<HTMLVideoElement>();
/** The video each overlay root follows, for cheap repositioning on scroll and resize. */
const overlayVideos = new WeakMap<HTMLDivElement, HTMLVideoElement>();

export function ensureVideoOverlayButtons(options: VideoOverlayOptions): void {
  const targets = options.targets.filter((target) => !dismissedVideos.has(target.video));
  const activeIds = new Set(targets.map((target) => target.id));

  for (const target of targets) {
    const host = overlayHost();
    if (!host) {
      continue;
    }

    const existing = host.querySelector<HTMLDivElement>(`[${ROOT_ATTR}="${target.id}"]`);
    const signature = candidateSignature(target.candidate);
    if (existing) {
      if (existing.dataset.ldmCandidateSignature !== signature) {
        existing.replaceWith(createOverlayRoot(target, options.onDownload));
      } else {
        positionOverlayRoot(existing, target.video);
      }
      continue;
    }

    host.append(createOverlayRoot(target, options.onDownload));
  }

  document.querySelectorAll<HTMLDivElement>(`[${ROOT_ATTR}]`).forEach((root) => {
    const targetId = root.getAttribute(ROOT_ATTR);
    if (targetId && !activeIds.has(targetId)) {
      root.remove();
    }
  });
}

/** Moves every button to its video's current position without re-evaluating candidates. */
export function repositionVideoOverlays(): void {
  document.querySelectorAll<HTMLDivElement>(`[${ROOT_ATTR}]`).forEach((root) => {
    const video = overlayVideos.get(root);
    if (video?.isConnected) {
      positionOverlayRoot(root, video);
    } else {
      root.remove();
    }
  });
}

export function resetDismissedOverlays(): void {
  dismissedVideos = new WeakSet<HTMLVideoElement>();
}

function overlayHost(): HTMLElement | null {
  return document.body;
}

function createOverlayRoot(target: VideoOverlayTarget, onDownload: VideoOverlayOptions["onDownload"]): HTMLDivElement {
  const root = document.createElement("div");
  overlayVideos.set(root, target.video);
  root.setAttribute(ROOT_ATTR, target.id);
  root.dataset.ldmCandidateSignature = candidateSignature(target.candidate);
  root.dataset.ldmOverlay = "true";
  root.style.cssText = [
    "position:fixed",
    "z-index:2147483647",
    "display:flex",
    "align-items:center",
    "gap:5px",
    "font:700 12px/1.2 Segoe UI,Tahoma,Arial,sans-serif",
    "color:#eefcf4",
    "pointer-events:auto",
    "isolation:isolate",
    "filter:drop-shadow(0 6px 14px rgba(0,0,0,.34))",
  ].join(";");
  positionOverlayRoot(root, target.video);

  const action = document.createElement("button");
  action.type = "button";
  action.title = titleForCandidate(target.candidate);
  action.setAttribute("aria-label", action.title);
  action.style.cssText = [
    "display:flex",
    "align-items:center",
    "justify-content:center",
    "gap:5px",
    "height:26px",
    "min-width:58px",
    "border:1px solid rgba(203,215,230,.34)",
    "border-radius:5px",
    "background:linear-gradient(180deg,rgba(35,46,61,.96),rgba(12,19,29,.95))",
    "backdrop-filter:blur(8px)",
    "box-shadow:inset 0 1px 0 rgba(255,255,255,.12),0 0 0 1px rgba(0,0,0,.24),0 6px 16px rgba(0,0,0,.24)",
    "color:#f8fff9",
    "font:800 12px/1 Segoe UI,Tahoma,Arial,sans-serif",
    "letter-spacing:0",
    "padding:0 8px 0 6px",
    "cursor:pointer",
  ].join(";");

  const icon = createBrandMarkElement(16);

  const label = document.createElement("span");
  label.textContent = "LDM";
  label.style.cssText = ["display:inline-grid", "align-items:center", "height:13px"].join(";");

  const close = document.createElement("button");
  close.type = "button";
  close.title = "Hide LDM button";
  close.setAttribute("aria-label", "Hide LDM button");
  close.style.cssText = [
    "position:relative",
    "display:grid",
    "place-items:center",
    "width:23px",
    "height:23px",
    "padding:0",
    "border:1px solid rgba(203,215,230,.24)",
    "border-radius:5px",
    "background:linear-gradient(180deg,rgba(28,38,52,.9),rgba(10,17,26,.88))",
    "box-shadow:inset 0 1px 0 rgba(255,255,255,.09),0 0 0 1px rgba(0,0,0,.2),0 5px 14px rgba(0,0,0,.22)",
    "cursor:pointer",
    "opacity:.86",
  ].join(";");

  const closeLineA = createCloseLine(45);
  const closeLineB = createCloseLine(-45);
  close.append(closeLineA, closeLineB);

  const inspect = createRoundIconButton("Inspect stream candidates");
  inspect.textContent = "i";
  inspect.style.font = "700 12px/1 Segoe UI,Arial,sans-serif";
  inspect.style.color = "#d9e6ef";

  action.append(icon, label);
  action.addEventListener("pointerdown", stopPlayerEvent);
  action.addEventListener("mousedown", stopPlayerEvent);
  action.addEventListener("click", (event) => {
    stopPlayerEvent(event);
    void startCandidateDownload(root, target, onDownload);
  });
  close.addEventListener("pointerdown", stopPlayerEvent);
  close.addEventListener("mousedown", stopPlayerEvent);
  close.addEventListener("click", (event) => {
    stopPlayerEvent(event);
    // Remember the dismissal, otherwise the next render pass would recreate the button.
    dismissedVideos.add(target.video);
    root.remove();
  });
  inspect.addEventListener("pointerdown", stopPlayerEvent);
  inspect.addEventListener("mousedown", stopPlayerEvent);
  inspect.addEventListener("click", (event) => {
    stopPlayerEvent(event);
    setCandidateInspector(root, target);
  });

  root.append(action, inspect, close);
  return root;
}

async function startCandidateDownload(
  root: HTMLDivElement,
  target: VideoOverlayTarget,
  onDownload: VideoOverlayOptions["onDownload"],
): Promise<void> {
  const { candidate } = target;
  if (await needsUsageNotice(candidate)) {
    setUsageNotice(root, candidate, async () => {
      await rememberUsageNotice(candidate);
      await submitDownload(root, target, onDownload);
    });
    return;
  }

  await submitDownload(root, target, onDownload);
}

function positionOverlayRoot(root: HTMLElement, video: HTMLVideoElement): void {
  const rect = video.getBoundingClientRect();
  const visibleRect = visibleVideoViewportRect(rect);
  if (!visibleRect || !video.isConnected) {
    root.style.display = "none";
    root.style.pointerEvents = "none";
    return;
  }

  root.style.display = "flex";
  root.style.pointerEvents = "auto";

  const top = visibleRect.top + OVERLAY_INSET;
  const left = visibleRect.right - OVERLAY_INSET;

  root.style.top = `${top}px`;
  root.style.left = `${left}px`;
  root.style.right = "auto";
  root.style.transform = "translateX(-100%)";
}

function visibleVideoViewportRect(rect: DOMRect): DOMRect | null {
  const viewportWidth = window.innerWidth || document.documentElement.clientWidth;
  const viewportHeight = window.innerHeight || document.documentElement.clientHeight;
  const left = Math.max(0, rect.left);
  const top = Math.max(0, rect.top);
  const right = Math.min(viewportWidth, rect.right);
  const bottom = Math.min(viewportHeight, rect.bottom);

  if (right - left < MIN_VISIBLE_VIDEO_WIDTH || bottom - top < MIN_VISIBLE_VIDEO_HEIGHT) {
    return null;
  }

  return DOMRect.fromRect({
    x: left,
    y: top,
    width: right - left,
    height: bottom - top,
  });
}

function stopPlayerEvent(event: Event): void {
  event.preventDefault();
  event.stopPropagation();
  event.stopImmediatePropagation();
}

function createCloseLine(rotation: number): HTMLSpanElement {
  const line = document.createElement("span");
  line.setAttribute("aria-hidden", "true");
  line.style.cssText = [
    "position:absolute",
    "left:50%",
    "top:50%",
    "width:10px",
    "height:2px",
    "border-radius:2px",
    "background:#d9e6ef",
    `transform:translate(-50%,-50%) rotate(${rotation}deg)`,
    "transform-origin:center",
  ].join(";");
  return line;
}

function createRoundIconButton(title: string): HTMLButtonElement {
  const button = document.createElement("button");
  button.type = "button";
  button.title = title;
  button.setAttribute("aria-label", title);
  button.style.cssText = [
    "position:relative",
    "display:grid",
    "place-items:center",
    "width:23px",
    "height:23px",
    "padding:0",
    "border:1px solid rgba(203,215,230,.24)",
    "border-radius:5px",
    "background:linear-gradient(180deg,rgba(28,38,52,.9),rgba(10,17,26,.88))",
    "box-shadow:inset 0 1px 0 rgba(255,255,255,.09),0 0 0 1px rgba(0,0,0,.2),0 5px 14px rgba(0,0,0,.22)",
    "cursor:pointer",
    "opacity:.86",
  ].join(";");
  return button;
}

async function submitDownload(
  root: HTMLDivElement,
  target: VideoOverlayTarget,
  onDownload: VideoOverlayOptions["onDownload"],
): Promise<void> {
  let startNoticeTimer: number | undefined;
  const clearStartNoticeTimer = () => {
    if (startNoticeTimer !== undefined) {
      window.clearTimeout(startNoticeTimer);
      startNoticeTimer = undefined;
    }
  };
  const setStatus = (message: string) => {
    clearStartNoticeTimer();
    setNotice(root, message, 0);
    if (message === "Sending to LDM...") {
      startNoticeTimer = window.setTimeout(() => {
        setNotice(root, "Starting desktop app...", 0);
      }, 600);
    }
  };

  setStatus("Preparing stream...");

  try {
    const result = await onDownload(target.candidate, {
      video: target.video,
      setStatus,
      debugCandidates: target.debugCandidates,
    });
    setNotice(root, result.ok ? "Sent to LDM." : overlayErrorMessage(result.message, result.code));
  } finally {
    clearStartNoticeTimer();
  }
}

function setUsageNotice(
  root: HTMLDivElement,
  candidate: DetectedDownloadCandidate,
  onConfirm: () => void | Promise<void>,
): void {
  root.querySelector("[data-ldm-notice]")?.remove();

  const notice = document.createElement("div");
  notice.dataset.ldmNotice = "true";
  notice.style.cssText = [
    "position:absolute",
    "top:36px",
    "right:0",
    "width:286px",
    "max-width:min(286px,calc(100vw - 32px))",
    "border:1px solid rgba(255,255,255,.14)",
    "border-radius:5px",
    "background:rgba(12,18,28,.94)",
    "box-shadow:0 10px 30px rgba(0,0,0,.32)",
    "color:#eef2f7",
    "font:500 12px/1.35 Segoe UI,Arial,sans-serif",
    "padding:10px",
  ].join(";");

  const message = document.createElement("div");
  message.textContent = PLATFORM_USAGE_NOTICE;
  message.title = platformLabel(candidate.providerId);

  const actions = document.createElement("div");
  actions.style.cssText = ["display:flex", "justify-content:flex-end", "gap:8px", "margin-top:10px"].join(";");

  const cancel = document.createElement("button");
  cancel.type = "button";
  cancel.textContent = "Cancel";
  cancel.style.cssText = noticeButtonStyles("rgba(255,255,255,.08)", "#dbe6ef");

  const confirm = document.createElement("button");
  confirm.type = "button";
  confirm.textContent = "Start download";
  confirm.style.cssText = noticeButtonStyles("#23d160", "#07111f");

  cancel.addEventListener("click", () => {
    notice.remove();
  });
  confirm.addEventListener("click", () => {
    notice.remove();
    void onConfirm();
  });

  actions.append(cancel, confirm);
  notice.append(message, actions);
  root.append(notice);
}

async function needsUsageNotice(candidate: DetectedDownloadCandidate): Promise<boolean> {
  if (!candidate.requiresUsageNotice || !candidate.providerId) {
    return false;
  }
  const key = usageNoticeKey(candidate.providerId);
  if (!key) {
    return false;
  }
  return !(await storageGetBoolean(key));
}

async function rememberUsageNotice(candidate: DetectedDownloadCandidate): Promise<void> {
  if (!candidate.providerId) {
    return;
  }
  const key = usageNoticeKey(candidate.providerId);
  if (!key) {
    return;
  }
  await storageSetBoolean(key, true);
}

function usageNoticeKey(providerId: string): string | null {
  if (providerId === "youtube" || providerId === "x" || providerId === "tiktok") {
    return `ldm.providerNoticeAccepted.${providerId}`;
  }
  return null;
}

function platformLabel(providerId: string | undefined): string {
  if (providerId === "youtube") {
    return "YouTube";
  }
  if (providerId === "x") {
    return "X/Twitter";
  }
  if (providerId === "tiktok") {
    return "TikTok";
  }
  return "Provider";
}

function storageGetBoolean(key: string): Promise<boolean> {
  if (typeof chrome === "undefined" || !chrome.storage?.local) {
    return Promise.resolve(false);
  }

  return new Promise((resolve) => {
    chrome.storage.local.get(key, (items) => {
      resolve(items[key] === true);
    });
  });
}

function storageSetBoolean(key: string, value: boolean): Promise<void> {
  if (typeof chrome === "undefined" || !chrome.storage?.local) {
    return Promise.resolve();
  }

  return new Promise((resolve) => {
    chrome.storage.local.set({ [key]: value }, () => resolve());
  });
}

function setNotice(root: HTMLDivElement, message: string, autoHideMs = 5200): void {
  root.querySelector("[data-ldm-notice]")?.remove();

  const notice = document.createElement("div");
  notice.dataset.ldmNotice = "true";
  notice.textContent = message;
  notice.style.cssText = [
    "position:absolute",
    "top:36px",
    "right:0",
    "width:260px",
    "max-width:min(260px,calc(100vw - 32px))",
    "border:1px solid rgba(255,255,255,.14)",
    "border-radius:5px",
    "background:rgba(12,18,28,.94)",
    "box-shadow:0 10px 30px rgba(0,0,0,.32)",
    "color:#eef2f7",
    "font:500 12px/1.35 Segoe UI,Arial,sans-serif",
    "padding:10px 12px",
  ].join(";");

  root.append(notice);

  if (autoHideMs > 0) {
    window.setTimeout(() => {
      notice.remove();
    }, autoHideMs);
  }
}

function setCandidateInspector(root: HTMLDivElement, target: VideoOverlayTarget): void {
  root.querySelector("[data-ldm-notice]")?.remove();

  const diagnostics = buildCandidateDiagnostics(target.candidate, target.debugCandidates);
  const panel = document.createElement("div");
  panel.dataset.ldmNotice = "true";
  panel.style.cssText = [
    "position:absolute",
    "top:36px",
    "right:0",
    "width:390px",
    "max-width:min(390px,calc(100vw - 32px))",
    "max-height:min(520px,calc(100vh - 72px))",
    "overflow:auto",
    "border:1px solid rgba(255,255,255,.14)",
    "border-radius:5px",
    "background:rgba(12,18,28,.96)",
    "box-shadow:0 10px 30px rgba(0,0,0,.32)",
    "color:#eef2f7",
    "font:500 12px/1.35 Segoe UI,Arial,sans-serif",
    "padding:10px",
  ].join(";");

  const header = document.createElement("div");
  header.style.cssText = [
    "display:flex",
    "align-items:center",
    "justify-content:space-between",
    "gap:10px",
    "margin-bottom:8px",
  ].join(";");

  const title = document.createElement("strong");
  title.textContent = "Stream candidates";
  title.style.cssText = "font:700 12px/1.2 Segoe UI,Arial,sans-serif";

  const copy = document.createElement("button");
  copy.type = "button";
  copy.textContent = "Copy report";
  copy.style.cssText = noticeButtonStyles("rgba(255,255,255,.08)", "#dbe6ef");
  copy.addEventListener("click", () => {
    void copyCandidateReport(copy, target);
  });

  header.append(title, copy);

  const list = document.createElement("div");
  list.style.cssText = ["display:grid", "gap:7px"].join(";");
  for (const item of diagnostics) {
    const row = document.createElement("div");
    row.style.cssText = [
      "display:grid",
      "grid-template-columns:auto 1fr",
      "gap:4px 8px",
      "border:1px solid rgba(255,255,255,.1)",
      "border-radius:4px",
      "background:rgba(255,255,255,.04)",
      "padding:8px",
    ].join(";");

    const badge = document.createElement("span");
    badge.textContent = item.selected ? "selected" : item.origin;
    badge.style.cssText = [
      "align-self:start",
      "border-radius:3px",
      `background:${item.selected ? "#23d160" : "rgba(255,255,255,.1)"}`,
      `color:${item.selected ? "#07111f" : "#dbe6ef"}`,
      "font:700 11px/1 Segoe UI,Arial,sans-serif",
      "padding:4px 7px",
    ].join(";");

    const meta = document.createElement("div");
    meta.textContent = `${item.source} / ${item.adapterId} / ${item.quality}`;
    meta.style.cssText = "font:700 12px/1.25 Segoe UI,Arial,sans-serif;color:#f7fbff;min-width:0";

    const url = document.createElement("div");
    url.textContent = item.url;
    url.title = item.url;
    url.style.cssText = [
      "grid-column:1 / -1",
      "overflow:hidden",
      "text-overflow:ellipsis",
      "white-space:nowrap",
      "color:#aebccd",
      "font:500 11px/1.25 Consolas,monospace",
    ].join(";");

    row.append(badge, meta, url);
    if (item.fallbackCount > 0) {
      const fallback = document.createElement("div");
      fallback.textContent = `${item.fallbackCount} fallback stream${item.fallbackCount === 1 ? "" : "s"}`;
      fallback.style.cssText = "grid-column:1 / -1;color:#a5f3c0;font:600 11px/1.25 Segoe UI,Arial,sans-serif";
      row.append(fallback);
    }
    list.append(row);
  }

  const empty = document.createElement("div");
  empty.textContent = "No stream candidates captured yet.";
  empty.style.cssText = "color:#aebccd;padding:8px 0";
  if (diagnostics.length === 0) {
    list.append(empty);
  }

  panel.append(header, list);
  root.append(panel);
}

async function copyCandidateReport(button: HTMLButtonElement, target: VideoOverlayTarget): Promise<void> {
  const report = buildCandidateDebugReport(target.candidate, target.debugCandidates, target.video);
  const panel = button.closest<HTMLElement>("[data-ldm-notice]");
  panel?.querySelector("[data-ldm-copy-fallback]")?.remove();

  try {
    await copyText(report);
    button.textContent = "Copied";
  } catch {
    showManualCopyFallback(button, report);
    return;
  }

  window.setTimeout(() => {
    button.textContent = "Copy report";
  }, 1600);
}

async function copyText(value: string): Promise<void> {
  if (copyTextWithSelection(value)) {
    return;
  }

  if (navigator.clipboard?.writeText && window.isSecureContext) {
    try {
      await navigator.clipboard.writeText(value);
      return;
    } catch {
      // Some pages block clipboard access for content scripts; fall through to the manual path.
    }
  }

  throw new Error("Clipboard copy is unavailable.");
}

function copyTextWithSelection(value: string): boolean {
  const host = document.body ?? document.documentElement;
  const textArea = document.createElement("textarea");
  textArea.value = value;
  textArea.setAttribute("readonly", "true");
  textArea.style.cssText = [
    "position:fixed",
    "left:-9999px",
    "top:0",
    "width:1px",
    "height:1px",
    "opacity:0",
    "pointer-events:none",
  ].join(";");

  host.append(textArea);
  textArea.focus({ preventScroll: true });
  textArea.select();
  textArea.setSelectionRange(0, value.length);
  try {
    return document.execCommand("copy");
  } finally {
    textArea.remove();
  }
}

function showManualCopyFallback(button: HTMLButtonElement, report: string): void {
  const panel = button.closest<HTMLElement>("[data-ldm-notice]");
  if (!panel) {
    button.textContent = "Copy blocked";
    return;
  }

  const fallback = document.createElement("div");
  fallback.dataset.ldmCopyFallback = "true";
  fallback.style.cssText = [
    "grid-column:1 / -1",
    "margin-top:8px",
    "border:1px solid rgba(255,255,255,.18)",
    "border-radius:4px",
    "background:rgba(255,255,255,.05)",
    "padding:8px",
  ].join(";");

  const hint = document.createElement("div");
  hint.textContent = "Report selected. Press Ctrl+C to copy.";
  hint.style.cssText = "color:#dbe6ef;font:700 11px/1.25 Segoe UI,Arial,sans-serif;margin-bottom:6px";

  const textArea = document.createElement("textarea");
  textArea.value = report;
  textArea.readOnly = true;
  textArea.spellcheck = false;
  textArea.style.cssText = [
    "display:block",
    "box-sizing:border-box",
    "width:100%",
    "height:120px",
    "resize:vertical",
    "border:1px solid rgba(255,255,255,.16)",
    "border-radius:4px",
    "background:#07111f",
    "color:#eef2f7",
    "font:500 11px/1.35 Consolas,monospace",
    "padding:8px",
    "outline:none",
  ].join(";");

  fallback.append(hint, textArea);
  panel.append(fallback);
  button.textContent = "Press Ctrl+C";
  textArea.focus({ preventScroll: true });
  textArea.select();
  textArea.setSelectionRange(0, report.length);
}

function noticeButtonStyles(background: string, color: string): string {
  return [
    "height:26px",
    "border:1px solid rgba(255,255,255,.16)",
    "border-radius:4px",
    `background:${background}`,
    `color:${color}`,
    "font:700 12px/1 Segoe UI,Arial,sans-serif",
    "letter-spacing:0",
    "padding:0 10px",
    "cursor:pointer",
  ].join(";");
}

function titleForCandidate(candidate: DetectedDownloadCandidate): string {
  if (candidate.source === "provider") {
    return "Download this provider video with LDM";
  }

  if (candidate.source === "hls" || candidate.source === "dash") {
    return "Send stream manifest to LDM";
  }

  return "Send this media to LDM";
}

function candidateSignature(candidate: DetectedDownloadCandidate): string {
  return [
    candidate.adapterId,
    candidate.providerId ?? "",
    candidate.source,
    candidate.kind,
    candidate.id,
    candidate.contentLength?.toString() ?? "",
    candidate.sourceFallbackUrls?.join(",") ?? "",
    hashString(candidate.url),
    hashString(candidate.title ?? candidate.suggestedFilename ?? ""),
  ].join("|");
}
