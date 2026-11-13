import { create } from "zustand";
import { getLegalStatus } from "./api/legal";
import { checkForUpdate, type UpdateStatus } from "./api/updates";
import { UPDATE_CHECK_INTERVAL_MS } from "./updateModel";

const AUTOMATIC_KEY = "ldm.update.checkAutomatically";
const DISMISSED_KEY = "ldm.update.dismissedVersion";

export type UpdateStore = {
  /** The result of the last check that answered; `null` before the first one. */
  status: UpdateStatus | null;
  checking: boolean;
  /** Why the last check failed; cleared by the next one that answers. */
  error: string | null;
  /** Whether LDM asks on its own, at start and once a day. On unless the user turned it off. */
  checkAutomatically: boolean;
  /** The version the user answered "later" to. */
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

/** Asks GitHub now, whatever the automatic setting says. Used by "Check now". */
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

/**
 * The automatic check: nothing is asked while automatic checks are off, or before the
 * first-run notice is accepted, because until then the user has not seen that LDM contacts
 * GitHub at all.
 */
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

/**
 * Checks at start and then once a day for as long as the app runs, which in the tray can be
 * weeks. The first-run notice asks once more when it is accepted.
 */
export function startUpdateChecks(): () => void {
  void checkForUpdateIfAllowed();
  const timer = window.setInterval(() => void checkForUpdateIfAllowed(), UPDATE_CHECK_INTERVAL_MS);
  return () => window.clearInterval(timer);
}
