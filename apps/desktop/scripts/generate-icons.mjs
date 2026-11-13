// Regenerates every app icon from the logo source. Run from apps/desktop: `pnpm icons`.
//
// `tauri icon` rasterizes assets/logo/ldm-mark.svg into the full platform set; only the files
// Windows and the extension need are copied out of its output, the rest is discarded.
import { spawnSync } from "node:child_process";
import { copyFileSync, mkdirSync, mkdtempSync, rmSync } from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const desktopDir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const repoRoot = path.resolve(desktopDir, "..", "..");
const source = path.join(repoRoot, "assets", "logo", "ldm-mark.svg");
const desktopIcons = path.join(desktopDir, "src-tauri", "icons");
// WXT 0.20+ serves `<rootDir>/public` (the package folder), not `<srcDir>/public`.
const extensionIcons = path.join(repoRoot, "apps", "extension", "public", "icon");
const tauriCli = createRequire(import.meta.url).resolve("@tauri-apps/cli/tauri.js");

const output = mkdtempSync(path.join(tmpdir(), "ldm-icons-"));
try {
  const result = spawnSync(process.execPath, [tauriCli, "icon", source, "-o", output], {
    cwd: desktopDir,
    stdio: "inherit",
  });
  if (result.status !== 0) {
    throw new Error(`tauri icon failed with exit code ${result.status}`);
  }

  mkdirSync(desktopIcons, { recursive: true });
  for (const name of ["icon.ico", "icon.png", "32x32.png", "64x64.png", "128x128.png", "128x128@2x.png"]) {
    copyFileSync(path.join(output, name), path.join(desktopIcons, name));
  }

  mkdirSync(extensionIcons, { recursive: true });
  for (const [from, to] of [
    ["32x32.png", "32.png"],
    ["64x64.png", "64.png"],
    ["128x128.png", "128.png"],
  ]) {
    copyFileSync(path.join(output, from), path.join(extensionIcons, to));
  }
  console.log(`icons written to ${desktopIcons} and ${extensionIcons}`);
} finally {
  rmSync(output, { recursive: true, force: true });
}
