use crate::metadata::TrackMeta;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_shell::ShellExt;
use tokio::time::{Duration, sleep};

// -------------------- State Struct for Sync --------------------

#[derive(Serialize, Deserialize, Default)]
struct PlaylistState {
    playlist_name: String,
    downloaded_files: HashSet<String>,
}

// -------------------- Caching --------------------

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

// -------------------- Input Router --------------------

pub async fn resolve_input(app: &AppHandle, input: &str) -> Result<Vec<TrackMeta>, String> {
    let trimmed = input.trim();

    if trimmed.contains("open.spotify.com/track/") {
        return fetch_spotify_track(trimmed).await;
    }

    if trimmed.contains("open.spotify.com/playlist/") || trimmed.contains("open.spotify.com/album/")
    {
        return fetch_spotify_playlist(trimmed).await;
    }

    if (trimmed.contains("youtube.com") || trimmed.contains("youtu.be"))
        && trimmed.contains("list=")
    {
        return fetch_youtube_playlist(trimmed).await;
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
                folder_name: None,
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

// -------------------- Spotify Playlist --------------------

async fn fetch_spotify_playlist(url: &str) -> Result<Vec<TrackMeta>, String> {
    let id = url
        .split('/')
        .last()
        .unwrap_or_default()
        .split('?')
        .next()
        .unwrap_or_default();
    let embed_url = format!("https://open.spotify.com/embed/playlist/{}", id);

    let client = reqwest::Client::new();
    let resp = client
        .get(&embed_url)
        .header(
            "User-Agent",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36",
        )
        .send()
        .await
        .map_err(|e| format!("Network error: {e}"))?;

    let html = resp
        .text()
        .await
        .map_err(|e| format!("Failed to read page: {e}"))?;

    let script_re =
        Regex::new(r#"(?s)<script id="__NEXT_DATA__" type="application/json">(.*?)</script>"#)
            .unwrap();
    let script_match = script_re
        .captures(&html)
        .ok_or("Could not find track list data on the page. The link may be invalid or private.")?;

    let json_str = &script_match[1];
    let json: serde_json::Value =
        serde_json::from_str(json_str).map_err(|e| format!("Failed to parse JSON: {e}"))?;

    let entity = &json["props"]["pageProps"]["state"]["data"]["entity"];
    let playlist_name = entity["name"]
        .as_str()
        .unwrap_or("Unknown Playlist")
        .to_string();
    let track_list = entity["trackList"]
        .as_array()
        .ok_or("This playlist is empty or could not be parsed.")?;

    if track_list.is_empty() {
        return Err("The playlist has no tracks.".to_string());
    }

    let playlist_cover = entity["coverArt"]["sources"]
        .as_array()
        .and_then(|sources| sources.first())
        .and_then(|src| src["url"].as_str())
        .map(|s| s.to_string());

    let mut mapped = Vec::new();
    for (i, track) in track_list.iter().enumerate() {
        let uri = track["uri"].as_str().unwrap_or_default();
        let title = track["title"]
            .as_str()
            .unwrap_or("Unknown Title")
            .to_string();
        let artist = track["subtitle"]
            .as_str()
            .unwrap_or("Unknown Artist")
            .to_string();
        let duration_ms = track["duration"].as_u64().unwrap_or(0);

        if uri.is_empty() {
            continue;
        }

        let track_id = uri.split(':').last().unwrap_or_default();
        let track_url = format!("https://open.spotify.com/track/{}", track_id);

        mapped.push(TrackMeta {
            id: track_url,
            title,
            artists: vec![artist],
            album: String::new(),
            duration_ms,
            cover_url: playlist_cover.clone(),
            track_number: (i + 1) as u32,
            folder_name: Some(playlist_name.clone()),
            ..Default::default()
        });
    }

    Ok(mapped)
}

// -------------------- YouTube Playlist --------------------

async fn fetch_youtube_playlist(url: &str) -> Result<Vec<TrackMeta>, String> {
    let output = std::process::Command::new("yt-dlp")
        .arg("--flat-playlist")
        .arg("--dump-json")
        .arg(url)
        .output()
        .map_err(|e| format!("Failed to run yt-dlp for playlist: {e}"))?;

    if !output.status.success() {
        return Err("yt-dlp failed to fetch YouTube playlist.".to_string());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut mapped = Vec::new();
    let mut playlist_name = "YouTube Playlist".to_string();

    for (i, line) in stdout.lines().enumerate() {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(line) {
            if i == 0 {
                if let Some(p_name) = json["playlist_title"].as_str() {
                    playlist_name = p_name.to_string();
                }
            }

            let video_id = json["id"].as_str().unwrap_or_default();
            if video_id.is_empty() {
                continue;
            }

            let title = json["title"]
                .as_str()
                .unwrap_or("Unknown Title")
                .to_string();
            let uploader = json["uploader"]
                .as_str()
                .or_else(|| json["channel"].as_str())
                .unwrap_or("YouTube")
                .to_string();
            let cover_url = Some(format!("https://i.ytimg.com/vi/{}/hqdefault.jpg", video_id));

            mapped.push(TrackMeta {
                id: format!("https://www.youtube.com/watch?v={}", video_id),
                title,
                artists: vec![uploader],
                cover_url,
                folder_name: Some(playlist_name.clone()),
                ..Default::default()
            });
        }
    }

    if mapped.is_empty() {
        return Err("Could not extract any tracks from the YouTube playlist.".to_string());
    }

    Ok(mapped)
}

// -------------------- Single Tracks --------------------

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

    Ok(vec![TrackMeta {
        id: youtube_url.to_string(),
        title,
        artists: vec![artist],
        cover_url,
        folder_name: None,
        ..Default::default()
    }])
}

async fn fetch_spotify_track(spotify_url: &str) -> Result<Vec<TrackMeta>, String> {
    let client = reqwest::Client::new();

    let resp = client
        .get(spotify_url)
        .header(
            "User-Agent",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36",
        )
        .send()
        .await
        .map_err(|e| format!("Network error: {e}"))?;

    let html = resp
        .text()
        .await
        .map_err(|e| format!("Failed to read page: {e}"))?;

    let title = extract_meta_tag(&html, "property=\"og:title\"")
        .unwrap_or_else(|| "Unknown Title".to_string());
    let description = extract_meta_tag(&html, "property=\"og:description\"").unwrap_or_default();
    let artist = description
        .split(" · ")
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or("Unknown Artist")
        .trim()
        .to_string();
    let cover_url = extract_meta_tag(&html, "property=\"og:image\"");

    Ok(vec![TrackMeta {
        id: spotify_url.to_string(),
        title,
        artists: vec![artist],
        album: String::new(),
        release_date: None,
        cover_url,
        duration_ms: 0,
        folder_name: None,
        ..Default::default()
    }])
}

fn extract_meta_tag(html: &str, prop: &str) -> Option<String> {
    let prop_str = prop.trim();
    let search_key = format!("{} content=\"", prop_str);
    let start_idx = html.find(&search_key)? + search_key.len();
    let end_idx = html[start_idx..].find("\"")?;
    Some(html[start_idx..start_idx + end_idx].to_string())
}

// -------------------- Batch Downloader (With Sync Logic) --------------------

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

    #[cfg(target_os = "windows")]
    let ffmpeg_name = "ffmpeg.exe";
    #[cfg(not(target_os = "windows"))]
    let ffmpeg_name = "ffmpeg";

    let mut ffmpeg_path = app_data.join(ffmpeg_name);
    if !ffmpeg_path.exists() {
        ffmpeg_path = app_data.join("bin").join(ffmpeg_name);
    }
    #[cfg(target_os = "linux")]
    if !ffmpeg_path.exists() {
        ffmpeg_path = PathBuf::from("/usr/lib/d3khan-song-saver/bin").join(ffmpeg_name);
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

        // --- FOLDER & STATE LOGIC ---
        let mut target_dir = output_dir.to_path_buf();
        let mut state_file_path = None;
        let mut state = PlaylistState::default();

        if let Some(folder) = &meta.folder_name {
            let safe_folder = folder.replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_");
            target_dir = target_dir.join(&safe_folder);
            std::fs::create_dir_all(&target_dir)
                .map_err(|e| format!("Failed to create playlist folder: {e}"))?;

            let state_path = target_dir.join(".playlist_sync.json");
            state_file_path = Some(state_path.clone());

            if state_path.exists() {
                if let Ok(content) = std::fs::read_to_string(&state_path) {
                    if let Ok(parsed) = serde_json::from_str::<PlaylistState>(&content) {
                        state = parsed;
                    }
                }
            } else {
                // Scan directory to build initial state
                state.playlist_name = folder.clone();
                if let Ok(entries) = std::fs::read_dir(&target_dir) {
                    for entry in entries.flatten() {
                        if let Some(name) = entry.file_name().to_str() {
                            if name.ends_with(".mp3") || name.ends_with(".mp4") {
                                state.downloaded_files.insert(name.to_string());
                            }
                        }
                    }
                }
                let _ = std::fs::write(&state_path, serde_json::to_string_pretty(&state).unwrap());
            }
        }

        let safe_title = meta
            .title
            .replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_");
        let safe_artist = meta
            .artists
            .join(", ")
            .replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_");

        let ext = if is_youtube { "mp4" } else { "mp3" };
        let expected_filename = format!("{} - {}.{}", safe_title, safe_artist, ext);
        let out_base = target_dir.join(format!("{} - {}", safe_title, safe_artist));

        // --- SKIP IF ALREADY EXISTS ---
        if state.downloaded_files.contains(&expected_filename)
            || out_base.with_extension(ext).exists()
        {
            let _ = app.emit(
                "download-progress",
                json!({
                    "percent": ((i as f64 + 1.0) / total as f64) * 100.0,
                    "status": format!("Skipped (already exists): {}", meta.title)
                }),
            );
            continue;
        }

        let mut success = false;
        let mut attempts = 0;
        let max_attempts = 3;

        while attempts < max_attempts && !success {
            if attempts > 0 {
                let _ = app.emit("download-progress", json!({
                    "percent": (i as f64 / total as f64) * 100.0,
                    "status": format!("Retrying {} (Attempt {}/{})...", meta.title, attempts + 1, max_attempts)
                }));
                sleep(Duration::from_secs(3)).await;
            } else {
                let _ = app.emit(
                    "download-progress",
                    json!({
                        "percent": (i as f64 / total as f64) * 100.0,
                        "status": format!("Starting {}...", meta.title)
                    }),
                );
            }

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
                if err.contains("WinError 32")
                    || err.contains("being used by another process")
                    || err.contains("HTTP Error 416")
                {
                    attempts += 1;
                    if err.contains("HTTP Error 416") {
                        let final_file = out_base.with_extension(ext);
                        if final_file.exists() {
                            let _ = std::fs::remove_file(&final_file);
                        }
                        let temp_file = out_base.with_extension("temp.mp3");
                        if temp_file.exists() {
                            let _ = std::fs::remove_file(&temp_file);
                        }
                    }
                    continue;
                } else {
                    let _ = app.emit(
                        "download-error",
                        json!({ "error": format!("Failed to download {}: {}", meta.title, err) }),
                    );
                    break;
                }
            }

            if let Some(path) = downloaded_path {
                if path.exists() {
                    if let Err(e) = tag_file(app, &path, meta).await {
                        eprintln!("Failed to finalize {}: {}", meta.title, e);
                    }
                    success = true;
                }
            }
        }

        if success {
            // Add to state and save
            if let Some(state_path) = &state_file_path {
                state.downloaded_files.insert(expected_filename);
                let _ = std::fs::write(state_path, serde_json::to_string_pretty(&state).unwrap());
            }

            let _ = app.emit(
                "download-progress",
                json!({
                    "percent": ((i as f64 + 1.0) / total as f64) * 100.0,
                    "status": format!("Finished {}", meta.title)
                }),
            );
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
