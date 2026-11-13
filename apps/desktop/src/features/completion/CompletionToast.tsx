import type { DownloadJob } from "@ldm/contracts";
import { File, FolderOpen, X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { openJobFile, showJobInFolder } from "../../shared/api/downloads";
import {
  type CompletionPreferences,
  getCompletionPreferences,
  subscribeToCompletionPreferences,
} from "../../shared/completionPreferences";
import { useJobsStore } from "../../shared/jobsStore";

export function CompletionToast() {
  const jobs = useJobsStore((state) => state.jobs);
  const jobsLoaded = useJobsStore((state) => state.loaded);
  const [preferences, setPreferences] = useState(getCompletionPreferences);
  const [job, setJob] = useState<DownloadJob | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const knownCompletedIds = useRef<Set<string> | null>(null);

  useEffect(() => {
    return subscribeToCompletionPreferences(() => {
      setPreferences(getCompletionPreferences());
    });
  }, []);

  // Jobs that were already complete when the app started do not get a toast.
  useEffect(() => {
    if (!jobsLoaded) {
      return;
    }

    const completedJobs = jobs.filter((item) => item.status === "completed");
    if (knownCompletedIds.current === null) {
      knownCompletedIds.current = new Set(completedJobs.map((item) => item.id));
      return;
    }

    const nextCompletedJob = completedJobs.find((item) => !knownCompletedIds.current?.has(item.id));
    for (const completedJob of completedJobs) {
      knownCompletedIds.current.add(completedJob.id);
    }

    if (nextCompletedJob && preferences.enabled) {
      setActionError(null);
      setJob(nextCompletedJob);
    }
  }, [jobs, jobsLoaded, preferences.enabled]);

  if (!job || !preferences.enabled || !hasAnyAction(preferences)) {
    return null;
  }

  return (
    <aside className="completion-toast" aria-live="polite">
      <button className="completion-close" type="button" title="Close" onClick={() => setJob(null)}>
        <X size={14} />
      </button>
      <strong>Download complete</strong>
      <span className="truncate" title={job.filename}>
        {job.filename}
      </span>
      <div className="completion-actions">
        {preferences.showOpenFile ? (
          <button
            type="button"
            onClick={() => {
              setActionError(null);
              void openJobFile(job.id).catch((error: unknown) => {
                setActionError(actionErrorMessage(error));
              });
            }}
          >
            <File size={14} />
            <span>Open file</span>
          </button>
        ) : null}
        {preferences.showShowInFolder ? (
          <button
            type="button"
            onClick={() => {
              setActionError(null);
              void showJobInFolder(job.id).catch((error: unknown) => {
                setActionError(actionErrorMessage(error));
              });
            }}
          >
            <FolderOpen size={14} />
            <span>Open folder</span>
          </button>
        ) : null}
      </div>
      {actionError ? <small className="completion-error">{actionError}</small> : null}
    </aside>
  );
}

function hasAnyAction(preferences: CompletionPreferences): boolean {
  return preferences.showOpenFile || preferences.showShowInFolder;
}

function actionErrorMessage(error: unknown): string {
  if (error instanceof Error) {
    return error.message;
  }
  if (typeof error === "string") {
    return error;
  }
  return "Could not open download.";
}
