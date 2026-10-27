# Phase 4 — Module Interfaces (implementation companion)

Read `docs/phase4-implementation-contract.md` first. This file only fixes the **exact public
signatures** so the modules compile together without integration churn. Do not rename or change
these pub items. Private helpers are free.

Crate: `app_lib` (src-tauri/src). Pinned MSVC toolchain. Rust edition 2024.

Useful local reference sources (read these instead of guessing APIs):
- `C:\Users\User\.cargo\registry\src\index.crates.io-1949cf8c6b5b557f\rspotify-0.16.1`
- `...\rspotify-model-0.16.1`
- `...\lofty-0.25.5`
- `...\keyring-4.2.0` (if present)

Already-verified API notes:
- `AuthCodePkceSpotify::get_authorize_url(&mut self, Option<usize>) -> ClientResult<String>`;
  state is in `client.oauth.state`; verifier in `client.verifier`.
- `AuthCodePkceSpotify::from_token_with_config(token, creds, oauth, config)`.
- `rspotify::Token`; `client.token` is `Arc<rspotify::sync::Mutex<Option<Token>>>`, so
  `client.token.lock().await`.
- `Config { token_cached: false, token_refreshing: true, token_callback_fn, .. }` where
  `token_callback_fn: Arc<Option<rspotify::TokenCallback>>` and
  `TokenCallback(Box<dyn Fn(Token) -> Result<(), rspotify::CallbackError> + Send + Sync>)`.
- `PlaylistItem { item: Option<PlayableItem>, is_local: bool, .. }` (`track` field is deprecated;
  use `item`). `FullTrack.duration` is `chrono::Duration`; `FullTrack.id: Option<TrackId>`.
- `FullAlbum.artists: Vec<SimplifiedArtist>`, `images: Vec<Image>`; `album_track_manual` returns
  `Page<SimplifiedTrack>`; `playlist_items_manual` returns `Page<PlaylistItem>`; loop while
  `page.next.is_some()`.
- `Image.url: String`.
- `rspotify::model::idtypes::{TrackId, AlbumId, PlaylistId}` have `from_id(&str) -> Result<_, IdError>`.
- lofty 0.25.5: `use lofty::prelude::*;` for `Accessor`, `TagExt`, `AudioFile`, `TaggedFileExt`.
  `Tag::new(TagType::Id3v2)`, `tag.set_title/set_artist/set_album/set_track/set_disc`,
  `tag.save_to_path(path, WriteOptions::default())`.
- `keyring::Entry::new(service, user) -> Result<Entry>`; `get_password` returns
  `Err(keyring::Error::NoEntry)` when absent; `set_password`, `delete_credential`.
- tauri-plugin-dialog 2.8.3: `app.dialog().file().blocking_pick_folder() -> Option<FilePath>`;
  `FilePath::into_path() -> Result<PathBuf, _>`.

---

## auth.rs

```rust
pub const SERVICE: &str = "spotify_downloader";
pub const TOKEN_USER: &str = "spotify_token";
pub const SECRET_USER: &str = "spotify_secret";
pub const DEFAULT_REDIRECT_URI: &str = "http://127.0.0.1:8888/callback";

#[derive(Clone, Debug, Default)]
pub struct Settings {
    pub client_id: Option<String>,
    pub source: String,        // "settings" | "env" | "none"
    pub redirect_uri: String,
}

/// Loads `.env` candidates once (idempotent). Call once from setup.
pub fn load_env_files();

/// Reads settings.json (if any) then falls back to env. Never panics.
pub fn resolve_settings(settings_path: &std::path::Path) -> Settings;

/// Redirect URI from SPOTIFY_REDIRECT_URI or DEFAULT_REDIRECT_URI.
pub fn redirect_uri() -> String;
pub fn redirect_port(uri: &str) -> u16;

pub fn env_client_id() -> Option<String>;
pub fn env_client_secret() -> Option<String>;

pub fn write_settings_file(path: &std::path::Path, client_id: &str) -> Result<(), String>;
pub fn clear_settings_file(path: &std::path::Path) -> Result<(), String>;

pub fn store_token(token: &rspotify::Token) -> Result<(), String>;
pub fn load_token() -> Option<rspotify::Token>;
pub fn delete_token();

pub fn store_secret(secret: &str) -> Result<(), String>;
pub fn load_secret() -> Option<String>;
pub fn delete_secret();

pub fn build_client(client_id: &str, redirect_uri: &str) -> rspotify::AuthCodePkceSpotify;
pub fn restore_client(
    client_id: &str,
    redirect_uri: &str,
    token: rspotify::Token,
) -> rspotify::AuthCodePkceSpotify;

/// Persists the current token (after login / after metadata fetches).
pub async fn persist_token(client: &rspotify::AuthCodePkceSpotify);
pub async fn fetch_display_name(client: &rspotify::AuthCodePkceSpotify) -> Option<String>;

/// Bind before opening the browser so a busy port fails fast.
pub fn bind_callback(port: u16) -> Result<std::net::TcpListener, String>;

/// Nonblocking accept loop (100 ms poll), 3-minute deadline, cancel flag.
/// Returns the `code` query value; verifies `state`; "Sign-in cancelled" on cancel.
pub async fn await_callback(
    listener: std::net::TcpListener,
    expected_state: String,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> Result<String, String>;
```

`build_client` MUST install a `token_callback_fn` that stores the token in keyring, and use
`Config { token_cached: false, token_refreshing: true, .. }`. `resolve_settings` precedence:
settings.json > `.env` > process env.

## metadata.rs

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CollectionKind { Track, Album, Playlist }

#[derive(Clone, Debug)]
pub struct TrackMeta {
    pub id: String,
    pub title: String,
    pub artists: Vec<String>,
    pub album: String,
    pub album_id: Option<String>,
    pub track_number: u32,
    pub disc_number: u32,
    pub duration_ms: u64,
    pub cover_url: Option<String>,
    pub position: usize, // 1-based among accepted playlist tracks; 0 otherwise
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

/// Accepts open.spotify.com URLs and spotify: URIs; rejects artist/episode/show.
pub fn parse_spotify_url(input: &str) -> Result<(CollectionKind, String), String>;

pub async fn fetch_collection(
    client: &rspotify::AuthCodePkceSpotify,
    kind: CollectionKind,
    id: &str,
    source_url: &str,
) -> Result<Collection, String>;
```

`fetch_collection` does NOT persist tokens; the caller does.

## naming.rs

```rust
pub fn sanitize(input: &str, max_chars: usize) -> String;
pub fn display_artist(artists: &[String]) -> String;
pub fn playlist_folder(name: &str) -> String;
pub fn album_folder(artist_display: &str, album: &str) -> String;
pub fn single_track_filename(meta: &crate::metadata::TrackMeta) -> String;
pub fn playlist_track_filename(meta: &crate::metadata::TrackMeta, total_tracks: usize) -> String;
pub fn album_track_filename(meta: &crate::metadata::TrackMeta, multi_disc: bool) -> String;

/// Collision-safe child name inside `dir` (case-insensitive). Inserts " (n)" before the
/// extension. Never returns a name already present on disk.
pub fn unique_child_name(dir: &std::path::Path, desired: &str) -> String;
pub fn unique_dir(parent: &std::path::Path, desired: &str) -> std::path::PathBuf;
```

## sync.rs

```rust
pub const STATE_FILENAME: &str = ".playlist_sync.json";

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum EntryStatus {
    #[default]
    #[serde(rename = "ok")]
    Ok,
    #[serde(rename = "kept_removed")]
    KeptRemoved,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct SyncEntry {
    pub track_id: String,
    pub local_filename: String,
    pub title: String,
    pub artist: String,
    #[serde(default)]
    pub status: EntryStatus,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct SyncState {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    pub playlist_id: String,
    #[serde(default)]
    pub playlist_name: String,
    #[serde(default)]
    pub source_url: String,
    #[serde(default)]
    pub updated_at: Option<String>,
    #[serde(default)]
    pub entries: Vec<SyncEntry>,
}

#[derive(Clone, Debug)]
pub struct ScanMatch {
    pub dir: std::path::PathBuf,
    pub playlist_name: String,
    pub track_count: usize,
    pub updated_at: Option<String>,
}

#[derive(Clone, Debug)]
pub struct PlannedDownload {
    pub track_index: usize,        // index into the collection's tracks
    pub filename: String,
    pub entry_index: Option<usize>, // Some => redownload an existing entry
}

#[derive(Clone, Debug)]
pub struct RemovedTrack {
    pub entry_index: usize,
    pub track_id: String,
    pub title: String,
    pub artist: String,
    pub local_filename: String,
    pub exists: bool,
}

#[derive(Clone, Debug, Default)]
pub struct PlaylistPlan {
    pub downloads: Vec<PlannedDownload>,
    pub skipped: usize,
    pub removals: Vec<RemovedTrack>,
    pub kept_removed: usize,
}

pub fn state_path(dir: &std::path::Path) -> std::path::PathBuf;

/// Missing => fresh (false). Corrupt => backup + fresh + `true`.
pub fn load_state(dir: &std::path::Path, playlist_id: &str) -> (SyncState, bool);

/// Atomic pretty write; sets `updated_at`; retries rename on Windows 32/33.
pub fn save_state(dir: &std::path::Path, state: &mut SyncState) -> Result<(), String>;

/// Depth <= 6; skips hidden dirs, $RECYCLE.BIN, System Volume Information, junctions/symlinks.
pub fn scan_for_playlist(
    root: &std::path::Path,
    playlist_id: &str,
    exclude: &std::path::Path,
) -> Vec<ScanMatch>;

/// Mutates `state` in place (adopts on-disk casing, resets kept_removed -> ok when the
/// track reappears). Does not save.
pub fn plan_playlist(
    dir: &std::path::Path,
    playlist_name: &str,
    source_url: &str,
    tracks: &[crate::metadata::TrackMeta],
    state: &mut SyncState,
) -> PlaylistPlan;
```

## downloader.rs

```rust
pub struct Binaries {
    pub yt_dlp: std::path::PathBuf,
    pub ffmpeg_dir: Option<std::path::PathBuf>,
}

/// Discovery + version probes. Clear user-facing error if missing.
pub fn discover_binaries() -> Result<Binaries, String>;

pub enum DownloadError {
    Cancelled,
    Failed(String),
}
// impl std::fmt::Display + std::error::Error

/// Downloads, converts to MP3, renames onto `final_path`, tags ID3 text fields
/// (cover art best-effort/non-fatal), verifies size > 0.
/// Returns bytes written. No partial file at `final_path` on error.
pub async fn download_track(
    bins: &Binaries,
    meta: &crate::metadata::TrackMeta,
    final_path: &std::path::Path,
    dest_dir: &std::path::Path,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    on_progress: &(dyn Fn(u8) + Send + Sync),
) -> Result<u64, DownloadError>;
```

## Integration (written by orchestrator)

`lib.rs`: `AppState`, `JobControl`, `ConflictResolution`, `run()`.
`commands.rs`, `jobs.rs`.

AppState extra field (allowed): `conflict_matches:
Mutex<HashMap<String, Vec<PathBuf>>>` used only to validate `resolve_conflict`.
