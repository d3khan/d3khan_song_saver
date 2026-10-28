use crate::metadata::TrackMeta;
use regex::Regex;
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
        let mut tasks = Vec::new();
        for track in results {
            let track_id = track.uri.split(':').last().unwrap_or_default().to_string();
            let track_url = format!("https://open.spotify.com/track/{}", track_id);

            tasks.push(tokio::spawn(async move {
                fetch_spotify_track(&track_url).await
            }));
        }

        let mut mapped = Vec::new();
        for task in tasks {
            if let Ok(Ok(mut meta)) = task.await {
                if let Some(m) = meta.pop() {
                    mapped.push(m);
                }
            }
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
    let client = reqwest::Client::new();
    let resp = client.get(spotify_url)
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
        .send().await.map_err(|e| format!("Network error: {e}"))?;

    let html = resp
        .text()
        .await
        .map_err(|e| format!("Failed to read HTML: {e}"))?;

    let title_re = Regex::new(r#"<meta property="og:title" content="([^"]+)""#).unwrap();
    let raw_title = title_re
        .captures(&html)
        .map(|c| c[1].to_string())
        .unwrap_or_else(|| "Unknown Title".into());

    let desc_re = Regex::new(r#"<meta property="og:description" content="([^"]+)""#).unwrap();
    let desc = desc_re
        .captures(&html)
        .map(|c| c[1].to_string())
        .unwrap_or_default();

    let parts: Vec<&str> = desc.split(" · ").collect();
    let artist = if !parts.is_empty() {
        parts[0].to_string()
    } else {
        "Unknown Artist".to_string()
    };
    let year = if parts.len() >= 3 {
        Some(parts[2].to_string())
    } else {
        None
    };

    let mut clean_title = raw_title.clone();
    if clean_title.starts_with(&format!("{} - ", artist)) {
        clean_title = clean_title.replace(&format!("{} - ", artist), "");
    }

    let mut cover_url = None;
    if let Some(caps) = Regex::new(r#"<meta property="og:image" content="([^"]+)""#)
        .unwrap()
        .captures(&html)
    {
        cover_url = Some(caps[1].to_string());
    } else if let Some(caps) =
        Regex::new(r#""images":\[\{"url":"(https://i\.scdn\.co/image/[^"]+)""#)
            .unwrap()
            .captures(&html)
    {
        cover_url = Some(caps[1].to_string());
    } else if let Some(caps) = Regex::new(r#"(https://i\.scdn\.co/image/[a-zA-Z0-9]+)"#)
        .unwrap()
        .captures(&html)
    {
        cover_url = Some(caps[1].to_string());
    }

    let dur_re = Regex::new(r#""duration_ms":(\d+)"#).unwrap();
    let duration_ms = dur_re
        .captures(&html)
        .and_then(|c| c[1].parse::<u64>().ok())
        .unwrap_or(0);

    return Ok(vec![TrackMeta {
        id: spotify_url.to_string(),
        title: clean_title,
        artists: vec![artist],
        album: String::new(),
        release_date: year,
        cover_url,
        duration_ms,
        ..Default::default()
    }]);
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
                let _ = app.emit(
                    "download-progress",
                    json!({
                        "percent": ((i as f64 + 1.0) / total as f64) * 100.0,
                        "status": format!("Finished {}", meta.title)
                    }),
                );

                // Python already embedded cover art and tagged the file.
                // Rust only deletes the leftover backup .jpg if present.
                if let Err(e) = tag_file(app, &path, meta).await {
                    eprintln!("Failed to finalize {}: {}", meta.title, e);
                }
            }
        }

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

#[allow(unreachable_code)]
async fn tag_file(app: &AppHandle, path: &Path, _meta: &TrackMeta) -> Result<(), String> {
    let jpg_path = path.with_extension("jpg");
    if jpg_path.exists() {
        if std::fs::remove_file(&jpg_path).is_ok() {
            let _ = app.emit(
                "download-log",
                json!({ "message": format!("Cleaned backup .jpg: {}", jpg_path.display()) }),
            );
        } else {
            return Ok(()); // this is cuz I expect it to not keep a backup
            let msg = format!("Could not delete backup .jpg: {}", jpg_path.display());
            eprintln!("{}", msg);
            let _ = app.emit("download-log", json!({ "message": msg }));
        }
    }
    Ok(())
}
