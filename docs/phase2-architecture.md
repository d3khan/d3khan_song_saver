# Phase 2 Architecture — Spotify Downloader/Syncer (Tauri v2 + Rust)

Status: **Design** (no implementation in this phase)
Platform priority: **Windows first**; Linux/Android later (decisions below are POSIX-safe unless noted).
Last updated: 2026-10-26

---

## 0. Guiding constraints

1. **`rustifydl` API is unknown.** All pipeline interaction goes through one narrow trait (`Downloader`, §7.1). Sync/diff/state logic never imports pipeline types — it consumes `TrackMeta` and returns filesystem decisions.
2. **Windows is the reference platform.** Every filesystem rule assumes NTFS semantics: case-insensitive names, reserved device names, trailing-dot/space stripping, MAX_PATH, AV scanners holding file locks, junctions/reparse points.
3. **State files are the source of truth for playlists; deterministic filenames are the source of truth for albums/loose tracks.** See §5.4.
4. **Plan-then-execute.** Every sync job computes a full plan (diff) first, prompts once for destructive operations, then executes. Two-phase commit on state mutations.

### 0.1 Crate layout

```
spotify_downloader/
├── crates/
│   ├── core/            # pure logic: state schema, diff, naming, scan. No Tauri, no tokio deps (sync std only where possible)
│   ├── meta/            # MetadataProvider trait + rspotify-backed impl (or whatever metadata source we settle on)
│   ├── dl-adapter/      # Downloader trait + rustifydl adapter (separate crate so API churn is contained)
│   └── app/             # Tauri v2 app: commands, events, job worker, managed state
└── src/                 # frontend (tauri-app structure can merge crates/app; layout is indicative)
```

`core` is the heart of this doc. Everything in §1–§6 lives there; §7–§9 live in `app`.

---

## 1. `.playlist_sync.json` — versioned schema

### 1.1 Format rules

- Name: `.playlist_sync.json` (leading dot = hidden-ish on all platforms).
- Location: inside the playlist's output folder (e.g. `output/Chill Mix 2026/.playlist_sync.json`).
- Encoding: **UTF-8, no BOM**. serde_json rejects a BOM; the reader strips one if present (defensive, see §2.4).
- Pretty-printed with 2-space indent (git/backup diff friendly).
- **Casing: `snake_case`**, matching Spotify Web API field names (`duration_ms`, `track_number`, `disc_number`). See §2.2 for the full casing decision.
- `schema_version`: integer, semver-style as a single integer pair encoded as `1`, `2`, ... **Major = breaking field change, increment = additive-only change.** Initial value: `1`.

### 1.2 Exact schema (v1)

```jsonc
{
  "schema_version": 1,
  "kind": "playlist",                     // "playlist" (only value in v1; reserved for future album support)
  "playlist_id": "37i9dQZF1DXcBWIGoYBM5M",
  "playlist_name": "Chill Mix 2026",
  "source_url": "https://open.spotify.com/playlist/37i9dQZF1DXcBWIGoYBM5M",
  "created_at": "2026-10-26T14:03:11Z",
  "updated_at": "2026-10-26T15:44:02Z",
  "sync_disabled": false,                 // true = user chose "keep files & stop syncing"; future runs warn, don't auto-sync
  "entries": [
    {
      "track_id": "3n3Ppam7vgaVa1iaRUc9Lp",
      "local_filename": "03 - Daft Punk - Get Lucky.mp3",
      "title": "Get Lucky",
      "artist": "Daft Punk",
      "track_number": 1,
      "disc_number": 1,
      "album": "Random Access Memories",
      "duration_ms": 368400,
      "added_at": "2026-10-26T15:44:02Z",   // when this entry was downloaded (audit/debug)
      "file_size_bytes": 9210834,
      "status": "ok"                        // "ok" | "kept_removed" (user kept a file whose track left the playlist)
    }
  ]
}
```

### 1.3 Field justification (extras beyond the required minimum)

| Extra field | Why it earns its place |
|---|---|
| `kind` | One file format for playlists now, albums later, without a migration. Cost: one byte of ambiguity. |
| `playlist_name`, `source_url` | Needed for the nested-scan conflict UI (show the human name + original URL when a folder was renamed) and for "open original" actions. |
| `created_at` / `updated_at` | Distinguishing two competing sync files in the conflict UI ("this one is 3 weeks older"); debugging. |
| `track_number`, `disc_number`, `album`, `duration_ms` | **Re-tagging without re-fetching metadata** (id3 repair is a planned feature), and future album-mode sync reuses the same entry shape. `duration_ms` also feeds the progress UI's per-track estimates. |
| `added_at` | Audit trail; lets the diff log distinguish "added today" from "added months ago" when reporting redownloads. |
| `file_size_bytes` | Zero-cost reconcile check: entry says the file exists but size is 0 → treat as corrupt → redownload (§9). Not used for change detection (mtime/hash would be overkill and slow). |
| `status: "kept_removed"` | Makes the user's "Keep" decision durable so we never re-prompt for the same track on every re-run (§3.5). |
| `sync_disabled` | Durable representation of the "Keep files & stop syncing this folder" choice (§3.5, §6). |

Deliberately **excluded** (YAGNI): `sha256` (we never verify audio content), `isrc` (belongs to `TrackMeta` at download time, not state), `added_by`, `position` (position is frozen in the `local_filename` numbering; re-deriving it invites mass renames), `cover_url` (fetched fresh at tag time).

### 1.4 Compatibility rules

**Backward compatible (older file, newer app):**
- Every struct field gets `#[serde(default)]` (via container-level `#[serde(default)]` + `Default` impls). A missing field parses as its default; on next write it is materialized.
- Unknown fields: serde ignores unknown fields by default — **do not add `deny_unknown_fields`**. Forward-compatible read is free.

**Forward compatible (newer file, older app):**
- Additive change (`schema_version` bumps by minor increment, e.g. `1.1` encoded as `2` where `1` fields are untouched): older app reads fine (unknown fields ignored). Acceptable.
- Breaking change (field removed/renamed, value semantics changed): `schema_version` major bump. Rule: **an app must refuse to rewrite a state file whose `schema_version` major is greater than the app's supported major.** It may still read-and-sync from it (defaults cover missing fields) but writes abort with a user-facing error: *"This playlist folder was written by a newer app version. Update the app to sync this folder."* This prevents an old app from silently dropping new fields.

**Migration path:** a `migrations: [fn(serde_json::Value) -> Result<Value>]` array in `core::state`, indexed by version. v1 is terminal for now; the mechanism is:

```rust
const SUPPORTED_MAJOR: u64 = 1;

pub fn load(raw: &str) -> Result<SyncState, StateError> {
    let v: serde_json::Value = strip_bom(raw).parse()...;
    let ver = v.get("schema_version").and_then(|x| x.as_u64()).unwrap_or(1);
    if ver > SUPPORTED_MAJOR * 10 { /* major greater → StateError::NewerVersion */ }
    for m in MIGRATIONS.iter().take_while(|m| m.from < ver) { v = m.apply(v)?; }
    serde_json::from_value(v).map_err(StateError::Parse)
}
```

Migrations are pure `Value → Value` functions, unit-tested against committed fixture JSONs (`core/tests/fixtures/state_v*.json`).

---

## 2. Rust structs, read/write helpers

### 2.1 Struct definitions

```rust
// crates/core/src/state.rs
use serde::{Deserialize, Serialize};

pub const STATE_FILENAME: &str = ".playlist_sync.json";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case", default)]        // container default: every missing field -> Default
pub struct SyncState {
    pub schema_version: u64,                        // default 1 via Default impl below
    pub kind: String,                               // "playlist"
    pub playlist_id: String,
    pub playlist_name: String,
    pub source_url: String,
    pub created_at: Option<String>,                 // RFC3339 UTC; Option so very old files parse
    pub updated_at: Option<String>,
    pub sync_disabled: bool,
    pub entries: Vec<Entry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case", default)]
pub struct Entry {
    pub track_id: String,
    pub local_filename: String,                     // bare filename, never a path, never absolute
    pub title: String,
    pub artist: String,                             // joined primary display string ("A, B")
    pub track_number: Option<u32>,
    pub disc_number: Option<u32>,
    pub album: Option<String>,
    pub duration_ms: Option<u64>,
    pub added_at: Option<String>,
    pub file_size_bytes: Option<u64>,
    pub status: EntryStatus,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EntryStatus { #[default] Ok, KeptRemoved }

impl Default for SyncState {
    fn default() -> Self {
        Self { schema_version: 1, kind: "playlist".into(),
               sync_disabled: false, entries: vec![], ..Default::default() }
    }
}
```

`local_filename` is a **bare filename only** — the folder is implied (the folder the state file lives in). This is what makes "move the folder" and the nested-scan "sync here instead" free operations.

### 2.2 Casing decision

- **State file: `snake_case`** — matches Spotify Web API JSON, matches Rust field names (no `rename` noise), and the file is hand-readable.
- **Tauri event payloads & command args: `camelCase`** — JS convention, and Tauri v2 already expects camelCase argument names from JS. Each event payload struct carries `#[serde(rename_all = "camelCase")]`.
- These are two different struct families. Do **not** serialize `SyncState` itself over the bridge; mirror small payload structs instead. This keeps the on-disk format free to evolve without breaking the frontend.

### 2.3 Atomic write helper

```rust
// crates/core/src/state_io.rs
pub fn write_state_atomic(path: &Path, state: &SyncState) -> Result<(), StateIoError> {
    let dir = path.parent().ok_or(StateIoError::NoParent)?;
    let tmp = dir.join(format!(".{}.{}.tmp", STATE_FILENAME, uuid::Uuid::new_v4().simple()));

    let mut f = std::fs::File::create(&tmp)?;
    serde_json::to_writer_pretty(&mut f, state)?;   // to_writer, not to_string: no double buffering
    f.sync_all()?;                                  // flush user buffers + FlushFileBuffers (fsync)

    // Windows: std::fs::rename == MoveFileExW(MOVEFILE_REPLACE_EXISTING), so it REPLACES an
    // existing destination (unlike raw rename on some other APIs). But it fails with a sharing
    // violation if the destination is open elsewhere (classic AV-scanner race), so retry.
    let mut last = None;
    for delay in [0, 100, 300, 800] {
        if delay > 0 { std::thread::sleep(Duration::from_millis(delay)); }
        match std::fs::rename(&tmp, path) {
            Ok(()) => { last = None; break; }
            Err(e) if is_retryable_win(&e) => { last = Some(e); }
            Err(e) => { last = Some(e); break; }
        }
    }
    if let Some(e) = last { let _ = std::fs::remove_file(&tmp); return Err(e.into()); }
    Ok(())
}

fn is_retryable_win(e: &std::io::Error) -> bool {
    // ERROR_SHARING_VIOLATION (32), ERROR_LOCK_VIOLATION (33) surface as
    // os error 32/33 on Windows; treat those as retryable.
    e.raw_os_error().map(|c| c == 32 || c == 33).unwrap_or(false)
}
```

**fsync considerations on Windows:**
- `File::sync_all()` on Windows calls `FlushFileBuffers` — that is the fsync. Requires the handle opened for write (ours is).
- There is **no portable directory fsync on Windows** through std (.FlushFileBuffers on a directory handle requires special access). Skip it; an NTFS rename is metadata-journaled and this is the accepted industry norm. On Linux later: `fs::File::open(dir)?.sync_all()` after rename — put this behind `#[cfg(unix)]` in the same helper.
- Never write the tmp file with a "hidden" attribute dance; a leading-dot tmp in the same directory is enough (same volume ⇒ rename is atomic, never cross-device).

### 2.4 Read helper + corrupt-file recovery

```rust
pub enum StateLoad {
    Fresh,                       // no file (or unrecoverable corrupt file): start empty
    Loaded(SyncState),
}

pub fn load_state(path: &Path) -> Result<StateLoad, StateError> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(StateLoad::Fresh),
        Err(e) => return Err(e.into()),
    };
    let text = String::from_utf8_lossy(&bytes);           // tolerant decode; UTF-8 expected
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);   // strip BOM if present

    match crate::state::load(text) {                      // §1.4 migration entry point
        Ok(state) => Ok(StateLoad::Loaded(state)),
        Err(StateError::NewerVersion { found }) => Err(StateError::NewerVersion { found }),  // fatal: do NOT overwrite
        Err(_) => {
            // Corrupt: back it up, never delete silently.
            let backup = path.with_extension(format!("json.corrupt-{}", timestamp_compact()));
            let _ = std::fs::rename(path, &backup);
            Ok(StateLoad::Fresh)   // sync proceeds as a first-time download; backup kept for manual recovery
        }
    }
}
```

Rules:
- Corrupt file ⇒ **rename to `.playlist_sync.json.corrupt-<ts>` in place**, log at `warn`, proceed with `Fresh`. Never `remove_file` a state file we failed to parse.
- `NewerVersion` ⇒ hard error for this folder; job reports it as a per-item failure and continues with other folders in bulk mode (§8).

### 2.5 Never store absolute paths in state

`local_filename` is relative to the state file's directory. The backend resolves `state_dir.join(&entry.local_filename)` at use time. Consequence: moving/renaming the playlist folder anywhere keeps state valid (§10 manual case "move folder").

---

## 3. Diff algorithm

### 3.1 Inputs

- `fresh`: `Vec<FreshTrack>` from the metadata provider, in Spotify playlist order. `FreshTrack { position: usize, track: Option<TrackMeta> }` — `None` = Spotify returned a null item (removed/unavailable track).
- `state`: loaded `SyncState` (may be `Fresh` = empty entries).
- `fs_map`: lowercase-file map of the playlist folder, built once:

```rust
/// key: lowercased filename, value: actual name on disk.
/// Built with std::fs::read_dir; works identically on case-sensitive Linux
/// (where it simply behaves as an exact-match map plus collision detection).
pub struct FsMap(HashMap<String, String>);
```

All "does the file exist" questions go through `FsMap`, never `Path::exists()`. This makes Windows case-insensitivity explicit and testable.

### 3.2 Classification (exact states)

| State | Meaning | Action |
|---|---|---|
| `SKIP` | in fresh ∧ in state ∧ file present | nothing |
| `REDOWNLOAD` | in fresh ∧ in state ∧ file missing (no recovery match) | download |
| `NEW` | in fresh ∧ not in state | download |
| `ADOPT_RENAME` | in fresh ∧ in state ∧ file missing ∧ exactly one loose-name recovery match in folder | point `local_filename` at the found file (no re-download, no user rename) |
| `REMOVED_PROMPT` | not in fresh ∧ in state ∧ `status == ok` ∧ (per-track lookup says track is gone) | collect for user prompt |
| `UNAVAILABLE_KEEP` | not in fresh ∧ in state ∧ per-track lookup says track still exists globally (temporarily unavailable) | auto-keep, set `status = kept_removed`? **No** — set `status = ok` and add flag `missing_from_playlist: true` (new optional field, additive) so a later re-run re-checks it |
| `KEPT_SKIP` | not in fresh ∧ in state ∧ `status == kept_removed` | nothing (already decided; reported in summary) |
| `UNTRACKED` | file on disk not referenced by any state entry, not adopted | report only. **Never delete, never adopt silently.** |
| `CORRUPT` | state entry's file present but `file_size_bytes == 0` or read fails | treat as `REDOWNLOAD` |

### 3.3 Pseudocode (with duplicate track_ids handled)

```text
diff(fresh, state, fs_map, target_dir) -> Plan

  # --- occurrence matching for duplicate track_ids ---
  # The same track_id can legitimately appear at two positions in a Spotify playlist.
  # Rule: match the k-th occurrence in `fresh` to the k-th occurrence in `state`
  # (both lists ordered by position). Each occurrence owns its own file, named by
  # the position it was FIRST downloaded at; numbering is frozen, never renumbered (§5.3).
  by_id_state: Map<track_id, Deque<Entry>>   # entries pushed in state order
  for e in state.entries: by_id_state[e.track_id].push_back(e)

  fresh_ids: Set<track_id> = { t.id for t in fresh if t.track.is_some() }
  matched_entries: Set<*Entry> = {}

  plan.added = []; plan.skip = []; plan.redownload = []; plan.adopt = []

  for ft in fresh:                              # in Spotify order
    if ft.track is None:
        continue                                # null item; leftover state matching happens below
    if deque = by_id_state[ft.track.id], non-empty:
        e = deque.pop_front(); matched_entries.add(e)
        actual = fs_map.get(lower(e.local_filename))
        if actual is None or is_zero_or_unreadable(actual):
            cand = recover_match(target_dir, fs_map, e)         # §3.4
            if cand is Some(name) and name != e.local_filename:
                plan.adopt.push(Adopt { entry: e, new_name: name })
            else:
                plan.redownload.push(Redownload { entry: e, meta: ft.track })
        else:
            plan.skip.push(Skip { entry: e, actual_case: actual })
    else:
        plan.added.push(New { meta: ft.track, position: ft.position })

  # --- leftovers: in state, not matched to any fresh occurrence ---
  plan.removal_prompt = []
  plan.unavailable_keep = []
  for e in state.entries where e not in matched_entries:
      if e.status == kept_removed:
          plan.kept_skip.push(e)                                 # silent, summary only
      else if e.track_id in fresh_ids:
          # weird: id present but occurrence count shrank while this one unmatched.
          # (e.g. track removed from pos 3, remains at pos 9, state had both.)
          # The survivor matched an earlier occurrence; treat this one as removed.
          plan.removal_prompt.push(e)
      else:
          exists = per_track_lookup(e.track_id)                   # 1 GET /tracks/{id}; batched
          if exists: plan.unavailable_keep.push(e)                # auto-keep, flag, no prompt
          else:      plan.removal_prompt.push(e)                  # genuinely gone → prompt

  # --- untracked files ---
  referenced = { lower(e.local_filename) for e in state.entries }
  referenced += { lower(a.new_name) for a in plan.adopt }
  plan.untracked = [name for (lower, name) in fs_map if lower not in referenced
                    and not name.starts_with(".") and not is_tmp(name)]
```

Duplicate-track semantics, spelled out (this is the subtle case):

- Playlist has track T at positions 3 and 9. First download produces `03 - …` and `09 - …`, two state entries, same `track_id`.
- Spotify later removes position 3. Fresh list has T only at position 9. Occurrence matching pairs state-occurrence#0 (the `03` entry) with the surviving fresh occurrence. Result: `03 - …` is the file that "survives" and `09 - …` is flagged `REMOVED_PROMPT`. **Filenames are not renumbered** — stability beats numbering purity; the numbers reflect when tracks were added, which is exactly what a user expects from "skip already-present tracks".
- If Spotify adds T at a *third* position → `NEW` at that position's number.

### 3.4 `recover_match` (renamed/moved file detection — bounded and conservative)

```text
recover_match(target_dir, fs_map, entry):
  candidates = []
  for (lower, actual) in fs_map where lower not referenced_by_state:
      if lower == lower(entry.local_filename):        # case-only rename on Linux, or user recased
          candidates.push(actual)
      else if lower contains lower(sanitized("{artist} - {title}")):   # loose match
          candidates.push(actual)
  if |candidates| == 1: return Some(candidates[0])    # adopt
  return None                                         # 0 or ≥2 candidates → REDOWNLOAD (never guess)
```

Never search outside the playlist folder for recovery matches — cross-folder moves are handled by the nested-scan conflict path (§4), which is user-visible, not magic.

### 3.5 Decision application (what each outcome writes)

- `ADOPT_RENAME`: on execute, update `entry.local_filename = new_name`, `updated_at = now`, write state. **The user's file is not renamed** — the state follows the user.
- `UNAVAILABLE_KEEP`: set `entry.missing_from_playlist = true` (additive optional field, default false), write state once after the loop.
- `REMOVED_PROMPT` + user **Delete**: per-track sequence (strict order, §3.6): delete file → remove entry → atomic state write.
- `REMOVED_PROMPT` + user **Keep**: set all these entries `status = kept_removed`, one state write. Next run: `KEPT_SKIP`, never re-prompted.
- `REMOVED_PROMPT` + user **KeepAndDisable**: set entries `kept_removed` **and** top-level `sync_disabled = true`, one state write. Future runs on this URL in this folder: worker refuses to auto-sync; it emits a log + `sync://conflict`-style prompt asking whether to re-enable or download into a fresh folder. (Re-enable writes `sync_disabled = false` and proceeds.)

### 3.6 Exact ordering of operations (the invariant)

**Invariant: state never references a deleted file, and a deleted file is never resurrected silently.**

Execution order for a sync run:

```text
1. acquire folder lock                      (§7.5)
2. load state (Fresh or Loaded)             (§2.4)
3. fetch fresh list (metadata)              ← CANCELLABLE
4. build fs_map
5. compute plan (diff)
6. if plan.removal_prompt non-empty and state kind == playlist and not sync_disabled:
      PROMPT  (sync://removed_tracks)       ← CANCELLABLE; timeout ⇒ Keep
      apply decision to state (§3.5); write state atomically
7. for each item in added ∪ redownload:     ← CANCELLABLE between items
      download → tag → write/patch state (per track, §9.1)
8. if user chose Delete:
      for each removal entry, in plan order:
          delete file (send to recycle bin if feasible, else unlink)   ← NOT cancellable from here on
          remove entry from state
          write state atomically                                  ← per track, keeps invariant on crash
9. cleanup: delete stale `.tmp`/`.part` files older than 1 h   (§9.2)
10. write final updated_at; emit sync://complete
11. release folder lock
```

Why deletion happens **last**, after downloads: if a download phase fails hard (fatal error, §8), we still want removals not to have destroyed files while the folder is half-updated. And why the delete loop is **not cancellable**: once the user confirmed deletion, cancellation mid-way would leave a state/file mismatch; the loop is short (unlink + state write per item) and idempotent-on-retry (§3.7).

**Crash analysis of the delete loop:** crash between `delete file` and `state write` ⇒ state references a file that's gone, and its track_id is absent from fresh. Next run classifies it as… `REMOVED_PROMPT` again (track_id ∉ fresh_ids ⇒ per-track lookup ⇒ gone ⇒ prompt). User confirms ⇒ no-op delete of an already-absent file (must be idempotent: "delete if exists"). So the worst crash outcome is one redundant confirmation — no data loss, no silent redownload. Accepted residual risk; documented.

Also note step 6 writes state *before* any downloads: the Keep/kept_removed decision is durable even if the run then crashes.

### 3.7 State machine

```text
                ┌────────────────────────────────────────────────────┐
                v                                                    │
Idle ──> Locking ──> LoadingState ──> FetchingMetadata ──> Diffing ──┤
                                                                     │
   ┌────────────────────────────────────── no prompt needed ─────────┘
   │
   ├─ AwaitingDeletionDecision ──(decision)──> Downloading
   │                │                                          │
   │                ├─ timeout ⇒ auto-Keep ───────────────────>│
   │                └─ cancel  ⇒ Cancelling                    │
   │                                                           v
   └──────────────────────────────────────────────> ApplyingRemovals
                                                   (only if Delete)
                                                         │
                                                         v
                                                    Finalizing (cleanup, updated_at)
                                                         │
                                                         v
                                                      Complete / Failed / Cancelled

Downloading: per track: ResolvingSource ──> DownloadingAudio ──> Tagging ──> WritingState
             any per-track error ⇒ logged, batch continues (§8); fatal ⇒ jump to Failed
```

Cancellation is checked at: Locking (before acquire), after FetchingMetadata, after Diffing, between tracks in Downloading, between delete items is **not** checked. AwaitingDeletionDecision cancels via dropping the oneshot sender (§6.3).

---

## 4. Nested-folder prevention

Goal: re-running playlist URL P when a folder for P already exists somewhere under the output tree must warn and offer to sync there, instead of creating `…/Chill Mix (1)/`.

### 4.1 Scan algorithm

We use **`walkdir` with a `filter_entry` predicate** plus canonicalization guards. Manual `read_dir` recursion is the fallback if walkdir's pruning turns out not to cover junctions on Windows; the predicate below is written so it works with either.

```rust
pub struct ScanMatch {
    pub dir: PathBuf,            // absolute, canonical-ish (see below)
    pub playlist_name: String,   // from the state file, may differ from folder name
    pub playlist_id: String,
    pub track_count: usize,
    pub updated_at: Option<String>,
}

pub fn scan_for_playlist(root: &Path, target_dir: &Path, playlist_id: &str,
                         max_depth: usize /* = 4 */) -> Result<Vec<ScanMatch>, ScanError>
```

```text
scan_for_playlist(root, target, playlist_id):
  if !root.exists(): return []
  root_c   = longpath_aware_canonicalize(root)     # §5.6 helper
  target_c = longpath_aware_canonicalize(target)   # may not exist yet → canonicalize parent, join name

  matches = []
  for entry in WalkDir::new(root_c)
        .min_depth(1).max_depth(max_depth)
        .follow_links(false)                       # symlinks: never follow
        .into_iter()
        .filter_entry(|e| !is_excluded(e, root_c, target_c)):

    if !entry.file_type().is_dir(): continue
    d = entry.path()

    if d == target_c: continue        # do not treat the target folder itself as a conflict —
                                      # re-running in place is the normal case (§4.4 rule A)
    state_path = d / ".playlist_sync.json"
    if state_path exists:
        match load_state(state_path):
            Loaded(s) if s.playlist_id == playlist_id && s.kind == "playlist" && !s.sync_disabled:
                matches.push(ScanMatch { dir: d, playlist_name: s.playlist_name,
                                         track_count: s.entries.len(), updated_at: s.updated_at })
            Loaded(_) | Fresh | Err(_): ignore    # other playlist / corrupt / newer-version file:
                                                  # a NewerVersion file must still count as a match if
                                                  # playlist_id is readable — parse id manually before
                                                  # full migration (see note below)
  return matches (sorted by updated_at desc)
```

**NewerVersion edge:** if `load_state` fails with `NewerVersion`, attempt a raw `serde_json::Value` read and extract `playlist_id` for matching only; if that also fails, log and treat as non-match (we can't confirm; creating a duplicate folder is the safe failure mode — the user still sees two folders with the same playlist content and can clean up).

### 4.2 Exclusion predicate (`is_excluded`)

Prune (return `false` ⇒ subtree skipped):

1. `entry.depth() > max_depth` — depth limit 4 covers `output/master/playlist/sub-thing`; keeps the scan O(small) on huge libraries. (Configurable, default 4.)
2. Directory name starts with `.` (hidden convention; also covers our own tmp files) — except we must not prune `root` itself (filter_entry is also called with depth 0; guard: only apply exclusions 2–5 when `depth >= 1`).
3. Name is `"$RECYCLE.BIN"` or `"System Volume Information"` (recycle bin / metadata dirs).
4. **Reparse points / junctions / symlinks:** on Windows, check `entry.metadata()` (for `DirEntry` this is *not* follow-symlinks) and test `FILE_ATTRIBUTE_REPARSE_POINT (0x400)` via `std::os::windows::fs::MetadataExt::file_attributes()`. Junctions are **not** `is_symlink()` in Rust — they are reported as directories with the reparse attribute, so walkdir's `follow_links(false)` does **not** protect against them; the attribute check does. Also covers mounted drives and OneDrive placeholders' rare reparse usage (OneDrive placeholder files are fine — only directories are pruned here). `#[cfg(unix)]`: skip entries where `file_type().is_symlink()` (walkdir already does).
5. The entry is a file (implied by the dir check, listed for clarity).

Additional guards:

- **Do not descend into a matched folder's *state-relevant* children?** No — we do continue scanning inside matched folders (a playlist folder could theoretically nest another playlist folder; both are matches). Fine, cheap.
- **Scan cost:** read_dir per directory; only opens `.playlist_sync.json` when present. A 10k-folder library scans in well under a second on SSD.

### 4.3 Canonicalization guards

- Both `root` and `target` are canonicalized **before** comparison, and comparisons use the canonical forms. Case: user passes `C:\Users\User\Music` but the state file recorded a path through a subst drive or 8.3 alias — canonicalization collapses these.
- Canonicalize can fail (target doesn't exist yet): canonicalize the **existing parent** and join the new folder name; document that this yields a canonical target unless the parent itself is a junction (excluded by rule 4 above in the scan; for `target` construction it's acceptable because the target is where *we* will write, and creating inside a junction is the user's explicit choice).
- All path comparisons after canonicalization use `PathBuf` equality; additionally compare `to_string_lossy().to_lowercase()` as a fallback for case-only differences (Windows canonicalization preserves the user's casing, `Music` vs `music`).

### 4.4 Decision table

Inputs: `matches` (excluding target itself), `target` = the folder the job would create/use.

| Case | Behavior |
|---|---|
| **0 matches** | No conflict. Derive folder name (§5.5), create, download. |
| **1 match, `match.dir == target`** | Re-run in place. Normal sync path, no UI. (This is rule A: the target folder itself is never a conflict against itself.) |
| **1 match, elsewhere** | Emit `sync://conflict` with the one match. Buttons: **"Sync there"** / **"Create new folder anyway"** / **"Cancel"**. |
| **N matches** | Emit `sync://conflict` with all matches (sorted newest-first). Buttons: **"Sync in…"** (user picks one of the N, or master root) / **"Create new folder anyway"** / **"Cancel"**. |
| **Folder renamed** (name ≠ playlist_name) | Handled naturally: matching is by `playlist_id` from the state file, not by folder name. UI shows `"Old name" (now: C:\…\Renamed Folder), last synced 2026-09-01, 47 tracks`. |
| **Case where folder name differs from playlist name** (created that way, e.g. user renamed) | Same as above; the state file is the identity, the folder is just a container. When "Sync there" is chosen, the folder is *not* renamed back — we respect the user's name. |

**What happens to the returned path:**

- **Sync there / Sync in:** the job's effective `target_dir` is replaced by `match.dir`; the job proceeds exactly like a re-run: load that folder's state, diff, prompt-on-removal, download added tracks. The originally requested folder path is discarded (nothing was created yet). In bulk mode, per-item retargeting is allowed (each URL resolves independently); the master folder stays as the container for loose tracks.
- **Create new folder anyway:** proceed at the original target. Log a `warn` line: `"Playlist 'X' already exists at '<match dir>'; created duplicate folder '<target>' by user choice."` Collision suffixing still applies if the name collides (§5.7) — the user's explicit choice overrides the warning, but not physics.
- **Cancel:** the whole job aborts (in bulk mode: remaining items are skipped; completed items stay complete). Nothing was created; no state written.

### 4.5 UI flow (modal contract)

```text
sync://conflict payload:
{
  "jobId": "…", "requestId": "…",
  "playlistName": "Chill Mix 2026",
  "playlistId": "37i9dQZF1DXcBWIGoYBM5M",
  "targetDir": "C:\\Users\\User\\Music\\Chill Mix 2026",
  "matches": [
    { "dir": "C:\\Users\\User\\Music\\Old Name", "playlistName": "Old Name",
      "trackCount": 47, "updatedAt": "2026-09-01T10:00:00Z" }
  ],
  "timeoutMs": 600000
}
```

Modal shows: *“A folder for this playlist already exists”* + match list (path, stored name, track count, last sync). Buttons per §4.4. Timeout ⇒ **Cancel** (conflict is a fork in the road; the safe default is to do nothing, unlike the deletion prompt where the safe default is Keep).

---

## 5. Folder & filename rules

### 5.1 Sanitizer spec (exact, ordered)

Input: arbitrary Unicode string (playlist name, track title, artist join, album name).

```text
sanitize(input, max_chars):
  1. Unicode NFC-normalize                    (unicode-normalization crate)
  2. replace every char in {'<','>',':','"','/','\\','|','?','*'} with '_'
  3. replace every control char (U+0000..=U+001F, U+007F) with '_'
  4. collapse runs of whitespace (incl. tabs/newlines) to a single ' '
  5. trim leading/trailing whitespace
  6. truncate to max_chars, cutting at a char boundary (chars().take(max_chars))
  7. REPEAT step 5 (truncation can expose a trailing dot/space)
  8. strip ALL trailing '.' and ' ' chars (Windows strips them silently on create;
                                              stripping first keeps write/read consistent)
  9. if result is empty (or was only illegal chars): return "Untitled"
 10. reserved-device check on the part before the first '.':
       upper = result.split('.').next().to_uppercase()
       if upper ∈ {CON, PRN, AUX, NUL} ∪ {COM1..COM9} ∪ {LPT1..LPT9}:
           result = result + "_"      (e.g. "CON.txt" → "CON_.txt"; "con" → "con_")
       (Windows 11 22H2+ relaxed some of this; we enforce the classic rules anyway —
        safe on every Windows version and harmless elsewhere.)
 11. return result
```

Also strip leading dots of the *result*? No — a leading dot is legal on Windows and only hides the file in some shells; but a state/local filename starting with `.` would confuse the untracked-file filter (§3.3 excludes names starting with `.`). Rule: if the sanitized name starts with `.`, prefix `_` (e.g. `.hack` → `_.hack`).

### 5.2 Length limits

| Component | Limit (UTF-16 code units ≈ chars) | Rationale |
|---|---|---|
| Folder (playlist/album/master child) | 64 | sanity + keeps total path short |
| Filename **stem** | 100 | leaves room for `NN - ` prefix + `.mp3` + suffixes |
| Filename with extension | 128 | |
| Any single component | ≤ 255 (NTFS hard limit) — never approached given the above | |
| Full path | see §5.6 long-path strategy | |

`max_chars` truncation operates on the *stem*; the numbering prefix and extension are added after sanitization and don't count toward the sanitized portion.

### 5.3 Numbering & name formats (deterministic)

Position numbers are assigned from the fresh list at first download and **frozen** in `local_filename`. Never renumber on playlist growth/removal (§3.3).

| Mode | Folder | Filename |
|---|---|---|
| Single link, single track | none (file directly in user-chosen output folder) | `sanitize("{artist} - {title}", 100).mp3` |
| Single link, album/playlist | `sanitize(playlist_or_album_name, 64)/` | playlist: `"{pos:0W} - {artist} - {title}.mp3"`; album: `"{track:02} - {artist} - {title}.mp3"` (multi-disc: `"{disc:02}-{track:02} - …"`) |
| Bulk, loose track | none (file in master) | as single |
| Bulk, album | `master/sanitize("{artist} - {album}", 64)/` | as album above |
| Bulk, playlist | `master/sanitize(playlist_name, 64)/` | as playlist above |

- `W` (width) = `max(2, digits(position))` computed **at download time from the position of that track in the fresh list** — not from the total (a 100-track playlist's track 100 gets `100`; existing `01` files are untouched).
- `{artist}` = artists joined with `", "` (max 2 artists, then `"+N"`? — no: join ALL with `", "` then sanitize truncates. Simple, deterministic).
- Deterministic folder-name derivation for playlist/album = **`sanitize(name, 64)`** — nothing else (no timestamps, no hashes, no ids). Determinism matters for the "re-run same URL" flow and for tests. Collisions resolved by §5.7 suffixing at creation time only.
- Album folder: `{artist} - {album}` so multiple albums by one artist group in listings.

### 5.4 Statefulness matrix

| Mode | Skip-detection mechanism |
|---|---|
| Playlist | `.playlist_sync.json` (stateful diff, §3) |
| Album (any mode) | **Stateless**: recompute expected filenames from fresh album listing; file present in `FsMap` ⇒ skip. No state file. (Albums rarely change; avoids a second schema.) |
| Loose/single track | **Stateless**: expected filename present ⇒ skip. |

Consequence (accepted): if a user renames an album's file manually, re-running the album redownloads it. Documented, cheap.

### 5.5 Collision-safe final path resolution (the algorithm)

```text
resolve_final_path(dir, desired_filename) -> PathBuf:
  # desired_filename is fully sanitized; ext included.
  # dir listing is cached in FsMap for the duration of one resolve call.
  candidates = [ desired_filename ]
  n = 1
  loop:
    c = candidates.last()
    if lower(c) not in fs_map(dir):          # case-INSENSITIVE compare — the whole point
        return dir.join(c)
    candidates.push(insert_suffix(c, " (n)"))   # "03 - A - B.mp3" → "03 - A - B (1).mp3"
    n += 1
    if n > 500: fail with CorruptNaming error    # runaway guard
  # insert_suffix: inserts before the extension, preserving numbering prefix.

refresh fs_map(dir) after each successful create (or use dir-scan-on-demand).
```

The case-insensitive map is mandatory: on Windows, `Path::exists("Track.mp3")` returns true even for an on-disk `track.mp3` (so creation would fail or collide silently); on Linux it doesn't, so we maintain the map ourselves and get identical behavior both places (plus we learn the *actual* casing for state).

### 5.6 Long-path (>260) strategy

1. **Primary defense: don't build long paths.** Component limits (§5.2) plus a 64-char folder cap keep typical trees at `root(≈80) + 64 + 128 + slack < 260` for the common case of `output/Playlist/file`.
2. **Fallback: extended-length prefix.** Before *any* filesystem operation on a path whose UTF-16 length ≥ 240, convert: `\\?\C:\…` — absolute, canonical, backslash-only, no `.`/`..` components. Helper:

```rust
pub fn win_abs(p: &Path) -> PathBuf {          // for std calls that may hit long paths
    #[cfg(windows)] {
        let s = p.to_string_lossy();
        if s.starts_with("\\\\?\\") { return p.into(); }
        let canon = dunce::canonicalize(p).unwrap_or_else(|_| p.to_path_buf()); // dunce: no \\?\ when short
        if canon.as_os_str().len() >= 240 { format!("\\\\?\\{}", canon.display()).into() }
        else { canon }
    }
    #[cfg(not(windows))] { p.to_path_buf() }
}
```

3. Do **not** rely on the `longPathAware` manifest + registry opt-in alone, but DO include the manifest (harmless, helps other tools): `<ws2 longPathAware>true` in the Tauri binary manifest plus the `LongPathsEnabled` note in docs.
4. In state we store only bare filenames, so state files never contain long-path forms.
5. `dunce`-style canonicalization (plain absolute path when short enough) everywhere, so paths we hand to the shell/explorer/other tools never carry `\\?\` unnecessarily.

### 5.7 Folder-name collisions

When creating `output/Chill Mix/` and it exists (case-insensitively) and is **not** a match for this playlist (i.e. user chose "create anyway" over an existing unrelated folder, or a same-named *different* playlist exists): suffix `sanitize_name (2)`, `(3)`, … before creating. For playlists, first check the nested-scan (§4) — this path only executes after the user explicitly chose "create new folder anyway".

---

## 6. Sync confirmation flow (plan-then-execute, two-phase)

### 6.1 Shape

```text
worker: compute plan ──► if removal_prompt non-empty:
          create tokio::sync::oneshot::channel::<DeletionDecision>()
          insert Sender into PendingDeletions[request_id]
          emit sync://removed_tracks {…}
          await Receiver with timeout(DELETION_PROMPT_TIMEOUT, default 10 min)
          match result:
            Ok(decision) => proceed with decision
            Err(_recv_dropped) => CANCELLED (coordinator removed the entry — §6.3)
            Timeout          => auto-Keep, log warn, proceed
```

The user decision overlaps with nothing (prompt comes before downloads) — deliberately sequential: deletions affect *state*, downloads affect *state*, both mutate the same file, and sequencing them avoids a second conflict class. The 10-minute window is the UX cost; acceptable.

### 6.2 Managed state & exact payloads

```rust
// app/src/decisions.rs
pub struct PendingDeletions(pub Mutex<HashMap<String, oneshot::Sender<DeletionDecision>>>);

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DeletionDecision { Delete, Keep, KeepAndDisable }
// JS sends: { "decision": "delete" | "keep" | "keepAndDisable" }
```

Event `sync://removed_tracks` payload (exact):

```jsonc
{
  "jobId": "b7e1…",
  "requestId": "0192…",                  // uuidv7
  "folder": "C:\\Users\\User\\Music\\Chill Mix 2026",
  "playlistName": "Chill Mix 2026",
  "tracks": [
    { "trackId": "3n3…", "title": "Get Lucky", "artist": "Daft Punk",
      "localFilename": "03 - Daft Punk - Get Lucky.mp3" }
  ],
  "timeoutMs": 600000
}
```

Modal copy (exact strings, frontend contract):
- Title: `“Tracks removed from playlist”`
- Body: `“N tracks in '<playlistName>' are no longer on Spotify. Delete the downloaded files?”` + list of `localFilename`s.
- Buttons: **`Delete files`** → `{decision:"delete"}`; **`Keep files`** → `{decision:"keep"}`; **`Keep files & stop syncing this folder`** → `{decision:"keepAndDisable"}`.
- On any click: disable all buttons, then invoke `confirm_deletions`. Show per-button spinner only until invocation resolves.
- If invocation returns error `request-already-resolved` or `unknown-request`: show toast `“This prompt already expired or was answered.”` (covers double-click after timeout).

Command:

```rust
#[tauri::command]
pub async fn confirm_deletions(
    request_id: String,
    decision: DeletionDecision,          // camelCase "decision" from JS; Tauri v2 handles casing
    state: tauri::State<'_, PendingDeletions>,
) -> Result<(), String> {
    let sender = state.0.lock().await.remove(&request_id)
        .ok_or_else(|| "request-already-resolved".to_string())?;   // double-click → Err, no double effect
    sender.send(decision).map_err(|_| "worker-gone".to_string())?;
    Ok(())
}
```

`resolve_conflict(request_id, choice)` is the mirror command for §4.5, with its own `PendingConflicts` map and `ConflictResolution { SyncHere{folder} } CreateNew | Cancel` (camelCase tag `"kind"`), same take-from-map semantics.

### 6.3 Edge behaviors (all specified)

| Scenario | Behavior |
|---|---|
| **Duplicate clicks / double-decision** | `remove()` from the map is atomic; second call gets `None` → error string returned to JS, no state change. |
| **Timeout (no answer in 10 min)** | Worker treats as `Keep`, logs `warn`, continues the job. The map entry is removed by the worker after timeout (drop sender). Frontend modal closes itself on its own timer (`timeoutMs` in payload). |
| **App closed while waiting** | Process dies; per-track state writes already committed (§9.1); removals hadn't started ⇒ state on disk is exactly "user said nothing" ⇒ next run re-prompts. Consistent by construction. |
| **Job cancelled while modal open** | Coordinator removes `PendingDeletions[request_id]` → drops the `Sender` → worker's `recv()` returns `Err` → worker transitions to `Cancelled`, applies no deletion, writes nothing. Frontend should also close the modal when it receives `download://log {level:"warn", message:"Job cancelled"}` or a `job-cancelled` sentinel — simplest: frontend calls `cancel_job` when the user closes the modal without answering, which produces the same path. |
| **Two jobs wanting prompts** | Impossible: single worker (§7.2), one prompt at a time. The maps are per-request-id anyway. |
| **Frontend not loaded / event missed** | Timeout → Keep. Safe default is "keep files"; never delete on silence. |

---

## 7. Job orchestration

### 7.1 The `Downloader` trait (the swappable boundary)

```rust
// crates/dl-adapter/src/lib.rs  (trait lives here or in core; core must not depend on it)
pub use core::meta::TrackMeta;

#[async_trait::async_trait]
pub trait Downloader: Send + Sync + 'static {
    /// Resolve + download ONE track to `dest_path`.
    /// Contract:
    ///   * `dest_path` is the FINAL absolute path; parent dir exists.
    ///   * Implementation writes to its own temp file in the same directory
    ///     (e.g. `.~dl-<uuid>-<name>.part`), then sync_all + renames onto dest_path
    ///     (Windows: replace semantics; retry sharing violations like §2.3).
    ///   * On failure: no partial file at dest_path; implementation removes its temp file.
    ///   * Implementation must not touch any other file in the directory.
    ///   * progress_cb is invoked from arbitrary threads; cheap, non-blocking.
    async fn download_track(
        &self,
        meta: &TrackMeta,
        dest_path: &Path,
        progress_cb: ProgressSink,
    ) -> Result<DownloadOutcome, DownloaderError>;
}

pub type ProgressSink = std::sync::Arc<dyn Fn(TrackProgress) + Send + Sync>;

pub struct TrackMeta {
    pub id: String,
    pub title: String,
    pub artists: Vec<String>,
    pub album: String,
    pub track_number: u32,
    pub disc_number: u32,
    pub duration_ms: u64,
    pub isrc: Option<String>,
    pub cover_url: Option<String>,
}

pub struct DownloadOutcome { pub final_path: PathBuf, pub bytes_written: u64, pub container: AudioContainer }

#[derive(Debug)]
pub struct DownloaderError {
    pub class: ErrorClass,       // Transient | Permanent   (drives retry, §8)
    pub message: String,         // already user-safe-ish; adapter maps raw errors
}
pub enum ErrorClass { Transient, Permanent }
```

Tagging (ID3, cover art embedding) is done by the app core **after** `download_track` returns, on the final path, using metadata we already have + a separately fetched cover image. This keeps the trait purely "acquire audio" and keeps rustifydl swappable without touching tag logic.

Metadata (playlists/tracks/albums) comes from a separate `MetadataProvider` trait (`meta` crate) — one line here because §3 depends on it:

```rust
#[async_trait]
pub trait MetadataProvider: Send + Sync {
    async fn playlist_tracks(&self, url: &str) -> Result<Vec<FreshTrack>, MetaError>; // ordered, Option<TrackMeta> per item
    async fn album_tracks(&self, url: &str) -> Result<Vec<TrackMeta>, MetaError>;
    async fn track(&self, url: &str) -> Result<TrackMeta, MetaError>;
    async fn track_exists(&self, id: &str) -> Result<bool, MetaError>;   // for leftover verification (§3.3)
    async fn playlist_brief(&self, url: &str) -> Result<(String /*id*/, String /*name*/), MetaError>;
}
```

### 7.2 Single worker + mpsc queue

```rust
pub enum JobRequest {
    Start(StartJob),      // StartJob { mode: Single{url,output_dir} | Bulk{urls,master_dir}, settings: SyncSettings }
    Cancel { job_id: String },
}
```

- One long-lived tokio task (`spawn`) owns an `mpsc::Receiver<JobRequest>`. `start_job` sends `Start`; **if a job is running, `start_job` returns `Err("job-already-running")`** (frontend disables the button). No queueing of multiple jobs — YAGNI, and it avoids all interleaved-write questions.
- `cancel_job(job_id)` sends `Cancel`; mismatched job_id → `Err`.
- The worker owns a `CancellationToken` (tokio_util) per job.

### 7.3 Event names & payloads (exact)

All emitted via `AppHandle::emit(name, payload)`; payloads camelCase. Every event carries `jobId` so the frontend can route.

| Event | Payload |
|---|---|
| `download://progress` | `{ jobId, phase: "resolvingMetadata"\|"downloading"\|"tagging"\|"finalizing", current, total, track: {position, title, artist}\|null, bytesDone, bytesTotal, percent }` — `current`/`total` are per-batch track indices (1-based); `percent` 0–100 or `null` for indeterminate phases (`resolvingMetadata` is always indeterminate). |
| `download://log` | `{ jobId, level: "debug"\|"info"\|"warn"\|"error", message, timestamp }` |
| `download://track_failed` | `{ jobId, trackId, title, artist, position, errorClass: "transient"\|"permanent", message, attempts }` |
| `sync://removed_tracks` | §6.2 |
| `sync://conflict` | §4.5 |
| `sync://complete` | `{ jobId, folder, added, skipped, redownloaded, adopted, removed, kept, keptRemoved, failed: [TrackFailed…], untracked: [String], durationMs }` |

Indeterminate phases: `resolvingMetadata` (whole fresh list, no per-track counts), and per-track source resolution inside rustifydl when it must search YouTube (bytes unknown → `percent: null`, `bytesTotal: null`; spinner, not progress bar).

### 7.4 Panic isolation per track

Each track's download runs in `tokio::spawn(...)`; the worker `await`s the `JoinHandle`:

```rust
match tokio::spawn(job_item).await {
    Ok(Ok(outcome)) => { tag; write state; }
    Ok(Err(e))      => { classify/retry/log (§8); continue; }
    Err(join_err) if join_err.is_panic() => {
        log track_failed with errorClass "permanent", message "internal error (panic): {payload}";
        continue;                       // batch continues
    }
    Err(_cancelled) => break,           // cancelled
}
```

A panicking track must not poison the worker. State written *before* the spawn (the "current position" is implicitly in `entries`) so no state corruption; a panic'd track's temp file is cleaned by §9.2's sweep since the adapter's cleanup runs in its own task — belt-and-braces: the pre-execution sweep (step 9 of §3.6, executed at job start too) removes orphans.

### 7.5 Per-folder lock

Two layers:

1. **In-memory:** `FolderLocks(Mutex<HashMap<PathBuf /*canonical*/, String /*job_id*/>>)`. Acquired for every folder a job touches (master + each playlist/album folder), held for the folder's whole sync, released at folder completion. Prevents two *logical* operations in one process from touching one folder (future-proofing for multi-job).
2. **Lock file (cross-process):** `<folder>/.folder.lock`, atomic-write (§2.3 helper, same code path) containing `{ "pid": 4242, "acquiredAt": "…", "host": "DESKTOP-…" }`.

**Stale-lock rule:** on acquire, if `.folder.lock` exists: read it; if `pid` is not alive on this host (`sysinfo` or `tasklist` probe; keep it cheap — one probe) OR `acquiredAt` older than 24 h ⇒ stale ⇒ log `warn "stale lock stolen (created X by pid Y)"`, overwrite. Otherwise ⇒ `Err` → job reports per-item failure `folder-locked`: *"'<folder>' is locked by another download (pid 4242). Close it or delete .folder.lock if that's stale."* Lock file removed on completion; if removal fails (AV lock), leave it — the stale rule cleans it next time.

---

## 8. Error strategy

### 8.1 Classification & policy

| Class | Examples | Policy |
|---|---|---|
| **Transient** | network timeout, DNS flake, HTTP 5xx/429, yt-dlp transient exit codes, disk temporarily unavailable (sharing violation after retries) | retry: **3 attempts total**, backoff **2 s / 8 s / 30 s ±25 % jitter**; after final failure → `track_failed`, batch continues |
| **Permanent** | malformed URL, 404, private/nonexistent playlist (metadata layer), track unavailable, disk full, state file newer-version, invalid path after sanitize | **no retry**; playlist-level: abort that item, continue bulk; track-level: log + continue |
| **Fatal** | output root not writable, worker panic outside a track, two jobs racing (shouldn't happen), out-of-memory | abort job; state stays consistent (per-track writes); `sync://complete` with `failed` and a fatal banner |

- **Playlist-level failure** (bad URL, private playlist): item marked failed with a clear message; in bulk, the run continues with remaining URLs. Message wording: *"Couldn't access 'https://…'. It may be private, deleted, or the link is invalid."*
- **Per-track failure**: *"Failed: <artist> – <title> (<reason>). Continuing with the rest."* Reason strings are pre-baked, never raw library dumps: `"network error"`, `"no source found"`, `"track unavailable on Spotify"`, `"disk error: <io-message>"`, `"internal error"`.
- Retries happen *inside* one track slot (progress UI stays on that track); the per-track retry loop checks the cancellation token between attempts.
- **No DRM bypass.** Audio is acquired exclusively via `yt-dlp`/`rustifydl` (public sources, best-effort match by ISRC/title/artist). Track metadata comes from Spotify's API. This is a product/legal constraint, restated in non-goals.

---

## 9. Crash & consistency rules

1. **State write cadence:** immediately after each successful track (download + tag complete): append/patch entry, `write_state_atomic`. Cost is trivial (file ≈ tens of KB). Never batch state writes; they are the recovery log.
2. **Temp files:** all in-flight audio lives as `.~dl-<uuid>-<name>.part` in the destination folder. Sweep at job start (for that folder) and at job end: delete `.~dl-*.part` and `.playlist_sync.json.*.tmp` older than 1 hour (age guard avoids nuking a concurrently running other instance — the per-folder lock file also covers this, the age guard is defense-in-depth).
3. **Partial MP3s:** impossible at the final path by construction (temp+rename contract, §7.1). Tag failure after rename ⇒ file kept, entry written with `status: "ok"` and a `warn` log (untagged file is usable); re-tagging is a future feature, not a sync concern.
4. **Reconcile-on-startup (optional setting, default off):** `reconcile_on_startup: bool`. When on: a lightweight job at app start walks the output root (same walkdir rules as §4.2, depth 4), loads every state file, and emits `download://log` summaries per folder: `"Chill Mix 2026: 47/47 present"` / `"Chill Mix 2026: 3 entries missing files (would redownload on next sync)"`. **It never modifies anything** — reconciliation proper is just re-running the URL, which is the normal flow. This is a health report, not a repair.
5. **Zero-byte or unreadable referenced file** ⇒ `CORRUPT` ⇒ redownload (§3.2). Caught by the `FsMap` + size check at diff time.
6. **Kill-during-download** ⇒ orphan `.part` (cleaned per rule 2), state untouched, final file absent ⇒ next run: `REDOWNLOAD`. Consistent.

---

## 10. Test plan

### 10.1 Unit tests (`crates/core/tests/`, all with fixture JSON + tempdir)

Diff classification (`diff_tests.rs`):
1. `first_sync_empty_state` — empty state + 10 fresh tracks ⇒ 10 × NEW.
2. `unchanged_rerun` — state matches fresh, all files present ⇒ 10 × SKIP, zero writes beyond `updated_at`.
3. `one_track_added` — fresh has 11, state 10, new at end ⇒ 10 SKIP + 1 NEW at position 11.
4. `one_track_removed_then_delete_decision` — leftover entry ⇒ REMOVED_PROMPT; simulate Delete ⇒ file gone, entry gone, state has 10.
5. `one_track_removed_then_keep_decision` ⇒ entry `status=kept_removed`; next diff ⇒ KEPT_SKIP, no prompt.
6. `keep_and_disable` ⇒ `sync_disabled=true`; diff of disabled state yields `Disabled` short-circuit (assert worker-level behavior).
7. `file_missing_redownload` — entry present, file deleted ⇒ REDOWNLOAD (not NEW); assert plan distinguishes.
8. `renamed_file_adopted` — entry says `03 - A - B.mp3`, disk has `03 - A - B (fixed).mp3`, no other untracked ⇒ ADOPT_RENAME; assert state update only.
9. `ambiguous_rename_not_adopted` — two loose candidates ⇒ REDOWNLOAD.
10. `duplicate_track_two_positions_both_kept` — T at pos 3, 9 in state and fresh ⇒ both SKIP.
11. `duplicate_track_first_removed` — state pos 3+9, fresh pos 9 only ⇒ `03` file survives, `09` entry REMOVED_PROMPT (assert pairing order).
12. `untracked_files_reported_only` — stray files ⇒ untracked list, plan has no deletions.
13. `unavailable_track_still_exists` — leftover + per-track lookup true ⇒ UNAVAILABLE_KEEP, no prompt, flag set.
14. `zero_byte_file` ⇒ CORRUPT ⇒ REDOWNLOAD.
15. `removed_file_and_track_gone_after_crash_between_delete_and_write` — entry references missing file, track ∉ fresh ⇒ REMOVED_PROMPT (verifies §3.6 crash analysis).

Sanitizer (`naming_tests.rs`):
16. `illegal_chars_replaced` — `AC/DC: Back? *<>"|` ⇒ underscores, no illegal chars remain.
17. `reserved_names` — `CON`, `con.txt`, `LPT1`, `NUL` ⇒ `CON_`, `con_.txt`, `LPT1_`, `NUL_`.
18. `trailing_dots_spaces` — `"name. . "` ⇒ `"name"`; write-then-read name equality (create the file in tempdir to prove consistency).
19. `truncation_boundary` — 150-char emoji title truncated at 100 without splitting a surrogate; re-sanitize is idempotent.
20. `empty_after_sanitize` ⇒ `"Untitled"`.
21. `leading_dot` ⇒ `_`-prefixed.
22. `nfc_normalization` — NFD input ⇒ NFC output, byte-identical across runs.
23. `collision_suffixing` — resolve twice in same dir ⇒ `X.mp3`, `X (1).mp3`; case variant `x.mp3` on disk still suffixes on Windows-semantics map.

Nested scan (`scan_tests.rs`):
24. `finds_match_at_depth_3`.
25. `depth_limit_4_prunes_deeper`.
26. `skips_junctions` (Windows-only test: create a junction to the root itself ⇒ no infinite loop, no false match).
27. `skips_hidden_and_recycle`.
28. `target_folder_not_a_conflict` (§4.4 rule A).
29. `renamed_folder_still_matches_by_playlist_id`.
30. `corrupt_state_not_a_match_but_newer_version_file_is` (id-parse fallback).

State migration (`state_tests.rs`):
31. `v1_roundtrip_all_fields`.
32. `missing_fields_defaulted` (strip each field from fixture, parse, defaults applied).
33. `unknown_fields_ignored_and_preserved_on_raw_level` (unknown keys don't break parse; note: they are NOT preserved on rewrite — document in test comment as accepted).
34. `newer_version_rejected_on_write` (v2 file ⇒ write refused).
35. `bom_and_corrupt_handling` (backup file created, `Fresh` returned).

### 10.2 Manual test matrix (Windows, real Spotify + network)

| # | Scenario | Steps | Expected |
|---|---|---|---|
| M1 | First download | fresh output dir, playlist URL | new folder named after playlist, numbered tracks, state file with N entries, all files tagged |
| M2 | Re-run unchanged | same URL, same folder | all SKIP; summary `added 0 skipped N`; no prompts |
| M3 | Add track on Spotify | add 1 track server-side, re-run | 1 NEW appended with next position number; existing files untouched (mtimes unchanged) |
| M4 | Remove track → Delete | remove 1 server-side, re-run, choose Delete | modal lists the file; file deleted, entry removed, state consistent; run completes |
| M5 | Remove track → Keep | same, choose Keep | file kept, entry `kept_removed`; next run no prompt (M2-style summary shows keptRemoved: 1) |
| M6 | Remove track → KeepAndDisable | same, choose disable | file kept, `sync_disabled=true`; re-run ⇒ warning prompt offering re-enable/fresh folder |
| M7 | Move folder | move playlist folder to another drive, re-run URL | nested-scan finds it by playlist_id; "Sync there" retargets; sync works (bare-filename state) |
| M8 | Rename folder | rename, re-run | as M7, UI shows old playlist name + new path |
| M9 | Duplicate URL, another folder | re-run URL with different output root | conflict modal naming the existing folder; all three buttons behave per §4.4 |
| M10 | Bulk mixed | master folder + 2 loose + 1 album + 1 playlist | loose in master, album & playlist as subfolders; sequential downloads; single completion summary |
| M11 | Invalid / private playlist | bulk with a bad URL first | per-item failure message, remaining items proceed, summary lists failure |
| M12 | Network failure mid-batch | disconnect during track 3/5 | 3 retries with backoff on track 3 → `track_failed`, track 4–5 proceed, summary `failed: 1`; reconnect + re-run ⇒ REDOWNLOAD only track 3 |
| M13 | Prompt timeout | trigger removal modal, ignore 10 min (or dev-shrunk timer) | auto-Keep logged, run completes, modal auto-closed |
| M14 | Double-click confirm | click Delete twice fast | one deletion, second click gets "already resolved" toast |
| M15 | Cancel during downloads | start long playlist, cancel mid-track | current track's `.part` removed or swept next run; state contains only completed tracks; no deletions applied if modal was open |
| M16 | App kill mid-download | Task Manager kill | next run: sweep removes `.part`, redownloads missing, no orphan final files |
| M17 | Reserved-name track | track titled `CON` | file `CON_.mp3` created and referenced in state |
| M18 | Long path | root deep in nested dirs + long playlist name | files created (via `\\?\` fallback), state stores bare names, re-run works |
| M19 | Case-collision | existing `03 - a - b.mp3`, new track resolves to same stem case-insensitively | suffixed `(1)` file, no overwrite |
| M20 | Stale lock | create `.folder.lock` with pid 999999 | warn + steal, sync proceeds |

---

## 11. Non-goals & risks

### 11.1 Explicit non-goals (Phase 2)

- **No DRM circumvention / no Spotify audio stream capture.** Audio via public sources (yt-dlp/rustifydl) only; no interaction with Spotify's encrypted stream.
- No concurrent multi-job downloads, no download queue UI.
- No automatic deletion without confirmation, ever (that's a product rule, not a tech limitation).
- No renumbering of existing files on playlist changes.
- No lyric fetching, no replay-gain/normalization, no bitrate/format selection UI in v1 (container fixed in settings, default MP3).
- No cross-machine state sync; no SQLite migration (JSON state is sufficient at playlist scale).
- Android: deferred; all Windows-specific code is `cfg`-gated so it's not load-bearing elsewhere.
- No "adopt untracked files" flow beyond the bounded rename-recovery (§3.4).

### 11.2 Top 3 risks & mitigations

1. **`rustifydl` API mismatch / churn** — the adapter crate exists precisely for this; the `Downloader` trait + `ErrorClass` mapping is a ≤200-line surface. Mitigation: write contract tests against a `MockDownloader` first, keep the adapter behind one integration point, and time-box a "what if rustifydl can't do X" fallback (e.g. shell out to yt-dlp directly as a second adapter — the trait makes that a drop-in).
2. **Windows filesystem edge cases** (long paths, reserved names, AV locking renames, junction loops) — mitigated by §5 (sanitizer + `\\?\` fallback + retry on rename) and §4.2 (reparse pruning). Residual risk: enterprise AV that holds files for seconds; the 100/300/800 ms retry ladder plus classifying persistent locks as transient-track-failure keeps the batch alive.
3. **Confirmation-flow races** (timeout vs click, cancel vs click, app exit) — mitigated by the take-from-map oneshot pattern (§6.2–6.3) with safe defaults (Keep on silence, Cancel on conflict silence) and the crash analysis in §3.6/§3.7. The double-click case is structurally impossible to double-apply.

(Noted but not top-3: Spotify API rate limits/auth — mitigated by batching leftover lookups and treating 429 as transient with the standard backoff; auth model is an escalation below.)

---

## Escalations (decisions needed from the orchestrator)

1. **Spotify auth model.** Private playlists (a stated use case for sync) require **user OAuth**, not client-credentials. This adds a login flow + token storage (Tauri plugin or local server callback). The sync architecture is auth-agnostic, but the metadata crate needs a decision before Phase 3. Default proposal: client-credentials for public + optional user OAuth for private.
2. **Audio container.** Filenames and state `local_filename` pin the extension. Default proposal: MP3 (universal tag support, yt-dlp transcode trivially). If we want OPUS/M4A options later, state entries keep their original ext (already specified) but the settings UI needs a "container" concept in Phase 3.
3. **"KeepAndDisable" re-enable UX.** Spec'd as: future runs emit a blocking prompt (re-enable / download elsewhere / cancel). Confirm this blocking prompt is acceptable vs. a silent skip-and-log.
4. **Deletion = recycle bin or hard delete?** Spec'd "recycle bin if feasible, else unlink" (`trash` crate handles Windows recycle bin). If product wants hard delete only, drop the crate dependency.
