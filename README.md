# LDM

A download manager for Windows with a browser extension. The extension picks up downloads, videos, and streams in Chrome, Edge, or Brave and hands them to the desktop app, which downloads them with a queue, resume, and parallel connections. Everything stays on your PC.

LDM is alpha software. Use it only for content you are allowed to download.

## Features

- Takes over browser downloads and download links, by file type and per site
- Detects videos and HLS/DASH streams on a page
- Parallel connections, resume after errors and restarts
- YouTube, X, and TikTok through [yt-dlp](https://github.com/yt-dlp/yt-dlp)
- Queue, history, and one folder per file type
- Tells you when a new version is out

## Install

1. Download `LDM_<version>_x64-setup.exe` from [Releases](../../releases) and run it. The installer is not signed, so SmartScreen warns: "More info", then "Run anyway".
2. Download `ldm-extension-<version>.zip` and unpack it into a folder you keep. Open `chrome://extensions`, turn on Developer mode, and choose "Load unpacked".
3. For YouTube, X, TikTok, and streams, install yt-dlp and FFmpeg:

   ```
   winget install yt-dlp.yt-dlp
   winget install Gyan.FFmpeg
   ```

After the first start, LDM shows what is still missing. To update, install the new version over the old one.

## Not supported

- DRM, paywalls, logins, or anti-bot measures
- Live streams that are still running
- Running downloaded files automatically

## Privacy

No account, no telemetry. LDM only asks GitHub for the latest versions of LDM and yt-dlp; the LDM check can be turned off in the settings. See the [privacy notice](docs/legal/PRIVACY_NOTICE.md).

## Development

See [CONTRIBUTING.md](CONTRIBUTING.md) and [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## License

MIT, see [LICENSE](LICENSE). yt-dlp, FFmpeg, and Deno are separate projects with their own licenses.
