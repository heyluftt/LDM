// Builds the native host in release mode and places it where Tauri picks it up as a sidecar.
//
// Tauri expects `binaries/ldm-native-host-<target-triple>.exe` next to tauri.conf.json and
// installs it as `ldm-native-host.exe` beside the desktop app, which is where the host looks
// for `ldm-desktop.exe` when the browser starts it.
import { copyFileSync, mkdirSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { nativeHostSidecarPath, repoRoot, run } from "./lib.mjs";

run("cargo", ["build", "--release", "-p", "ldm-native-host"], repoRoot);

const source = resolve(repoRoot, "target/release/ldm-native-host.exe");
const destination = nativeHostSidecarPath();
mkdirSync(dirname(destination), { recursive: true });
copyFileSync(source, destination);
console.log(`Native host ready for bundling: ${destination}`);
