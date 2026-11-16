// Reads the workspace version from the root Cargo.toml. Plain JS to avoid Node types in the extension.
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const cargoToml = fileURLToPath(new URL("../../Cargo.toml", import.meta.url));

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

/** Browsers accept only four numbers in `version`; a pre-release suffix goes into `version_name`. */
export function manifestVersion(version = workspaceVersion()) {
  const numeric = version.split(/[-+]/u)[0];
  if (!/^\d+(\.\d+){0,3}$/u.test(numeric)) {
    throw new Error(`"${version}" does not start with a version a browser manifest accepts`);
  }
  return numeric === version ? { version } : { version: numeric, version_name: version };
}
