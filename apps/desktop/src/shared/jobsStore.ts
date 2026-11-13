import type { DownloadJob } from "@ldm/contracts";
import { listen } from "@tauri-apps/api/event";
import { create } from "zustand";
import { listJobs } from "./api/downloads";

/** Event name the backend emits; mirrored in `apps/desktop/src-tauri/src/job_events.rs`. */
export const JOBS_CHANGED_EVENT = "ldm://jobs-changed";

/** Payload of `JOBS_CHANGED_EVENT`: one job's snapshot, or a request to reload the list. */
export type JobsChangedPayload = { type: "job"; job: DownloadJob } | { type: "reload" };

export type JobsStore = {
  /** Every job the backend knows, queue and history, in creation order. */
  jobs: DownloadJob[];
  /** False until the first `list_jobs` answered; views keep their empty state until then. */
  loaded: boolean;
  setJobs: (jobs: DownloadJob[]) => void;
  upsertJob: (job: DownloadJob) => void;
  removeJobs: (jobIds: Iterable<string>) => void;
};

export const useJobsStore = create<JobsStore>((set) => ({
  jobs: [],
  loaded: false,
  setJobs: (jobs) => set({ jobs, loaded: true }),
  upsertJob: (job) =>
    set((state) => {
      const index = state.jobs.findIndex((current) => current.id === job.id);
      if (index === -1) {
        return { jobs: [...state.jobs, job] };
      }
      const jobs = state.jobs.slice();
      jobs[index] = job;
      return { jobs };
    }),
  removeJobs: (jobIds) => {
    const removed = new Set(jobIds);
    set((state) => ({ jobs: state.jobs.filter((job) => !removed.has(job.id)) }));
  },
}));

let reloadInFlight: Promise<void> | null = null;
let reloadRequestedAgain = false;

/** Reloads the job list, coalescing bursts of requests into one call at a time. */
export function reloadJobs(): Promise<void> {
  if (reloadInFlight) {
    reloadRequestedAgain = true;
    return reloadInFlight;
  }

  reloadInFlight = (async () => {
    try {
      do {
        reloadRequestedAgain = false;
        useJobsStore.getState().setJobs(await listJobs());
      } while (reloadRequestedAgain);
    } finally {
      reloadInFlight = null;
    }
  })();
  return reloadInFlight;
}

/**
 * Loads the job list once and applies the backend's change events until the returned function
 * is called. Meant to be started once by the app root.
 */
export function startJobsSync(): () => void {
  let active = true;
  let unlisten: (() => void) | null = null;

  void listen<JobsChangedPayload>(JOBS_CHANGED_EVENT, (event) => {
    if (!active) {
      return;
    }
    if (event.payload.type === "job") {
      useJobsStore.getState().upsertJob(event.payload.job);
    } else {
      void reloadJobs().catch(() => undefined);
    }
  }).then((stop) => {
    if (active) {
      unlisten = stop;
    } else {
      stop();
    }
  });
  void reloadJobs().catch(() => undefined);

  return () => {
    active = false;
    unlisten?.();
    unlisten = null;
  };
}
