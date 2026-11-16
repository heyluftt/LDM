# Contributing

You need Windows, Node 24, pnpm 9, Rust stable, and the Tauri 2 prerequisites.

## Build

```
pnpm install
pnpm --filter @ldm/desktop build
cargo build --workspace
pnpm --filter @ldm/extension build
```

Load `apps/extension/.output/chrome-mv3` as an unpacked extension and register the debug native host:

```powershell
scripts\dev\register-native-host.ps1 -HostExe target\debug\ldm-native-host.exe
```

Run the app with `pnpm --filter @ldm/desktop tauri dev`. `pnpm build:installer` builds the installer.

## Checks

```
pnpm lint
pnpm -r check
pnpm -r test
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```
