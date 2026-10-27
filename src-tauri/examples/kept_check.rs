use std::path::PathBuf;

use app_lib::metadata::TrackMeta;
use app_lib::sync;

fn number(name: &str) -> usize {
    name.split_once(" - ")
        .and_then(|(prefix, _)| prefix.parse().ok())
        .unwrap_or(usize::MAX)
}

fn main() {
    let folder = PathBuf::from(r"C:\Users\User\Music\music\UK rap");
    let text = std::fs::read_to_string(folder.join(".playlist_sync.json")).expect("state");
    let mut state: sync::SyncState = serde_json::from_str(&text).expect("parse");
    let playlist_name = state.playlist_name.clone();
    let source_url = state.source_url.clone();

    let mut rows: Vec<(String, String, String)> = state
        .entries
        .iter()
        .filter(|entry| entry.status == sync::EntryStatus::Ok)
        .map(|entry| {
            (
                entry.track_id.clone(),
                entry.title.clone(),
                entry.artist.clone(),
            )
        })
        .collect();
    rows.sort_by_key(|(id, _, _)| {
        let entry = state.entries.iter().find(|entry| &entry.track_id == id).unwrap();
        number(&entry.local_filename)
    });

    let tracks: Vec<TrackMeta> = rows
        .iter()
        .enumerate()
        .map(|(index, (id, title, artist))| TrackMeta {
            id: id.clone(),
            title: title.clone(),
            artists: artist.split(", ").map(str::to_string).collect(),
            position: index + 1,
            track_number: (index + 1) as u32,
            ..TrackMeta::default()
        })
        .collect();

    let plan = sync::plan_playlist(&folder, &playlist_name, &source_url, &tracks, &mut state);
    println!(
        "downloads={} renames={} skipped={} removals={} kept_removed={}",
        plan.downloads.len(),
        plan.renames.len(),
        plan.skipped,
        plan.removals.len(),
        plan.kept_removed
    );
    for rename in &plan.renames {
        println!(
            "  {:?} {} -> {}",
            rename.track_index, rename.from, rename.to
        );
    }
}
