# Phase 4 — Backend Implementation Contract

Status: **authoritative for implementation**. Where this document and the longer
research docs disagree, **this document wins** (it encodes the approved scope cuts).

> **Post-implementation deltas (approved during Phase 4).** The sections below describe the
> original plan; the shipped code differs in these places:
>
> 1. **Frontend was edited**, despite the "frozen" note: the account/settings modal is
>    session-only (no Client ID, secret, redirect URI, or source badge), the preview/demo
>    chrome is gone, the footer is `© 2026 d3khan`, and filenames are `Title - Artist`.
> 2. **Tagging is richer than §ID3 tagging**: a full ID3v2.3 set is written (title, artist,
>    album artist, album, track/total, disc, release date, genres, ISRC, label, copyright,
>    explicit flag, Spotify URL comment, front-cover APIC). ID3v2.3 is deliberate — Windows
>    Explorer renders APIC art from v2.3 as the file thumbnail but often shows a generic icon
>    for v2.4. Tags are written to the temp file **before** the rename.
> 3. **Playlist metadata** must not call `client.playlist()`; it panics when Spotify omits the
>    `tracks`/`items` object. Use `playlist_items_manual` for tracks plus a lenient reqwest
>    summary call for name/cover.
> 4. **Album metadata**: `/v1/albums?ids=` returns 403 for this app, so enrichment falls back
>    to per-album `client.album()` lookups.
> 5. **Binaries ship with the installer** through `bundle.resources`; they are not a
>    user-supplied prerequisite.
> 6. **Search fallbacks**: YouTube returns zero hits for some queries (verified with
>    `Rustage - FUCK AI`, where the word is silently filtered), and yt-dlp exits 0 with no
>    file. `download_track` therefore tries `artist - title`, `artist title`, and the same
>    pair with filtered words removed before reporting failure.

Read alongside:
- `docs/phase1-research.md` — verified crate versions, rspotify/Tauri APIs
- `docs/phase1b-auth-research.md` — PKCE loopback flow, keyring, static frontend
- `docs/phase2-architecture.md` — detailed sync rationale (implement the simplified version below)

---

## 0. Non-negotiables

1. **No `rustifydl`.** The pipeline is `rspotify` → `yt-dlp` → `FFmpeg` → `lofty`.
2. **Frontend is frozen** (`src/index.html`, `styles.css`, `app.js`). Do not change it.
   If the contract cannot be met, report the mismatch instead of editing the UI.
3. Windows first. Avoid Unix-only APIs; use `#[cfg(windows)]` for reparse-point checks.
4. Build with the pinned MSVC toolchain (`rust-toolchain.toml`). `cargo check` must pass.
5. No DRM bypassing. Audio comes only from `yt-dlp`/public sources. Spotify is metadata only.
6. Never hardcode or log the client secret or tokens. `.env` is gitignored.

---

## 1. Files to create / replace

```
src-tauri/src/
├── main.rs          # already exists — leave it (calls app_lib::run())
├── lib.rs           # replace stub: AppState init, plugins, invoke_handler, run()
├── commands.rs      # Tauri commands + DTOs
├── auth.rs          # PKCE client, loopback callback, keyring, settings
├── metadata.rs      # Spotify URL parsing + rspotify fetching -> TrackMeta/Collection
├── naming.rs        # sanitize, folder/file names, case-insensitive collision resolution
├── sync.rs          # .playlist_sync.json structs, atomic IO, diff, nested-folder scan
├── downloader.rs    # yt-dlp search/download, ffmpeg conversion, lofty ID3 tagging
└── jobs.rs          # job runner, events, deletion/conflict prompts, cancellation
```

Do **not** add a workspace or extra crates. `Cargo.toml` is already pinned and fetched.

---

## 2. Shared types

```rust
// metadata.rs
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CollectionKind { Track, Album, Playlist }

#[derive(Clone, Debug)]
pub struct TrackMeta {
    pub id: String,
    pub title: String,
    pub artists: Vec<String>,      // display join: artists.join(", ")
    pub album: String,
    pub album_id: Option<String>,
    pub track_number: u32,
    pub disc_number: u32,
    pub duration_ms: u64,
    pub cover_url: Option<String>,
    pub position: usize,           // 1-based position for playlists; 0 otherwise
}

#[derive(Clone, Debug)]
pub struct Collection {
    pub kind: CollectionKind,
    pub id: String,
    pub name: String,
    pub cover_url: Option<String>,
    pub tracks: Vec<TrackMeta>,
    pub source_url: String,
}
```

`AppState` (in `lib.rs`):

```rust
pub struct AppState {
    pub auth: tokio::sync::Mutex<Option<rspotify::AuthCodePkceSpotify>>,
    pub auth_cancel: std::sync::atomic::AtomicBool,
    pub auth_waiting: std::sync::atomic::AtomicBool,
    pub display_name: std::sync::Mutex<Option<String>>,
    pub settings: std::sync::Mutex<Settings>,          // client id, source, paths
    pub settings_path: std::path::PathBuf,
    pub running: std::sync::atomic::AtomicBool,
    pub job: std::sync::Mutex<Option<JobControl>>,
    pub pending_deletions: std::sync::Mutex<
        std::collections::HashMap<String, tokio::sync::oneshot::Sender<String>>>,
    pub pending_conflicts: std::sync::Mutex<
        std::collections::HashMap<String, tokio::sync::oneshot::Sender<ConflictResolution>>>,
}

pub struct JobControl { pub job_id: String, pub cancel: std::sync::Arc<std::sync::atomic::AtomicBool> }
pub enum ConflictResolution { SyncHere(std::path::PathBuf), CreateNew, Cancel }
```

`AppState` must be `Send + Sync`. Initialize it in `.setup()` (needs `app.path().app_config_dir()`),
then `app.manage(state)`.

---

## 3. Tauri commands — exact names and signatures

All return `Result<T, String>` (map internal errors to short user-facing strings).

| Command | Rust params (JS keys are camelCase) | Returns |
|---|---|---|
| `pick_folder` | `app: AppHandle` | `Option<String>` |
| `auth_status` | `state` | `AuthStatus` |
| `auth_connect` | `app`, `state` | `AuthStatus` |
| `auth_cancel` | `state` | `AuthStatus` |
| `auth_logout` | `state` | `AuthStatus` |
| `get_settings` | `state` | `SettingsView` |
| `save_settings` | `client_id: String`, `client_secret: Option<String>`, `state` | `SettingsView` |
| `start_job` | `app`, `state`, `mode: String`, `input_mode: String`, `url: Option<String>`, `urls: Option<Vec<String>>`, `output_dir: String` | `String` (job id) |
| `cancel_job` | `job_id: String`, `state` | `()` |
| `confirm_deletions` | `request_id: String`, `decision: String`, `state` | `()` |
| `resolve_conflict` | `request_id: String`, `choice: String`, `dir: Option<String>`, `state` | `()` |

Use `#[tauri::command(rename_all = "camelCase")]` on commands with multi-word params.
The frontend invokes `start_job` with flat keys `{ mode, inputMode, url, urls, outputDir }`
(not wrapped in `args`).

`pick_folder`: `app.dialog().file().blocking_pick_folder()`; convert the returned path to `String`.
Async command is fine (runs off the main thread). Do not spawn a nested dialog thread.

### DTOs (serde `rename_all = "camelCase"`)

```rust
pub struct AuthStatus {
    pub authenticated: bool,
    pub configured: bool,
    pub display_name: Option<String>,
    pub redirect_uri: String,
    pub error: Option<String>,
}

pub struct SettingsView {
    pub client_id: Option<String>,
    pub has_secret: bool,
    pub redirect_uri: String,
    pub source: String,              // "settings" | "env" | "none"
}
```

`save_settings` semantics:
- `client_id` empty ⇒ **clear saved settings**: remove settings file values, delete stored token +
  optional secret from keyring, drop in-memory auth, return source `env`/`none`.
- New `client_id` different from the active one ⇒ delete stored token (tokens are per client).
- `client_secret: None` or empty ⇒ keep any existing stored secret.
- Write only `{ "client_id": "..." }` to `settings.json` (secret goes to keyring, optional).
- Emit `auth://status` after changes.

---

## 4. Tauri events — exact names/payloads

Payload structs use `#[serde(rename_all = "camelCase")]`. Emit with `app.emit(...)` (`use tauri::Emitter`).

```jsonc
// download://progress  (percent is 0-100 or null; current/total are track counts)
{ "jobId": "...", "phase": "downloading", "current": 3, "total": 12,
  "percent": 21, "title": "Daft Punk - Get Lucky", "message": "Downloading audio" }

// download://log
{ "jobId": "...", "level": "info|warn|error|success",
  "message": "...", "timestamp": "2026-10-27T12:00:00+00:00" }

// download://track_failed
{ "jobId": "...", "title": "...", "artist": "...", "error": "..." }

// sync://removed_tracks
{ "jobId": "...", "requestId": "...", "folder": "C:\\...",
  "playlistName": "...", "tracks": [
    { "trackId": "...", "title": "...", "artist": "...", "localFilename": "03 - ...mp3" }
  ], "timeoutMs": 600000 }

// sync://conflict
{ "jobId": "...", "requestId": "...", "playlistName": "...", "playlistId": "...",
  "targetDir": "C:\\...", "matches": [
    { "dir": "C:\\...", "playlistName": "...", "trackCount": 47,
      "updatedAt": "2026-09-01T10:00:00Z" }
  ], "timeoutMs": 600000 }

// sync://complete  (used for download jobs too)
{ "jobId": "...", "status": "completed|cancelled|failed",
  "downloaded": 0, "skipped": 0, "failed": 0, "removed": 0, "message": "..." }
```

- Log timestamps: `chrono::Utc::now().to_rfc3339()`.
- **Job start race:** the frontend sets `state.jobId` only after `start_job` returns.
  Sleep ~200 ms at the top of the spawned job before emitting the first event.
- Emit `sync://complete` exactly once on every exit path (including cancel/fatal).
- Also emit `auth://status` with `AuthStatus` on auth transitions.

---

## 5. Auth (PKCE)

Reference: `docs/phase1b-auth-research.md` §1, §2, §5.

- Redirect URI: env `SPOTIFY_REDIRECT_URI`, default `http://127.0.0.1:8888/callback`.
  Parse the port for the listener (default 8888).
- Scopes: `playlist-read-private`, `playlist-read-collaborative`, `user-read-private`, `user-read-email`.
- Build:
  ```rust
  let creds = Credentials::new_pkce(&client_id);
  let oauth = OAuth { redirect_uri, scopes: scopes!(...), ..Default::default() };
  let config = Config { token_cached: false, token_refreshing: true, ..Default::default() };
  let mut client = AuthCodePkceSpotify::with_config(creds, oauth, config);
  ```
- `let url = client.get_authorize_url(None)?;` (must be called on the same instance used for
  `request_token`; verifier lives in `client.verifier`). Capture `client.oauth.state`.
- **Bind `127.0.0.1:<port>` before opening the browser** so a busy port fails fast. Use a blocking
  `std::net::TcpListener` inside `spawn_blocking`, nonblocking accept loop with 100 ms sleep,
  3-minute deadline, and an `AtomicBool` cancel flag. Handle `/callback`; ignore favicon with 404.
  Parse `code`, `state`, `error`; verify state; respond with a small HTML success page.
- `client.request_token(&code).await?;` then store in `state.auth`, persist token, set display name
  via `current_user()` (best-effort), emit `auth://status`.
- `auth_cancel` sets the flag; `auth_connect` returns `Err("Sign-in cancelled")` within ~200 ms.
- Persist **full `Token` JSON** to Windows Credential Manager:
  `keyring::Entry::new("spotify_downloader", "spotify_token")`; `set_password` / `get_password`;
  missing = `keyring::Error::NoEntry`. Service constant `spotify_downloader`.
- Restore at startup: read token, `AuthCodePkceSpotify::from_token_with_config(token, creds, oauth, config)`.
  Keep `token_cached: false` (never write `.spotify_token_cache.json`).
- Auto-refresh is enabled. **PKCE refresh tokens rotate**, so call a `persist_token(&client)` helper
  after `request_token` and after every metadata fetch/job (read `client.token.lock().await`).
  If `Config::token_callback_fn` is easy to wire, use it too; do not block on it.
- Logout: `delete_credential` (ignore `NoEntry`), drop client, clear display name, emit disconnected.
- Refresh tokens for Dashboard apps expire after ~6 months; surface `error` on auth failures so the
  UI can ask the user to reconnect.

### Settings / env
- Settings path: `app.path().app_config_dir()?.join("settings.json")`.
- Env precedence: saved settings > `.env` > process env.
- Load `.env` explicitly (CWD under `tauri dev` is `src-tauri/`):
  try `.env`, `../.env`, `../../.env`, and `CARGO_MANIFEST_DIR/../.env`.
- Keys: `SPOTIFY_CLIENT_ID` (fallback `RSPOTIFY_CLIENT_ID`), `SPOTIFY_CLIENT_SECRET`
  (fallback `RSPOTIFY_CLIENT_SECRET`), `SPOTIFY_REDIRECT_URI`.
- Secret is not used by PKCE; keep it optional and never return it to the frontend.

---

## 6. Metadata (rspotify 0.16.1)

- Parse URLs with regex `(track|album|playlist)/([A-Za-z0-9]{22})` and
  `spotify:(track|album|playlist):([A-Za-z0-9]{22})`. Reject artist URLs.
- Fetch with the shared client (lock only for the duration of the network calls):
  - Playlist: `spotify.playlist(id, None, None)` for name/cover; `playlist_items_manual(id, None, None, Some(100), Some(offset))`
    in a loop until `page.next.is_none()`. For each item use `item.item` (`Option<PlayableItem>`);
    skip `Episode`, `Unknown`, `is_local`, and tracks with no `id`. `PlayableItem::Track(t)`.
    `t.duration` is `chrono::Duration`; use `.num_milliseconds().max(0) as u64`.
    Set `position` = 1-based index among accepted music tracks.
  - Album: `spotify.album(id, None, None)` + `album_track_manual(id, None, Some(50), Some(offset))`
    until `page.next.is_none()`. Use `track_number`, `disc_number`, `name`, `artists`,
    `duration`. `position = 0`.
  - Track: `spotify.track(id, None, None)`; `position = 0`.
- `FullTrack.id` is `Option<TrackId>`; IDs via `id.id()`.
- Artists: join all names with `", "` for display; keep Vec.
- Cover: first album/playlist image URL (best-effort, may be unused).
- After the fetch, call `persist_token` (refresh may have rotated).
- On `ClientError`, return a short message including the Spotify error text.

---

## 7. Naming & folder layout

Implement in `naming.rs`; pure functions, unit-testable.

**Sanitize** (order):
1. NFC normalize (optional; if no crate, skip).
2. Replace `< > : " / \ | ? *` and control chars with `_`.
3. Collapse whitespace, trim.
4. Truncate to `max_chars` chars (char boundaries), trim again, strip trailing `.` and spaces.
5. Empty ⇒ `"Untitled"`.
6. Reserved device names (`CON`, `PRN`, `AUX`, `NUL`, `COM1..9`, `LPT1..9`) on the part before the
   first `.` ⇒ append `_`.
7. Leading `.` ⇒ prefix `_`.

**Folders / files**
- Playlist folder: `sanitize(playlist_name, 64)`.
- Album folder: `sanitize("{artist_display} - {album}", 64)`.
- Single track file: `sanitize("{artist} - {title}", 120) + ".mp3"`.
- Playlist track: `"{pos:0W} - {artist} - {title}.mp3"`, `W = max(2, digits(total_tracks))`,
  `pos` = 1-based position among accepted tracks. Sanitize the artist/title pieces; keep the prefix.
- Album track: `"{track:02} - {artist} - {title}.mp3"`; if the album has >1 disc,
  `"{disc:02}-{track:02} - {artist} - {title}.mp3"`.
- Folder creation: `create_dir_all`.
- **Case-insensitive collision check** on every directory: read_dir into a lowercase map; if the
  desired name exists, insert `" (n)"` before `.mp3` (n = 1, 2, …; guard 500).
- Sync must **not** renumber existing files: reuse `state.entry.local_filename`.
- If the state file references a file that exists with different casing, adopt the on-disk name
  (update state) instead of redownloading.
- Long paths: Windows long paths are enabled; do not add `\\?\` complexity in v1.

**Single vs bulk**
- Single link: output = chosen folder. Track ⇒ file directly there. Album/playlist ⇒ subfolder.
- Bulk: create a master folder **inside** the chosen folder:
  `Bulk Download YYYY-MM-DD HH-MM-SS` (sanitize; collide-suffix if needed).
  Loose tracks go in the master; albums/playlists get subfolders inside the master.
  Bulk + Sync is rejected (frontend already disables it).
- The scan root for nested-folder prevention is the chosen output folder (single) or the master
  (bulk).

---

## 8. Playlist sync state

`.playlist_sync.json` inside the playlist folder. Required by spec plus small safe extras.

```rust
pub const STATE_FILENAME: &str = ".playlist_sync.json";

#[derive(Serialize, Deserialize, Clone)]
pub struct SyncState {
    #[serde(default = "default_schema_version")] pub schema_version: u32,
    pub playlist_id: String,
    #[serde(default)] pub playlist_name: String,
    #[serde(default)] pub source_url: String,
    #[serde(default)] pub updated_at: Option<String>,
    #[serde(default)] pub entries: Vec<SyncEntry>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct SyncEntry {
    pub track_id: String,
    pub local_filename: String,          // bare filename only
    pub title: String,
    pub artist: String,
    #[serde(default)] pub status: EntryStatus,   // Ok | KeptRemoved
}
```

- Read: missing file ⇒ empty/fresh. Parse error ⇒ rename to `.playlist_sync.json.corrupt-<ts>`,
  log warn, start fresh (never overwrite silently).
- Write atomically: serialize pretty to `.<name>.<uuid>.tmp` in the same dir, `sync_all`, rename
  (retry os error 32/33 a few times), remove tmp on failure.
- Update after **each successful track download** (and after a keep/delete decision).
- Status `kept_removed`: user chose Keep for a track removed from Spotify; never prompt again.
  If the track reappears in the playlist, reset to `ok`.

**Diff**
- Build a case-insensitive map of files in the playlist folder once.
- Match fresh tracks to state entries by `track_id`, k-th occurrence for duplicate IDs
  (first unused entry per fresh occurrence). Do **not** renumber filenames.
- Per matched entry:
  - file exists and size > 0 ⇒ skip; if `status == kept_removed`, mark ok.
  - file missing/unreadable ⇒ redownload using the same `local_filename`.
- Unmatched fresh track ⇒ new download with a collision-safe filename.
- Leftover entries:
  - `kept_removed` ⇒ keep silently (count as skipped).
  - `ok` ⇒ **removed from Spotify** ⇒ prompt.
- Untracked files ⇒ report in log only; never delete/adopt.
- Execution order: prompt removals **before** downloads (matches UI); then downloads; then deletions
  (delete file if present, remove entry, write state after each). If the user chooses Keep, set
  `kept_removed` and write once.
- On any per-track failure, log/emit and continue. Leave the state entry absent (new) or unchanged
  (redownload) so the next run retries.

**Prompt**
- Emit `sync://removed_tracks` with a UUID `request_id`, wait on `oneshot` up to 600 s.
  - `"delete"` ⇒ delete + remove entries; `"keep"` ⇒ mark kept_removed.
  - timeout ⇒ Keep; cancel/dropped sender ⇒ job cancelled.
- `confirm_deletions` removes the sender from the map atomically (double-click safe).

**Nested-folder scan**
- Before creating a playlist folder, scan the scan root recursively (max depth 6) for
  `.playlist_sync.json` with the same `playlist_id`, excluding:
  - hidden dirs (name starts `.`), `$RECYCLE.BIN`, `System Volume Information`
  - reparse points/junctions on Windows (`file_attributes() & 0x400`), symlinks elsewhere
  - the target folder itself
- Parse only `playlist_id`, `playlist_name`, `entries.len()`, `updated_at` (tolerate unknown fields).
- If the target folder already has matching state ⇒ normal in-place sync, no prompt.
- Otherwise, if matches exist ⇒ emit `sync://conflict`, wait up to 600 s:
  - `"syncHere"` + `dir` ⇒ use that folder.
  - `"createNew"` ⇒ create with a unique suffix.
  - `"cancel"` ⇒ skip this URL (job continues in bulk; single ⇒ cancelled).
  - timeout ⇒ cancel.
- `resolve_conflict` validates `dir` is one of the offered matches.

---

## 9. Download pipeline (`downloader.rs`)

**Binary discovery** (return a clear error if missing):
- `yt-dlp`: env `YTDLP_PATH`; `<exe_dir>/binaries/yt-dlp.exe`; `<cwd>/binaries/yt-dlp.exe`;
  `<cwd>/../src-tauri/binaries/yt-dlp.exe`; `src-tauri/binaries/yt-dlp.exe` relative to project;
  finally `yt-dlp` on PATH (probe `--version`).
- `ffmpeg`: env `FFMPEG_PATH`; `<exe_dir>/binaries/ffmpeg.exe`; `src-tauri/binaries/ffmpeg.exe`;
  finally `ffmpeg` on PATH. Pass `--ffmpeg-location <dir>` when a path is known.
- Do this once per job (or cache with `OnceLock`). Error text:
  `"yt-dlp not found. Install it (winget install yt-dlp.yt-dlp) or place yt-dlp.exe in src-tauri/binaries/."`

**Per-track download**
1. Query `ytsearch1:{artist_display} - {title}` (one arg).
2. Temp template in the destination dir: `.{uuid}.%(ext)s`.
3. Args:
   ```
   --no-playlist --newline --no-warnings --ignore-config
   -f bestaudio/best -x --audio-format mp3 --audio-quality 0
   [--ffmpeg-location <dir>] -o <temp_template> <query>
   ```
4. Stream stdout lines for `[download]  NN.N%`; call the progress callback (per-track 0..100).
5. On success find the produced `.mp3` with the temp prefix; rename onto the final path
   (remove a stale final file first if overwriting a redownload). Clean up temp/`.part` files.
6. On failure capture the last ~20 stderr lines, clean temp files, return a typed error.
7. Retry once (2 s delay) for transient patterns: `403`, `429`, `timed out`, `Connection reset`,
   `Unable to download`, `Temporary failure`.
8. Timeout: 10 minutes per track; kill the child.
9. Cancellation: poll the job `AtomicBool` every 250 ms; kill the child, clean up, return cancelled.

**ID3 tagging** (after rename):
- `lofty::tag::Tag::new(TagType::Id3v2)`; set title, artist (display string), album, track, disc.
- `tag.save_to_path(&path, WriteOptions::default())`.
- Cover art is **optional** in v1; do not fail the track if it cannot be embedded. Text tags are
  required. If cover embedding is quick to add, fetch the `cover_url` with reqwest and attach it;
  otherwise skip and leave a TODO.
- Verify the final file exists and size > 0.

**Output contract**: `download_track(meta, final_path, dest_dir, cancel, on_progress) -> Result<u64, DownloadError>`
returns bytes written and guarantees no partial file at `final_path` on error.

---

## 10. Job runner (`jobs.rs`)

`start_job`:
1. Validate `mode` ∈ {download, sync}, `input_mode` ∈ {single, bulk}; sync+bulk ⇒ error.
2. Validate output dir exists (create if missing) and is a directory.
3. Build URL list: single ⇒ `url`; bulk ⇒ `urls`, trim, drop empty, de-dupe preserving order.
4. Check auth client exists (return `"Connect Spotify in Settings first."`).
5. Check yt-dlp/ffmpeg available early; return a clear error.
6. Acquire the single-job lock (`compare_exchange`); spawn; return `job_id`.
7. Spawned job sleeps 200 ms, then runs; a guard resets `running`/`job` on every exit.

Job flow:
1. Resolve each URL (metadata) sequentially. Log per-URL failure and continue. If none resolve,
   emit `sync://complete` with status `failed`.
2. If bulk, create the master folder and log it.
3. Compute a rough total = sum of resolved track counts. Emit initial progress.
4. For each collection:
   - **Track (single)**: output filename; if exists ⇒ skip; else download; no state.
   - **Album**: create album folder; each track: expected filename; if exists ⇒ skip; else download.
   - **Playlist**:
     - folder name; nested-scan conflict prompt; load state.
     - diff; if removals and mode == sync ⇒ deletion prompt (mode download ⇒ leave entries, no prompt).
     - download new/missing; write state after each success.
     - if delete chosen ⇒ delete files + entries after downloads.
   - Bulk playlists use the same logic inside the master; sync mode is single-only.
5. Emit `download://progress` per processed track (current/total, percent, title).
   Use phases: `"preparing"`, `"metadata"`, `"scanning"`, `"awaiting confirmation"`,
   `"downloading"`, `"tagging"`, `"finalizing"`.
6. Emit `download://log` at key milestones and for every failure.
7. Emit `sync://complete` once with counts and a human summary like
   `"Downloaded 8, skipped 4, failed 1."` (cancelled ⇒ `"Cancelled."`; fatal ⇒ error text).
8. `cancel_job`: verify id, set flag, drop pending oneshot senders to unblock prompts.
   `cancel_job` during a track kills the yt-dlp child and finishes as `cancelled`.

Folder state for playlists must be created even in **download** mode (spec: downloading a playlist
creates `.playlist_sync.json`). Download mode skips files already tracked/present but never deletes
or prompts for removals.

---

## 11. Error handling

- Commands: `Result<_, String>`; internal `thiserror` enum is fine.
- Per-track failures never abort the batch: log, emit `download://track_failed`, increment `failed`.
- Fatal job errors (no auth, missing binaries, invalid output dir, metadata for every URL failed):
  emit `sync://complete` status `failed` with the message; never panic.
- Use `eprintln!` for diagnostics (no logger plugin).
- Do not include tokens/secrets in errors.
- Path errors should include the path in user-facing text.

---

## 12. Validation for Phase 4

Must pass:
```powershell
cd src-tauri
cargo check
```
Optional but valuable: `cargo test` for `sanitize`, filename generation, state JSON round-trip,
and the diff classification.

Self-review checklist:
- [ ] All commands in §3 registered in `invoke_handler`.
- [ ] Event payload field names match §4 exactly (camelCase).
- [ ] `start_job` args are flat, not nested.
- [ ] State file atomic write + corrupt backup.
- [ ] Sync never renumbers existing filenames; state never references deleted files.
- [ ] Nested scan skips junctions/hidden dirs and excludes the target.
- [ ] Deletion timeout defaults to Keep; conflict timeout defaults to Cancel.
- [ ] No secret/token logging, no `.spotify_token_cache.json`.
- [ ] No frontend edits and no `rustifydl` dependency.
