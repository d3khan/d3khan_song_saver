use crate::metadata::TrackMeta;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::Path;

#[derive(Serialize, Deserialize, Default, Clone)]
pub struct PlaylistState {
    pub playlist_name: String,
    pub downloaded_tracks: HashSet<String>,
    pub manifest: std::collections::HashMap<String, String>,
}

pub fn sync_key(meta: &TrackMeta, mode: &str) -> String {
    format!("{}::{}", meta.id, mode)
}

pub fn display_name(meta: &TrackMeta) -> String {
    match (meta.folder_name.as_deref(), meta.playlist_url.as_deref()) {
        (Some(n), Some(u)) if !n.is_empty() && !u.is_empty() => format!("{n} || {u}"),
        (Some(n), _) => n.to_string(),
        (None, Some(u)) if !u.is_empty() => format!(" || {u}"),
        _ => String::new(),
    }
}

pub fn load_or_new(path: &Path, meta: &TrackMeta) -> PlaylistState {
    if path.exists() {
        if let Ok(text) = std::fs::read_to_string(path) {
            if let Ok(s) = serde_json::from_str::<PlaylistState>(&text) {
                return s;
            }
        }
    }
    PlaylistState {
        playlist_name: display_name(meta),
        downloaded_tracks: HashSet::new(),
        manifest: Default::default(),
    }
}

pub fn save(path: &Path, state: &PlaylistState) {
    let _ = std::fs::write(path, serde_json::to_string_pretty(state).unwrap_or_default());
}

pub fn remove_files_from_manifest(state_path: &Path, files: &[String]) {
    if let Ok(text) = std::fs::read_to_string(state_path) {
        if let Ok(mut state) = serde_json::from_str::<PlaylistState>(&text) {
            for p in files {
                state.manifest.retain(|_, v| v != p);
                state
                    .downloaded_tracks
                    .retain(|k| state.manifest.get(k).map(|v| v != p).unwrap_or(true));
            }
            save(state_path, &state);
        }
    }
}

pub fn orphans(state: &PlaylistState, current: &HashSet<String>) -> Vec<String> {
    state
        .manifest
        .iter()
        .filter(|(id, p)| !current.contains(*id) && Path::new(p).exists())
        .map(|(_, p)| p.clone())
        .collect()
}
