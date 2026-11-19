use crate::metadata::TrackMeta;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::DialogExt;

// -------------------- Settings --------------------

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Settings {
    #[serde(default = "default_source")]
    pub source: String,
    #[serde(default)]
    pub output_dir: Option<String>,
    #[serde(default = "default_media_mode")]
    pub media_mode: String, // "audio" | "video"
    #[serde(default)]
    pub overwrite: bool,
}

fn default_source() -> String {
    "none".into()
}
fn default_media_mode() -> String {
    "audio".into()
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            source: "none".into(),
            output_dir: None,
            media_mode: "audio".into(),
            overwrite: false,
        }
    }
}

pub fn resolve_settings(path: &std::path::Path) -> Settings {
    if path.exists() {
        if let Ok(text) = fs::read_to_string(path) {
            if let Ok(s) = serde_json::from_str(&text) {
                return s;
            }
        }
    }
    Settings::default()
}

#[tauri::command(rename_all = "camelCase")]
pub async fn get_settings(app: AppHandle) -> Result<Settings, String> {
    let path = app
        .path()
        .app_config_dir()
        .map_err(|e| format!("Cannot resolve config dir: {e}"))?
        .join("settings.json");
    Ok(resolve_settings(&path))
}

#[tauri::command(rename_all = "camelCase")]
pub async fn save_settings(app: AppHandle, settings: Settings) -> Result<Settings, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| format!("Cannot resolve config dir: {e}"))?;
    fs::create_dir_all(&dir).map_err(|e| format!("Cannot create config dir: {e}"))?;
    let path = dir.join("settings.json");
    let text = serde_json::to_string_pretty(&settings)
        .map_err(|e| format!("Serialize settings: {e}"))?;
    fs::write(&path, text).map_err(|e| format!("Write settings: {e}"))?;
    Ok(settings)
}

// -------------------- Track result DTO --------------------

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TrackResult {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub cover_url: Option<String>,
    pub year: Option<String>,
    pub duration_ms: u64,
    pub folder_name: Option<String>,
}

// -------------------- Resolve single input --------------------

#[tauri::command(rename_all = "camelCase")]
pub async fn resolve_input(app: AppHandle, input: String) -> Result<Vec<TrackResult>, String> {
    let tracks = crate::downloader::resolve_input(&app, &input).await?;
    Ok(tracks
        .into_iter()
        .map(|t| TrackResult {
            id: t.id,
            title: t.title,
            artist: t.artists.join(", "),
            cover_url: t.cover_url,
            year: t.release_date,
            duration_ms: t.duration_ms,
            folder_name: t.folder_name,
        })
        .collect())
}

// -------------------- Resolve bulk inputs --------------------

#[tauri::command(rename_all = "camelCase")]
pub async fn resolve_bulk_input(
    app: AppHandle,
    inputs: Vec<String>,
) -> Result<Vec<TrackResult>, String> {
    let mut all_tracks = Vec::new();
    for input in inputs {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(tracks) = crate::downloader::resolve_input(&app, trimmed).await {
            all_tracks.extend(tracks);
        }
    }
    Ok(all_tracks
        .into_iter()
        .map(|t| TrackResult {
            id: t.id,
            title: t.title,
            artist: t.artists.join(", "),
            cover_url: t.cover_url,
            year: t.release_date,
            duration_ms: t.duration_ms,
            folder_name: t.folder_name,
        })
        .collect())
}

// -------------------- Fast YouTube query search --------------------

#[tauri::command(rename_all = "camelCase")]
pub async fn search_youtube(
    query: String,
    limit: Option<u32>,
) -> Result<Vec<TrackResult>, String> {
    let metas = crate::downloader::search_youtube(&query, limit.unwrap_or(10)).await?;
    Ok(metas
        .into_iter()
        .map(|t| TrackResult {
            id: t.id,
            title: t.title,
            artist: t.artists.join(", "),
            cover_url: t.cover_url,
            year: t.release_date,
            duration_ms: t.duration_ms,
            folder_name: t.folder_name,
        })
        .collect())
}

// -------------------- Folder picker --------------------

#[tauri::command(rename_all = "camelCase")]
pub async fn pick_folder(app: AppHandle) -> Result<Option<String>, String> {
    let folder = app
        .dialog()
        .file()
        .set_title("Choose output folder")
        .blocking_pick_folder();
    Ok(folder.map(|p| p.to_string()))
}

// -------------------- Download --------------------

#[tauri::command(rename_all = "camelCase")]
pub async fn download_track(
    app: AppHandle,
    tracks: Vec<TrackResult>,
    output_dir: String,
    mode: Option<String>,          // "download" | "sync"  → overwrite behaviour
    media_mode: Option<String>,    // "audio" | "video"    → media type
) -> Result<(), String> {
    let out_path = PathBuf::from(&output_dir);

    // Existing contract: "download" means overwrite; anything else is sync.
    let overwrite = mode.as_deref() == Some("download");

    let media = media_mode
        .as_deref()
        .unwrap_or("audio")
        .to_lowercase();
    let media = if media == "video" { "video" } else { "audio" };

    let metas: Vec<TrackMeta> = tracks
        .into_iter()
        .map(|t| TrackMeta {
            id: t.id,
            title: t.title,
            artists: t.artist.split(", ").map(|s| s.to_string()).collect(),
            cover_url: t.cover_url,
            release_date: t.year,
            duration_ms: t.duration_ms,
            folder_name: t.folder_name,
            ..Default::default()
        })
        .collect();

    crate::downloader::download_batch(&app, metas, &out_path, overwrite, media).await
}

#[tauri::command(rename_all = "camelCase")]
pub async fn delete_orphaned_files(
    files: Vec<String>,
    state_path: String,
) -> Result<(), String> {
    for path_str in &files {
        let p = std::path::Path::new(path_str);
        if p.exists() {
            let _ = std::fs::remove_file(p);
        }
    }
    crate::downloader::remove_files_from_manifest(std::path::Path::new(&state_path), &files);
    Ok(())
}