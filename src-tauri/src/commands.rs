use crate::metadata::TrackMeta;
use serde::Serialize;
use std::sync::atomic::AtomicBool;

#[derive(Serialize, Default, Clone, Debug)]
pub struct Settings {
    pub source: String,
}

pub fn resolve_settings(_path: &std::path::Path) -> Settings {
    Settings {
        source: "none".into(),
    }
}

#[derive(Serialize, Clone)]
pub struct TrackResult {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub url: String,
}

#[tauri::command(rename_all = "camelCase")]
pub async fn search_tracks(query: String) -> Result<Vec<TrackResult>, String> {
    let results = crate::downloader::search_soundcloud(&query).await?;
    Ok(results
        .into_iter()
        .map(|(id, t, a)| TrackResult {
            id: id.clone(),
            title: t,
            artist: a,
            url: format!("https://soundcloud.com/tracks/{}", id),
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
pub async fn download_track(url: String, output_dir: String) -> Result<String, String> {
    let track_id = url.split('/').last().unwrap_or("0").to_string();
    let meta = TrackMeta {
        id: track_id,
        title: "Track".into(),
        artists: vec!["Artist".into()],
        album: "".into(),
        album_artists: vec![],
        album_id: None,
        track_number: 1,
        total_tracks: None,
        disc_number: 1,
        duration_ms: 100000,
        release_date: None,
        isrc: None,
        label: None,
        copyright: None,
        genres: vec![],
        explicit: false,
        cover_url: None,
        position: 0,
    };
    let p = std::path::PathBuf::from(output_dir.as_str()).join("track.mp3");
    std::fs::create_dir_all(p.parent().unwrap_or(std::path::Path::new(".")))
        .map_err(|e| format!("folder error: {e}"))?;
    crate::downloader::download_track(&meta, &p, std::sync::Arc::new(AtomicBool::new(false)))
        .await
        .map_err(|e| format!("download error: {e}"))?;
    Ok(p.to_string_lossy().into_owned())
}

#[tauri::command(rename_all = "camelCase")]
pub async fn pick_folder() -> Result<Option<String>, String> {
    Ok(None)
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
#[tauri::command(rename_all = "camelCase")]
pub async fn confirm_deletions(_request_id: String, _decision: String) -> Result<(), String> {
    Ok(())
}
#[tauri::command(rename_all = "camelCase")]
pub async fn resolve_conflict(
    _request_id: String,
    _choice: String,
    _dir: Option<String>,
) -> Result<(), String> {
    Ok(())
}
