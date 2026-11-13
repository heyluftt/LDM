# Contributing

## Requirements

Windows 11, Node 24, pnpm 9, Rust stable, and the Tauri 2 prerequisites (MSVC build tools, WebView2).

## Build and run

```
pnpm install
pnpm --filter @ldm/desktop build
cargo build --workspace
pnpm --filter @ldm/extension build
```

Load `apps/extension/.output/chrome-mv3` as an unpacked extension, then register the debug native host:

```powershell
scripts\dev\register-native-host.ps1 -HostExe target\debug\ldm-native-host.exe
```

Start the app with `pnpm --filter @ldm/desktop tauri dev`. Quit an installed LDM first, since only one instance runs.

yt-dlp and FFmpeg are found on `PATH`, through `LDM_YTDLP_EXE` and `LDM_FFMPEG_EXE`, or in `apps/desktop/src-tauri/resources/tools/`.

## Checks

CI runs these on every push:

```
pnpm lint
pnpm -r check
pnpm -r test
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

`cargo test` also regenerates the TypeScript types in `packages/contracts/src/generated`. Commit them together with the Rust change.

## Installer

`pnpm build:installer` builds `target\release\bundle\nsis\LDM_<version>_x64-setup.exe`.

## Releases

1. Raise `version` in the root `Cargo.toml` and run `cargo check`.
2. Commit, tag the commit `v<version>`, and push the tag.
3. The release workflow creates a draft with the installer, the extension, and checksums. Check it and publish it.
