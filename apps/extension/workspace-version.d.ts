/** The version as written in the root Cargo.toml, for example `0.2.0` or `0.2.0-beta.1`. */
export function workspaceVersion(): string;

/** The manifest fields for a version; a pre-release suffix moves into `version_name`. */
export function manifestVersion(version?: string): { version: string; version_name?: string };
