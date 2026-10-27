# d3khan song saver

A Windows-first desktop app (Tauri v2 + Rust) that downloads Spotify tracks, albums, and
playlists as **tagged MP3 files** and keeps playlist folders in sync over time. The UI is a
self-contained vanilla HTML/CSS/JS frontend.

Spotify is used for **metadata only**. Audio is fetched from public sources through
[yt-dlp](https://github.com/yt-dlp/yt-dlp) and converted/tagged locally with FFmpeg and
[lofty](https://github.com/Serial-ATA/lofty-rs). There is no DRM bypass and no Spotify audio
download.

> **Legal note** — Downloading audio may violate the terms of service of the source site and
> the rights of copyright holders. Use this tool only for content you are allowed to download
> (for example your own uploads or public-domain material), and obey the law where you live.
> The authors take no responsibility for misuse. Spotify Premium does not grant download
> rights to third-party audio.

---

## Features

- **Single track** → one MP3 directly in the chosen folder, named `Title - Artist.mp3`.
- **Album** → a named subfolder with `01 - Title - Artist.mp3` numbering.
- **Playlist** → a named subfolder with zero-padded position prefixes (`01 - Title - Artist.mp3`).
- **Bulk mode** → a timestamped master folder; loose tracks live in the master, albums and
  playlists get subfolders inside it.
- **Playlist sync** via a `.playlist_sync.json` state file in each playlist folder:
  - skips tracks that are already present,
  - downloads newly added tracks,
  - prompts before deleting files for tracks removed from the playlist,
  - never renumbers existing files,
  - detects the same playlist in another folder and asks whether to sync there instead.
- **ID3v2.3 tags** written from the Spotify metadata: title, artist, album artist, album,
  track/total, disc, release date, genres, ISRC, label, copyright, explicit flag, and the
  Spotify URL as a comment. ID3v2.3 is intentional — Windows Explorer's thumbnail handler
  reads APIC art from v2.3 but often shows a generic icon for v2.4, so every downloaded file
  gets its cover as the Explorer thumbnail (cover art is still best-effort).
- Per-track failure isolation: a failed track is reported and the job continues. If YouTube
  silently drops a search term (it returns zero hits for some explicit words), the app retries
  with a cleaned query before reporting the track as failed.
- Full **Spotify Authorization Code + PKCE** login (no client secret required). Tokens are
  stored in Windows Credential Manager and refreshed automatically. The public Client ID is
  compiled into release builds, so people who install the app never enter developer
  credentials.

---

## Prerequisites (Windows)

| Requirement | Notes |
|---|---|
| Windows 10/11 + WebView2 | WebView2 ships with current Windows; otherwise install the Evergreen Runtime |
| Rust (MSVC toolchain) | Install via [rustup](https://rustup.rs). The repo pins `stable-x86_64-pc-windows-msvc` in `rust-toolchain.toml`; Visual Studio Build Tools with the C++ workload are required |
| Node.js 20+ | Only for the Tauri CLI (`npm install`, `npm run dev`) |
| yt-dlp + FFmpeg | **Bundled with the installer.** Source builds read them from `src-tauri/binaries/`; install them with `winget install yt-dlp.yt-dlp` and `winget install Gyan.FFmpeg` if you build from source |

Binary lookup order (first hit wins):

1. `YTDLP_PATH` / `FFMPEG_PATH` environment variables,
2. next to the app executable (where the installer places the bundled tools),
3. `binaries/` next to the executable,
4. `binaries/` in the current working directory (`src-tauri/binaries` in development),
5. `../src-tauri/binaries/`,
6. `src-tauri/binaries/` relative to the crate,
7. `yt-dlp` / `ffmpeg` on `PATH`.

If the binaries are missing, starting a job fails with an instruction instead of a stack trace.

---

## Spotify app setup (source builds only)

Installed builds already contain the developer's public Client ID and redirect URI. You only
need this section if you build the app yourself:

1. Open the [Spotify Developer Dashboard](https://developer.spotify.com/dashboard) and create
   an app (any name; choose **Web API**).
2. Add this **Redirect URI** verbatim:
   `http://127.0.0.1:8888/callback`
3. Copy the **Client ID**. The Client Secret is optional (PKCE does not use it).
4. While the app is in Development Mode, add your Spotify account under **User Management**
   (up to 25 users).

The app requests these scopes: `playlist-read-private`, `playlist-read-collaborative`,
`user-read-private`, `user-read-email`.

### Client ID configuration (build time)

The build script reads `SPOTIFY_CLIENT_ID` from `.env` and compiles the value into the binary.
That is why the installed app has no credential fields: end users only sign in.

1. Runtime overrides (development): a saved settings file, then `.env` / process environment.
2. Release builds otherwise use the Client ID embedded at compile time by `build.rs`.
3. If no Client ID is available at build time, the app shows “Spotify isn’t configured for
   this build.”

```dotenv
SPOTIFY_CLIENT_ID=your_client_id
SPOTIFY_CLIENT_SECRET=optional
SPOTIFY_REDIRECT_URI=http://127.0.0.1:8888/callback
```

`.env` is gitignored, and the Client Secret is never embedded in the binary. The optional
secret (if provided) is stored in Windows Credential Manager under the service
`spotify_downloader`; it is never returned to the UI or logged. Use **Log out** in Settings to
clear the saved Spotify session.

---

## Automated Build (One-Click)

For the fastest way to build from source, you can use our automated build scripts. These scripts will automatically detect or clone the repository, fetch the correct `yt-dlp` and `FFmpeg` binaries for your OS, install dependencies, and compile the app. 

If the download or build fails halfway, **simply run the command again** — the scripts are designed to resume from where they left off.

### NOTE:
As the installation script is running create the .env file and fill it with the correct details so the app may build properly

**Windows (PowerShell)**
```powershell
iwr -useb https://raw.githubusercontent.com/d3khan/spotify_downloader/main/installers/build_windows.ps1 | iex
```

**Windows (CMD)**
```cmd
curl -sSL https://raw.githubusercontent.com/d3khan/spotify_downloader/main/installers/build_windows.cmd -o build.cmd && build.cmd
```

**Linux / macOS**
```bash
curl -sSL https://raw.githubusercontent.com/d3khan/spotify_downloader/main/installers/build_linux.sh | bash
```

Once completed, your installers will be available in the `dist/` folder.

---

## Manual Build

If you prefer to build manually, or if you are developing the app:

```powershell
npm install
npm run dev          # tauri dev
npm run build        # NSIS installer in src-tauri/target/release/bundle/nsis
```

`npm run build` bundles `yt-dlp.exe`, `ffmpeg.exe`, and `ffprobe.exe` next to the installed
executable, so end users need no extra downloads. The installer is therefore several hundred
megabytes.

`tauri dev` serves `src/` directly — there is no Vite/webpack step. Opening `src/index.html`
in a plain browser still works for layout work: without `__TAURI__` a mock engine answers the
same commands as the Rust backend.

Backend checks:

```powershell
cd src-tauri
cargo check
cargo test
```

---

## How downloads are stored

Single link, **Download** mode:

```
<chosen folder>\
├── Get Lucky - Daft Punk.mp3                    # track link: "<title> - <artist>"
├── Daft Punk - Random Access Memories\          # album link: "<artist> - <album>"
│   └── 01 - Give Life Back to Music - Daft Punk.mp3
└── My Playlist\                                 # playlist link
    ├── 01 - Get Lucky - Daft Punk.mp3
    └── .playlist_sync.json
```

Bulk mode creates one master folder per job:

```
<chosen folder>\Bulk Download 2026-10-27 12-00-00\
├── Loose Track - Artist.mp3
├── Album Artist - Album\
└── Playlist Name\
```

Names are sanitized for Windows (`< > : " / \ | ? *`, control characters), reserved device
names are escaped, and case-insensitive collisions get a ` (2)`, ` (3)`, … suffix. Existing
files are skipped, never overwritten (a zero-byte file is re-downloaded).

### Tags and thumbnails

Every file is tagged before it is moved into place, so a failed tag write never leaves a
half-tagged file under the final name:

| Frame | Source |
|---|---|
| `TIT2`, `TPE1`, `TALB`, `TPE2` | Spotify title, artists, album, album artists |
| `TRCK`, `TPOS` | track/total and disc number |
| `TYER` + `TDAT`, `TCON` | release date, genres |
| `TSRC`, `TCOP`, `TPUB` | ISRC, copyright, label |
| `COMM` | canonical `open.spotify.com` URL |
| `APIC` | cover art (front cover, JPEG) |

Tags are written as **ID3v2.3** on purpose. Windows Explorer shows the embedded cover as the
file thumbnail for v2.3 frames but falls back to a generic icon (for example the VLC cone) for
many v2.4 files, and every mainstream player reads v2.3.

---

## Playlist sync

`.playlist_sync.json` lives inside the playlist folder and records the Spotify playlist id plus
one entry per downloaded track:

```json
{
  "schema_version": 1,
  "playlist_id": "37i9dQZF1DX...",
  "playlist_name": "My Playlist",
  "source_url": "https://open.spotify.com/playlist/...",
  "updated_at": "2026-10-27T12:00:00+00:00",
  "entries": [
    {
      "track_id": "4uLU6hMCjMI75M1A2tKUQC",
      "local_filename": "01 - Get Lucky - Daft Punk.mp3",
      "title": "Title",
      "artist": "Artist",
      "status": "ok"
    }
  ]
}
```

What a sync run does:

1. Resolve the playlist and diff it against the state file and the files on disk.
2. If another folder in the output tree tracks the same playlist, ask:
   **Sync here**, **Create a new folder**, or **Cancel** (timeout = cancel).
3. Prompt about tracks that disappeared from Spotify: **Keep** or **Delete** the local file
   (timeout = keep). Kept tracks are marked `kept_removed` and are not asked about again.
4. Download new/missing tracks (a missing file is re-downloaded under its original name).
5. Delete files only after downloads finish, only if you chose **Delete**.
6. Leave untracked files alone — they are reported in the log, never deleted or adopted.

The state file is written atomically after every successful track. A corrupt file is backed
up as `.playlist_sync.json.corrupt-<timestamp>` and a fresh state is started. Sync is available
for **one playlist link at a time**; bulk sync is intentionally disabled. Download mode still
creates the state file so the folder can be synced later; it never deletes or prompts.

---

## Project layout

```
src/                      frozen frontend (index.html, styles.css, app.js) + preview README
src-tauri/
├── src/
│   ├── lib.rs            AppState, startup, invoke handler
│   ├── commands.rs       Tauri commands + DTOs
│   ├── auth.rs           PKCE loopback flow, keyring, baked client ID
│   ├── metadata.rs       Spotify URL parsing + rspotify fetching
│   ├── naming.rs         sanitizing, folder/file names, collisions
│   ├── sync.rs           state file, atomic IO, diff, nested-folder scan
│   ├── downloader.rs     yt-dlp/FFmpeg pipeline + ID3 tagging
│   └── jobs.rs           job runner, events, prompts, cancellation
├── binaries/             yt-dlp.exe / ffmpeg.exe / ffprobe.exe (gitignored; bundled by npm run build)
├── icons/
└── tauri.conf.json
docs/                     research, architecture, implementation contract
```

Tokens: Windows Credential Manager (`spotify_downloader` / `spotify_token`), full rspotify
`Token` JSON. The access token is refreshed five minutes before it expires; if Spotify's
refresh response omits the refresh token, the existing one is carried forward instead of
being dropped. Refresh tokens last six months from authorization, so the app can sit unused
for weeks and still reconnect; after six months it shows a disconnected state and you sign
in again.

---

## Troubleshooting

| Symptom | Fix |
|---|---|
| “yt-dlp not found” | Reinstall the app (the installer bundles it), or for source builds put `yt-dlp.exe` in `src-tauri/binaries/`; verify with `yt-dlp --version` |
| “Port 8888 is already in use” | Close the other process or set `SPOTIFY_REDIRECT_URI` to another loopback port **and** register it in the Spotify dashboard |
| Spotify rejects the playlist (404) | Editorial/algorithmic playlists (e.g. “Today’s Top Hits” IDs starting `37i9…`) are not readable by third-party apps; use a playlist you own or follow, and check User Management |
| “Connect Spotify in Settings first” | Complete the browser round-trip from the auth banner |
| "Spotify isn’t configured for this build" | Rebuild with `.env` containing `SPOTIFY_CLIENT_ID` so `build.rs` can embed it |
| Explorer shows a generic icon or the VLC cone instead of the cover | The file was tagged as ID3v2.4 by an older build; re-download it (or re-tag it to v2.3) with the current version |
| Tracks download with the wrong audio | yt-dlp matches the first search result (`artist - title`). Rename/curate the source or improve tags; no perfect matching is guaranteed |
| “No YouTube results for … (N query variants tried)” | YouTube returns zero hits for that exact query. The app already retries without the separator and without explicit words; if the title is still too unusual, add the track manually or rename the source |
| Login worked months ago, now fails | Dashboard refresh tokens expire after ~6 months; sign out and connect again |
| Build fails on `link.exe` not found | Install Visual Studio Build Tools with the “Desktop development with C++” workload; the pinned toolchain is MSVC |

MP3 is the only output format in v1. Cover art embedding is best-effort: a failed cover fetch
never fails the track, but text tags are required. Files written by builds before the switch to
ID3v2.3 keep their old tags until they are re-downloaded.
```

### Notes on the `curl` command:
* **Linux:** `curl -sSL ... | bash` downloads the script and pipes it directly to bash.
* **Windows PowerShell:** `iwr -useb ... | iex` is the standard PowerShell way to download and execute a script in memory without saving it to disk first.
* **Windows CMD:** CMD doesn't have a native equivalent to piping into memory, so the command downloads it as `build.cmd` and then executes it immediately.
* **Important:** Make sure you actually push the `installers` folder to your GitHub repo under the `main` branch for these links to work!