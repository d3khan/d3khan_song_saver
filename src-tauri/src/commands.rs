use crate::metadata::{SearchResult, TrackMeta};
use serde::{Deserialize, Serialize};
use std::sync::atomic::AtomicBool;
use tauri::AppHandle;
use tauri_plugin_dialog::DialogExt;

#[derive(Serialize, Default, Clone, Debug)]
pub struct Settings {
    pub source: String,
}

pub fn resolve_settings(_path: &std::path::Path) -> Settings {
    Settings {
        source: "none".into(),
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TrackResult {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub cover_url: Option<String>,
    pub year: Option<String>,
    pub duration_ms: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct DownloadResult {
    pub status: String, // "downloaded" or "skipped"
    pub path: String,
    pub message: String,
}

#[tauri::command(rename_all = "camelCase")]
pub async fn search_tracks(app: AppHandle, query: String) -> Result<Vec<TrackResult>, String> {
    let results = crate::downloader::search_tracks(&app, &query).await?;
    Ok(results.into_iter().map(map_to_track_result).collect())
}

#[tauri::command(rename_all = "camelCase")]
pub async fn resolve_input(app: AppHandle, input: String) -> Result<Vec<TrackResult>, String> {
    let results = crate::downloader::resolve_input(&app, &input).await?;
    Ok(results.into_iter().map(map_to_track_result).collect())
}

fn map_to_track_result(item: SearchResult) -> TrackResult {
    TrackResult {
        id: item.id,
        title: item.title,
        artist: item.artist,
        cover_url: item.cover_url,
        year: item.year,
        duration_ms: item.duration_ms,
    }
}

#[tauri::command(rename_all = "camelCase")]
pub async fn pick_folder(app: AppHandle) -> Result<Option<String>, String> {
    let folder = app
        .dialog()
        .file()
        .set_title("Choose output folder")
        .blocking_pick_folder();
    match folder {
        Some(path) => Ok(Some(path.to_string())),
        None => Ok(None),
    }
}

#[tauri::command(rename_all = "camelCase")]
pub async fn download_track(
    app: AppHandle,
    track: TrackResult,
    output_dir: String,
) -> Result<DownloadResult, String> {
    let safe_title = track
        .title
        .replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_");
    let safe_artist = track
        .artist
        .replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_");
    let file_name = format!("{} - {}.mp3", safe_title, safe_artist);
    let final_path = std::path::PathBuf::from(&output_dir).join(&file_name);

    // 1. DUPLICATE CHECK
    if final_path.exists() {
        return Ok(DownloadResult {
            status: "skipped".to_string(),
            path: final_path.to_string_lossy().into_owned(),
            message: "File already exists in the target folder.".to_string(),
        });
    }

    let meta = TrackMeta {
        id: track.id,
        title: track.title,
        artists: vec![track.artist],
        album: String::new(),
        album_artists: vec![],
        album_id: None,
        track_number: 1,
        total_tracks: None,
        disc_number: 1,
        duration_ms: track.duration_ms,
        release_date: track.year,
        isrc: None,
        label: None,
        copyright: None,
        genres: vec![],
        explicit: false,
        cover_url: track.cover_url,
        position: 0,
    };

    std::fs::create_dir_all(&output_dir).map_err(|e| format!("folder error: {e}"))?;

    crate::downloader::download_track(
        &app,
        &meta,
        &final_path,
        std::sync::Arc::new(AtomicBool::new(false)),
    )
    .await
    .map_err(|e| format!("download error: {e}"))?;

    Ok(DownloadResult {
        status: "downloaded".to_string(),
        path: final_path.to_string_lossy().into_owned(),
        message: "Successfully downloaded.".to_string(),
    })
}

#[tauri::command(rename_all = "camelCase")]
pub async fn get_settings() -> Result<Settings, String> {
    Ok(Settings {
        source: "none".into(),
    })
}

#[tauri::command(rename_all = "camelCase")]
pub async fn save_settings() -> Result<Settings, String> {
    Ok(Settings {
        source: "none".into(),
    })
}
