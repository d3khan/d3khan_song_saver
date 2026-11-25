# d3khan song saver

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Release](https://img.shields.io/github/v/release/d3khan/d3khan_song_saver?label=Latest%20Release)](https://github.com/d3khan/d3khan_song_saver/releases)

`d3khan song saver` is a cross-platform desktop app (built with **Tauri v2 + Rust**) that downloads Spotify and YouTube tracks, albums, and playlists as properly tagged MP3 or MP4 files. 

> **⚠️ Legal Note:** Downloading copyrighted audio may violate the terms of service of source sites and the rights of copyright holders. Use this tool only for content you are legally allowed to download (e.g., your own uploads, public-domain material, or with explicit permission). The authors take no responsibility for misuse.

---

## 🚀 What's New in v0.2.9

- **🎵 High-Quality Official Audio:** Integrated `ytmusicapi` to search the official YouTube Music catalog. When downloading audio, it now fetches the exact official track instead of random user-uploaded videos.
- **🎬 YouTube Music Link Support:** You can now paste `music.youtube.com` links directly, and the app will resolve and download them seamlessly.
- **🧠 Smart Sync & Bandwidth Saving:** In "Sync" mode, the app first checks if a video file (MP4/WebM) of the track already exists locally. If it does, it **automatically converts it to MP3** using the bundled FFmpeg, saving you massive amounts of bandwidth and download time.
- **📦 Private FFmpeg Bundle:** The app now ships with its own private `d3khan-ffmpeg` binary. It works perfectly out-of-the-box on any machine without conflicting with your system's installed FFmpeg.
- **✨ Refined UI/UX:** 
  - Removed annoying auto-search while typing. Searches now only trigger when you click "Find" or press Enter.
  - "Sync" mode is now cleanly hidden in Bulk Link mode, with a helpful note explaining that bulk operations default to standard download behavior.

---

## ⚙️ How It Works

1. **Search or Paste:** Enter a Spotify/YouTube link, a YouTube Music link, or a general search query. The app resolves the metadata (title, artist, cover art) instantly.
2. **Smart Download:** 
   - If downloading a playlist in **Sync** mode, it checks for existing local videos and converts them to audio first.
   - If no local file exists, it uses `ytmusicapi` (for audio) or `yt-dlp` (for video) to fetch the highest quality media.
3. **Tag & Organize:** Files are saved in organized folders (e.g., `Playlist Name/Artist - Title.mp3`) with embedded Spotify cover art and ID3v2.3 tags, ensuring they display perfectly in Windows Explorer, VLC, and other media players.

---

## 🛠️ Prerequisites (For Development / Building)

| Requirement | Notes |
| --- | --- |
| **Node.js 20+** | Required for the Tauri CLI (`npm install`, `npm run dev`) |
| **Rust (stable)** | Install via `rustup`. Visual Studio Build Tools with C++ workload required on Windows. |
| **Python 3.10+** | Required to build the `downloader` and `spotify_meta` sidecars via PyInstaller. |
| **FFmpeg** | Bundled automatically by the CI pipeline, but having it on your system PATH is helpful for local development. |

---

## 🏗️ Build Instructions

### 1. Install Dependencies
```bash
npm install
pip install --force-reinstall git+https://github.com/sigma67/ytmusicapi.git
pip install --force-reinstall "yt-dlp[default] @ https://github.com/yt-dlp/yt-dlp/archive/master.tar.gz"
pip install pyinstaller requests spotifyscraper
```

### 2. Build Python Sidecars
The app relies on two compiled Python executables. Build them for your target platform:

**For Linux:**
```bash
pyinstaller --onefile --name downloader downloader.py
pyinstaller --onefile --name spotify_meta spotify_meta_sidecar.py
mv dist/downloader src-tauri/binaries/downloader-x86_64-unknown-linux-gnu
mv dist/spotify_meta src-tauri/binaries/spotify_meta-x86_64-unknown-linux-gnu
chmod +x src-tauri/binaries/*
```

**For Windows:**
```powershell
pyinstaller --onefile --name downloader downloader.py
pyinstaller --onefile --name spotify_meta spotify_meta_sidecar.py
move dist\downloader.exe src-tauri\binaries\downloader-x86_64-pc-windows-msvc.exe
move dist\spotify_meta.exe src-tauri\binaries\spotify_meta-x86_64-pc-windows-msvc.exe
```

### 3. Run in Development Mode
```bash
cargo tauri dev
```

### 4. Create a Release Build
```bash
cargo tauri build
```
*This will generate an `.exe` installer (Windows) or `.deb`/`.rpm` packages (Linux) in `src-tauri/target/release/bundle/`.*

---

## 📄 License

This project is licensed under the **MIT License**. See the [LICENSE](LICENSE) file for details.

---
*© 2026 d3khan. Built with Tauri, Rust, yt-dlp, and ytmusicapi.*