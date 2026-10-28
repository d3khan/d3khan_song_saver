use crate::metadata::TrackMeta;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};
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
        })
        .collect())
}

#[tauri::command(rename_all = "camelCase")]
pub async fn pick_folder(app: AppHandle) -> Result<Option<String>, String> {
    let folder = app
        .dialog()
        .file()
        .set_title("Choose output folder")
        .blocking_pick_folder();
    Ok(folder.map(|p| p.to_string()))
}

#[tauri::command(rename_all = "camelCase")]
pub async fn download_track(
    app: AppHandle,
    tracks: Vec<TrackResult>,
    output_dir: String,
) -> Result<(), String> {
    let out_path = std::path::PathBuf::from(&output_dir);

    let metas: Vec<TrackMeta> = tracks
        .into_iter()
        .map(|t| TrackMeta {
            id: t.id,
            title: t.title,
            artists: t.artist.split(", ").map(|s| s.to_string()).collect(),
            cover_url: t.cover_url,
            release_date: t.year,
            duration_ms: t.duration_ms,
            ..Default::default()
        })
        .collect();

    let app_clone = app.clone();
    // Await the batch download so the frontend knows when it's truly finished
    crate::downloader::download_batch(
        &app_clone,
        metas,
        &out_path,
        |current, total, title, status| {
            let _ = app_clone.emit(
                "download-progress",
                serde_json::json!({
                    "current": current,
                    "total": total,
                    "title": title,
                    "status": status,
                }),
            );
        },
    )
    .await?;

    Ok(())
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
