import type { DownloadKind } from "@ldm/contracts";
import { invoke } from "./invoke";

export type TargetFolder = {
  kind: DownloadKind;
  path: string;
  defaultPath: string;
  custom: boolean;
};

export function getTargetFolders(): Promise<TargetFolder[]> {
  return invoke<TargetFolder[]>("target_folders");
}

/** Stores `path` for `kind`; `null` resets the kind to its default folder. */
export function setTargetFolder(kind: DownloadKind, path: string | null): Promise<TargetFolder[]> {
  return invoke<TargetFolder[]>("set_target_folder", { kind, path });
}

export function pickTargetFolder(kind: DownloadKind): Promise<TargetFolder[]> {
  return invoke<TargetFolder[]>("pick_target_folder", { kind });
}

export type ConcurrencySettings = {
  maxConcurrentDownloads: number;
  min: number;
  max: number;
};

export function getConcurrencySettings(): Promise<ConcurrencySettings> {
  return invoke<ConcurrencySettings>("concurrency_settings");
}

/** Stores the parallel-download limit; the backend clamps it to `min..=max`. */
export function setMaxConcurrentDownloads(value: number): Promise<ConcurrencySettings> {
  return invoke<ConcurrencySettings>("set_max_concurrent_downloads", { value });
}
