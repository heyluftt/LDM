import { spawnSync } from "node:child_process";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

export const desktopDir = resolve(dirname(fileURLToPath(import.meta.url)), "..");
export const repoRoot = resolve(desktopDir, "../..");
export const srcTauriDir = resolve(desktopDir, "src-tauri");

/** Runs `command` with inherited stdio and ends the script with its status when it fails. */
export function run(command, args, cwd) {
  console.log(`> ${command} ${args.join(" ")}`);
  const result = spawnSync(command, args, { cwd, stdio: "inherit" });
  if (result.error) {
    throw result.error;
  }
  if (result.status !== 0) {
    console.error(`${command} exited with status ${result.status}`);
    process.exit(result.status ?? 1);
  }
}

/** The target triple Tauri expects in sidecar file names, taken from the installed toolchain. */
export function hostTargetTriple() {
  const result = spawnSync("rustc", ["-vV"], { encoding: "utf8" });
  const match = result.stdout?.match(/^host:\s*(\S+)/mu);
  if (!match) {
    throw new Error("could not read the host target triple from `rustc -vV`");
  }
  return match[1];
}

/** `ldm-native-host-<triple>.exe` under src-tauri/binaries, the file Tauri bundles as a sidecar. */
export function nativeHostSidecarPath() {
  return resolve(srcTauriDir, "binaries", `ldm-native-host-${hostTargetTriple()}.exe`);
}
