//! Playlist sync state (`.playlist_sync.json`), atomic persistence, diffing and
//! the nested-folder duplicate scan.

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::metadata::TrackMeta;
use crate::naming;

pub const STATE_FILENAME: &str = ".playlist_sync.json";
const CURRENT_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum EntryStatus {
    #[default]
    #[serde(rename = "ok")]
    Ok,
    #[serde(rename = "kept_removed")]
    KeptRemoved,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SyncEntry {
    pub track_id: String,
    pub local_filename: String,
    pub title: String,
    pub artist: String,
    #[serde(default)]
    pub status: EntryStatus,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
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

fn default_schema_version() -> u32 {
    CURRENT_SCHEMA_VERSION
}

#[derive(Clone, Debug)]
pub struct ScanMatch {
    pub dir: PathBuf,
    pub playlist_name: String,
    pub track_count: usize,
    pub updated_at: Option<String>,
}

#[derive(Clone, Debug)]
pub struct PlannedDownload {
    pub track_index: usize,
    pub filename: String,
    pub entry_index: Option<usize>,
}

#[derive(Clone, Debug)]
pub struct PlannedRename {
    pub entry_index: usize,
    /// Playlist row this file belongs to; `None` for a file kept after its
    /// track left the playlist.
    pub track_index: Option<usize>,
    pub from: String,
    pub to: String,
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
    pub renames: Vec<PlannedRename>,
    pub skipped: usize,
    pub removals: Vec<RemovedTrack>,
    pub kept_removed: usize,
}

pub fn state_path(dir: &Path) -> PathBuf {
    dir.join(STATE_FILENAME)
}

/// True when `name` is a single, plain file name that is safe to join onto a
/// playlist folder: no absolute paths, drive prefixes, `.`/`..`, or separators.
pub fn is_safe_filename(name: &str) -> bool {
    if name.is_empty() || name.contains('\0') || name == STATE_FILENAME {
        return false;
    }

    let mut components = Path::new(name).components();
    matches!(components.next(), Some(std::path::Component::Normal(_)))
        && components.next().is_none()
}

/// Load state for `dir`. Missing file yields a fresh state. A corrupt file is
/// backed up as `.playlist_sync.json.corrupt-<timestamp>` and a fresh state is
/// returned with the boolean set to `true`.
pub fn load_state(dir: &Path, playlist_id: &str) -> (SyncState, bool) {
    let path = state_path(dir);
    let fresh = || SyncState {
        schema_version: CURRENT_SCHEMA_VERSION,
        playlist_id: playlist_id.to_string(),
        playlist_name: String::new(),
        source_url: String::new(),
        updated_at: None,
        entries: Vec::new(),
    };

    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return (fresh(), false),
        Err(error) => {
            eprintln!("[sync] cannot read {}: {error}", path.display());
            return (fresh(), false);
        }
    };

    match serde_json::from_str::<SyncState>(&text) {
        Ok(mut state) => {
            if state.playlist_id.is_empty() {
                state.playlist_id = playlist_id.to_string();
            }
            (state, false)
        }
        Err(error) => {
            eprintln!("[sync] corrupt state {}: {error}", path.display());
            let stamp = chrono::Utc::now().timestamp();
            let backup = dir.join(format!("{STATE_FILENAME}.corrupt-{stamp}"));
            if let Err(rename_error) = std::fs::rename(&path, &backup) {
                eprintln!(
                    "[sync] could not back up corrupt state {}: {rename_error}",
                    path.display()
                );
            }
            (fresh(), true)
        }
    }
}

/// Atomically write the state file (tmp + fsync + rename with retries).
pub fn save_state(dir: &Path, state: &mut SyncState) -> Result<(), String> {
    state.schema_version = CURRENT_SCHEMA_VERSION;
    state.updated_at = Some(chrono::Utc::now().to_rfc3339());
    let path = state_path(dir);
    let json = serde_json::to_string_pretty(state)
        .map_err(|error| format!("Could not serialize playlist state: {error}"))?;
    let tmp = dir.join(format!(".{STATE_FILENAME}.{}.tmp", uuid::Uuid::new_v4()));

    let write_result = (|| -> std::io::Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&tmp)?;
        file.write_all(json.as_bytes())?;
        file.sync_all()?;
        Ok(())
    })();

    if let Err(error) = write_result {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!(
            "Could not write playlist state in {}: {error}",
            dir.display()
        ));
    }

    let mut last_error = None;
    for attempt in 0..5 {
        match std::fs::rename(&tmp, &path) {
            Ok(()) => return Ok(()),
            Err(error) => {
                let retryable = matches!(error.raw_os_error(), Some(32) | Some(33)) && attempt < 4;
                last_error = Some(error);
                if retryable {
                    std::thread::sleep(std::time::Duration::from_millis(60));
                } else {
                    break;
                }
            }
        }
    }

    let _ = std::fs::remove_file(&tmp);
    let error = last_error
        .map(|error| error.to_string())
        .unwrap_or_else(|| "unknown error".to_string());
    Err(format!(
        "Could not replace playlist state in {}: {error}",
        dir.display()
    ))
}

/// Recursively look for another folder that tracks the same playlist.
pub fn scan_for_playlist(root: &Path, playlist_id: &str, exclude: &Path) -> Vec<ScanMatch> {
    let mut matches = Vec::new();
    if !root.exists() {
        return matches;
    }

    let walker = walkdir::WalkDir::new(root)
        .max_depth(6)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| !skip_entry(entry));

    for entry in walker.flatten() {
        if entry.depth() == 0 || !entry.file_type().is_dir() {
            continue;
        }
        let dir = entry.path();
        if same_path(dir, exclude) {
            continue;
        }
        let state_file = state_path(dir);
        if !state_file.is_file() {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&state_file) else {
            continue;
        };
        let Ok(parsed) = serde_json::from_str::<ScanState>(&text) else {
            continue;
        };
        if parsed.playlist_id != playlist_id {
            continue;
        }
        matches.push(ScanMatch {
            dir: dir.to_path_buf(),
            playlist_name: parsed.playlist_name,
            track_count: parsed.entries.len(),
            updated_at: parsed.updated_at,
        });
    }

    matches.sort_by(|a, b| a.dir.cmp(&b.dir));
    matches
}

/// Diff the Spotify track list against the folder state and on-disk files.
pub fn plan_playlist(
    dir: &Path,
    playlist_name: &str,
    source_url: &str,
    tracks: &[TrackMeta],
    state: &mut SyncState,
) -> PlaylistPlan {
    state.schema_version = CURRENT_SCHEMA_VERSION;
    state.playlist_name = playlist_name.to_string();
    if !source_url.is_empty() {
        state.source_url = source_url.to_string();
    }

    let file_map = read_file_map(dir);
    let mut plan = PlaylistPlan::default();
    let total_tracks = tracks.len();

    let mut by_track_id: HashMap<String, VecDeque<usize>> = HashMap::new();
    for (index, entry) in state.entries.iter().enumerate() {
        by_track_id
            .entry(entry.track_id.clone())
            .or_default()
            .push_back(index);
    }

    // Match every playlist row to its state entry first; the matched entries own
    // the files that may be renamed, so their current names are freed below.
    let mut matched = vec![false; state.entries.len()];
    let mut entry_for_track: Vec<Option<usize>> = Vec::with_capacity(total_tracks);
    for track in tracks {
        let index = by_track_id
            .get_mut(&track.id)
            .and_then(|queue| queue.pop_front());
        if let Some(index) = index {
            matched[index] = true;
        }
        entry_for_track.push(index);
    }

    // Names that must stay taken: files no current playlist track owns (removed
    // tracks kept on disk, untracked files) plus every file we cannot account
    // for in the state. Names of matched entries are released because those
    // files may move to a new position number.
    let mut assigned: HashSet<String> = file_map.keys().cloned().collect();
    for (index, entry) in state.entries.iter().enumerate() {
        if matched[index] && is_safe_filename(&entry.local_filename) {
            assigned.remove(&entry.local_filename.to_lowercase());
        }
    }

    // A file kept after its track left the playlist must not hold a position
    // number: otherwise a current track gets renumbered into the same slot and
    // the folder ends up with two `18 - ...` files. Move kept files out of the
    // numbering scheme first so their slot is free for the playlist itself.
    for (index, entry) in state.entries.iter().enumerate() {
        if matched[index] || entry.status != EntryStatus::KeptRemoved {
            continue;
        }
        if !is_safe_filename(&entry.local_filename) {
            continue;
        }
        let Some(actual) = file_map.get(&entry.local_filename.to_lowercase()).cloned() else {
            continue;
        };
        if !naming::has_playlist_number_prefix(&actual) {
            continue;
        }
        let base = naming::removed_track_filename(&entry.title, &entry.artist);
        let desired = unique_planned_name(&base, &assigned);
        assigned.remove(&actual.to_lowercase());
        assigned.insert(desired.to_lowercase());
        plan.renames.push(PlannedRename {
            entry_index: index,
            track_index: None,
            from: actual,
            to: desired,
        });
    }

    for (track_index, track) in tracks.iter().enumerate() {
        let base = naming::playlist_track_filename(track, total_tracks);
        let desired = unique_planned_name(&base, &assigned);
        assigned.insert(desired.to_lowercase());

        match entry_for_track[track_index] {
            Some(index) => {
                let entry = &state.entries[index];
                let existing = if is_safe_filename(&entry.local_filename) {
                    file_map.get(&entry.local_filename.to_lowercase()).cloned()
                } else {
                    None
                };
                let size = existing
                    .as_ref()
                    .and_then(|name| std::fs::metadata(dir.join(name)).ok())
                    .map(|metadata| metadata.len())
                    .unwrap_or(0);

                if let Some(actual) = existing.filter(|_| size > 0) {
                    if actual != desired {
                        // Order, title or artist changed: move the existing file
                        // to the number it has in the playlist right now.
                        plan.renames.push(PlannedRename {
                            entry_index: index,
                            track_index: Some(track_index),
                            from: actual,
                            to: desired,
                        });
                    } else {
                        plan.skipped += 1;
                        if state.entries[index].status != EntryStatus::Ok {
                            state.entries[index].status = EntryStatus::Ok;
                        }
                    }
                } else {
                    plan.downloads.push(PlannedDownload {
                        track_index,
                        filename: desired,
                        entry_index: Some(index),
                    });
                }
            }
            None => {
                plan.downloads.push(PlannedDownload {
                    track_index,
                    filename: desired,
                    entry_index: None,
                });
            }
        }
    }

    for (index, entry) in state.entries.iter().enumerate() {
        if matched[index] {
            continue;
        }
        match entry.status {
            EntryStatus::KeptRemoved => plan.kept_removed += 1,
            EntryStatus::Ok => {
                let exists = file_map.contains_key(&entry.local_filename.to_lowercase());
                plan.removals.push(RemovedTrack {
                    entry_index: index,
                    track_id: entry.track_id.clone(),
                    title: entry.title.clone(),
                    artist: entry.artist.clone(),
                    local_filename: entry.local_filename.clone(),
                    exists,
                });
            }
        }
    }

    plan
}

/// Apply the planned renames in two phases so swaps and shifts (`02 -> 03`
/// while `03 -> 02`) cannot collide. Successful moves update `state` in place;
/// when a move fails, the files that already moved stay renamed and their state
/// entries are updated too, so the next sync never redownloads them.
pub fn apply_renames(
    dir: &Path,
    renames: &[PlannedRename],
    state: &mut SyncState,
) -> Result<usize, String> {
    if renames.is_empty() {
        return Ok(0);
    }

    // Phase 1: move every source aside to a unique temporary name.
    let mut staged: Vec<&PlannedRename> = Vec::with_capacity(renames.len());
    let mut staged_paths: Vec<PathBuf> = Vec::with_capacity(renames.len());
    for (position, rename) in renames.iter().enumerate() {
        let from = dir.join(&rename.from);
        let stage = dir.join(format!(
            ".d3khan-renumber-{position}-{}.tmp",
            uuid::Uuid::new_v4()
        ));
        if let Err(error) = rename_with_retry(&from, &stage) {
            for (index, staged_path) in staged_paths.iter().enumerate().rev() {
                let _ = std::fs::rename(staged_path, dir.join(&staged[index].from));
            }
            return Err(format!("Could not renumber {}: {error}", from.display()));
        }
        staged.push(rename);
        staged_paths.push(stage);
    }

    // Phase 2: move each staged file onto its new name.
    let mut moved = 0;
    for position in 0..staged.len() {
        let rename = staged[position];
        let target = dir.join(&rename.to);
        if target.exists() {
            for index in (position..staged.len()).rev() {
                let _ = std::fs::rename(&staged_paths[index], dir.join(&staged[index].from));
            }
            return Err(format!(
                "Could not renumber to {}: the file already exists",
                target.display()
            ));
        }
        if let Err(error) = rename_with_retry(&staged_paths[position], &target) {
            for index in (position..staged.len()).rev() {
                let _ = std::fs::rename(&staged_paths[index], dir.join(&staged[index].from));
            }
            return Err(format!(
                "Could not renumber to {}: {error}",
                target.display()
            ));
        }
        if let Some(entry) = state.entries.get_mut(rename.entry_index) {
            entry.local_filename = rename.to.clone();
        }
        moved += 1;
    }

    Ok(moved)
}

fn rename_with_retry(from: &Path, to: &Path) -> std::io::Result<()> {
    let mut last_error = None;
    for attempt in 0..5 {
        match std::fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(error) => {
                last_error = Some(error);
                if attempt < 4 {
                    std::thread::sleep(std::time::Duration::from_millis(60));
                }
            }
        }
    }
    Err(last_error.unwrap_or_else(|| std::io::Error::other("unknown rename error")))
}

/// Files in the playlist folder that no state entry tracks (logged, never touched).
pub fn untracked_files(dir: &Path, state: &SyncState) -> Vec<String> {
    let tracked: HashSet<String> = state
        .entries
        .iter()
        .filter(|entry| is_safe_filename(&entry.local_filename))
        .map(|entry| entry.local_filename.to_lowercase())
        .collect();

    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };

    let mut names = Vec::new();
    for entry in entries.flatten() {
        if !entry
            .file_type()
            .map(|file_type| file_type.is_file())
            .unwrap_or(false)
        {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        // State file, backups and temp files are implementation details.
        if name.starts_with('.') {
            continue;
        }
        if !tracked.contains(&name.to_lowercase()) {
            names.push(name);
        }
    }

    names.sort_by_key(|name| name.to_lowercase());
    names
}

/// Lowercase name -> actual on-disk name for regular files.
fn read_file_map(dir: &Path) -> HashMap<String, String> {
    let mut map = HashMap::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            if entry
                .file_type()
                .map(|kind| kind.is_file())
                .unwrap_or(false)
            {
                let name = entry.file_name().to_string_lossy().to_string();
                map.insert(name.to_lowercase(), name);
            }
        }
    }
    map
}

fn unique_planned_name(desired: &str, used: &HashSet<String>) -> String {
    if !used.contains(&desired.to_lowercase()) {
        return desired.to_string();
    }
    for n in 1..=500u32 {
        let candidate = naming::collision_suffix(desired, n);
        if !used.contains(&candidate.to_lowercase()) {
            return candidate;
        }
    }
    desired.to_string()
}

#[derive(Deserialize)]
struct ScanState {
    #[serde(default)]
    playlist_id: String,
    #[serde(default)]
    playlist_name: String,
    #[serde(default)]
    updated_at: Option<String>,
    #[serde(default)]
    entries: Vec<serde_json::Value>,
}

fn skip_entry(entry: &walkdir::DirEntry) -> bool {
    if entry.depth() == 0 {
        return false;
    }
    let name = entry.file_name().to_string_lossy();
    if name.starts_with('.') {
        return true;
    }
    let upper = name.to_ascii_uppercase();
    if upper == "$RECYCLE.BIN" || upper == "SYSTEM VOLUME INFORMATION" {
        return true;
    }
    if entry.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if let Ok(metadata) = entry.metadata() {
            const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
            if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                return true;
            }
        }
    }
    false
}

fn same_path(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("spotify-sync-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    fn track(id: &str, position: usize) -> TrackMeta {
        TrackMeta {
            id: id.to_string(),
            title: format!("Title {id}"),
            artists: vec!["Artist".to_string()],
            album: "Album".to_string(),
            track_number: position as u32,
            disc_number: 1,
            duration_ms: 1000,
            position,
            ..TrackMeta::default()
        }
    }

    #[test]
    fn state_round_trip() {
        let dir = temp_dir();
        let (mut state, corrupt) = load_state(&dir, "playlist1");
        assert!(!corrupt);
        assert_eq!(state.playlist_id, "playlist1");
        state.playlist_name = "My List".to_string();
        state.entries.push(SyncEntry {
            track_id: "t1".to_string(),
            local_filename: "01 - A - B.mp3".to_string(),
            title: "B".to_string(),
            artist: "A".to_string(),
            status: EntryStatus::Ok,
        });
        save_state(&dir, &mut state).expect("save");
        assert!(state.updated_at.is_some());

        let (loaded, corrupt) = load_state(&dir, "playlist1");
        assert!(!corrupt);
        assert_eq!(loaded.entries.len(), 1);
        assert_eq!(loaded.playlist_name, "My List");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_state_is_backed_up() {
        let dir = temp_dir();
        std::fs::write(state_path(&dir), "{not json").expect("write");
        let (state, corrupt) = load_state(&dir, "playlist1");
        assert!(corrupt);
        assert!(state.entries.is_empty());
        let backups: Vec<_> = std::fs::read_dir(&dir)
            .expect("read")
            .flatten()
            .filter(|entry| entry.file_name().to_string_lossy().contains(".corrupt-"))
            .collect();
        assert_eq!(backups.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn plan_skips_existing_and_flags_removals() {
        let dir = temp_dir();
        std::fs::write(dir.join("01 - A - B.mp3"), b"audio").expect("file");
        let mut state = SyncState {
            schema_version: 1,
            playlist_id: "p".to_string(),
            playlist_name: "List".to_string(),
            source_url: String::new(),
            updated_at: None,
            entries: vec![
                SyncEntry {
                    track_id: "t1".to_string(),
                    local_filename: "01 - A - B.mp3".to_string(),
                    title: "B".to_string(),
                    artist: "A".to_string(),
                    status: EntryStatus::Ok,
                },
                SyncEntry {
                    track_id: "t2".to_string(),
                    local_filename: "02 - A - C.mp3".to_string(),
                    title: "C".to_string(),
                    artist: "A".to_string(),
                    status: EntryStatus::KeptRemoved,
                },
                SyncEntry {
                    track_id: "t3".to_string(),
                    local_filename: "03 - A - D.mp3".to_string(),
                    title: "D".to_string(),
                    artist: "A".to_string(),
                    status: EntryStatus::Ok,
                },
            ],
        };

        let tracks = vec![track("t1", 1), track("t4", 2)];
        let plan = plan_playlist(&dir, "List", "url", &tracks, &mut state);
        assert_eq!(plan.skipped, 0);
        assert_eq!(plan.renames.len(), 1);
        assert_eq!(plan.renames[0].from, "01 - A - B.mp3");
        assert_eq!(plan.renames[0].to, "01 - Title t1 - Artist.mp3");
        assert_eq!(plan.downloads.len(), 1);
        assert_eq!(plan.downloads[0].filename, "02 - Title t4 - Artist.mp3");
        assert_eq!(plan.kept_removed, 1);
        assert_eq!(plan.removals.len(), 1);
        assert_eq!(plan.removals[0].track_id, "t3");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn plan_renumbers_when_playlist_order_changes() {
        let dir = temp_dir();
        std::fs::write(dir.join("01 - A - One.mp3"), b"one").expect("one");
        std::fs::write(dir.join("02 - A - Two.mp3"), b"two").expect("two");
        std::fs::write(dir.join("03 - A - Three.mp3"), b"three").expect("three");

        let entry = |id: &str, filename: &str, title: &str| SyncEntry {
            track_id: id.to_string(),
            local_filename: filename.to_string(),
            title: title.to_string(),
            artist: "A".to_string(),
            status: EntryStatus::Ok,
        };
        let mut state = SyncState {
            schema_version: 1,
            playlist_id: "p".to_string(),
            playlist_name: "List".to_string(),
            source_url: String::new(),
            updated_at: None,
            entries: vec![
                entry("t1", "01 - A - One.mp3", "One"),
                entry("t2", "02 - A - Two.mp3", "Two"),
                entry("t3", "03 - A - Three.mp3", "Three"),
            ],
        };

        let mut a = track("t1", 2);
        a.title = "One".to_string();
        a.artists = vec!["A".to_string()];
        let mut b = track("t2", 3);
        b.title = "Two".to_string();
        b.artists = vec!["A".to_string()];
        let mut c = track("t3", 1);
        c.title = "Three".to_string();
        c.artists = vec!["A".to_string()];
        let tracks = vec![c, a, b];
        for (index, track) in tracks.iter().enumerate() {
            assert_eq!(track.position, index + 1);
        }

        let plan = plan_playlist(&dir, "List", "url", &tracks, &mut state);
        assert_eq!(plan.downloads.len(), 0);
        assert_eq!(plan.skipped, 0);
        assert_eq!(plan.renames.len(), 3);

        let moved = apply_renames(&dir, &plan.renames, &mut state).expect("renames");
        assert_eq!(moved, 3);
        assert_eq!(
            std::fs::read(dir.join("01 - Three - A.mp3")).unwrap(),
            b"three"
        );
        assert_eq!(std::fs::read(dir.join("02 - One - A.mp3")).unwrap(), b"one");
        assert_eq!(std::fs::read(dir.join("03 - Two - A.mp3")).unwrap(), b"two");
        assert_eq!(state.entries[0].local_filename, "02 - One - A.mp3");
        assert_eq!(state.entries[1].local_filename, "03 - Two - A.mp3");
        assert_eq!(state.entries[2].local_filename, "01 - Three - A.mp3");

        // A second sync sees the new names as current and plans nothing.
        let plan = plan_playlist(&dir, "List", "url", &tracks, &mut state);
        assert!(plan.renames.is_empty());
        assert!(plan.downloads.is_empty());
        assert_eq!(plan.skipped, 3);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn apply_renames_swaps_files_without_collision() {
        let dir = temp_dir();
        std::fs::write(dir.join("a.mp3"), b"AAA").expect("a");
        std::fs::write(dir.join("b.mp3"), b"BBB").expect("b");
        let mut state = SyncState {
            schema_version: 1,
            playlist_id: "p".to_string(),
            playlist_name: "List".to_string(),
            source_url: String::new(),
            updated_at: None,
            entries: vec![
                SyncEntry {
                    track_id: "t1".to_string(),
                    local_filename: "a.mp3".to_string(),
                    title: String::new(),
                    artist: String::new(),
                    status: EntryStatus::Ok,
                },
                SyncEntry {
                    track_id: "t2".to_string(),
                    local_filename: "b.mp3".to_string(),
                    title: String::new(),
                    artist: String::new(),
                    status: EntryStatus::Ok,
                },
            ],
        };
        let renames = vec![
            PlannedRename {
                entry_index: 0,
                track_index: Some(0),
                from: "a.mp3".to_string(),
                to: "b.mp3".to_string(),
            },
            PlannedRename {
                entry_index: 1,
                track_index: Some(1),
                from: "b.mp3".to_string(),
                to: "a.mp3".to_string(),
            },
        ];

        assert_eq!(apply_renames(&dir, &renames, &mut state), Ok(2));
        assert_eq!(std::fs::read(dir.join("a.mp3")).unwrap(), b"BBB");
        assert_eq!(std::fs::read(dir.join("b.mp3")).unwrap(), b"AAA");
        assert_eq!(state.entries[0].local_filename, "b.mp3");
        assert_eq!(state.entries[1].local_filename, "a.mp3");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn kept_removed_file_cannot_duplicate_a_playlist_number() {
        let dir = temp_dir();
        std::fs::write(dir.join("01 - One - A.mp3"), b"one").expect("one");
        std::fs::write(dir.join("03 - Two - A.mp3"), b"two").expect("two");
        std::fs::write(dir.join("02 - Old - A.mp3"), b"old").expect("old");

        let entry = |id: &str, filename: &str, title: &str, status: EntryStatus| SyncEntry {
            track_id: id.to_string(),
            local_filename: filename.to_string(),
            title: title.to_string(),
            artist: "A".to_string(),
            status,
        };
        let mut state = SyncState {
            schema_version: 1,
            playlist_id: "p".to_string(),
            playlist_name: "List".to_string(),
            source_url: String::new(),
            updated_at: None,
            entries: vec![
                entry("t1", "01 - One - A.mp3", "One", EntryStatus::Ok),
                entry("t2", "03 - Two - A.mp3", "Two", EntryStatus::Ok),
                entry("old", "02 - Old - A.mp3", "Old", EntryStatus::KeptRemoved),
            ],
        };

        let mut one = track("t1", 1);
        one.title = "One".to_string();
        one.artists = vec!["A".to_string()];
        let mut two = track("t2", 2);
        two.title = "Two".to_string();
        two.artists = vec!["A".to_string()];
        let tracks = vec![one, two];

        let plan = plan_playlist(&dir, "List", "url", &tracks, &mut state);
        assert_eq!(plan.kept_removed, 1);
        assert_eq!(plan.skipped, 1);
        assert_eq!(plan.renames.len(), 2);
        assert!(plan.renames.iter().any(|rename| {
            rename.track_index.is_none()
                && rename.from == "02 - Old - A.mp3"
                && rename.to == "Removed - Old - A.mp3"
        }));
        assert!(plan.renames.iter().any(|rename| {
            rename.from == "03 - Two - A.mp3" && rename.to == "02 - Two - A.mp3"
        }));

        assert_eq!(apply_renames(&dir, &plan.renames, &mut state), Ok(2));
        let mut names: Vec<String> = std::fs::read_dir(&dir)
            .expect("read dir")
            .flatten()
            .filter(|item| item.file_type().map(|kind| kind.is_file()).unwrap_or(false))
            .map(|item| item.file_name().to_string_lossy().to_string())
            .collect();
        names.sort();
        assert_eq!(
            names,
            vec![
                "01 - One - A.mp3".to_string(),
                "02 - Two - A.mp3".to_string(),
                "Removed - Old - A.mp3".to_string(),
            ]
        );
        assert_eq!(state.entries[2].status, EntryStatus::KeptRemoved);
        assert_eq!(state.entries[2].local_filename, "Removed - Old - A.mp3");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scan_finds_duplicate_playlist_folder() {
        let root = temp_dir();
        let inner = root.join("Some Playlist");
        std::fs::create_dir_all(&inner).expect("dir");
        let (mut state, _) = load_state(&inner, "p1");
        state.playlist_name = "Some Playlist".to_string();
        save_state(&inner, &mut state).expect("save");

        let matches = scan_for_playlist(&root, "p1", &root.join("other"));
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].dir, inner);

        let excluded = scan_for_playlist(&root, "p1", &inner);
        assert!(excluded.is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn rejects_unsafe_state_filenames() {
        assert!(is_safe_filename("01 - A - B.mp3"));
        assert!(is_safe_filename(".hidden.mp3"));
        assert!(!is_safe_filename(""));
        assert!(!is_safe_filename("."));
        assert!(!is_safe_filename(".."));
        assert!(!is_safe_filename("sub/track.mp3"));
        assert!(!is_safe_filename("sub\\track.mp3"));
        assert!(!is_safe_filename("C:\\Windows\\evil.mp3"));
        assert!(!is_safe_filename(STATE_FILENAME));
    }

    #[test]
    fn plan_regenerates_unsafe_state_filename() {
        let dir = temp_dir();
        let mut state = SyncState {
            schema_version: 1,
            playlist_id: "p".to_string(),
            playlist_name: "List".to_string(),
            source_url: String::new(),
            updated_at: None,
            entries: vec![SyncEntry {
                track_id: "t1".to_string(),
                local_filename: "../escape.mp3".to_string(),
                title: "Title t1".to_string(),
                artist: "Artist".to_string(),
                status: EntryStatus::Ok,
            }],
        };

        let tracks = vec![track("t1", 1)];
        let plan = plan_playlist(&dir, "List", "url", &tracks, &mut state);

        assert_eq!(plan.downloads.len(), 1);
        assert_eq!(plan.downloads[0].filename, "01 - Title t1 - Artist.mp3");
        // The unsafe name is replaced only after the download succeeds.
        assert_eq!(state.entries[0].local_filename, "../escape.mp3");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn untracked_files_exclude_state_and_tracked_audio() {
        let dir = temp_dir();
        std::fs::write(dir.join("01 - A - B.mp3"), b"audio").expect("tracked");
        std::fs::write(dir.join("extra.mp3"), b"extra").expect("extra");
        std::fs::write(state_path(&dir), b"{}").expect("state");

        let mut state = SyncState {
            schema_version: 1,
            playlist_id: "p".to_string(),
            playlist_name: "List".to_string(),
            source_url: String::new(),
            updated_at: None,
            entries: vec![SyncEntry {
                track_id: "t1".to_string(),
                local_filename: "01 - A - B.mp3".to_string(),
                title: "B".to_string(),
                artist: "A".to_string(),
                status: EntryStatus::Ok,
            }],
        };
        state.schema_version = 1;

        assert_eq!(untracked_files(&dir, &state), vec!["extra.mp3"]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
