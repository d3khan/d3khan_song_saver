use crate::batch::BatchControl;
use crate::metadata::TrackMeta;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_shell::ShellExt;
use tokio::time::{sleep, Duration};

// -------------------- State Struct for Sync --------------------

#[derive(Serialize, Deserialize, Default, Clone)]
pub(crate) struct PlaylistState {
    pub playlist_name: String,
    pub downloaded_tracks: HashSet<String>,
    pub manifest: std::collections::HashMap<String, String>,
}

pub fn remove_files_from_manifest(state_path: &Path, files: &[String]) {
    if let Ok(text) = std::fs::read_to_string(state_path) {
        if let Ok(mut state) = serde_json::from_str::<PlaylistState>(&text) {
            for path_str in files {
                state.manifest.retain(|_, v| v != path_str);
                state.downloaded_tracks.retain(|k| {
                    state.manifest.get(k).map(|v| v != path_str).unwrap_or(true)
                });
            }
            let _ = std::fs::write(state_path, serde_json::to_string_pretty(&state).unwrap());
        }
    }
}

// -------------------- Caching --------------------

fn get_cache_path(app: &AppHandle) -> PathBuf {
    let mut path = app.path().app_local_data_dir().unwrap_or_else(|_| PathBuf::from("."));
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

// -------------------- Bundled binary discovery --------------------

fn find_bundled_binary(name: &str) -> Option<PathBuf> {
    let triple = env!("TAURI_ENV_TARGET_TRIPLE");
    let exe = std::env::current_exe().ok()?;
    let exe_dir = exe.parent()?;
    let direct_name = if cfg!(windows) { format!("{name}.exe") } else { name.to_string() };
    let direct = exe_dir.join(&direct_name);
    if direct.exists() { return Some(direct); }
    let suffixed_name = if cfg!(windows) { format!("{name}-{triple}.exe") } else { format!("{name}-{triple}") };
    for ancestor in exe_dir.ancestors() {
        let candidate = ancestor.join("binaries").join(&suffixed_name);
        if candidate.exists() { return Some(candidate); }
    }
    None
}

fn find_ffmpeg(_app: &AppHandle) -> Option<PathBuf> {
    if let Some(p) = find_bundled_binary("d3khan-ffmpeg") { return Some(p); }
    if let Ok(p) = which::which("ffmpeg") { return Some(p); }
    None
}

// -------------------- Fast YouTube search --------------------

pub async fn search_youtube(query: &str, limit: u32) -> Result<Vec<TrackMeta>, String> {
    // ALWAYS append "official audio" to UI searches to ensure high-quality results
    let target = format!("ytsearch{}:{} official audio", limit, query.trim());
    
    let output = std::process::Command::new("yt-dlp")
        .args([&target, "--flat-playlist", "--dump-json", "--no-warnings", "--no-playlist", "--extractor-retries", "1"])
        .output()
        .map_err(|e| format!("yt-dlp search failed to start: {e}"))?;

    if !output.status.success() {
        return Err(format!("yt-dlp search failed: {}", String::from_utf8_lossy(&output.stderr)));
    }

    let mut results = Vec::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        if line.trim().is_empty() { continue; }
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
            let id = v.get("id").and_then(|x| x.as_str()).unwrap_or_default();
            if id.is_empty() { continue; }
            results.push(TrackMeta {
                id: format!("https://www.youtube.com/watch?v={}", id),
                title: v.get("title").and_then(|x| x.as_str()).unwrap_or("Unknown Title").to_string(),
                artists: vec![v.get("uploader").and_then(|x| x.as_str()).or_else(|| v.get("channel").and_then(|x| x.as_str())).unwrap_or("YouTube").to_string()],
                cover_url: Some(format!("https://i.ytimg.com/vi/{}/hqdefault.jpg", id)),
                duration_ms: v.get("duration").and_then(|d| d.as_f64()).map(|s| (s * 1000.0) as u64).unwrap_or(0),
                ..Default::default()
            });
        }
    }
    Ok(results)
}

// -------------------- Input Router --------------------

pub async fn resolve_input(app: &AppHandle, input: &str) -> Result<Vec<TrackMeta>, String> {
    let trimmed = input.trim();
    let is_yt_domain = trimmed.contains("youtube.com") || trimmed.contains("music.youtube.com") || trimmed.contains("youtu.be");
    
    if trimmed.contains("open.spotify.com/track/") { return fetch_spotify_track(trimmed).await; }
    if trimmed.contains("open.spotify.com/playlist/") || trimmed.contains("open.spotify.com/album/") {
        return fetch_spotify_playlist(trimmed, app).await;
    }
    if is_yt_domain && trimmed.contains("list=") {
        return fetch_youtube_playlist(trimmed).await;
    }
    if is_yt_domain {
        return fetch_youtube_track(trimmed).await;
    }

    let cache_key = trimmed.to_lowercase();
    let mut cache = load_cache(app);
    if let Some(cached_results) = cache.get(&cache_key) { return Ok(cached_results.clone()); }
    
    let results = search_youtube(trimmed, 20).await?;
    if !results.is_empty() {
        cache.insert(cache_key, results.clone());
        save_cache(app, &cache);
        return Ok(results);
    }
    Err("No tracks found on YouTube for your query.".into())
}

// -------------------- Spotify Playlist (Uses Python Sidecar) --------------------

async fn fetch_spotify_playlist(url: &str, app: &AppHandle) -> Result<Vec<TrackMeta>, String> {
    let sidecar_cmd = app.shell().sidecar("spotify_meta").map_err(|e| format!("Failed to find spotify_meta sidecar: {e}"))?;
    let output = sidecar_cmd.args([url]).output().await.map_err(|e| format!("Failed to execute sidecar: {e}"))?;

    if !output.status.success() {
        return Err(format!("spotify_meta sidecar failed: {}", String::from_utf8_lossy(&output.stderr)));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json_str = stdout.lines().rev().find(|line| line.trim().starts_with('{')).unwrap_or_default();
    let parsed: serde_json::Value = serde_json::from_str(json_str).map_err(|e| format!("Parse error: {e}"))?;

    if let Some(err) = parsed.get("error").or_else(|| parsed.get("message")).and_then(|e| e.as_str()) {
        return Err(format!("Spotify scraper error: {err}"));
    }

    let tracks_array = parsed["tracks"].as_array().ok_or("Sidecar response missing tracks")?;
    let mut mapped = Vec::new();
    for (i, t) in tracks_array.iter().enumerate() {
        mapped.push(TrackMeta {
            id: t["id"].as_str().unwrap_or("").to_string(),
            title: t["title"].as_str().unwrap_or("Unknown Title").to_string(),
            artists: t["artists"].as_array().unwrap_or(&vec![]).iter().filter_map(|a| a.as_str().map(String::from)).collect(),
            duration_ms: t["duration_ms"].as_u64().unwrap_or(0),
            cover_url: t["cover_url"].as_str().map(String::from),
            track_number: (i + 1) as u32,
            folder_name: t["folder_name"].as_str().map(String::from),
            playlist_url: Some(url.to_string()),
            ..Default::default()
        });
    }
    if mapped.is_empty() { return Err("Could not extract any tracks.".to_string()); }
    Ok(mapped)
}

// -------------------- YouTube Playlist --------------------

async fn fetch_youtube_playlist(url: &str) -> Result<Vec<TrackMeta>, String> {
    let output = std::process::Command::new("yt-dlp")
        .args(["--flat-playlist", "--dump-json", "--yes-playlist", "--ignore-errors", "--extractor-args", "youtubetab:skip=webpage", "--extractor-retries", "1", "--socket-timeout", "15", url])
        .output().map_err(|e| format!("yt-dlp failed: {e}"))?;

    if !output.status.success() { return Err("yt-dlp failed to fetch YouTube playlist.".to_string()); }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut mapped = Vec::new();
    let mut playlist_name = "YouTube Playlist".to_string();
    let mut track_number = 0u32;

    for line in stdout.lines() {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(line) {
            if track_number == 0 {
                if let Some(p_name) = json["playlist_title"].as_str() { playlist_name = p_name.to_string(); }
            }
            let video_id = json["id"].as_str().unwrap_or_default();
            if video_id.is_empty() { continue; }
            track_number += 1;
            mapped.push(TrackMeta {
                id: format!("https://www.youtube.com/watch?v={}", video_id),
                title: json["title"].as_str().unwrap_or("Unknown Title").to_string(),
                artists: vec![json["uploader"].as_str().or_else(|| json["channel"].as_str()).unwrap_or("YouTube").to_string()],
                cover_url: Some(format!("https://i.ytimg.com/vi/{}/hqdefault.jpg", video_id)),
                track_number,
                folder_name: Some(playlist_name.clone()),
                playlist_url: Some(url.to_string()),
                ..Default::default()
            });
        }
    }
    if mapped.is_empty() { return Err("Could not extract any tracks from the YouTube playlist.".to_string()); }
    Ok(mapped)
}

// -------------------- Single Tracks --------------------

async fn fetch_youtube_track(youtube_url: &str) -> Result<Vec<TrackMeta>, String> {
    let encoded = urlencoding::encode(youtube_url);
    let client = reqwest::Client::new();
    let resp = client.get(&format!("https://www.youtube.com/oembed?url={}&format=json", encoded))
        .header("User-Agent", "Mozilla/5.0").send().await.map_err(|e| format!("Network error: {e}"))?;
    let json: serde_json::Value = resp.json().await.unwrap_or_default();
    
    Ok(vec![TrackMeta {
        id: youtube_url.to_string(),
        title: json["title"].as_str().unwrap_or("YouTube Video").to_string(),
        artists: vec![json["author_name"].as_str().unwrap_or("YouTube").to_string()],
        cover_url: json["thumbnail_url"].as_str().map(String::from),
        ..Default::default()
    }])
}

async fn fetch_spotify_track(spotify_url: &str) -> Result<Vec<TrackMeta>, String> {
    let client = reqwest::Client::new();
    let resp = client.get(spotify_url).header("User-Agent", "Mozilla/5.0").send().await.map_err(|e| format!("Network error: {e}"))?;
    let html = resp.text().await.map_err(|e| format!("Failed to read page: {e}"))?;

    let title = extract_meta_tag(&html, "property=\"og:title\"").unwrap_or_else(|| "Unknown Title".to_string());
    let description = extract_meta_tag(&html, "property=\"og:description\"").unwrap_or_default();
    let artist = description.split(" · ").next().filter(|s| !s.is_empty()).unwrap_or("Unknown Artist").trim().to_string();
    let cover_url = extract_meta_tag(&html, "property=\"og:image\"");

    Ok(vec![TrackMeta { id: spotify_url.to_string(), title, artists: vec![artist], cover_url, ..Default::default() }])
}

fn extract_meta_tag(html: &str, prop: &str) -> Option<String> {
    let prop_str = prop.trim();
    let re_str = format!(r#"(?i){}\s+content=["']([^"']+)["']|content=["']([^"']+)["']\s+{}"#, prop_str, prop_str);
    if let Ok(re) = Regex::new(&re_str) {
        if let Some(caps) = re.captures(html) {
            return caps.get(1).or_else(|| caps.get(2)).map(|m| m.as_str().to_string());
        }
    }
    None
}

// -------------------- Helpers --------------------

fn is_bot_check(text: &str) -> bool {
    text.contains("Sign in to confirm") || text.contains("not a bot") || text.contains("LOGIN_REQUIRED")
}

fn is_retryable(err: &str) -> bool {
    err.contains("WinError 32") || err.contains("being used by another process") || err.contains("HTTP Error 416")
}

fn playlist_display_name(meta: &TrackMeta) -> String {
    match (meta.folder_name.as_deref(), meta.playlist_url.as_deref()) {
        (Some(name), Some(url)) if !name.is_empty() && !url.is_empty() => format!("{} || {}", name, url),
        (Some(name), _) => name.to_string(),
        (None, Some(url)) if !url.is_empty() => format!(" || {}", url),
        _ => String::new(),
    }
}

fn sync_key(meta: &TrackMeta, media_mode: &str) -> String {
    format!("{}::{}", meta.id, media_mode)
}

// -------------------- Audio-from-existing-video conversion --------------------

async fn convert_existing_video_to_audio(video_path: &Path, audio_path: &Path, ffmpeg: &Path) -> Result<(), String> {
    let output = tokio::process::Command::new(ffmpeg)
        .args(["-i", video_path.to_str().unwrap_or_default(), "-vn", "-acodec", "libmp3lame", "-q:a", "2", "-y", audio_path.to_str().unwrap_or_default()])
        .output().await.map_err(|e| format!("ffmpeg spawn failed: {e}"))?;
    if !output.status.success() {
        return Err(format!("ffmpeg conversion failed: {}", String::from_utf8_lossy(&output.stderr)));
    }
    Ok(())
}

// -------------------- Batch Downloader --------------------

pub async fn download_batch(
    app: &AppHandle,
    tracks: Vec<TrackMeta>,
    output_dir: &Path,
    overwrite_mode: bool,
    media_mode: &str,
) -> Result<(), String> {
    let total = tracks.len();
    let is_video = media_mode == "video";
    let ffmpeg_path = find_ffmpeg(app).ok_or_else(|| "FFmpeg not found.".to_string())?;
    let ffmpeg_str = ffmpeg_path.to_string_lossy().to_string();

    let mut current_folder_state: Option<(PathBuf, PlaylistState)> = None;
    let mut last_state_path = PathBuf::new();

    for (i, meta) in tracks.iter().enumerate() {
        let is_youtube = meta.id.starts_with("http")
            && (meta.id.contains("youtube.com") || meta.id.contains("music.youtube.com") || meta.id.contains("youtu.be"));
        let query = if is_youtube { meta.id.clone() } else { format!("{} {}", meta.title, meta.artists.join(", ")) };

        let mut audio_dir = output_dir.to_path_buf();
        let mut target_dir = output_dir.to_path_buf();

        if let Some(folder) = &meta.folder_name {
            let safe_folder = folder.replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_");
            audio_dir = output_dir.join(&safe_folder);
            target_dir = if is_video { audio_dir.join("videos") } else { audio_dir.clone() };
            std::fs::create_dir_all(&target_dir).map_err(|e| format!("Failed to create folder: {e}"))?;
        } else if is_video {
            target_dir = output_dir.to_path_buf();
        }

        let state_path = audio_dir.join(".playlist_sync.json");
        last_state_path = state_path.clone();
        let needs_load = current_folder_state.as_ref().map(|(p, _)| p != &state_path).unwrap_or(true);

        if needs_load {
            let state = if state_path.exists() {
                std::fs::read_to_string(&state_path).ok().and_then(|c| serde_json::from_str(&c).ok()).unwrap_or_default()
            } else {
                PlaylistState {
                    playlist_name: playlist_display_name(meta),
                    downloaded_tracks: HashSet::new(),
                    manifest: std::collections::HashMap::new(),
                }
            };
            current_folder_state = Some((state_path.clone(), state));
        }

        let (_, state) = current_folder_state.as_mut().unwrap();

        let safe_title = meta.title.replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_");
        let safe_artist = meta.artists.join(", ").replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_");
        let file_stem = format!("{} - {}", safe_title, safe_artist);
        let out_base = target_dir.join(&file_stem);
        let audio_base = audio_dir.join(&file_stem);

        // --- AUDIO MODE: Check for existing video and convert first (ONLY for YouTube links) ---
        if !is_video && is_youtube {
            let candidate_video_bases = vec![
                audio_dir.join(&file_stem),
                audio_dir.join("videos").join(&file_stem),
            ];

            let mut converted = false;
            for base in &candidate_video_bases {
                for ext in &["mp4", "webm", "mkv"] {
                    let video_path = base.with_extension(ext);
                    if !video_path.exists() { continue; }
                    
                    let audio_path = audio_base.with_extension("mp3");
                    if audio_path.exists() && !overwrite_mode { converted = true; break; }
                    
                    let _ = app.emit("download-progress", json!({
                        "percent": (i as f64 / total as f64) * 100.0,
                        "status": format!("Converting existing video to MP3: {}", meta.title)
                    }));
                    
                    match convert_existing_video_to_audio(&video_path, &audio_path, &ffmpeg_path).await {
                        Ok(()) => {
                            let key = sync_key(meta, "audio");
                            state.downloaded_tracks.insert(key.clone());
                            state.manifest.insert(key, audio_path.to_string_lossy().to_string());
                            let _ = std::fs::write(&state_path, serde_json::to_string_pretty(state).unwrap());
                            let _ = app.emit("download-progress", json!({
                                "percent": ((i as f64 + 1.0) / total as f64) * 100.0,
                                "status": format!("Converted (bandwidth saved): {}", meta.title)
                            }));
                            converted = true;
                            break;
                        }
                        Err(e) => {
                            let _ = app.emit("download-error", json!({
                                "error": format!("Video→audio conversion failed for {}: {}", meta.title, e)
                            }));
                        }
                    }
                }
                if converted { break; }
            }
            if converted { continue; }
        }

        // --- NORMAL DOWNLOAD PROCESS ---
        let possible_exts: &[&str] = if is_video { &["mp4", "webm", "mkv", "m4a"] } else { &["mp3", "m4a", "opus", "webm"] };

        if !overwrite_mode {
            let key = sync_key(meta, media_mode);
            if state.downloaded_tracks.contains(&key) {
                let _ = app.emit("download-progress", json!({"percent": ((i as f64 + 1.0) / total as f64) * 100.0, "status": format!("Skipped: {}", meta.title)}));
                continue;
            }

            if let Some(existing) = possible_exts.iter().map(|e| out_base.with_extension(e)).find(|p| p.exists()) {
                state.downloaded_tracks.insert(key.clone());
                state.manifest.insert(key, existing.to_string_lossy().to_string());
                let _ = std::fs::write(&state_path, serde_json::to_string_pretty(state).unwrap());
                let _ = app.emit("download-progress", json!({"percent": ((i as f64 + 1.0) / total as f64) * 100.0, "status": format!("Skipped (exists): {}", meta.title)}));
                continue;
            }
        } else {
            for ext in possible_exts {
                let p = out_base.with_extension(ext);
                if p.exists() { let _ = std::fs::remove_file(&p); }
            }
            for ext in ["part", "ytdl", "temp.mp3", "temp.mp4"] {
                let p = out_base.with_extension(ext);
                if p.exists() { let _ = std::fs::remove_file(&p); }
            }
        }

        let mut success = false;
        let mut attempts = 0u32;
        let mut final_downloaded_path: Option<PathBuf> = None;
        const MAX_ATTEMPTS: u32 = 3;
        let mut error_emitted = false;
        let mut bot_check_paused_once = false;

        let _ = app.emit("download-progress", json!({
            "percent": (i as f64 / total as f64) * 100.0,
            "status": format!("Waiting 5s before downloading {}...", meta.title)
        }));
        sleep(Duration::from_secs(5)).await;

        while attempts < MAX_ATTEMPTS && !success {
            if attempts > 0 {
                let _ = app.emit("download-progress", json!({
                    "percent": (i as f64 / total as f64) * 100.0,
                    "status": format!("Retrying {} (Attempt {}/{})...", meta.title, attempts + 1, MAX_ATTEMPTS)
                }));
                sleep(Duration::from_secs(3)).await;
            } else {
                let _ = app.emit("download-progress", json!({
                    "percent": (i as f64 / total as f64) * 100.0,
                    "status": format!("Starting {}...", meta.title)
                }));
            }

            let sidecar_command = app.shell().sidecar("downloader").map_err(|e| format!("Sidecar error: {e}"))?
                .args([query.as_str(), out_base.to_string_lossy().as_ref(), ffmpeg_str.as_str(), meta.title.as_str(), meta.artists.join(", ").as_str(), media_mode]);

            let (mut rx, _child) = sidecar_command.spawn().map_err(|e| format!("Spawn failed: {e}"))?;
            let mut downloaded_path: Option<PathBuf> = None;
            let mut error_msg: Option<String> = None;
            let mut terminated_code: Option<i32> = None;

            while let Some(event) = rx.recv().await {
                match event {
                    tauri_plugin_shell::process::CommandEvent::Stdout(line) => {
                        let text = String::from_utf8_lossy(&line);
                        if text.contains("\"status\": \"success\"") {
                            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                                if let Some(path_str) = json["path"].as_str() { downloaded_path = Some(PathBuf::from(path_str)); }
                            }
                            break;
                        } else if text.contains("\"status\": \"error\"") {
                            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                                error_msg = Some(json["message"].as_str().unwrap_or("Unknown error").to_string());
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
                        let _ = app.emit("batch-paused", json!({
                            "reason": "bot-check",
                            "title": meta.title,
                            "message": "YouTube is asking for verification. Watch any short video in your browser, then click Continue."
                        }));
                        let control = app.state::<BatchControl>();
                        control.pause();
                        control.wait_until_resumed().await;
                        sleep(Duration::from_secs(2)).await;
                        continue;
                    } else {
                        let _ = app.emit("download-error", json!({
                            "error": format!("YouTube still requires verification for \"{}\". Skipping.", meta.title)
                        }));
                        error_emitted = true;
                        break;
                    }
                }

                if is_retryable(&err) {
                    if err.contains("HTTP Error 416") {
                        if let Some(p) = downloaded_path.as_ref() { let _ = std::fs::remove_file(p); }
                        for e in ["mp3", "mp4", "webm", "mkv", "m4a", "part", "ytdl", "temp.mp3", "temp.mp4"] {
                            let p = out_base.with_extension(e);
                            if p.exists() { let _ = std::fs::remove_file(&p); }
                        }
                    }
                    attempts += 1;
                    continue;
                }

                let _ = app.emit("download-error", json!({"error": format!("Failed to download {}: {}", meta.title, err)}));
                error_emitted = true;
                break;
            }

            if let Some(path) = downloaded_path {
                if path.exists() {
                    if let Err(e) = tag_file(app, &path, meta).await {
                        eprintln!("Failed to finalize {}: {}", meta.title, e);
                    }
                    success = true;
                    final_downloaded_path = Some(path);
                } else {
                    attempts += 1;
                }
            } else {
                attempts += 1;
            }
        }

        if success {
            let key = sync_key(meta, media_mode);
            state.downloaded_tracks.insert(key.clone());
            if let Some(ref path) = final_downloaded_path {
                state.manifest.insert(key, path.to_string_lossy().to_string());
            }
            let _ = std::fs::write(&state_path, serde_json::to_string_pretty(state).unwrap());
            let _ = app.emit("download-progress", json!({
                "percent": ((i as f64 + 1.0) / total as f64) * 100.0,
                "status": format!("Finished {}", meta.title)
            }));
        } else if !error_emitted {
            let _ = app.emit("download-error", json!({
                "error": format!("Failed to download {} after {} attempts", meta.title, attempts)
            }));
        }
    }

    // -------------------- SYNC DELETION PROMPT --------------------
    if !overwrite_mode && !last_state_path.as_os_str().is_empty() {
        if let Some((_, state)) = current_folder_state.as_ref() {
            let current_ids: HashSet<String> = tracks.iter().map(|m| sync_key(m, media_mode)).collect();
            let mut orphaned_files = Vec::new();

            for (id, path_str) in &state.manifest {
                if !current_ids.contains(id) && std::path::Path::new(path_str).exists() {
                    orphaned_files.push(path_str.clone());
                }
            }

            if !orphaned_files.is_empty() {
                let _ = app.emit("sync-deletion-prompt", json!({
                    "files": orphaned_files,
                    "timeout_ms": 10000,
                    "state_path": last_state_path.to_string_lossy().to_string()
                }));
            }
        }
    }

    let _ = app.emit("download-progress", json!({ "percent": 100.0, "status": "All done!" }));
    Ok(())
}

async fn tag_file(app: &AppHandle, _path: &Path, _meta: &TrackMeta) -> Result<(), String> {
    let _ = app;
    Ok(())
}