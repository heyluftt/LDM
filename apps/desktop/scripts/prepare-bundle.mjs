import { copyFileSync, mkdirSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { nativeHostSidecarPath, repoRoot, run } from "./lib.mjs";

run("cargo", ["build", "--release", "-p", "ldm-native-host"], repoRoot);

const source = resolve(repoRoot, "target/release/ldm-native-host.exe");
const destination = nativeHostSidecarPath();
mkdirSync(dirname(destination), { recursive: true });
copyFileSync(source, destination);
console.log(`Native host ready for bundling: ${destination}`);
