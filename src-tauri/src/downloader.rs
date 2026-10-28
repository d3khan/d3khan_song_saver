use crate::metadata::TrackMeta;
use serde_json::json;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_shell::ShellExt;
use tokio::time::{Duration, sleep};

fn get_cache_path(app: &AppHandle) -> PathBuf {
    let mut path = app
        .path()
        .app_local_data_dir()
        .unwrap_or_else(|_| PathBuf::from("."));
    path.push("search_cache.bin");
    path
}

fn load_cache(app: &AppHandle) -> std::collections::HashMap<String, Vec<TrackMeta>> {
    let path = get_cache_path(app);
    if let Ok(data) = std::fs::read(path) {
        bincode::deserialize(&data).unwrap_or_default()
    } else {
        std::collections::HashMap::new()
    }
}

fn save_cache(app: &AppHandle, cache: &std::collections::HashMap<String, Vec<TrackMeta>>) {
    let path = get_cache_path(app);
    if let Ok(data) = bincode::serialize(cache) {
        let _ = std::fs::write(path, data);
    }
}

pub async fn resolve_input(app: &AppHandle, input: &str) -> Result<Vec<TrackMeta>, String> {
    let trimmed = input.trim();

    if trimmed.contains("open.spotify.com/track/") {
        return fetch_spotify_track(trimmed).await;
    }

    if trimmed.contains("youtube.com") || trimmed.contains("youtu.be") {
        return fetch_youtube_track(trimmed).await;
    }

    let cache_key = trimmed.to_lowercase();
    let mut cache = load_cache(app);

    if let Some(cached_results) = cache.get(&cache_key) {
        return Ok(cached_results.clone());
    }

    let mut search = spotapi::SpotifySearch::new();
    let results = search
        .tracks(trimmed, 10)
        .await
        .map_err(|e| format!("Spotify search failed: {e}"))?;

    if !results.is_empty() {
        let mut mapped = Vec::new();
        for track in results {
            let track_id = track.uri.split(':').last().unwrap_or_default().to_string();
            let track_url = format!("https://open.spotify.com/track/{}", track_id);

            mapped.push(TrackMeta {
                id: track_url,
                title: track.name,
                artists: track.artists,
                album: track.album_name,
                duration_ms: track.duration_ms as u64,
                cover_url: None,
                ..Default::default()
            });
        }

        if !mapped.is_empty() {
            cache.insert(cache_key, mapped.clone());
            save_cache(app, &cache);
        }
        return Ok(mapped);
    }

    Err("No tracks found on Spotify for your query.".into())
}

async fn fetch_youtube_track(youtube_url: &str) -> Result<Vec<TrackMeta>, String> {
    let encoded = urlencoding::encode(youtube_url);
    let oembed_url = format!("https://www.youtube.com/oembed?url={}&format=json", encoded);
    let client = reqwest::Client::new();

    let resp = client
        .get(&oembed_url)
        .header(
            "User-Agent",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36",
        )
        .send()
        .await
        .map_err(|e| format!("Network error: {e}"))?;

    let json: serde_json::Value = resp.json().await.unwrap_or_default();

    let title = json["title"]
        .as_str()
        .unwrap_or("YouTube Video")
        .to_string();
    let artist = json["author_name"]
        .as_str()
        .unwrap_or("YouTube")
        .to_string();
    let cover_url = json["thumbnail_url"].as_str().map(|s| s.to_string());

    return Ok(vec![TrackMeta {
        id: youtube_url.to_string(),
        title,
        artists: vec![artist],
        cover_url,
        ..Default::default()
    }]);
}

async fn fetch_spotify_track(spotify_url: &str) -> Result<Vec<TrackMeta>, String> {
    // Fetch the Spotify track page directly (oEmbed does not include artist)
    let client = reqwest::Client::new();

    let resp = client
        .get(spotify_url)
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
        .send()
        .await
        .map_err(|e| format!("Network error: {e}"))?;

    let html = resp
        .text()
        .await
        .map_err(|e| format!("Failed to read page: {e}"))?;

    // Parse og:title for song title
    let title = extract_meta_tag(&html, "property=\"og:title\"")
        .unwrap_or_else(|| "Unknown Title".to_string());

    // Parse og:description for artist (format: "Artist · Album · Song · Year" or similar)
    let description = extract_meta_tag(&html, "property=\"og:description\"").unwrap_or_default();
    let artist = description
        .split(" · ")
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or("Unknown Artist")
        .trim()
        .to_string();

    let cover_url = extract_meta_tag(&html, "property=\"og:image\"");

    return Ok(vec![TrackMeta {
        id: spotify_url.to_string(),
        title,
        artists: if artist.is_empty() || artist == "Unknown Artist" {
            vec!["Unknown Artist".to_string()]
        } else {
            vec![artist]
        },
        album: String::new(),
        release_date: None,
        cover_url,
        duration_ms: 0,
        ..Default::default()
    }]);
}

fn extract_meta_tag(html: &str, prop: &str) -> Option<String> {
    let prop_str = prop.trim();
    // Find the content attribute value in the meta tag that contains this prop
    let search_key = format!("{} content=\"", prop_str);
    let start_idx = html.find(&search_key)? + search_key.len();
    let end_idx = html[start_idx..].find("\"")?;
    Some(html[start_idx..start_idx + end_idx].to_string())
}

pub async fn download_batch(
    app: &AppHandle,
    tracks: Vec<TrackMeta>,
    output_dir: &Path,
) -> Result<(), String> {
    let total = tracks.len();

    let app_data = app
        .path()
        .app_local_data_dir()
        .unwrap_or_else(|_| PathBuf::from("."));
    let mut ffmpeg_path = app_data.join("ffmpeg.exe");
    if !ffmpeg_path.exists() {
        ffmpeg_path = app_data.join("bin").join("ffmpeg.exe");
    }

    let ffmpeg_str = if ffmpeg_path.exists() {
        ffmpeg_path.to_string_lossy().to_string()
    } else {
        "".to_string()
    };

    for (i, meta) in tracks.iter().enumerate() {
        let is_youtube = meta.id.starts_with("http")
            && (meta.id.contains("youtube.com") || meta.id.contains("youtu.be"));

        let query = if is_youtube {
            meta.id.clone()
        } else {
            format!("{} {}", meta.title, meta.artists.join(", "))
        };

        let safe_title = meta
            .title
            .replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_");
        let safe_artist = meta
            .artists
            .join(", ")
            .replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_");
        let out_base = output_dir.join(format!("{} - {}", safe_title, safe_artist));

        let _ = app.emit(
            "download-progress",
            json!({
                "percent": (i as f64 / total as f64) * 100.0,
                "status": format!("Starting {}...", meta.title)
            }),
        );

        let sidecar_command = app
            .shell()
            .sidecar("downloader")
            .map_err(|e| format!("Failed to find sidecar: {e}"))?
            .args([
                query.as_str(),
                out_base.to_string_lossy().as_ref(),
                ffmpeg_str.as_str(),
                meta.title.as_str(),
                meta.artists.join(", ").as_str(),
            ]);

        let (mut rx, _child) = sidecar_command
            .spawn()
            .map_err(|e| format!("Failed to spawn: {e}"))?;

        let mut downloaded_path: Option<PathBuf> = None;
        let mut error_msg: Option<String> = None;

        while let Some(event) = rx.recv().await {
            if let tauri_plugin_shell::process::CommandEvent::Stdout(line) = event {
                let text = String::from_utf8_lossy(&line);
                if text.contains("\"status\": \"success\"") {
                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                        if let Some(path_str) = json["path"].as_str() {
                            downloaded_path = Some(PathBuf::from(path_str));
                        }
                    }
                    break;
                } else if text.contains("\"status\": \"error\"") {
                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                        error_msg = Some(
                            json["message"]
                                .as_str()
                                .unwrap_or("Unknown error")
                                .to_string(),
                        );
                    }
                    break;
                }
            }
        }

        if let Some(err) = error_msg {
            let _ = app.emit(
                "download-error",
                json!({ "error": format!("Failed to download {}: {}", meta.title, err) }),
            );
            continue;
        }

        if let Some(path) = downloaded_path {
            if path.exists() {
                // Python already embedded the cover art and tagged the file.
                // Rust just cleans up the leftover backup .jpg
                if let Err(e) = tag_file(app, &path, meta).await {
                    eprintln!("Failed to finalize {}: {}", meta.title, e);
                }
            }
        }

        let _ = app.emit(
            "download-progress",
            json!({
                "percent": ((i as f64 + 1.0) / total as f64) * 100.0,
                "status": format!("Finished {}", meta.title)
            }),
        );

        if i < total - 1 {
            sleep(Duration::from_secs(5)).await;
        }
    }

    let _ = app.emit(
        "download-progress",
        json!({ "percent": 100.0, "status": "All done!" }),
    );
    Ok(())
}

async fn tag_file(app: &AppHandle, path: &Path, _meta: &TrackMeta) -> Result<(), String> {
    let jpg_path = path.with_extension("jpg");
    if jpg_path.exists() {
        if std::fs::remove_file(&jpg_path).is_ok() {
            let _ = app.emit(
                "download-log",
                json!({ "message": format!("Cleaned backup .jpg: {}", jpg_path.display()) }),
            );
        }
    }
    Ok(())
}
