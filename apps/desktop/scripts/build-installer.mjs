// Runs `tauri build` with the bundle overlays that must not live in tauri.conf.json.
//
// The native host sidecar is declared in tauri.bundle.conf.json: tauri-build copies every
// `bundle.externalBin` during `cargo build` and fails when the file is missing, so declaring it
// in tauri.conf.json would make plain `cargo clippy`/`cargo test` runs (and CI) depend on
// `prepare:bundle`. The optional tools folder is added only when it exists, because Tauri
// aborts on a resource glob that matches nothing.
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
