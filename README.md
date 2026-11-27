# d3khan song saver

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Release](https://img.shields.io/github/v/release/d3khan/d3khan_song_saver?label=Latest%20Release)](https://github.com/d3khan/d3khan_song_saver/releases)

A cross-platform desktop app (**Tauri v2 + Rust + Python**) that turns Spotify links, YouTube links, YouTube Music links, or plain search queries into properly tagged **MP3** or **MP4** files.

> **⚠️ Legal note:** Downloading copyrighted audio may violate the terms of service of the source sites and the rights of copyright holders. Use this only for content you are legally allowed to download — your own uploads, public-domain material, or with explicit permission. The author accepts no responsibility for misuse.

## 🚀 What's new in 0.2.12

**Official studio audio, not music-video rips.** Audio downloads now resolve through the YouTube Music catalog via `ytmusicapi`, then score candidates to prefer album/art tracks (`MUSIC_VIDEO_TYPE_ATV`) over official videos, live takes, remixes, covers and sped-up edits. Matching uses the artist, title and track duration already known from Spotify metadata.

**Zero external binaries required.** Search, playlist expansion and downloads all run inside the frozen Python sidecar. You do **not** need `yt-dlp` installed on your system, and the app no longer shells out to it.

**Private FFmpeg, cached properly.** The app ships its own `d3khan-ffmpeg`, so it never overwrites or conflicts with your system FFmpeg. It is staged once into a persistent cache instead of a fresh temp copy per track — a 300-song playlist no longer leaves gigabytes in your temp folder.

**Smart data saver (YouTube sources only).** In Audio mode, if the track came from a YouTube link and you already have its video on disk, the app converts that local video to MP3 with the bundled FFmpeg — **zero bytes downloaded** — and embeds cover art plus title/artist tags. Spotify and search-metadata downloads always fetch fresh official audio instead.

**Cover art and settings actually persist.** Fixed a serialisation mismatch that stopped cover art and durations from reaching the UI, and stopped your output folder / media mode from being remembered between launches.

**Friendlier expiry.** When a build expires, the dialog offers an **Update** button that opens the releases page directly.

## ⚙️ How it works

1. **Resolve** — paste a Spotify track/playlist/album, a YouTube or YouTube Music link, or type a query. Metadata (title, artist, duration, cover) is resolved before anything downloads.
2. **Pick the source**
   - YouTube / YouTube Music link → used directly
   - Audio + anything else → YouTube Music catalog, scored for the official studio track
   - Video + anything else → YouTube search for the official video
3. **Download & organise** — files land in `Output/Playlist Name/Title - Artist.mp3` (videos go in a `videos/` subfolder), with embedded thumbnails and tags.
4. **Sync** — re-running a playlist skips what you already have, and offers to remove files for tracks that disappeared from the playlist. You always get a prompt before anything is deleted.

## 🛠️ Requirements (for building)

| Requirement | Notes |
| --- | --- |
| **Node.js 20+** | Tauri CLI and frontend tooling |
| **Rust (stable)** | Via `rustup`. On Windows you also need VS Build Tools (C++ workload) |
| **Python 3.10+** | Builds the two sidecars with PyInstaller |
| **Linux webkit deps** | `libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf` |

FFmpeg is downloaded and bundled automatically by CI. For local development, drop a copy into `src-tauri/binaries/` named `d3khan-ffmpeg-<your-target-triple>`.

## 🏗️ Build

### Install dependencies
```bash
npm install
pip install --force-reinstall "yt-dlp[default] @ https://github.com/yt-dlp/yt-dlp/archive/master.tar.gz"
pip install --force-reinstall git+https://github.com/sigma67/ytmusicapi.git
pip install pyinstaller requests spotifyscraper
```

### Build the sidecars

**Linux**
```bash
pyinstaller --onefile --name downloader --collect-all ytmusicapi --collect-all yt_dlp downloader.py
pyinstaller --onefile --name spotify_meta spotify_meta_sidecar.py
mv dist/downloader   src-tauri/binaries/downloader-x86_64-unknown-linux-gnu
mv dist/spotify_meta src-tauri/binaries/spotify_meta-x86_64-unknown-linux-gnu
chmod +x src-tauri/binaries/*
```

**Windows**
```powershell
pyinstaller --onefile --name downloader --collect-all ytmusicapi --collect-all yt_dlp downloader.py
pyinstaller --onefile --name spotify_meta spotify_meta_sidecar.py
move dist\downloader.exe   src-tauri\binaries\downloader-x86_64-pc-windows-msvc.exe
move dist\spotify_meta.exe src-tauri\binaries\spotify_meta-x86_64-pc-windows-msvc.exe
```

The `--collect-all` flags are required: `ytmusicapi` ships locale data and `yt_dlp` ships extractor assets that PyInstaller will not pick up otherwise.

### Run / package
```bash
cargo tauri dev      # develop
cargo tauri build    # produce installers
```

Installers are written to `src-tauri/target/release/bundle/`. Pushing a `v*` tag builds Windows and Linux installers automatically and attaches them to a draft release.

## ⚠️ Known limitations

- **MP4 cover art** is not embedded — yt-dlp needs AtomicParsley for MP4 thumbnails, which is not bundled. Audio (MP3) art works.
- **Album name and track number** are not yet written to file tags; title, artist and cover are.
- **yt-dlp is frozen at build time.** YouTube changes frequently, so older builds can start failing. Reinstall the latest release if downloads break.
- Free YouTube Music streams top out around 128 kbps AAC / 160 kbps Opus. MP3 output is a transcode for compatibility, not a quality gain.

## 📄 License

MIT — see [LICENSE](LICENSE).

*© 2026 d3khan. Built with Tauri, Rust, yt-dlp and ytmusicapi.*