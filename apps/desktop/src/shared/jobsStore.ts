import type { DownloadJob } from "@ldm/contracts";
import { listen } from "@tauri-apps/api/event";
import { create } from "zustand";
import { listJobs } from "./api/downloads";

/** Mirrored in `src-tauri/src/job_events.rs`. */
export const JOBS_CHANGED_EVENT = "ldm://jobs-changed";

export type JobsChangedPayload = { type: "job"; job: DownloadJob } | { type: "reload" };

export type JobsStore = {
  jobs: DownloadJob[];
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
