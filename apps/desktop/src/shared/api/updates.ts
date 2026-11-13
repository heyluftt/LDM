import { invoke } from "./invoke";

/** What the latest release on GitHub is, compared with the running version. */
export type UpdateStatus = {
  currentVersion: string;
  /** The latest released version; the running one while nothing has been released. */
  latestVersion: string;
  updateAvailable: boolean;
};

/** Asks GitHub for the latest release. This is a network request. */
export function checkForUpdate(): Promise<UpdateStatus> {
  return invoke<UpdateStatus>("check_for_update");
}

/** Opens the page of the latest release in the default browser. */
export function openReleasePage(): Promise<void> {
  return invoke<void>("open_release_page");
}
