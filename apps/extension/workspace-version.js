// The project has one version: the one under [workspace.package] in the root Cargo.toml, which
// Cargo and Tauri read by themselves. This reads it for the extension manifest. It is plain
// JavaScript with a declaration file next to it so that the extension's type check does not
// need the Node.js types, which would also change the types of timers in the extension code.
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const cargoToml = fileURLToPath(new URL("../../Cargo.toml", import.meta.url));

/** The version as written in Cargo.toml, for example `0.2.0` or `0.2.0-beta.1`. */
export function workspaceVersion() {
  const section = readFileSync(cargoToml, "utf8")
    .split(/^\[/mu)
    .find((part) => part.startsWith("workspace.package]"));
  const version = section?.match(/^version\s*=\s*"([^"]+)"/mu)?.[1];
  if (!version) {
    throw new Error(`no version under [workspace.package] in ${cargoToml}`);
  }
  return version;
}

/**
 * The manifest fields for a version. Browsers accept only up to four numbers in `version`, so
 * a pre-release suffix goes into `version_name`, which is what the extensions page shows.
 */
export function manifestVersion(version = workspaceVersion()) {
  const numeric = version.split(/[-+]/u)[0];
  if (!/^\d+(\.\d+){0,3}$/u.test(numeric)) {
    throw new Error(`"${version}" does not start with a version a browser manifest accepts`);
  }
  return numeric === version ? { version } : { version: numeric, version_name: version };
}
