use crate::metadata::TrackMeta;
use serde::{Deserialize, Serialize};
#[allow(unused_imports)]
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
pub async fn pick_folder(_app: AppHandle) -> Result<Option<String>, String> {
    #[cfg(target_os = "android")]
    {
        // Get the app's private data directory
        // Resolves to something like: /data/user/0/com.d3khan.songsaver/files
        let base_dir = _app
            .path()
            .app_data_dir()
            .map_err(|e| format!("Failed to get Android storage path: {}", e))?;

        // Create a "Music" subfolder inside it
        let music_dir = base_dir.join("Music");

        // Ensure the directory actually exists on the device
        std::fs::create_dir_all(&music_dir)
            .map_err(|e| format!("Failed to create Music folder: {}", e))?;

        // Return the path as a string to the frontend
        return Ok(Some(music_dir.to_string_lossy().to_string()));
    }

    #[cfg(not(target_os = "android"))]
    {
        use tauri_plugin_dialog::DialogExt;
        let folder = _app
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
    let out_path = std::path::PathBuf::from(&output_dir);

    // Overwrite is true only when the frontend explicitly passes "download".
    // Anything else (missing, "sync", "download-but-typo") keeps the safe
    // skip-based behaviour of sync mode.
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
