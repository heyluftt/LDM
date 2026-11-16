# Architecture

LDM has three parts:

| Part | Code | Job |
| --- | --- | --- |
| Browser extension | `apps/extension` | Finds downloads, videos, and streams and builds download jobs |
| Native host | `crates/ldm-native-host` | Started by the browser; passes messages to the desktop app and starts it when needed |
| Desktop app | `apps/desktop` | Queue, history, and settings; runs the download core in `crates/ldm-core` |

The extension talks to the native host over Native Messaging. The native host forwards each message to the desktop app over the named pipe `\\.\pipe\ldm-native-host`, which only the current user can open.

The message and job types are defined in `crates/ldm-types`. Their TypeScript versions in `packages/contracts/src/generated` are generated from them.

## Downloads

- Direct files use HTTP with resume. Large files that allow ranges are downloaded over several connections into one file. Network errors are retried.
- HLS streams are fetched segment by segment and remuxed to MP4 with FFmpeg. Live streams are rejected.
- YouTube, X, TikTok, and streams LDM cannot handle itself go through yt-dlp. yt-dlp and FFmpeg run in a Windows job object, so they end together with LDM.

When the browser hands over a download and the server refuses LDM, for example because the file needs the browser's login, the download goes back to the browser.

## Data on disk

In `%APPDATA%\dev.ldm.desktop\`:

- `download-state.json`: queue and history (the 200 newest finished jobs)
- `download-settings.json`: folders per file type and the parallel-download limit
- `browser-integration.json`: how browser downloads are handled
- `logs\`: one log file per day

Partial downloads are kept in a hidden `.ldm-partials` folder inside the target folder.

## Extension ID

The extension manifest contains a public key, so the extension ID is always `dpfifoelihelgibggmccbedplhjpfmdc`. The desktop app registers the native host for this ID on every start.
