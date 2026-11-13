import type { ExtensionContact } from "@ldm/contracts";
import type { ToolStatus } from "../../shared/api/tools";

export type SetupStepId = "extension" | "ytDlp" | "ffmpeg";

/** One thing a new installation still needs, or already has. */
export type SetupStep = {
  id: SetupStepId;
  title: string;
  done: boolean;
  /** One line: what was found, or what is missing and what it is for. */
  status: string;
  /** What to do about it; only for a step that is open. */
  hint?: string;
  /** A command to run in a terminal, offered with a copy button. */
  command?: string;
};

/**
 * The steps of a first setup, in the order to take them. Plain file downloads need only the
 * extension; yt-dlp and FFmpeg are what provider pages and streams add.
 */
export function setupSteps(contact: ExtensionContact | null, tools: ToolStatus[]): SetupStep[] {
  const ytDlp = tools.find((tool) => tool.kind === "ytDlp");
  const ffmpeg = tools.find((tool) => tool.kind === "ffmpeg");

  return [
    contact
      ? { id: "extension", title: "Browser extension", done: true, status: "Connected." }
      : {
          id: "extension",
          title: "Browser extension",
          done: false,
          status: "Not connected yet.",
          hint: 'Unpack the extension archive from the release into a folder that stays where it is. Open chrome://extensions in Chrome, Edge, or Brave, switch on Developer mode, choose "Load unpacked", pick that folder, and reload any page.',
        },
    ytDlp?.found
      ? { id: "ytDlp", title: "yt-dlp", done: true, status: "Found." }
      : {
          id: "ytDlp",
          title: "yt-dlp",
          done: false,
          status: "Missing. YouTube, X, TikTok, and streams need it.",
          command: "winget install yt-dlp.yt-dlp",
        },
    ffmpeg?.found
      ? { id: "ffmpeg", title: "FFmpeg", done: true, status: "Found." }
      : {
          id: "ffmpeg",
          title: "FFmpeg",
          done: false,
          status: "Missing. Videos that come as separate picture and sound need it.",
          command: "winget install Gyan.FFmpeg",
        },
  ];
}

export function isSetupComplete(steps: SetupStep[]): boolean {
  return steps.every((step) => step.done);
}
