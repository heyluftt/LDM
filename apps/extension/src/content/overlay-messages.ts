import type { RejectionCode } from "@ldm/contracts";

const MAX_NOTICE_LENGTH = 180;

/** Short overlay text for a rejected download. */
export function overlayErrorMessage(message: string, code?: RejectionCode): string {
  if (code === "firstRunSetupRequired") {
    return "Open LDM and complete the first-run setup.";
  }
  if (code === "desktopUnavailable") {
    return "Desktop app could not be reached. Start or reopen LDM.";
  }
  if (message.includes("Current video stream is not ready")) {
    return "Start playback once, then click LDM again.";
  }
  if (message.includes("Current video stream could not be captured")) {
    return "Keep the video playing, then click LDM again.";
  }
  return truncateOverlayMessage(message || "LDM could not start this download.");
}

function truncateOverlayMessage(message: string): string {
  const normalized = message.replace(/\s+/gu, " ").trim();
  if (normalized.length <= MAX_NOTICE_LENGTH) {
    return normalized;
  }

  return `${normalized.slice(0, MAX_NOTICE_LENGTH - 3)}...`;
}
