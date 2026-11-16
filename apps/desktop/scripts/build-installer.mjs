// Runs `tauri build` with overlays that stay out of tauri.conf.json (externalBin and resources
// would break `cargo clippy` and `cargo test` when the files are missing).
import { existsSync } from "node:fs";
import { createRequire } from "node:module";
import { resolve } from "node:path";
import { desktopDir, nativeHostSidecarPath, run, srcTauriDir } from "./lib.mjs";

const sidecar = nativeHostSidecarPath();
if (!existsSync(sidecar)) {
  console.error(`Missing ${sidecar}. Run "pnpm --filter @ldm/desktop prepare:bundle" first.`);
  process.exit(1);
}

const overlays = ["--config", resolve(srcTauriDir, "tauri.bundle.conf.json")];
const toolsDir = resolve(srcTauriDir, "resources/tools");
if (existsSync(toolsDir)) {
  overlays.push("--config", JSON.stringify({ bundle: { resources: ["resources/tools/*"] } }));
  console.log(`Bundling the tools from ${toolsDir}`);
} else {
  console.log(`No ${toolsDir}; the installer ships without yt-dlp and FFmpeg.`);
}

const require = createRequire(import.meta.url);
const tauriCli = resolve(require.resolve("@tauri-apps/cli/package.json"), "..", "tauri.js");
run(process.execPath, [tauriCli, "build", ...overlays], desktopDir);
