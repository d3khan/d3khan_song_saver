use crate::metadata::TrackMeta;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

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
    pub folder_name: Option<String>,
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
            folder_name: t.folder_name,
        })
        .collect())
}

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

#[tauri::command(rename_all = "camelCase")]
pub async fn pick_folder(app: AppHandle) -> Result<Option<String>, String> {
    #[cfg(target_os = "android")]
    {
        let plugin_handle = app.state::<tauri::plugin::PluginHandle<tauri::Wry>>();
        let response = plugin_handle
            .run_mobile_plugin::<serde_json::Value>("pickFolder", serde_json::json!({}))
            .map_err(|e| format!("Plugin error: {}", e))?;

        if let Some(path) = response.get("path").and_then(|v| v.as_str()) {
            Ok(Some(path.to_string()))
        } else {
            Ok(None)
        }
    }

    #[cfg(not(target_os = "android"))]
    {
        use tauri_plugin_dialog::DialogExt;
        let folder = app
            .dialog()
            .file()
            .set_title("Choose output folder")
            .blocking_pick_folder();
        Ok(folder.map(|p| p.to_string()))
    }
}

#[tauri::command(rename_all = "camelCase")]
pub async fn download_track(
    app: AppHandle,
    tracks: Vec<TrackResult>,
    output_dir: String,
    mode: Option<String>,
) -> Result<(), String> {
    // On Android, output_dir is a content:// URI.
    // We pass it directly to the Kotlin plugin to handle.
    let out_path = std::path::PathBuf::from(&output_dir);

    let overwrite = mode.as_deref() == Some("download");

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

    let app_clone = app.clone();
    if let Err(e) = crate::downloader::download_batch(&app_clone, metas, &out_path, overwrite).await
    {
        return Err(format!("Batch download failed: {}", e));
    }

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
