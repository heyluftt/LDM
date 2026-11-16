import { invoke } from "./invoke";

export type UpdateStatus = {
  currentVersion: string;
  latestVersion: string;
  updateAvailable: boolean;
};

export function checkForUpdate(): Promise<UpdateStatus> {
  return invoke<UpdateStatus>("check_for_update");
}

export function openReleasePage(): Promise<void> {
  return invoke<void>("open_release_page");
}
