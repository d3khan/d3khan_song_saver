# d3khan song saver

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

A cross-platform desktop app (Tauri v2 + Rust) that downloads Spotify and YouTube tracks, albums, and playlists as tagged MP3/MP4 files.

> **Legal note:** Downloading copyrighted audio may violate the terms of service of source sites and the rights of copyright holders. Use this tool only for content you are allowed to download. The author takes no responsibility for misuse.

## What's new in v0.2.5
- **300+ track playlists:** Fetches entire Spotify/YouTube playlists via a dedicated Python sidecar (no more 100-track cap, no Cloudflare blocks).
- **Smart Sync:** In Sync mode, if a video of the track already exists locally, it is converted to MP3 with FFmpeg instead of re-downloading (saves bandwidth).
- **Orphan cleanup:** Detects tracks removed from the playlist and prompts (10s auto-dismiss) to delete the leftover files.
- **Expiry guard:** Builds stop working after a hardcoded date so users must update.

## How it works
1. Paste a Spotify/YouTube link or a search query; metadata is resolved.
2. Rust spawns the `downloader` sidecar (yt-dlp + FFmpeg) to fetch audio/video.
3. Files are saved into a playlist folder with ID3 tags and cover art.

## Prerequisites (dev/build)
| Requirement | Notes |
| --- | --- |
| Node.js 20+ | Tauri CLI (`npm install`, `npm run dev`) |
| Rust (stable) | via rustup; MSVC + C++ workload on Windows |
| Python 3.10+ | `pip install yt-dlp pyinstaller requests spotifyscraper` |
| FFmpeg | on PATH, or bundled in `src-tauri/binaries/` |

## Build
```bash
npm install
pyinstaller --onefile --name downloader downloader.py
pyinstaller --onefile --name spotify_meta spotify_meta_sidecar.py
# then move the outputs into src-tauri/binaries/ with the target-triple suffix
npm run tauri dev        # development
npm run tauri build      # release installers
```

## License
MIT - see [LICENSE](LICENSE).

(c) 2026 d3khan. Built with Tauri, Rust, and yt-dlp.
