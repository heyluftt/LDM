import { getCurrentWindow } from "@tauri-apps/api/window";
import { invoke } from "./api/invoke";

export async function startAppWindowDrag(): Promise<void> {
  await runWindowAction((window) => window.startDragging());
}

export async function minimizeAppWindow(): Promise<void> {
  await runWindowAction((window) => window.minimize());
}

export async function toggleAppWindowMaximize(): Promise<void> {
  await runWindowAction((window) => window.toggleMaximize());
}

export async function closeAppWindow(): Promise<void> {
  await runWindowAction((window) => window.hide());
}

export async function quitApp(): Promise<void> {
  if (!isTauriRuntime()) {
    return;
  }

  try {
    await invoke("quit_app");
  } catch {
    await runWindowAction((window) => window.close());
  }
}

async function runWindowAction(action: (window: ReturnType<typeof getCurrentWindow>) => Promise<void>): Promise<void> {
  if (!isTauriRuntime()) {
    return;
  }

  try {
    await action(getCurrentWindow());
  } catch {
    // Window controls are best-effort when the UI is previewed outside Tauri.
  }
}

function isTauriRuntime(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}
