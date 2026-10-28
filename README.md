# d3khan song saver

`d3khan song saver` is a Windows-first desktop app (**Tauri v2 + Rust**) that downloads
Spotify tracks, albums, and playlists as **tagged MP3 files**. The UI is a self-contained
vanilla HTML/CSS/JS frontend.

Spotify provides the **metadata** (title, artist, album, year, cover art URL), and
YouTube provides the **audio** via `yt-dlp`. A Python sidecar (compiled with PyInstaller
into `downloader.exe`) handles the download and embeds the YouTube thumbnail; Rust then
uses `lofty` to clean the title, add the year, and swap in the Spotify cover art as the
ID3v2.3 front cover.

> **Legal note** — Downloading audio may violate the terms of service of the source sites
> and the rights of copyright holders. Use this tool only for content you are allowed to
> download (for example your own uploads or public-domain material), and obey the law
> where you live. The authors take no responsibility for misuse.

---

## How it works

1. **Search or paste a link** — the user pastes a Spotify track/album/playlist URL or a
   search query. Rust resolves metadata from Spotify (title, artist, cover URL) and sends
   it to the frontend.
2. **Download** — Rust spawns the Python sidecar (`downloader.exe`), which uses `yt-dlp`
   and FFmpeg to download the MP3 from YouTube and embed the thumbnail.
3. **Tag** — Rust opens the file with `lofty`, cleans the title, adds the year, and swaps
   the YouTube thumbnail for the Spotify cover art (ID3v2.3). If the Spotify cover can't
   be fetched, the original YouTube thumbnail is left intact.

---

## Features

- **Search** — type a query, pick from Spotify-matched results, download a single track.
- **Single download** — paste a Spotify track link to download one MP3 named
  `Title - Artist.mp3` into your chosen folder.
- **Bulk / playlist** — paste a Spotify playlist or album link, or drop multiple links in
  bulk mode, to download many tracks into a named folder.
- **Cover art** — Spotify cover art is embedded as the ID3v2.3 front cover so it shows in
  Windows Explorer and VLC. A local `.jpg` backup is kept next to each file as a fallback.
- **ID3v2.3 tags** — title, artist, album, year, and cover art are written as ID3v2.3,
  which Windows Explorer reads reliably for file thumbnails.

---

## Prerequisites (Windows)

| Requirement | Notes |
|---|---|
| Windows 10/11 + WebView2 | WebView2 ships with current Windows; otherwise install the Evergreen Runtime |
| Rust (MSVC toolchain) | Install via [rustup](https://rustup.rs). The repo pins `stable-x86_64-pc-windows-msvc` in `rust-toolchain.toml`; Visual Studio Build Tools with the C++ workload are required for source builds |
| Node.js 20+ | Only for the Tauri CLI (`npm install`, `npm run dev`) |
| FFmpeg | Must be installed in AppData (the app looks for `ffmpeg.exe` in `%APPDATA%` and `%APPDATA%\bin`) |
| Python sidecar | `downloader.exe` is bundled in `src-tauri/binaries/`; built from `downloader.py` with PyInstaller |

### FFmpeg

FFmpeg is required in AppData. The Rust backend searches for `ffmpeg.exe` in:

1. `%APPDATA%\ffmpeg.exe`
2. `%APPDATA%\bin\ffmpeg.exe`

Install FFmpeg (e.g. via [gyan.dev](https://www.gyan.dev/ffmpeg/builds/) or
`winget install Gyan.FFmpeg`) and place `ffmpeg.exe` (and `ffprobe.exe`) in
`%APPDATA%` or `%APPDATA%\bin`.

---

## Build

### Development

```powershell
npm install
npm run dev          # tauri dev
```

`npm run dev` serves `src/` directly — there is no Vite/webpack step. Opening
`src/index.html` in a plain browser still works for layout work.

Backend checks:

```powershell
cd src-tauri
cargo check
cargo test
```

### Python sidecar

The downloader sidecar is compiled with PyInstaller:

```powershell
pyinstaller --onefile --name downloader downloader.py
move dist\downloader.exe src-tauri\binaries\downloader-x86_64-pc-windows-msvc.exe
```

### Release build

```powershell
npm run build        # NSIS installer in src-tauri/target/release/bundle/nsis
```

The installer bundles `downloader.exe`; end users only need FFmpeg in AppData.

---

## Project layout

```
src/                      vanilla HTML/CSS/JS frontend (index.html, styles.css, app.js)
src-tauri/
├── src/
│   ├── lib.rs            Tauri app setup, tray, invoke handler
│   ├── commands.rs       Tauri commands (resolve_input, pick_folder, download_track)
│   ├── downloader.rs     Search, batch download, Python sidecar spawn, ID3 tagging
│   └── metadata.rs       Spotify URL parsing + metadata extraction
├── binaries/             downloader-x86_64-pc-windows-msvc.exe (bundled sidecar)
├── icons/
└── tauri.conf.json
downloader.py             Python sidecar source (yt-dlp + FFmpeg)
docs/                     research, architecture, implementation contract
```

---

## Troubleshooting

| Symptom | Fix |
|---|---|
| Cover art missing in Explorer / VLC | Re-download the track; the app swaps in the Spotify cover as ID3v2.3 art and keeps a local `.jpg` fallback next to the file |
| "FFmpeg not found" | Install FFmpeg and place `ffmpeg.exe` in `%APPDATA%` or `%APPDATA%\bin` |
| "Sidecar not found" | Build `downloader.exe` with PyInstaller and place it in `src-tauri/binaries/` as `downloader-x86_64-pc-windows-msvc.exe` |
| Track downloads with wrong audio | yt-dlp matches the first YouTube result for `artist - title`; no perfect matching is guaranteed |
| Build fails on `link.exe` | Install Visual Studio Build Tools with the "Desktop development with C++" workload; the pinned toolchain is MSVC |

---

## Testing

After building, run `npm run tauri dev`, search for a song (e.g. "Middle of the Night"),
download it, and verify the Spotify cover art appears in Windows Explorer and VLC.
