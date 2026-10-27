use crate::metadata::TrackMeta;
use serde::{Deserialize, Serialize}; // Added Deserialize
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

#[derive(Serialize, Deserialize, Clone, Debug)] // Added Deserialize
pub struct TrackResult {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub url: String,
    pub cover_url: Option<String>,
    pub year: Option<String>,
}

#[tauri::command(rename_all = "camelCase")]
pub async fn search_tracks(query: String) -> Result<Vec<TrackResult>, String> {
    let results = crate::downloader::search_soundcloud(&query).await?;
    Ok(results
        .into_iter()
        .map(|(id, title, artist, cover, year)| TrackResult {
            id: id.clone(),
            title,
            artist,
            url: format!("https://soundcloud.com/tracks/{}", id),
            cover_url: cover,
            year,
        })
        .collect())
}

#[tauri::command(rename_all = "camelCase")]
pub async fn resolve_spotify_link(url: String) -> Result<String, String> {
    let api_url = format!("https://api.song.link/v1-alpha.1/links?url={}", url);
    let resp = reqwest::get(&api_url).await.map_err(|e| e.to_string())?;
    let json: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    if let Some(links) = json.get("linksByPlatform") {
        if let Some(sc) = links.get("soundcloud") {
            if let Some(u) = sc.get("url").and_then(|u| u.as_str()) {
                return Ok(u.to_string());
            }
        }
    }
    Err("No SoundCloud match found on Odesli".into())
}

#[tauri::command(rename_all = "camelCase")]
pub async fn pick_folder(app: AppHandle) -> Result<Option<String>, String> {
    let folder = app
        .dialog()
        .file()
        .set_title("Choose output folder")
        .blocking_pick_folder();

    match folder {
        Some(path) => Ok(Some(path.to_string())), // Fixed FilePath error
        None => Ok(None),
    }
}

#[tauri::command(rename_all = "camelCase")]
pub async fn download_track(track: TrackResult, output_dir: String) -> Result<String, String> {
    let safe_title = track
        .title
        .replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_");
    let safe_artist = track
        .artist
        .replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_");
    let file_name = format!("{} - {}.mp3", safe_title, safe_artist);

    let final_path = std::path::PathBuf::from(&output_dir).join(&file_name);

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
        duration_ms: 0,
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

    crate::downloader::download_track_hybrid(
        &meta,
        &final_path,
        std::sync::Arc::new(AtomicBool::new(false)),
    )
    .await
    .map_err(|e| format!("download error: {e}"))?;

    Ok(final_path.to_string_lossy().into_owned())
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

#[tauri::command(rename_all = "camelCase")]
pub async fn start_job() -> Result<String, String> {
    Ok("job-id".into())
}

#[tauri::command(rename_all = "camelCase")]
pub async fn cancel_job(_job_id: String) -> Result<(), String> {
    Ok(())
}
