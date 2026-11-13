import { invoke } from "./invoke";

export type ToolKind = "ytDlp" | "ffmpeg" | "ffprobe" | "deno";
export type ToolSource = "bundled" | "env" | "path";

export type ToolStatus = {
  kind: ToolKind;
  name: string;
  envVar: string;
  required: boolean;
  found: boolean;
  source?: ToolSource;
  path?: string;
  message: string;
};

export type ToolStatusReport = {
  tools: ToolStatus[];
};

export function getToolStatus(): Promise<ToolStatusReport> {
  return invoke<ToolStatusReport>("tool_status");
}

/** `yt-dlp --version` of the discovered executable. */
export type YtDlpVersion = {
  version?: string | null;
  path?: string | null;
  /** `yt-dlp -U`, or `pip install -U yt-dlp` for a pip-installed executable. */
  updateCommand: string;
  error?: string | null;
};

export type LatestYtDlpVersion = {
  version: string;
  /** `installed` is older than `version`; null when not comparable. */
  updateAvailable?: boolean | null;
};

export function getYtDlpVersion(): Promise<YtDlpVersion> {
  return invoke<YtDlpVersion>("ytdlp_version");
}

/** Latest release tag from GitHub, compared with `installed`. Rejects when offline. */
export function getLatestYtDlpVersion(installed: string | null): Promise<LatestYtDlpVersion> {
  return invoke<LatestYtDlpVersion>("latest_ytdlp_version", { installed });
}

/** Both versions side by side for the settings view; a failed lookup is reported, never thrown. */
export type YtDlpVersionInfo = {
  installed: string | null;
  installedError: string | null;
  latest: string | null;
  latestError: string | null;
  updateAvailable: boolean;
  updateCommand: string;
};

export async function loadYtDlpVersionInfo(): Promise<YtDlpVersionInfo> {
  const installed = await getYtDlpVersion();
  let latest: LatestYtDlpVersion | null = null;
  let latestError: string | null = null;
  try {
    latest = await getLatestYtDlpVersion(installed.version ?? null);
  } catch (error) {
    latestError = error instanceof Error ? error.message : String(error);
  }
  return {
    installed: installed.version ?? null,
    installedError: installed.error ?? null,
    latest: latest?.version ?? null,
    latestError,
    updateAvailable: latest?.updateAvailable === true,
    updateCommand: installed.updateCommand,
  };
}
