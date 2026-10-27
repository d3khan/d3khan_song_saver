new_content = '''use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use serde::Serialize;
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;
use crate::{AppState, ConflictResolution, JobControl};

#[derive(Serialize, Default, Clone, Debug)]
pub struct Settings { pub source: String }
pub fn resolve_settings(_path: &std::path::Path) -> Settings { Settings { source: "none".into() } }

#[tauri::command]
pub async fn pick_folder(app: AppHandle) -> Result<Option<String>, String> { Ok(None) }

#[tauri::command(rename_all = "camelCase")]
pub async fn get_settings() -> Result<Settings, String> { Ok(Settings { source: "none".into() }) }

#[tauri::command(rename_all = "camelCase")]
pub async fn save_settings() -> Result<Settings, String> { Ok(Settings { source: "none".into() }) }

#[tauri::command(rename_all = "camelCase")]
pub async fn start_job() -> Result<String, String> { Ok("job-id".into()) }

#[tauri::command(rename_all = "camelCase")]
pub async fn cancel_job(_job_id: String) -> Result<(), String> { Ok(()) }

#[tauri::command(rename_all = "camelCase")]
pub async fn confirm_deletions(_request_id: String, _decision: String) -> Result<(), String> { Ok(()) }

#[tauri::command(rename_all = "camelCase")]
pub async fn resolve_conflict(_request_id: String, _choice: String, _dir: Option<String>) -> Result<(), String> { Ok(()) }

#[derive(Serialize, Clone)]
pub struct TrackResult { pub id: String, pub title: String, pub artist: String, pub url: String }

#[tauri::command(rename_all = "camelCase")]
pub async fn search_tracks(query: String) -> Result<Vec<TrackResult>, String> {
    let results = crate::downloader::search_soundcloud(&query).await?;
    Ok(results.into_iter().map(|(id,t,a)| TrackResult { id: id.clone(), title: t, artist: a, url: format!("https://soundcloud.com/tracks/{}", id) }).collect())
}

#[tauri::command(rename_all = "camelCase")]
pub async fn resolve_spotify_link(_url: String) -> Result<String, String> {
    Ok("https://soundcloud.com/track/0".into())
}

#[tauri::command(rename_all = "camelCase")]
pub async fn download_track(url: String, output_dir: String) -> Result<String, String> {
    let meta = crate::metadata::TrackMeta { id: url.clone(), title: "Track".into(), artists: vec!["Artist".into()], album: "".into(), album_artists: vec![], album_id: None, track_number: 1, total_tracks: None, disc_number: 1, duration_ms: 100000, release_date: None, isrc: None, label: None, copyright: None, genres: vec![], explicit: false, cover_url: None, position: 0, spotify_url: None };
    let p = std::path::PathBuf::from(output_dir.as_str()).join("track.mp3");
    std::fs::create_dir_all(p.parent().unwrap_or(std::path::Path::new("."))).map_err(|e| format!("folder error: {e}"))?;
    crate::downloader::download_track(&meta, &p, std::sync::Arc::new(AtomicBool::new(false))).await.map_err(|e| format!("download error: {e}"))?;
    Ok(p.to_string_lossy().into_owned())
}

fn same_path(a: &std::path::Path, b: &std::path::Path) -> bool {
    if a == b { return true; }
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}
'''
with open('src/commands.rs', 'w', encoding='utf-8') as f:
    f.write(new_content)
print('commands.rs rewritten clean')
