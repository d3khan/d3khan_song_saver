# Phase 1 Research — Spotify Downloader (Tauri v2 + Rust)

**Date:** 2026-10-26
**Scope:** Backend crate selection (rustifydl verdict first), Spotify API client (rspotify), Tauri v2 stack,
scaffolding, external binaries (yt-dlp/FFmpeg), build/runtime prerequisites, verified dependency block.
**Method:** Every version below was verified live against the crates.io API, npm registry, docs.rs,
GitHub (source/README/tags), and official Tauri/Spotify/yt-dlp docs on 2026-10-26. No version numbers were guessed.

---

## 1. CRITICAL — `rustifydl`: does it exist and can it carry this app?

### 1.1 Existence & metadata (VERIFIED)

| Field | Value | Source |
|---|---|---|
| Exists on crates.io | **YES** | https://crates.io/api/v1/crates/rustifydl |
| Latest version | **0.2.41** (published 2026-01-29; 0.2.x line started 2026-01-27) | crates.io API versions[] |
| Repository | https://github.com/ThePangel/RustifyDL | crates.io API `repository` |
| Docs | https://docs.rs/rustifydl/0.2.41 (renders README; docs.rs build succeeds) | docs.rs |
| License | **GPL-3.0** (`license = "GPL-3.0"` in Cargo.toml; LICENSE in repo) | Cargo.toml @ main |
| Downloads | 2,774 total, ~20 recent | crates.io API |
| Maintenance | Created 2025-08-04; last push **2026-01-29** (≈9 months stale as of today); 9 stars; 0 open issues; author status note: *"Active development (When and if I feel like it, works for now)"* | GitHub API + README |
| Category | CLI tool **with a library target** (`src/lib.rs` + `src/bin/cli.rs`) | Cargo.toml |

`cargo search`-equivalent query `https://crates.io/api/v1/crates?q=rustifydl` returns only this crate.
It is real, published, documented, and installable (`cargo install rustifydl`).

### 1.2 Real public API (quoted from source, not invented)

Source: https://raw.githubusercontent.com/ThePangel/RustifyDL/main/src/lib.rs (v0.2.41)

```rust
pub struct DownloadOptions {
    pub url: String,                  // Spotify URL (track/album/playlist)
    pub client_id: String,            // Spotify Client ID
    pub client_secret: String,        // Spotify Client Secret
    pub output_dir: String,           // Destination folder for audio files
    pub concurrent_downloads: usize,  // max parallel downloads
    pub no_dupes: bool,               // skip duplicate track names
    pub bitrate: String,              // ffmpeg target, e.g. "192k"
    pub format: String,               // "mp3" (also flac/ogg/opus/m4a/wav per CLI)
    pub verbosity: String,            // "full"|"info"|"debug"|"no-bars"|"none"
    pub no_tag: bool,                 // don't write tags/cover art
}

pub async fn download_spotify(
    options: DownloadOptions,
    ytdlp_dir: String,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> { ... }

pub fn extract_id_from_url(url: &str) -> Option<String> { ... }   // track|album|playlist|artist
pub mod metadata;   // metadata::metadata(song, track, options) — lofty tagging
pub mod spotify;    // fetch_track / fetch_album / fetch_playlist -> HashMap<String, Track>
pub mod youtube;    // search_yt, download, download_ytdlp, update_ytdlp
```

README usage example (https://docs.rs/crate/rustifydl/0.2.41):

```rust
use rustifydl::{download_spotify, DownloadOptions};
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let opts = DownloadOptions {
        url: "https://open.spotify.com/album/xxxxxxxx".into(),
        client_id: "<your_spotify_client_id>".into(),
        client_secret: "<your_spotify_client_secret>".into(),
        output_dir: "./downloads".into(),
        concurrent_downloads: 6,
        no_dupes: true,
        bitrate: "192k".into(),
        format: "mp3".into(),
        verbosity: "info".into(),
        no_tag: false,
    };
    let ytdlp_dir = String::from("./ytdlp");
    download_spotify(opts, ytdlp_dir).await?;
    Ok(())
}
```

### 1.3 What it actually does internally (verified from source)

Project structure (README + repo listing): `lib.rs` (orchestration), `metadata.rs` (lofty tagging),
`spotify.rs` (spotify-rs), `youtube.rs` (rustypipe search + yt-dlp + ffmpeg), `bin/cli.rs` (clap CLI).

- **Spotify metadata:** `spotify-rs` `ClientCredsClient::authenticate(client_id, client_secret)` (client-credentials flow),
  `fetch_track/fetch_album/fetch_playlist` with `tracks.get_all(&spotify)` pagination (`src/spotify.rs`).
  Episodes in playlists are skipped (`PlayableItem::Episode(_) => {}`), null items logged `"No song found."`.
- **YouTube search:** `RustyPipe::new().query().music_search_tracks(name)` then **blindly takes the first hit**:
  `download(search_results.items.items[0].id.as_str(), ...)` (`src/youtube.rs`, line ~40).
- **yt-dlp:** `download_ytdlp(ytdlp_dir)` **downloads the latest yt-dlp binary at runtime via system `curl`**
  (`https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe` on Windows) into
  `%APPDATA%/RustifyDL/yt-dlp.exe`, self-updates by comparing `SHA-256SUMS` line `nth(2)`, then spawns it:
  `Command::new(ytdlp_path).args(["--audio-format","opus","-N",N,"--format","bestaudio","-o",temp_path,"-x", video_id])`.
- **ffmpeg:** `Command::new("ffmpeg").args(["-i", input, "-b:a", bitrate, "-threads","0","-y", output])` — **must be on PATH**.
- **ID3/tagging:** `lofty` 0.22 — ID3v2.3 for MP3 (`use_id3v23(true)`), title/artist/album/genre/track/disc/year/front cover
  (cover fetched with `reqwest::get(album.images[0].url)`); `tag.save_to_path(...).expect(...)` (panic on failure).
- **Output naming:** `output_dir` is configurable; **filename is hard-coded** `sanitize_filename("Artists - Title") + "." + format`,
  written **flat** into `output_dir` (temp files under `output_dir/temp`). `sanitize_filename` only strips `<>:"/\|?*` + control chars.
  `DownloadResult::{Completed, Skipped}` — skip iff final file already exists.

### 1.4 Requirement-by-requirement verdict

| Requirement | Status | Evidence |
|---|---|---|
| Spotify metadata → YouTube search → yt-dlp → ffmpeg → ID3 | ✅ Full pipeline inside one call | src/*.rs quoted above |
| Playlist/album/track batching + concurrency | ✅ (`tokio::spawn` + `Semaphore`, `no_dupes`) | lib.rs |
| Output **directory** configurable | ✅ `output_dir` field | lib.rs |
| Output **filename/folder layout** configurable | ❌ **Flat `"Artist - Title.ext"` only** — no template, no per-artist/album folders, no strict layout | youtube.rs |
| `.playlist_sync.json` incremental sync state | ❌ Not supported — only "file exists ⇒ skip"; no API to enumerate planned files, no callbacks, no resumable state contract | youtube.rs `processed_file.exists()` |
| Per-track progress/status for GUI events | ❌ **No callbacks/events.** Progress = `indicatif` bars on stdout + `log` macros only | lib.rs |
| Typed error reporting (which track failed, why) | ❌ Error type is `Box<dyn std::error::Error + Send + Sync>` (unmatchable). Worse: `download_and_tag_tracks` **swallows per-track errors** — `Ok(Err(e)) => error!("Task failed: {e}")` — so `download_spotify` returns `Ok(())` even if every track failed | lib.rs |
| Safe in a Tauri host process | ❌ `download_spotify` **re-initializes the global logger** (`env_logger::Builder...init()` / `LogWrapper...try_init().unwrap()`) → panics/conflicts if the app (e.g. `tauri-plugin-log`) already installed a logger; writes `./checksums` into **process CWD**; hard `println!`s | lib.rs, youtube.rs |
| Robustness | ⚠️ Panics possible: `items[0]` on empty search results, `album.images[0]`, `album.release_date[..4]`, `.expect` in tag write, `artists[0]` | youtube.rs/metadata.rs |
| Network/config requirements | Spotify client_id/secret **must** be passed in `DownloadOptions`; system `curl` needed for yt-dlp bootstrap/update; ffmpeg on PATH; config file `%APPDATA%/RustifyDL/config.toml` (ytdlp_dir) | src |
| Async requirements | **async only** (`async fn`, `tokio::spawn`, tokio `full` features) — needs a running Tokio runtime; CLI uses `#[tokio::main]` | Cargo.toml |
| **Build on Windows** | ⚠️ Depends on `rquickjs = { features = ["bindgen"] }` (bindgen ⇒ **libclang/LLVM + C toolchain at build time**) plus QuickJS C compilation. **Verified: `rquickjs` is never referenced in any file under `src/`** (dead dependency), and the crate has **no `build.rs`** so its `[build-dependencies] bindgen` is also dead — yet both still force native build requirements | deps list + source grep 2026-10-26 |
| License compatibility | ⚠️ **GPL-3.0** — linking this crate into our app makes the whole distributed app GPL-3.0 (source disclosure obligation on release) | Cargo.toml |
| Artist URLs | ❌ Explicitly rejected: `"Artist URLs are not supported..."` | lib.rs |

**Verdict: `rustifydl` exists and is a working proof of the pipeline, but it is NOT viable as this app's backend.**
Unt met requirements (exactly):
1. **Strict configurable folder layout / filename templates** (hard-coded flat naming).
2. **`.playlist_sync.json` sync-state & incremental sync** (only whole-file existence skip; no enumeration hook).
3. **Progress/error event stream to the UI** (no callbacks; per-track errors swallowed; untyped `Box<dyn Error>`).
4. **Host-process safety in Tauri** (global logger re-init panics, CWD pollution, panicking unwrap paths).
5. **License** — GPL-3.0 copyleft may be acceptable only if the entire app is shipped as GPL open source.
6. **Windows build friction** — unused `rquickjs(bindgen)` forces LLVM/libclang builds on every dev machine/CI.

### 1.5 Closest real alternatives (verified, no invented APIs)

| Option | What it is | Pros | Cons |
|---|---|---|---|
| **A. DIY pipeline (recommended)** `rspotify` + `rustypipe` (or raw Innertube) + `yt-dlp` + `ffmpeg` + `lofty` | Compose the stages yourself in `src-tauri` | Full control of layout, sync JSON, per-track progress events, typed errors, GPL-free; all crates verified alive (versions §7) | More code; you own YouTube-search robustness |
| **B. `sptfydl` 0.3.10** — https://crates.io/crates/sptfydl (repo https://github.com/Trevrosa/sptfydl, updated 2026-03-18, **MIT**) | lib+bin crate (`src/lib.rs` exists): client-credentials token (`src/spotify/access_token.rs`), raw HTTP `GET /v1/playlists/{id}` + `next`-page following, **YouTube Music search requiring a browser cookie** (`dialoguer` prompts), external `yt-dlp`, `lofty` tagging, ISRC-preferred search | Actively updated **after** Spotify's Feb-2026 migration; MIT; light on API calls | Written as an interactive CLI (prompts/stdin cookie), YouTube search depends on user cookies, no strict-layout/sync state either; would still need adaptation |
| **C. `yt-dlp` crate** (crates.io **2.9.1**, updated 2026-10-26) | Rust wrapper "with auto dependencies installation" for YouTube downloading | Could replace raw `Command` spawning | Untested by us; still needs ffmpeg; no Spotify layer |
| **D. Reimplement stages from rustifydl source as reference** (rustypipe search + yt-dlp spawn + lofty tagging code is GPL — **do not copy verbatim** unless going GPL) | Learn from, don't paste | — | License discipline required |

Decision driver: the app spec (strict folder layout + `.playlist_sync.json` + progress UI) **cannot** be met by
calling `rustifydl::download_spotify` as a black box → **backend = Option A**, with §2 (rspotify) as the Spotify layer.

---

## 2. `rspotify`

### 2.1 Version & features (VERIFIED)

| Field | Value | Source |
|---|---|---|
| Latest | **0.16.1** (published 2026-04-25; tag `v0.16.1`) | crates.io API; GitHub tags |
| License | MIT | Cargo.toml @ v0.16.1 |
| Repo | https://github.com/ramsayleung/rspotify | crates.io API |

Feature flags (verbatim from https://raw.githubusercontent.com/ramsayleung/rspotify/v0.16.1/Cargo.toml):

```toml
[features]
default = ["client-reqwest", "reqwest-default-tls"]
cli = ["webbrowser"]            # only for browser OAuth helpers (prompt_for_token etc.)
env-file = ["dotenvy"]
client-ureq = ["rspotify-http/client-ureq", "__sync"]
client-reqwest = ["rspotify-http/client-reqwest", "__async"]
reqwest-default-tls = ["rspotify-http/reqwest-default-tls"]   # native-tls (schannel on Windows)
reqwest-rustls-tls = ["rspotify-http/reqwest-rustls-tls"]
reqwest-native-tls / reqwest-native-tls-vendored / ureq-* ...
reqwest-middleware = [...]
```

**Client Credentials needs no special feature** — `default` (= `client-reqwest` + `reqwest-default-tls`) is sufficient.
The `cli` feature is only needed for OAuth-in-browser helpers; the `client_creds` example only requires
`env-file` (for `Credentials::from_env()`) + `client-reqwest`. For Linux-friendliness (avoid OpenSSL) use
`default-features = false, features = ["client-reqwest", "reqwest-rustls-tls"]`.
`reqwest-http` pins **reqwest 0.12** (`reqwest = { version = "0.12.1", ... }`) — if your app also needs a direct
HTTP client, use the **0.12 line** to avoid compiling two reqwest majors.

Async: `0.12+` is async by default on `client-reqwest` (`__async`); `ClientCredsSpotify` is the Client-Credentials client.

### 2.2 Construction + token (verified against `src/client_creds.rs` @ v0.16.1)

```rust
use rspotify::{ClientCredsSpotify, Credentials, prelude::*};

let creds = Credentials { id: "your-client-id".into(), secret: Some("your-secret".into()) };
let spotify = ClientCredsSpotify::new(creds);          // or ClientCredsSpotify::with_config(creds, config)
spotify.request_token().await?;                        // must be called before any API call
// token auto-refetches: ClientCredsSpotify::refetch_token() (no refresh token in this flow)
```

(The `client_creds` example uses `Credentials::from_env()` which needs feature `env-file`.)

### 2.3 Endpoints: signatures (verified from `src/clients/base.rs` @ v0.16.1)

```rust
async fn track(&self, track_id: TrackId<'_>, market: Option<Market>) -> ClientResult<FullTrack>;
async fn album(&self, album_id: AlbumId<'_>, market: Option<Market>) -> ClientResult<FullAlbum>;
async fn playlist(&self, playlist_id: PlaylistId<'_>, fields: Option<&str>, market: Option<Market>)
    -> ClientResult<FullPlaylist>;

// manual pagination (Page = one HTTP call):
async fn playlist_items_manual(&self, playlist_id: PlaylistId<'_>, fields: Option<&str>,
    market: Option<Market>, limit: Option<u32>, offset: Option<u32>)
    -> ClientResult<Page<PlaylistItem>>;                      // GET playlists/{id}/items

// auto pagination (Paginator/Stream — use with futures):
fn playlist_items<'b,'a:'b>(&'a self, playlist_id: PlaylistId<'a>, fields: Option<&'a str>,
    market: Option<Market>) -> Paginator<'b, ClientResult<PlaylistItem>>;

fn album_track(...) -> Paginator<ClientResult<SimplifiedTrack>>;
async fn album_track_manual(&self, album_id: AlbumId<'_>, market: Option<Market>,
    limit: Option<u32>, offset: Option<u32>) -> ClientResult<Page<SimplifiedTrack>>;
```

> ⚠️ Note: `GET /tracks` (several tracks) and several others were **removed by Spotify Feb 2026** and are
> `#[deprecated]` in rspotify 0.16 (see issue https://github.com/ramsayleung/rspotify/issues/550 — "Spotify Web API changes (February 2026)").
> 0.16.1 is the release line that tracks those migrations (e.g. `PlaylistItem.track` renamed → `item`).

### 2.4 Playlist → all tracks with pagination (handles nulls, local tracks, episodes, unavailable)

```rust
use rspotify::{model::{PlaylistId, PlayableItem, Id}, prelude::*};

// NOTE: rspotify Id types accept ONLY "spotify:type:id" or "spotify/type/id" —
// parse_uri() rejects https://open.spotify.com/... URLs (verified in rspotify-model/src/idtypes.rs).
// Extract the ID from a URL first, e.g.:
fn spotify_id_from_url(url: &str) -> Option<String> {
    url.rsplit('/').filter(|s| !s.is_empty())
       .find(|s| s.len() >= 22)                 // (or a proper regex: (track|album|playlist)/([A-Za-z0-9]+))
       .map(|s| s.split('?').next().unwrap().to_string())
}

let playlist_id = PlaylistId::from_id(&id)?;    // or from_uri("spotify:playlist:...")

// Metadata (name, owner, description, images) — safe with app auth (security: oauth_2_0: [] in spec):
let meta = spotify.playlist(&playlist_id, None, None).await?;
println!("{} — {} tracks", meta.name, meta.tracks.total);

// All items, manual pagination (pagination_manual example pattern @ v0.16.1):
let (limit, mut offset) = (50u32, 0u32);
let mut skipped_unavailable = 0u32;
loop {
    let page = spotify.playlist_items_manual(&playlist_id, None, None, Some(limit), Some(offset)).await?;
    for item in page.items {                       // Page<T>: items already filtered of JSON nulls
        // item: PlaylistItem { added_at: Option<_>, added_by: Option<_>, is_local,
        //                       track (deprecated alias), item: Option<PlayableItem> }
        let Some(playable) = item.item.or(item.track) else { continue; };   // null / removed entry
        match playable {
            PlayableItem::Track(t) => {
                if t.is_local { /* local file: no Spotify ID, no preview — skip for downloading */ continue; }
                let track_id: Option<_> = t.id;          // None ⇒ unavailable/restricted/local
                if let Some(restrictions) = &t.restrictions { /* unavailable in market */ skipped_unavailable += 1; }
                let title  = t.name.clone();
                let artists = t.artists.iter().map(|a| a.name.as_str()).collect::<Vec<_>>().join(", ");
                let album  = t.album.name.clone();              // SimplifiedAlbum
                let track_no = t.track_number;                  // u32
                let disc_no  = t.disc_number;                   // i32
                let duration = t.duration;                      // chrono::Duration (serde name: duration_ms)
                let duration_ms = duration.num_milliseconds();
                // ... build download job; track_id.unwrap() only when Some
            }
            PlayableItem::Episode(_) => { /* podcasts — skip */ }
            PlayableItem::Unknown(v) => { /* schema drift buffer (added for Feb-2026 migration) */
                if playable_is_unknown(&playable) { log::warn!("unparsed playlist item: {v}"); }
            }
        }
    }
    if page.next.is_none() { break; }              // pagination end condition (official example)
    offset += limit;
}
```

Alternative (recommended for throughput): auto-pagination stream —
`let stream = spotify.playlist_items(&playlist_id, None, None); pin_mut!(stream); while let Some(item) = stream.try_next().await? {...}`
(from https://github.com/ramsayleung/rspotify/blob/v0.16.1/examples/pagination_async.rs; requires `futures`).

**Field extraction cheat-sheet (rspotify-model @ 0.16.1, verified):**

| Need | Type/field | Notes |
|---|---|---|
| playlist_id | `PlaylistId` via `from_id`/`from_uri` | URL must be pre-parsed (see above) |
| track_id | `FullTrack.id: Option<TrackId<'static>>` | `None` for local/unavailable |
| title | `FullTrack.name: String` | |
| artists | `FullTrack.artists: Vec<SimplifiedArtist>` | `.name` each |
| album | `FullTrack.album: SimplifiedAlbum` | `.name`, `.id`, `.images` |
| track_number | `u32` | |
| disc_number | `i32` | |
| duration | `chrono::Duration` (`duration_ms` in JSON) | `.num_milliseconds()` |
| local flag | `FullTrack.is_local`, `PlaylistItem.is_local` | |
| unavailable | `FullTrack.restrictions: Option<Restriction>` (+ `is_playable: Option<bool>`) | |
| episodes | `PlayableItem::Episode(FullEpisode)` | skip for music sync |
| unknown items | `PlayableItem::Unknown(serde_json::Value)` + `is_unknown()` | forward-compat safety net |

Album lookup: `spotify.album(AlbumId::from_id(id)?, None)` → `FullAlbum` (fields `tracks` page via `album_track_manual`;
note Feb-2026 removed `available_markets`, `external_ids`, `label`, `popularity` from `FullAlbum` — rspotify 0.16 marks them deprecated/skipped).
Track lookup: `spotify.track(TrackId::from_id(id)?, None)` → `FullTrack`.

### 2.5 Rate limits & Client-Credentials visibility (VERIFIED sources)

- **Rate limit:** Spotify returns **429**; *"Spotify's API rate limit is calculated based on the number of calls that your
  app makes to Spotify in a rolling 30 second window... The limit varies depending on whether your app is in development mode
  or extended quota mode."* — https://developer.spotify.com/documentation/web-api/concepts/rate-limits (fetched 2026-10-26).
  Practical: client-credentials tokens are per-app; playlist paging (50/100 per call) is cheap; rustifydl's pattern of
  re-authenticating **per track** (its `metadata()` calls `ClientCredsClient::authenticate` every time) is a bad example — do it once.
- **Client Credentials can read public playlists?** — *mostly confirmed, with one live ambiguity:*
  - `GET /playlists/{playlist_id}` (playlist metadata incl. embedded first page of tracks) is declared
    `security: - oauth_2_0: []` (no scope) in Spotify's OpenAPI spec → any token, incl. client-credentials, works.
    (Spec: https://developer.spotify.com/reference/web-api/open-api-schema.yaml, fetched 2026-10-26.)
  - Real-world proof post-Feb-2026: `sptfydl` 0.3.10 (updated **2026-03-18**) reads playlists end-to-end with a
    **client-credentials token** — `GET https://api.spotify.com/v1/playlists/{id}` then follows `resp.tracks.next` pages
    (source: https://raw.githubusercontent.com/Trevrosa/sptfydl/main/src/spotify/search/playlist.rs).
  - ⚠️ Contradicting signal: the spec's `GET /playlists/{playlist_id}/items` (and deprecated `/tracks`) declare
    `security: oauth_2_0: [playlist-read-private]` and the docs text says *"only accessible for playlists owned by the
    current user or playlists the user is a collaborator of. A 403... if the user is neither"* — i.e. item-listing may be
    user-auth-only in the current API surface. rspotify docs themselves state Client Credentials *"cannot be used to access
    or to manage the endpoints related to user private data"* (source: `src/client_creds.rs` @ v0.16.1).
  - **Action:** Phase 2 must run a 10-line live test with real credentials (Get Playlist + one `next` page + `playlist_items_manual`).
    Fallback if 401/403: switch to **Authorization Code + PKCE** (`AuthCodeSpotify`, scope `playlist-read-private`) —
    rspotify supports both clients with the same `BaseClient` methods; only `request_token` vs `prompt_for_token` differs.

---

## 3. Tauri v2 (verified against docs.rs 2.12.5, npm, official docs)

### 3.1 Exact dependencies (latest **stable** versions as of 2026-10-26; crates.io `max_version` for these crates currently shows 3.0.0-alpha — ignore alphas)

| Crate | Version | npm package | Version |
|---|---|---|---|
| `tauri` | **2.12.5** (2026-10-21) | `@tauri-apps/api` | **2.12.4** |
| `tauri-build` | **2.7.4** (2026-10-21) | `@tauri-apps/cli` | **2.12.4** |
| `tauri-plugin-dialog` | **2.8.3** (2026-10-14) | `@tauri-apps/plugin-dialog` | **2.8.3** |
| `tauri-plugin-opener` | **2.7.2** | `@tauri-apps/plugin-opener` | **2.7.2** |
| `tauri-plugin-shell` | **2.5.0** (only if spawning sidecars from **JS**) | `@tauri-apps/plugin-shell` | **2.5.0** |
| `tauri-plugin-fs` | **2.7.1** (only if you need JS fs access) | `@tauri-apps/plugin-fs` | **2.7.1** |
| — | — | `vite` | **8.3.5** (create-tauri-app template pins `^8.0.16`) |

Keep `tauri`, `tauri-build` and `@tauri-apps/cli` on the same minor (official guidance:
https://v2.tauri.app/develop/configuration-files/). Dialog plugin requires **Rust ≥ 1.90** (local Rust **1.97.1** ✅).

### 3.2 Project layout (official: https://v2.tauri.app/start/project-structure/)

```
.
├── package.json
├── index.html
├── src/                  # frontend (JS/TS/Vite)
├── src-tauri/
│   ├── Cargo.toml        # [lib] name = "app_lib" crate-type = ["staticlib","cdylib","rlib"]
│   ├── build.rs          # tauri_build::build()
│   ├── tauri.conf.json   # v2 schema: "$schema": "https://schema.tauri.app/config/2"
│   ├── src/
│   │   ├── main.rs       # desktop entry: fn main() { app_lib::run() }  — don't edit
│   │   └── lib.rs        # #[cfg_attr(mobile, tauri::mobile_entry_point)] pub fn run()
│   ├── icons/
│   └── capabilities/default.json
```

> `src-tauri/src/lib.rs` holds the app (mobile builds compile the lib, not the bin).
> `main.rs` just calls `app_lib::run()` where `app_lib` = `[lib] name` in Cargo.toml (the `_lib` suffix avoids
> Windows bin/lib name conflicts — https://github.com/rust-lang/cargo/issues/8519).

### 3.3 Capability files (official: https://v2.tauri.app/security/capabilities/, https://v2.tauri.app/reference/acl/core-permissions/)

- JSON (or TOML) files in `src-tauri/capabilities/`; **all files in that dir are auto-enabled** unless
  `app.security.capabilities` explicitly lists some (then only those are used).
- `core:default` = `core:app:default` + `core:event:default` + `core:image:default` + `core:menu:default` +
  `core:path:default` + `core:resources:default` + `core:tray:default` + `core:webview:default` + `core:window:default`.
- **`core:event:default` = `allow-listen`, `allow-unlisten`, `allow-emit`, `allow-emit-to`** (verified verbatim).
- Dialog default permission set (`dialog:default`) = `allow-message`, `allow-save`, **`allow-open`**.
- **Custom commands need NO permission entry**: *"By default, all commands that you registered in your app (using
  `tauri::Builder::invoke_handler`) are allowed to be used by all the windows and webviews of the app."*
  (to restrict them you'd configure `tauri_build::AppManifest::commands`).

**Exact capability file for this app** (`src-tauri/capabilities/default.json`):

```json
{
  "$schema": "../gen/schemas/desktop-schema.json",
  "identifier": "default",
  "description": "Main window: core APIs + folder picker + messages",
  "windows": ["main"],
  "permissions": [
    "core:default",
    "dialog:default",
    "opener:default"
  ]
}
```

- folder picker ⇒ covered by `dialog:default` (or minimum `dialog:allow-open`)
- event emit/listen from JS ⇒ covered by `core:default` → `core:event:default`
- `invoke()` of your own commands ⇒ allowed by default (no entry needed)
- only if spawning sidecars **from JS** add (verified format: https://v2.tauri.app/develop/sidecar/):

```json
{
  "identifier": "shell:allow-execute",
  "allow": [ { "name": "binaries/yt-dlp", "sidecar": true, "args": true } ]
}
```

### 3.4 Rust-side events (docs.rs/tauri/2.12.5 + https://v2.tauri.app/develop/calling-frontend/)

```rust
use tauri::Emitter;   // trait must be in scope

// global emit (to all windows)
app.emit("download-progress", payload)?;                 // S: Serialize + Clone
// to one webview
app.emit_to("main", "download-progress", payload)?;
// filtered targets
app.emit_filter("open-file", path, |t| matches!(t, EventTarget::WebviewWindow{label} if label == "main"))?;
// WebviewWindow implements Emitter too: window.emit("evt", data)?
```

`Emitter` methods on 2.12.5 (verified): `emit`, `emit_str`, `emit_to`, `emit_str_to`, `emit_filter`, `emit_str_filter` —
all `Result<()>`, payload `Serialize + Clone` (payload is JSON-stringified; docs warn events are *not* for high throughput).

**`Channel<T>` alternative (recommended for ordered per-track progress):**

```rust
use tauri::ipc::Channel;
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase", tag = "event", content = "data")]
enum SyncEvent { Started { total: u32 }, TrackDone { id: String }, Finished { ok: u32, failed: u32 } }

#[tauri::command]
async fn start_sync(on_event: Channel<SyncEvent>, state: tauri::State<'_, AppState>) -> Result<(), AppError> {
    on_event.send(SyncEvent::Started { total: 10 })?;
    ...
}
```

```ts
import { invoke, Channel } from '@tauri-apps/api/core';
const onEvent = new Channel<SyncEvent>();
onEvent.onmessage = (m) => console.log(m.event, m.data);
await invoke('start_sync', { onEvent });
```

### 3.5 Frontend events + dialog (official docs)

```ts
import { listen, once } from '@tauri-apps/api/event';   // invoke lives in '@tauri-apps/api/core'
const unlisten = await listen<number>('download-progress', (e) => setProgress(e.payload));
unlisten();                       // always await listen() first — calling unlisten on the Promise is a no-op bug
import { open } from '@tauri-apps/plugin-dialog';
const dir = await open({ directory: true, multiple: false });   // → string path | null
```

### 3.6 Commands: async + State + error serialization (official: https://v2.tauri.app/develop/calling-rust/)

```rust
#[derive(Debug, thiserror::Error)]
enum AppError {
    #[error(transparent)] Io(#[from] std::io::Error),
    #[error("spotify: {0}")] Spotify(String),
}
impl serde::Serialize for AppError {           // errors must be Serialize — String is simplest, this is structured
    fn serialize<S: serde::ser::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

#[tauri::command]
async fn sync_playlist(
    window: tauri::WebviewWindow,                       // args can be injected
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,                  // from Builder.manage(...)
    url: String,                                        // JS: invoke('sync_playlist', { url }) — camelCase!
) -> Result<SyncSummary, AppError> { ... }
```

- args must be a JSON object with **camelCase** keys; returns & errors are JSON (`invoke(...).catch(e => ...)`)
- async commands are the norm; borrowed `&str` args need `Result<_, ()>` or use `String`
- register once: `.invoke_handler(tauri::generate_handler![cmd_a, cmd_b])` (only the last `invoke_handler` counts)

---

## 4. Scaffolding (ready to copy)

### 4.1 `package.json` (based on create-tauri-app vanilla-ts template @ dev, versions verified)

```json
{
  "name": "spotify-downloader",
  "private": true,
  "version": "0.1.0",
  "type": "module",
  "scripts": {
    "dev": "vite",
    "build": "tsc && vite build",
    "preview": "vite preview",
    "tauri": "tauri"
  },
  "dependencies": {
    "@tauri-apps/api": "^2.12.4",
    "@tauri-apps/plugin-dialog": "^2.8.3",
    "@tauri-apps/plugin-opener": "^2.7.2"
  },
  "devDependencies": {
    "@tauri-apps/cli": "^2.12.4",
    "vite": "^8.0.16",
    "typescript": "~6.0.3"
  }
}
```

Dev run: `npm install && npm run tauri dev`  (uses the **npm CLI** — no `cargo install tauri-cli` needed).
Build: `npm run tauri build`.

### 4.2 `vite.config.ts` (official template)

```ts
import { defineConfig } from "vite";
// @ts-expect-error type error without @types/node package
import process from "node:process";
const host = process.env.TAURI_DEV_HOST;

export default defineConfig({
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: "ws", host, port: 1421 } : undefined,
    watch: { ignored: ["**/src-tauri/**"] },
  },
});
```

### 4.3 File tree

```
spotify_downloader/
├── docs/phase1-research.md
├── package.json
├── tsconfig.json
├── vite.config.ts
├── index.html
├── src/                      # UI (TS)
│   ├── main.ts
│   └── styles.css
└── src-tauri/
    ├── Cargo.toml
    ├── build.rs
    ├── tauri.conf.json
    ├── capabilities/default.json
    ├── icons/…               # `npm run tauri icon path/to/icon.png`
    ├── binaries/             # (later) yt-dlp-x86_64-pc-windows-msvc.exe, ffmpeg-…
    └── src/
        ├── main.rs
        └── lib.rs
```

### 4.4 Minimal compiling Rust skeleton — `src-tauri/src/lib.rs`

```rust
use tauri::Emitter;

#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {name}!")
}

#[tauri::command]
async fn start_demo(app: tauri::AppHandle) -> Result<String, String> {
    app.emit("download-progress", 42).map_err(|e| e.to_string())?;
    Ok("started".into())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![greet, start_demo])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

`src-tauri/src/main.rs` (never edit):

```rust
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
fn main() { app_lib::run() }
```

`src-tauri/build.rs`: `fn main() { tauri_build::build() }`

### 4.5 `src-tauri/tauri.conf.json` (verbatim from create-tauri-app template, adapted)

```json
{
  "$schema": "https://schema.tauri.app/config/2",
  "productName": "spotify-downloader",
  "version": "0.1.0",
  "identifier": "com.example.spotifydownloader",
  "build": {
    "beforeDevCommand": "npm run dev",
    "devUrl": "http://localhost:1420",
    "beforeBuildCommand": "npm run build",
    "frontendDist": "../dist"
  },
  "app": {
    "windows": [{ "title": "Spotify Downloader", "width": 1100, "height": 760 }],
    "security": { "csp": null }
  },
  "bundle": {
    "active": true,
    "targets": "all",
    "icon": ["icons/32x32.png", "icons/128x128.png", "icons/128x128@2x.png", "icons/icon.ico"]
  }
}
```

`src-tauri/capabilities/default.json` — see §3.3.

---

## 5. External binaries: yt-dlp & FFmpeg on Windows

### 5.1 The three options

| Option | How | When to use | Pitfalls |
|---|---|---|---|
| **A. PATH (dev only)** | `winget install yt-dlp.yt-dlp` + `winget install Gyan.FFmpeg` (or `yt-dlp.FFmpeg` = yt-dlp's own FFmpeg builds — both verified present in winget-pkgs: `manifests/y/yt-dlp/{yt-dlp,FFmpeg}`, `manifests/g/Gyan/FFmpeg`) | Day-1 development; the machine has neither installed today | Users of the shipped app won't have them → **not** a release strategy; PATH resolution quirks |
| **B. Tauri `externalBin` sidecar (recommended for release)** | Put binaries at `src-tauri/binaries/<name>[-TARGET_TRIPLE][.exe]`, declare `"bundle": { "externalBin": ["binaries/yt-dlp", "binaries/ffmpeg"] }`, spawn via `tauri_plugin_shell::ShellExt::sidecar()` or resolve + `std::process::Command` | Bundled, offline-first desktop app | Naming (below); must add per-target files; JS spawning needs `shell:allow-execute` scope; ffmpeg licensing (ship an LGPL/GPL build; GPL build ⇒ compatible only with GPL app) |
| **C. Download at runtime** | rustifydl-style: `curl -L https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe -o …` + SHA-256 verify | Want always-fresh yt-dlp (YouTube breaks often) | Needs network on first run; AV flags unsigned downloads; `curl.exe` exists on Win10 1803+ but must not be shadowed; writing into `%APPDATA%` must be implemented carefully; rustifydl's checksum-line-index approach (`nth(2)`) is fragile |

**Recommended hybrid:** ship sidecar yt-dlp+ffmpeg (B) for first-run correctness **and** an explicit "Update yt-dlp"
action (C) since YouTube breakage happens between releases. Keep ffmpeg static and rarely updated.

### 5.2 Windows naming rules (official: https://v2.tauri.app/develop/sidecar/)

- Sidecars must exist as `src-tauri/binaries/<name>-<TARGET_TRIPLE><.exe>`:
  `binaries/yt-dlp-x86_64-pc-windows-msvc.exe`, `binaries/ffmpeg-x86_64-pc-windows-msvc.exe`.
  Get the triple with `rustc --print host-tuple` (→ `x86_64-pc-windows-msvc` on this machine).
- `bundle.externalBin` entries reference the **unsuffixed** path: `"binaries/yt-dlp"`.
- Spawning from Rust: `app.shell().sidecar("binaries/yt-dlp")?.args([...]).spawn()?`
  (or bypass the plugin: `tauri::Manager::path().resolve("binaries/yt-dlp", BaseDirectory::Resource)` + `std::process::Command` —
  no plugin/permission needed that way; `Command` + args array handles spaces in paths safely).
- Typical pitfalls: forgetting the triple suffix (build fails: "external binary ... not found"); building for a second
  arch without a matching binary; `@tauri-apps/cli` `tauri icon`/resources confusion; shell-plugin arg scopes must list
  args exactly (or `"args": true`).

### 5.3 yt-dlp runtime realities (verified 2026-10-26)

- yt-dlp needs **ffmpeg binary** (not the pip package) for `-x/--audio-format` post-processing; point it via PATH or
  `--ffmpeg-location PATH` (README: "Location of the ffmpeg binary; either the path to the binary or its containing directory").
- **NEW: yt-dlp-ejs + JS runtime is required for full YouTube support** — yt-dlp README (master, today):
  *"`yt-dlp-ejs` - Required for full YouTube support... A JavaScript runtime/engine like deno (recommended), node.js, bun, or QuickJS is also required to run yt-dlp-ejs."*
  → **Shipping bare `yt-dlp.exe` may silently degrade YouTube support.** Open risk: must test; options are bundling a JS
  runtime (deno/node — large), using yt-dlp builds that embed an engine, or supplying QuickJS. Validate in Phase 2.
- **Bot-check / 403 waves:** yt-dlp issues titled "Sign in to confirm you're not a bot" spiked Feb–Mar 2026
  (#15796, #15800, #15865, #16147, #16221 — all now **closed**, i.e. fixed by releases). Repo is extremely active
  (202,555★, last push **today**, Unlicense license). ⇒ **Keep yt-dlp auto-updatable** and expose cookie-override
  options (`--cookies-from-browser` / cookie file) in advanced settings; 403s are inevitable periodically.
- FFmpeg builds: https://github.com/yt-dlp/FFmpeg-Builds (what yt-dlp recommends).

---

## 6. Build/runtime prerequisites & pitfalls (accounting for LOCAL ENV)

| Item | Requirement | Local machine |
|---|---|---|
| Rust | stable ≥ 1.90 (dialog plugin floor); tauri 2.12 compiles on current stable | **1.97.1 ✅** |
| MSVC Build Tools | "Desktop development with C++" (official prerequisites) — needed to link, and by sidecars/QuickJS C | ❓ verify installed (`rustc -vV` host = msvc; run a hello-world build) |
| WebView2 Runtime | Evergreen (official: install "Evergreen Bootstrapper") | **v155 ✅** (preinstalled on Win11) |
| Node | LTS per docs (docs example: v20); create-tauri-app now targets Vite 8 / TS 6 | **24.16.0 ✅** (fine) |
| Tauri CLI | `npm i -D @tauri-apps/cli` (2.12.4) — no `cargo install cargo-tauri` needed | `cargo-tauri` **not installed** → use npm route ✅ |
| ffmpeg | **not installed** → `winget install Gyan.FFmpeg` or `winget install yt-dlp.FFmpeg` | ❌ install first |
| yt-dlp | **not installed** → `winget install yt-dlp.yt-dlp` | ❌ install first |
| VBSCRIPT Windows feature | needed only for MSI (`.msi`) bundles — light.exe failures | enable if bundling msi |
| LLVM/libclang | only if we (ill-advisedly) depend on `rustifydl` (rquickjs bindgen) | avoid — one reason to drop rustifydl |
| Linux (later) | `libwebkit2gtk-4.1-dev build-essential curl wget file libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev` + `rustup target add` | n/a today |

**Pitfalls checklist**

1. **Spaces / non-ASCII in paths** — dev dir is `C:\Users\User\Downloads\Emem\projects\spotify_downloader` (ASCII ✅).
   Avoid spaces in the *project* path (Vite/Tauri/MSVC edge cases). Non-ASCII output folders (e.g. `C:\Users\Юра\...`)
   are usually fine with `String`/`OsString` args but break naive `to_str()?` code — rustifydl literally errors on
   non-UTF-8 paths (`ok_or("Invalid UTF-8 in file path")`); use `OsStr`/`PathBuf` throughout. Always pass args via
   `Command::args([...])`, never a shell-interpolated string.
2. **Windows long paths** — `target/` + deep node_modules exceed `MAX_PATH` (260). Enable
   `LongPathsEnabled=1` (https://learn.microsoft.com/windows/win32/fileio/maximum-file-path-limitation) and/or keep the
   repo shallow; `cargo` and WebView2 devtools handle long paths only when the OS flag is on.
3. **Antivirus** — Defender/heuristics slow or quarantine fresh `target/debug` exes and unsigned downloaded `yt-dlp.exe`;
   add project dir exclusions during dev; sign release binaries or expect SmartScreen prompts.
4. **yt-dlp 403/bot-check** — see §5.3; design retry + cookie override + updater now, not later.
5. **ffmpeg location** — decide early: PATH (dev) vs `--ffmpeg-location <sidecar>` (release). Log the resolved path in
   diagnostics; "file not found" from yt-dlp post-processing almost always = missing ffmpeg.
6. **Vite port** — template hard-pins 1420 with `strictPort: true`; free ports or expect failure.
7. **Logger clash** — if using `tauri-plugin-log`, never call crates that re-init the global logger (rustifydl does).
8. **Event listener timing** — `await listen(...)` before storing unlisten (official warning).
9. **CWD** — GUI apps have CWD ≠ exe dir; resolve resources via `app.path()` not relative paths (rustifydl writes `./checksums` — don't copy that pattern).
10. **Spotify credentials** — user must supply their own client_id/secret (create app at developer.spotify.com); first
    verification task in Phase 2 is the playlist-read test from §2.5.

---

## 7. Ready-to-copy `src-tauri/Cargo.toml` dependency block (verified versions)

```toml
[package]
name = "spotify-downloader"          # lib name set below as app_lib
version = "0.1.0"
edition = "2024"                     # create-tauri-app template uses edition 2024

[lib]
name = "app_lib"                     # must match main.rs's app_lib::run()
crate-type = ["staticlib", "cdylib", "rlib"]

[build-dependencies]
tauri-build = { version = "2.7.4", features = [] }

[dependencies]
# --- Tauri core (keep tauri / tauri-build / @tauri-apps/cli on same minor) ---
tauri = { version = "2.12.5", features = [] }
tauri-plugin-dialog = "2.8.3"        # folder picker (open({directory:true}))
tauri-plugin-opener = "2.7.2"        # reveal/open downloaded folders

# --- serde ---
serde = { version = "1.0.229", features = ["derive"] }
serde_json = "1.0.154"

# --- async runtime ---
tokio = { version = "1.54.1", features = ["rt-multi-thread", "macros", "process", "fs", "time", "sync"] }

# --- Spotify (Client Credentials; default features = client-reqwest + reqwest-default-tls) ---
# For rustls everywhere instead: default-features = false, features = ["client-reqwest", "reqwest-rustls-tls"]
rspotify = "0.16.1"

# --- HTTP (0.12 line to match rspotify-http's reqwest 0.12; use for cover art, etc.) ---
reqwest = { version = "0.12.28", default-features = false, features = ["rustls-tls", "json", "stream"] }

# --- tagging (MP3 ID3 / FLAC / M4A) — 0.25.5 is current; rustifydl's 0.22 is stale ---
lofty = "0.25.5"

# --- misc ---
regex = "1.13.1"                     # Spotify URL -> id parsing
thiserror = "2.0.21"                 # typed command errors (serialize via impl Serialize)
anyhow = "1.0.104"                   # internal error plumbing (not across invoke boundary)
log = "0.4.29"
futures = "0.3.34"                   # rspotify Paginator stream (pin_mut/try_next)
chrono = "0.4.45"                    # track.duration is chrono::Duration

# --- OPTIONAL (add only if chosen) ---
# rustypipe = "0.11.4"               # YouTube/YouTube Music Innertube search client (rustifydl uses it)
# tauri-plugin-shell = "2.5.0"       # only if spawning sidecars from JS
# tauri-plugin-log = "…"             # only if you want file logs (do NOT combine with rustifydl)

[profile.release]
codegen-units = 1
lto = true
opt-level = 3
panic = "abort"
strip = true
```

Notes:
- **Do not add `rustifydl`** (verdict §1) and **do not add `id3` + `lofty` both** (lofty handles ID3v2.3 itself → `id3 1.17.2` is a fallback only).
- No `tauri-plugin-fs` needed for a first version (Rust side does all file I/O; JS only needs the dialog picker).
- `yt-dlp` is spawned as an external binary — no Rust yt-dlp crate required.

### Verified facts table (claim | source | date)

| Claim | Source | Date |
|---|---|---|
| rustifydl exists, latest 0.2.41, GPL-3.0, repo ThePangel/RustifyDL | https://crates.io/api/v1/crates/rustifydl | 2026-10-26 |
| rustifydl public API: `DownloadOptions` + `download_spotify(opts, ytdlp_dir)` async, `Box<dyn Error>` | https://raw.githubusercontent.com/ThePangel/RustifyDL/main/src/lib.rs | 2026-10-26 |
| rustifydl pipeline: spotify-rs + rustypipe + yt-dlp (curl-downloaded) + ffmpeg(PATH) + lofty | repo `src/{spotify,youtube,metadata}.rs` | 2026-10-26 |
| rustifydl last push 2026-01-29, 9★, ~20 recent downloads | GitHub API / crates.io | 2026-10-26 |
| rustifydl has unused `rquickjs(bindgen)` dep and no build.rs (bindgen build-dep unused) | crates.io deps API + repo root listing + source grep | 2026-10-26 |
| rspotify latest 0.16.1 (2026-04-25), MIT | https://crates.io/api/v1/crates/rspotify | 2026-10-26 |
| rspotify default features = client-reqwest + reqwest-default-tls; no special feature for client credentials | Cargo.toml @ tag v0.16.1 | 2026-10-26 |
| ClientCredsSpotify::new + request_token; playlist/track/album + `_manual` pagination signatures | src/client_creds.rs, src/clients/base.rs @ v0.16.1 | 2026-10-26 |
| PlaylistItem.item/track Option + PlayableItem::{Track,Episode,Unknown} + FullTrack Option fields | rspotify-model @ v0.16.1 | 2026-10-26 |
| Id types accept `spotify:…` only — not open.spotify.com URLs | rspotify-model/src/idtypes.rs (`parse_uri`) | 2026-10-26 |
| Spotify Feb-2026 API migration (removed endpoints, field renames); rspotify issue #550 | https://github.com/ramsayleung/rspotify/issues/550 | 2026-10-26 |
| Spotify rate limit = rolling 30 s window, HTTP 429; dev-mode quota separate | https://developer.spotify.com/documentation/web-api/concepts/rate-limits | 2026-10-26 |
| GET /playlists/{id} declared `oauth_2_0: []`; GET /playlists/{id}/items declared scope `playlist-read-private` + owner/collaborator note | https://developer.spotify.com/reference/web-api/open-api-schema.yaml | 2026-10-26 |
| Client-credentials can page playlist tracks post-migration (real-world) | sptfydl 0.3.10 src/spotify/search/playlist.rs (updated 2026-03-18) | 2026-10-26 |
| tauri 2.12.5 / tauri-build 2.7.4 / plugin-dialog 2.8.3 / opener 2.7.2 / shell 2.5.0 / fs 2.7.1 (latest stable, alphas exist for v3) | crates.io versions API | 2026-10-26 |
| @tauri-apps/api 2.12.4, cli 2.12.4, plugin-dialog 2.8.3, vite 8.3.5 | registry.npmjs.org `/latest` | 2026-10-26 |
| capability file format, core:default contents, core:event:default = listen/unlisten/emit/emit-to, custom commands allowed by default | https://v2.tauri.app/security/capabilities/ + /reference/acl/core-permissions/ | 2026-10-26 |
| dialog default = allow-message/save/open; `open({directory:true})` | https://v2.tauri.app/plugin/dialog/ | 2026-10-26 |
| Emitter trait methods on tauri 2.12.5 | https://docs.rs/tauri/2.12.5/tauri/trait.Emitter.html | 2026-10-26 |
| Commands/State/error-Serialize/Channel/listen patterns | https://v2.tauri.app/develop/calling-rust/ + /develop/calling-frontend/ | 2026-10-26 |
| sidecar naming `name-<target-triple>.exe`, externalBin config, shell:allow-execute scope | https://v2.tauri.app/develop/sidecar/ | 2026-10-26 |
| Project layout lib.rs/main.rs + cfg_attr(mobile) + capabilities dir | https://v2.tauri.app/start/project-structure/ | 2026-10-26 |
| Windows prereqs: MSVC C++ Build Tools, WebView2 Evergreen, VBSCRIPT for MSI; Node LTS; dialog plugin needs Rust ≥1.90 | https://v2.tauri.app/start/prerequisites/ + /plugin/dialog/ | 2026-10-26 |
| tauri.conf.json v2 schema URL + config template contents | create-tauri-app templates @ dev (raw GitHub) | 2026-10-26 |
| yt-dlp requires ffmpeg binary + **yt-dlp-ejs with a JS runtime for full YouTube support**; `--ffmpeg-location` | https://raw.githubusercontent.com/yt-dlp/yt-dlp/master/README.md | 2026-10-26 |
| yt-dlp bot-check issues Feb–Mar 2026 all closed; repo very active (Unlicense) | GitHub search API on yt-dlp/yt-dlp | 2026-10-26 |
| winget packages `yt-dlp.yt-dlp`, `yt-dlp.FFmpeg`, `Gyan.FFmpeg` exist | microsoft/winget-pkgs manifests listing | 2026-10-26 |
| Verified crate versions: tokio 1.54.1, reqwest 0.13.5 (0.12 line used by rspotify/rustifydl), serde 1.0.229, serde_json 1.0.154, thiserror 2.0.21, anyhow 1.0.104, lofty 0.25.5, id3 1.17.2, regex 1.13.1, futures 0.3.34, chrono 0.4.45 | crates.io API | 2026-10-26 |
| rustypipe 0.11.4 (2025-04-23, codeberg.org/ThetaDev/rustypipe), spotify-rs 0.4.1 (2025-06-25), yt-dlp crate 2.9.1 (2026-10-26), sptfydl 0.3.10 MIT (2026-03-18) | crates.io API | 2026-10-26 |

### Open risks (ranked)

1. **Spotify playlist-items auth ambiguity (HIGHEST).** Spec + docs text suggest `GET /playlists/{id}/items` may be
   user-auth-only (`playlist-read-private`) post-Feb-2026, while Get-Playlist's embedded paging + sptfydl prove
   client-credentials reading works at least via `GET /playlists/{id}` + `next`. **Phase 2 gate #1:** live test with real
   credentials; fallback plan = Authorization Code + PKCE (rspotify `AuthCodeSpotify`) — same BaseClient methods.
2. **yt-dlp now needs a JS runtime (yt-dlp-ejs) for full YouTube support** — bundling strategy (§5) must be validated on a
   clean Windows VM; may force bundling deno/node/QuickJS or accepting degraded extraction.
3. **YouTube bot-check/403 waves** are recurring (Feb–Mar 2026 incident); need updater + cookies escape hatch + clear UX for failures.
4. **rustifydl licensing/build** — decided against; but its README/`cargo install rustifydl` remains a footgun for contributors (LLVM needed).
5. **Spotify rate limits/dev-mode quotas** — an app shipped to users with shared client credentials may hit extended-quota
   review; plan request budgeting (one auth, paged reads, cache metadata).
6. **GPL contamination risk** if anyone later "just calls rustifydl" — keep it out of Cargo.toml.
7. **Dual reqwest majors** if a dep later bumps (0.13) — pin direct `reqwest` to 0.12 line while rspotify uses 0.12.
8. **Mobile (Android) path later:** dialog plugin has **no folder picker on Android/iOS** (official docs) — folder
   layout UI must have a platform abstraction from day one.
9. **Windows long paths + AV** on dev machines — enable `LongPathsEnabled`, exclude build dirs early.
10. **Unverified locally:** nothing was compiled in Phase 1 (greenfield, read-only research); MSVC Build Tools presence
    on this machine not yet confirmed — first `npm run tauri dev` will prove it.
