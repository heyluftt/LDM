import type { DownloadJob } from "@ldm/contracts";
import { invoke } from "./invoke";

export function listJobs(): Promise<DownloadJob[]> {
  return invoke<DownloadJob[]>("list_jobs");
}

export function pauseJob(jobId: string): Promise<DownloadJob> {
  return invoke<DownloadJob>("pause_job", { jobId });
}

export function resumeJob(jobId: string): Promise<DownloadJob> {
  return invoke<DownloadJob>("resume_job", { jobId });
}

export function retryJob(jobId: string): Promise<DownloadJob> {
  return invoke<DownloadJob>("retry_job", { jobId });
}

export function cancelJob(jobId: string): Promise<DownloadJob> {
  return invoke<DownloadJob>("cancel_job", { jobId });
}

export function removeHistoryJob(jobId: string): Promise<DownloadJob> {
  return invoke<DownloadJob>("remove_history_job", { jobId });
}

export function clearHistory(): Promise<number> {
  return invoke<number>("clear_history");
}

export function openJobFile(jobId: string): Promise<void> {
  return invoke<void>("open_job_file", { jobId });
}

export function showJobInFolder(jobId: string): Promise<void> {
  return invoke<void>("show_job_in_folder", { jobId });
}
