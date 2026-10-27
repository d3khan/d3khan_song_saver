//! Pure naming helpers: sanitizing, folder/file naming, and case-insensitive
//! collision resolution.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::metadata::TrackMeta;

const FORBIDDEN: &[char] = &['<', '>', ':', '"', '/', '\\', '|', '?', '*'];

const RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Sanitize a single Windows path component.
///
/// Order: replace forbidden/control chars, collapse whitespace, truncate to
/// `max_chars` characters, trim, strip trailing dots/spaces, fall back to
/// "Untitled", escape reserved device names, prefix a leading dot.
pub fn sanitize(input: &str, max_chars: usize) -> String {
    let mut replaced = String::with_capacity(input.len());
    for ch in input.chars() {
        if FORBIDDEN.contains(&ch) || ch.is_control() {
            replaced.push('_');
        } else {
            replaced.push(ch);
        }
    }

    // Collapse runs of whitespace into a single space.
    let mut collapsed = String::with_capacity(replaced.len());
    let mut previous_whitespace = false;
    for ch in replaced.chars() {
        if ch.is_whitespace() {
            if !previous_whitespace {
                collapsed.push(' ');
            }
            previous_whitespace = true;
        } else {
            collapsed.push(ch);
            previous_whitespace = false;
        }
    }

    let mut name: String = collapsed.trim().chars().take(max_chars).collect();
    name = name.trim().to_string();
    while name.ends_with('.') || name.ends_with(' ') {
        name.pop();
    }

    if name.is_empty() {
        return "Untitled".to_string();
    }

    let stem = name
        .split('.')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_uppercase();
    if RESERVED.contains(&stem.as_str()) {
        name.push('_');
    }

    if name.starts_with('.') {
        name.insert(0, '_');
    }

    name
}

pub fn display_artist(artists: &[String]) -> String {
    if artists.is_empty() {
        "Unknown Artist".to_string()
    } else {
        artists.join(", ")
    }
}

pub fn playlist_folder(name: &str) -> String {
    sanitize(name, 64)
}

pub fn album_folder(artist_display: &str, album: &str) -> String {
    sanitize(&format!("{artist_display} - {album}"), 64)
}

pub fn single_track_filename(meta: &TrackMeta) -> String {
    let artist = display_artist(&meta.artists);
    format!(
        "{}.mp3",
        sanitize(&format!("{} - {artist}", meta.title), 120)
    )
}

pub fn playlist_track_filename(meta: &TrackMeta, total_tracks: usize) -> String {
    let width = std::cmp::max(2, digits(total_tracks));
    let artist = sanitize(&display_artist(&meta.artists), 60);
    let title = sanitize(&meta.title, 80);
    let position = if meta.position == 0 { 1 } else { meta.position };
    format!("{position:0width$} - {title} - {artist}.mp3")
}

/// True when a file name starts with a playlist position prefix (`NN - `).
pub fn has_playlist_number_prefix(name: &str) -> bool {
    match name.split_once(" - ") {
        Some((prefix, _)) => !prefix.is_empty() && prefix.chars().all(|ch| ch.is_ascii_digit()),
        None => false,
    }
}

/// File name for a track the user kept after it left the playlist. It carries
/// no position number so it can never shadow a playlist slot.
pub fn removed_track_filename(title: &str, artist: &str) -> String {
    format!(
        "Removed - {} - {}.mp3",
        sanitize(title, 80),
        sanitize(artist, 60)
    )
}

pub fn album_track_filename(meta: &TrackMeta, multi_disc: bool) -> String {
    let artist = sanitize(&display_artist(&meta.artists), 60);
    let title = sanitize(&meta.title, 80);
    let track = meta.track_number;
    if multi_disc {
        format!(
            "{disc:02}-{track:02} - {title} - {artist}.mp3",
            disc = meta.disc_number
        )
    } else {
        format!("{track:02} - {title} - {artist}.mp3")
    }
}

/// Insert ` (n)` before the extension of `name`.
pub fn collision_suffix(name: &str, n: u32) -> String {
    match name.rfind('.') {
        Some(index) if index > 0 => {
            let (stem, extension) = name.split_at(index);
            format!("{stem} ({n}){extension}")
        }
        _ => format!("{name} ({n})"),
    }
}

/// Return a child name that does not collide (case-insensitively) with any
/// existing entry in `dir`. Never called with the state filename.
pub fn unique_child_name(dir: &Path, desired: &str) -> String {
    let existing = existing_names(dir);
    if !existing.contains(&desired.to_lowercase()) {
        return desired.to_string();
    }
    for n in 1..=500u32 {
        let candidate = collision_suffix(desired, n);
        if !existing.contains(&candidate.to_lowercase()) {
            return candidate;
        }
    }
    desired.to_string()
}

/// Return a directory path inside `parent` that does not collide with an
/// existing entry.
pub fn unique_dir(parent: &Path, desired: &str) -> PathBuf {
    let existing = existing_names(parent);
    if !existing.contains(&desired.to_lowercase()) {
        return parent.join(desired);
    }
    for n in 1..=500u32 {
        let candidate = format!("{desired} ({n})");
        if !existing.contains(&candidate.to_lowercase()) {
            return parent.join(candidate);
        }
    }
    parent.join(desired)
}

/// Reuse an existing child directory whose name matches `desired`
/// case-insensitively; otherwise return a collision-safe path.
///
/// Album folders must be stable across runs so already-downloaded tracks are
/// skipped instead of creating a new `Album (2)` folder every time.
pub fn existing_or_unique_dir(parent: &Path, desired: &str) -> PathBuf {
    if let Ok(entries) = std::fs::read_dir(parent) {
        for entry in entries.flatten() {
            if entry.file_name().to_string_lossy().to_lowercase() != desired.to_lowercase() {
                continue;
            }
            if entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
                return entry.path();
            }
            // A file with this name must not be reused as a folder.
            break;
        }
    }
    unique_dir(parent, desired)
}

fn digits(value: usize) -> usize {
    let mut digits = 1;
    let mut remaining = value;
    while remaining >= 10 {
        remaining /= 10;
        digits += 1;
    }
    digits
}

fn existing_names(dir: &Path) -> HashSet<String> {
    let mut names = HashSet::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            names.insert(entry.file_name().to_string_lossy().to_lowercase());
        }
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(title: &str, position: usize) -> TrackMeta {
        TrackMeta {
            id: "id".to_string(),
            title: title.to_string(),
            artists: vec!["Daft Punk".to_string(), "Pharrell".to_string()],
            album: "Random Access Memories".to_string(),
            album_id: None,
            track_number: 8,
            disc_number: 1,
            duration_ms: 369_000,
            position,
            ..TrackMeta::default()
        }
    }

    #[test]
    fn sanitize_strips_forbidden_characters() {
        assert_eq!(sanitize("AC/DC", 64), "AC_DC");
        assert_eq!(sanitize("Back?*<>|\\\"", 64), "Back_______");
    }

    #[test]
    fn sanitize_handles_reserved_and_empty() {
        assert_eq!(sanitize("CON", 64), "CON_");
        assert_eq!(sanitize("con.txt", 64), "con.txt_");
        assert_eq!(sanitize("   ", 64), "Untitled");
        assert_eq!(sanitize("...", 64), "Untitled");
        assert_eq!(sanitize(".hidden", 64), "_.hidden");
    }

    #[test]
    fn sanitize_truncates_and_strips_trailing() {
        assert_eq!(sanitize("abcdefghij", 5), "abcde");
        assert_eq!(sanitize("abc. ", 64), "abc");
    }

    #[test]
    fn playlist_names_are_zero_padded() {
        let mut meta = track("Get Lucky", 3);
        meta.position = 3;
        assert_eq!(
            playlist_track_filename(&meta, 12),
            "03 - Get Lucky - Daft Punk, Pharrell.mp3"
        );
        meta.position = 7;
        assert_eq!(
            playlist_track_filename(&meta, 5),
            "07 - Get Lucky - Daft Punk, Pharrell.mp3"
        );
    }

    #[test]
    fn kept_removed_names_have_no_position_number() {
        assert!(has_playlist_number_prefix("01 - Get Lucky - Daft Punk.mp3"));
        assert!(!has_playlist_number_prefix(
            "Removed - Get Lucky - Daft Punk.mp3"
        ));
        assert!(!has_playlist_number_prefix("01-Get Lucky.mp3"));
        assert_eq!(
            removed_track_filename("Bow Down to the King", "Rustage"),
            "Removed - Bow Down to the King - Rustage.mp3"
        );
    }

    #[test]
    fn album_names_include_disc_when_needed() {
        let mut meta = track("Get Lucky", 0);
        meta.track_number = 8;
        assert_eq!(
            album_track_filename(&meta, false),
            "08 - Get Lucky - Daft Punk, Pharrell.mp3"
        );
        meta.disc_number = 2;
        assert_eq!(
            album_track_filename(&meta, true),
            "02-08 - Get Lucky - Daft Punk, Pharrell.mp3"
        );
    }

    #[test]
    fn collision_suffix_keeps_extension() {
        assert_eq!(collision_suffix("song.mp3", 2), "song (2).mp3");
        assert_eq!(collision_suffix("noext", 1), "noext (1)");
    }

    #[test]
    fn existing_album_dir_is_reused() {
        let root = std::env::temp_dir().join(format!("sd-naming-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("Artist - Album")).expect("dir");
        std::fs::write(root.join("not-a-dir"), b"x").expect("file");

        assert_eq!(
            existing_or_unique_dir(&root, "artist - album"),
            root.join("Artist - Album")
        );
        assert_eq!(
            existing_or_unique_dir(&root, "not-a-dir"),
            root.join("not-a-dir (1)")
        );

        let _ = std::fs::remove_dir_all(&root);
    }
}
