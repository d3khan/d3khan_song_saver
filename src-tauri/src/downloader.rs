use crate::batch::BatchControl;
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
    downloaded_tracks: HashSet<String>, // Stores the unique Track/Video IDs
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

fn is_bot_check(text: &str) -> bool {
    text.contains("Sign in to confirm")
        || text.contains("not a bot")
        || text.contains("LOGIN_REQUIRED")
        || text.contains("--cookies-from-browser")
}

fn is_retryable(err: &str) -> bool {
    err.contains("WinError 32")
        || err.contains("being used by another process")
        || err.contains("HTTP Error 416")
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
            playlist_url: Some(url.to_string()),
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
            let video_url = format!("https://www.youtube.com/watch?v={}", video_id);

            mapped.push(TrackMeta {
                id: video_url,
                title,
                artists: vec![uploader],
                cover_url,
                folder_name: Some(playlist_name.clone()),
            playlist_url: Some(url.to_string()),
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

// -------------------- Batch Downloader (With Bulletproof Sync) --------------------

fn playlist_display_name(meta: &TrackMeta) -> String {
    match (meta.folder_name.as_deref(), meta.playlist_url.as_deref()) {
        (Some(name), Some(url)) if !name.is_empty() && !url.is_empty() => {
            format!("{} || {}", name, url)
        }
        (Some(name), _) => name.to_string(),
        (None, Some(url)) if !url.is_empty() => format!(" || {}", url),
        _ => String::new(),
    }
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
        String::new()
    };

    let mut current_folder_state: Option<(PathBuf, PlaylistState)> = None;

    for (i, meta) in tracks.iter().enumerate() {
        let is_youtube = meta.id.starts_with("http")
            && (meta.id.contains("youtube.com") || meta.id.contains("youtu.be"));

        let query = if is_youtube {
            meta.id.clone()
        } else {
            format!("{} {}", meta.title, meta.artists.join(", "))
        };

        // ---------------- Resolve target folder ----------------
        let mut target_dir = output_dir.to_path_buf();
        if let Some(folder) = &meta.folder_name {
            let safe_folder = folder.replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_");
            let current_folder_name = target_dir
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();
            if current_folder_name != safe_folder {
                target_dir = target_dir.join(&safe_folder);
            }
            std::fs::create_dir_all(&target_dir)
                .map_err(|e| format!("Failed to create playlist folder: {e}"))?;
        }

        let state_path = target_dir.join(".playlist_sync.json");

        // ---------------- Load / cache state for this folder ----------------
        let needs_load = current_folder_state
            .as_ref()
            .map(|(p, _)| p != &state_path)
            .unwrap_or(true);

        if needs_load {
            let (mut state, mut upgraded) = if state_path.exists() {
                match std::fs::read_to_string(&state_path)
                    .ok()
                    .and_then(|c| serde_json::from_str::<PlaylistState>(&c).ok())
                {
                    Some(s) => (s, false),
                    None => (
                        PlaylistState {
                            playlist_name: playlist_display_name(meta),
                            downloaded_tracks: HashSet::new(),
                        },
                        false,
                    ),
                }
            } else {
                (
                    PlaylistState {
                        playlist_name: playlist_display_name(meta),
                        downloaded_tracks: HashSet::new(),
                    },
                    false,
                )
            };

            // Lazy upgrade: if the file was written by an older version
            // (playlist_name contains no " || " separator) and we now have
            // a URL available, upgrade the stored name so future runs have
            // the URL baked in.
            if !state.playlist_name.contains(" || ") {
                if let (Some(name), Some(url)) = (
                    meta.folder_name.as_deref(),
                    meta.playlist_url.as_deref(),
                ) {
                    if !name.is_empty() && !url.is_empty() {
                        state.playlist_name = format!("{} || {}", name, url);
                        upgraded = true;
                    }
                }
            }

            // Persist the upgrade immediately so the file is fixed even if
            // every track in this batch is skipped.
            if upgraded {
                let _ = std::fs::write(
                    &state_path,
                    serde_json::to_string_pretty(&state).unwrap(),
                );
            }

            current_folder_state = Some((state_path.clone(), state));
        }

        let (_, state) = current_folder_state.as_mut().unwrap();

        // ============================================================
        // PRIMARY SKIP: ID found in save state -> skip entirely
        // ============================================================
        if state.downloaded_tracks.contains(&meta.id) {
            let _ = app.emit(
                "download-progress",
                json!({
                    "percent": ((i as f64 + 1.0) / total as f64) * 100.0,
                    "status": format!("Skipped (already downloaded): {}", meta.title)
                }),
            );
            continue;
        }

        let safe_title = meta
            .title
            .replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_");
        let safe_artist = meta
            .artists
            .join(", ")
            .replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_");
        let out_base = target_dir.join(format!("{} - {}", safe_title, safe_artist));

        // ============================================================
        // SECONDARY SKIP: file already on disk -> record + skip
        // (handles .webm/.mkv/.m4a from yt-dlp as well as .mp3)
        // ============================================================
        let possible_exts: &[&str] = if is_youtube {
            &["mp4", "webm", "mkv", "m4a"]
        } else {
            &["mp3", "m4a", "opus"]
        };
        if let Some(existing) = possible_exts
            .iter()
            .map(|e| out_base.with_extension(e))
            .find(|p| p.exists())
        {
            state.downloaded_tracks.insert(meta.id.clone());
            let _ = std::fs::write(&state_path, serde_json::to_string_pretty(state).unwrap());
            let _ = app.emit(
                "download-progress",
                json!({
                    "percent": ((i as f64 + 1.0) / total as f64) * 100.0,
                    "status": format!(
                        "Skipped (file exists: {}): {}",
                        existing.file_name().unwrap_or_default().to_string_lossy(),
                        meta.title
                    )
                }),
            );
            continue;
        }

        // ---------------- Download with retries ----------------
        let mut success = false;
        let mut attempts = 0u32;
        const MAX_ATTEMPTS: u32 = 3;
        let mut error_emitted = false;
        let mut bot_check_paused_once = false;

        // 5-second pre-download wait — only before the first attempt of this track
        let _ = app.emit(
            "download-progress",
            json!({
                "percent": (i as f64 / total as f64) * 100.0,
                "status": format!("Waiting 5s before downloading {}...", meta.title)
            }),
        );
        sleep(Duration::from_secs(5)).await;

        while attempts < MAX_ATTEMPTS && !success {
            if attempts > 0 {
                let _ = app.emit(
                    "download-progress",
                    json!({
                        "percent": (i as f64 / total as f64) * 100.0,
                        "status": format!(
                            "Retrying {} (Attempt {}/{})...",
                            meta.title,
                            attempts + 1,
                            MAX_ATTEMPTS
                        )
                    }),
                );
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
            let mut terminated_code: Option<i32> = None;

            while let Some(event) = rx.recv().await {
                match event {
                    tauri_plugin_shell::process::CommandEvent::Stdout(line) => {
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
                    tauri_plugin_shell::process::CommandEvent::Stderr(line) => {
                        let text = String::from_utf8_lossy(&line);
                        if text.contains("ERROR:") || text.contains("Traceback") {
                            error_msg = Some(text.trim().to_string());
                        }
                    }
                    tauri_plugin_shell::process::CommandEvent::Terminated(payload) => {
                        terminated_code = payload.code;
                    }
                    _ => {}
                }
            }

            if error_msg.is_none() && downloaded_path.is_none() {
                match terminated_code {
                    Some(code) if code != 0 => {
                        error_msg = Some(format!("Downloader sidecar exited with code {code} without a status message"));
                    }
                    None => {
                        error_msg = Some("Downloader sidecar exited without a status message".to_string());
                    }
                    _ => {}
                }
            }

            if let Some(err) = error_msg {
                if is_bot_check(&err) {
                    if !bot_check_paused_once {
                        bot_check_paused_once = true;
                        let _ = app.emit(
                            "batch-paused",
                            json!({
                                "reason": "bot-check",
                                "title": meta.title,
                                "message": "YouTube is asking for verification. Watch any short video in your browser, then click Continue."
                            }),
                        );
                        let control = app.state::<BatchControl>();
                        control.pause();
                        control.wait_until_resumed().await;
                        sleep(Duration::from_secs(2)).await;
                        continue;
                    } else {
                        let _ = app.emit(
                            "download-error",
                            json!({
                                "error": format!("YouTube still requires verification for \"{}\". Skipping this track. Watch a video and retry the batch later.", meta.title)
                            }),
                        );
                        error_emitted = true;
                        break;
                    }
                }

                if is_retryable(&err) {
                    if err.contains("HTTP Error 416") {
                        if let Some(p) = downloaded_path.as_ref() {
                            let _ = std::fs::remove_file(p);
                        }
                        for e in [
                            "mp3", "mp4", "webm", "mkv", "m4a", "part", "ytdl", "temp.mp3",
                            "temp.mp4",
                        ] {
                            let p = out_base.with_extension(e);
                            if p.exists() { let _ = std::fs::remove_file(&p); }
                        }
                    }
                    attempts += 1;
                    continue;
                } else {
                    let _ = app.emit(
                        "download-error",
                        json!({ "error": format!("Failed to download {}: {}", meta.title, err) }),
                    );
                    error_emitted = true;
                    break;
                }
            }

            if let Some(path) = downloaded_path {
                if path.exists() {
                    if let Err(e) = tag_file(app, &path, meta).await {
                        eprintln!("Failed to finalize {}: {}", meta.title, e);
                    }
                    success = true;
                } else {
                    attempts += 1;
                    continue;
                }
            } else {
                attempts += 1;
                continue;
            }
        }

        if success {
            state.downloaded_tracks.insert(meta.id.clone());
            let _ = std::fs::write(&state_path, serde_json::to_string_pretty(state).unwrap());

            let _ = app.emit(
                "download-progress",
                json!({
                    "percent": ((i as f64 + 1.0) / total as f64) * 100.0,
                    "status": format!("Finished {}", meta.title)
                }),
            );
        } else if !error_emitted {
            let _ = app.emit(
                "download-error",
                json!({
                    "error": format!(
                        "Failed to download {} after {} attempts",
                        meta.title, attempts
                    )
                }),
            );
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
