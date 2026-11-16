import { create } from "zustand";
import { getLegalStatus } from "./api/legal";
import { checkForUpdate, type UpdateStatus } from "./api/updates";
import { UPDATE_CHECK_INTERVAL_MS } from "./updateModel";

const AUTOMATIC_KEY = "ldm.update.checkAutomatically";
const DISMISSED_KEY = "ldm.update.dismissedVersion";

export type UpdateStore = {
  status: UpdateStatus | null;
  checking: boolean;
  error: string | null;
  checkAutomatically: boolean;
  dismissedVersion: string | null;
  setCheckAutomatically: (enabled: boolean) => void;
  dismiss: (version: string) => void;
};

export const useUpdateStore = create<UpdateStore>((set) => ({
  status: null,
  checking: false,
  error: null,
  checkAutomatically: window.localStorage.getItem(AUTOMATIC_KEY) !== "0",
  dismissedVersion: window.localStorage.getItem(DISMISSED_KEY),
  setCheckAutomatically: (enabled) => {
    window.localStorage.setItem(AUTOMATIC_KEY, enabled ? "1" : "0");
    set({ checkAutomatically: enabled });
  },
  dismiss: (version) => {
    window.localStorage.setItem(DISMISSED_KEY, version);
    set({ dismissedVersion: version });
  },
}));

export async function checkForUpdateNow(): Promise<void> {
  if (useUpdateStore.getState().checking) {
    return;
  }
  useUpdateStore.setState({ checking: true });
  try {
    useUpdateStore.setState({ status: await checkForUpdate(), error: null, checking: false });
  } catch (error) {
    useUpdateStore.setState({
      error: error instanceof Error ? error.message : "The check failed.",
      checking: false,
    });
  }
}

// Don't ask GitHub before the first-run notice is accepted.
export async function checkForUpdateIfAllowed(): Promise<void> {
  if (!useUpdateStore.getState().checkAutomatically) {
    return;
  }
  try {
    if (!(await getLegalStatus()).accepted) {
      return;
    }
  } catch {
    return;
  }
  await checkForUpdateNow();
}

// The first-run notice checks again when accepted.
export function startUpdateChecks(): () => void {
  void checkForUpdateIfAllowed();
  const timer = window.setInterval(() => void checkForUpdateIfAllowed(), UPDATE_CHECK_INTERVAL_MS);
  return () => window.clearInterval(timer);
}
