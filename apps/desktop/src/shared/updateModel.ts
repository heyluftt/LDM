import type { UpdateStatus } from "./api/updates";

export const UPDATE_CHECK_INTERVAL_MS = 24 * 60 * 60 * 1000;

/** Show the notice unless the user said "later" to this exact version. */
export function shouldAnnounceUpdate(status: UpdateStatus | null, dismissedVersion: string | null): boolean {
  if (!status?.updateAvailable) {
    return false;
  }
  return status.latestVersion !== dismissedVersion;
}

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
