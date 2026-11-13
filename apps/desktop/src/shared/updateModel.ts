import type { UpdateStatus } from "./api/updates";

/** How long a running app waits before it asks again. */
export const UPDATE_CHECK_INTERVAL_MS = 24 * 60 * 60 * 1000;

/**
 * Whether to put the notice on screen: a newer version exists and the user has not said
 * "later" to exactly that version. A version after the one they waved off is announced again.
 */
export function shouldAnnounceUpdate(status: UpdateStatus | null, dismissedVersion: string | null): boolean {
  if (!status?.updateAvailable) {
    return false;
  }
  return status.latestVersion !== dismissedVersion;
}

/** One line for the settings: what is known about updates right now. */
export function updateSummary(status: UpdateStatus | null, checking: boolean, error: string | null): string {
  if (checking) {
    return "Checking…";
  }
  if (error) {
    return `Could not check: ${error}`;
  }
  if (!status) {
    return "Not checked yet.";
  }
  return status.updateAvailable ? `Version ${status.latestVersion} is available.` : "This is the latest version.";
}
