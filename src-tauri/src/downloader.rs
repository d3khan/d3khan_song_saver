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

#[derive(Serialize, Deserialize, Default)]
struct PlaylistState {
    playlist_name: String,
    downloaded_tracks: HashSet<String>,
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

// -------------------- Bundled binary discovery --------------------

fn find_bundled_binary(name: &str) -> Option<PathBuf> {
    let triple = env!("TAURI_ENV_TARGET_TRIPLE");
    let exe = std::env::current_exe().ok()?;
    let exe_dir = exe.parent()?;

    let direct_name = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    };
    let direct = exe_dir.join(&direct_name);
    if direct.exists() {
        return Some(direct);
    }

    let suffixed_name = if cfg!(windows) {
        format!("{name}-{triple}.exe")
    } else {
        format!("{name}-{triple}")
    };
    for ancestor in exe_dir.ancestors() {
        let candidate = ancestor.join("binaries").join(&suffixed_name);
        if candidate.exists() {
            return Some(candidate);
        }
    }

    None
}

fn find_ffmpeg() -> Option<PathBuf> {
    if let Some(p) = find_bundled_binary("ffmpeg") {
        return Some(p);
    }
    if let Ok(p) = which::which("ffmpeg") {
        return Some(p);
    }
    None
}

// -------------------- Fast YouTube search --------------------

pub async fn search_youtube(query: &str, limit: u32) -> Result<Vec<TrackMeta>, String> {
    let target = format!("ytsearch{}:{}", limit, query.trim());

    let output = std::process::Command::new("yt-dlp")
        .args([
            &target,
            "--flat-playlist",
            "--dump-json",
            "--no-warnings",
            "--no-playlist",
            "--extractor-retries",
            "1",
        ])
        .output()
        .map_err(|e| format!("yt-dlp search failed to start: {e}"))?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(format!("yt-dlp search failed: {err}"));
    }

    let mut results = Vec::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
            let id = v.get("id").and_then(|x| x.as_str()).unwrap_or_default();
            if id.is_empty() {
                continue;
            }
            let title = v
                .get("title")
                .and_then(|x| x.as_str())
                .unwrap_or("Unknown Title")
                .to_string();
            let uploader = v
                .get("uploader")
                .and_then(|x| x.as_str())
                .or_else(|| v.get("channel").and_then(|x| x.as_str()))
                .unwrap_or("YouTube")
                .to_string();
            let duration_ms = v
                .get("duration")
                .and_then(|d| d.as_f64())
                .map(|s| (s * 1000.0) as u64)
                .unwrap_or(0);
            let cover_url = Some(format!("https://i.ytimg.com/vi/{}/hqdefault.jpg", id));
            let url = format!("https://www.youtube.com/watch?v={}", id);

            results.push(TrackMeta {
                id: url,
                title,
                artists: vec![uploader],
                cover_url,
                duration_ms,
                folder_name: None,
                ..Default::default()
            });
        }
    }

    Ok(results)
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
    if (trimmed.contains("youtube.com") || trimmed.contains("youtu.be")) && trimmed.contains("list=")
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

    let results = search_youtube(trimmed, 10).await?;
    if !results.is_empty() {
        cache.insert(cache_key, results.clone());
        save_cache(app, &cache);
        return Ok(results);
    }

    Err("No tracks found on YouTube for your query.".into())
}

// -------------------- Spotify helpers --------------------

fn extract_spotify_id(url: &str, kind: &str) -> Result<String, String> {
    let marker = format!("open.spotify.com/{}/", kind);
    let after = url
        .split(&marker)
        .nth(1)
        .ok_or_else(|| format!("Could not find {} ID in URL", kind))?;
    after
        .split(['?', '/'])
        .next()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .ok_or_else(|| format!("Empty {} ID", kind))
}

// -------------------- Spotify Playlist (SpotAPI — full pagination) --------------------

async fn fetch_spotify_playlist(url: &str) -> Result<Vec<TrackMeta>, String> {
    let id = extract_spotify_id(url, "playlist")
        .or_else(|_| extract_spotify_id(url, "album"))?;

    let client = reqwest::Client::new();
    let user_agent = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36";

    // 1. Embed page → metadata + anon token (works, as before)
    let embed_url = format!("https://open.spotify.com/embed/playlist/{}", id);
    let resp = client
        .get(&embed_url)
        .header("User-Agent", user_agent)
        .send()
        .await
        .map_err(|e| format!("Embed request failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("Embed returned HTTP {}", resp.status()));
    }
    let html = resp.text().await.map_err(|e| format!("Embed read failed: {e}"))?;

    let re = Regex::new(r#"(?s)<script id="__NEXT_DATA__" type="application/json">(.*?)</script>"#)
        .map_err(|e| format!("Regex: {e}"))?;
    let caps = re.captures(&html).ok_or("No __NEXT_DATA__ in embed")?;
    let json: serde_json::Value = serde_json::from_str(&caps[1])
        .map_err(|e| format!("__NEXT_DATA__ parse: {e}"))?;

    let playlist_name = json["props"]["pageProps"]["state"]["data"]["entity"]["name"]
        .as_str().unwrap_or("Unknown Playlist").to_string();
    let playlist_cover = json["props"]["pageProps"]["state"]["data"]["entity"]["coverArt"]["sources"]
        .as_array().and_then(|a| a.first()).and_then(|s| s["url"].as_str()).map(String::from);

    let token = json["props"]["pageProps"]["state"]["settings"]["session"]["accessToken"]
        .as_str().filter(|s| !s.is_empty())
        .ok_or("No accessToken in __NEXT_DATA__")?.to_string();

    // 2. spclient playlist/v2 — anonymous token works here, api.spotify.com does not.
    let mut all_tracks: Vec<TrackMeta> = Vec::new();
    let mut offset: usize = 0;
    let limit: usize = 100;

    loop {
        let api_url = format!(
            "https://spclient.wg.spotify.com/playlist/v2/playlist/{}?from={}&length={}",
            id, offset, limit
        );
        let resp = client
            .get(&api_url)
            .header("Authorization", format!("Bearer {}", token))
            .header("User-Agent", user_agent)
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|e| format!("spclient request failed: {e}"))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("spclient HTTP {} at offset {}: {}", status, offset, body));
        }

        let page: serde_json::Value = resp.json().await
            .map_err(|e| format!("spclient JSON parse: {e}"))?;

        // spclient returns { "contents": { "items": [ { "uri": "spotify:track:..." }, ... ] } }
        let items = page["contents"]["items"]
            .as_array()
            .or_else(|| page["items"].as_array())
            .ok_or("spclient response missing contents.items")?;

        let before = all_tracks.len();
        for item in items {
            let uri = item["uri"].as_str().unwrap_or("");
            if !uri.starts_with("spotify:track:") {
                continue;
            }
            let track_id = uri.rsplit(':').next().unwrap_or("");
            all_tracks.push(TrackMeta {
                id: format!("https://open.spotify.com/track/{}", track_id),
                title: format!("Track {}", track_id), // spclient gives URI only
                artists: vec![],
                album: String::new(),
                duration_ms: 0,
                cover_url: playlist_cover.clone(),
                track_number: (all_tracks.len() + 1) as u32,
                folder_name: Some(playlist_name.clone()),
                playlist_url: Some(url.to_string()),
                ..Default::default()
            });
        }

        let got = all_tracks.len() - before;
        if got == 0 || items.len() < limit {
            break;
        }
        offset += limit;
    }

    if all_tracks.is_empty() {
        return Err("Playlist returned zero tracks".into());
    }
    Ok(all_tracks)
}

// -------------------- YouTube Playlist (fast, skip webpage) --------------------

async fn fetch_youtube_playlist(url: &str) -> Result<Vec<TrackMeta>, String> {
    // --extractor-args "youtubetab:skip=webpage" skips the slow initial
    // playlist-page download. This is the difference between ~100 s and
    // ~5 s on small playlists.
    let output = std::process::Command::new("yt-dlp")
        .args([
            "--flat-playlist",
            "--dump-json",
            "--yes-playlist",
            "--ignore-errors",
            "--extractor-args",
            "youtubetab:skip=webpage",
            "--extractor-retries",
            "1",
            "--socket-timeout",
            "15",
            url,
        ])
        .output()
        .map_err(|e| format!("Failed to run yt-dlp for playlist: {e}"))?;

    if !output.status.success() {
        return Err("yt-dlp failed to fetch YouTube playlist.".to_string());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut mapped = Vec::new();
    let mut playlist_name = "YouTube Playlist".to_string();
    let mut track_number = 0u32;

    for line in stdout.lines() {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(line) {
            if track_number == 0 {
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

            track_number += 1;
            mapped.push(TrackMeta {
                id: video_url,
                title,
                artists: vec![uploader],
                cover_url,
                track_number,
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
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
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
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
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

// -------------------- Helpers --------------------

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

fn sync_key(meta: &TrackMeta, media_mode: &str) -> String {
    format!("{}::{}", meta.id, media_mode)
}

// -------------------- Audio-from-existing-video conversion --------------------

async fn convert_existing_video_to_audio(
    video_path: &Path,
    audio_path: &Path,
    ffmpeg: &Path,
) -> Result<(), String> {
    let output = tokio::process::Command::new(ffmpeg)
        .args([
            "-i",
            video_path.to_str().unwrap_or_default(),
            "-vn",
            "-acodec",
            "libmp3lame",
            "-q:a",
            "2",
            "-y",
            audio_path.to_str().unwrap_or_default(),
        ])
        .output()
        .await
        .map_err(|e| format!("ffmpeg spawn failed: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("ffmpeg conversion failed: {stderr}"));
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

    let ffmpeg_path = find_ffmpeg().ok_or_else(|| {
        "FFmpeg not found. Expected a bundled binary next to the app executable \
         or `ffmpeg` on PATH."
            .to_string()
    })?;
    let ffmpeg_str = ffmpeg_path.to_string_lossy().to_string();

    let mut current_folder_state: Option<(PathBuf, PlaylistState)> = None;

    for (i, meta) in tracks.iter().enumerate() {
        let is_youtube = meta.id.starts_with("http")
            && (meta.id.contains("youtube.com") || meta.id.contains("youtu.be"));

        let query = if is_youtube {
            meta.id.clone()
        } else {
            format!("{} {}", meta.title, meta.artists.join(", "))
        };

        let mut audio_dir = output_dir.to_path_buf();
        let mut target_dir = output_dir.to_path_buf();

        if let Some(folder) = &meta.folder_name {
            let safe_folder = folder.replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_");
            audio_dir = output_dir.join(&safe_folder);
            target_dir = if is_video {
                audio_dir.join("videos")
            } else {
                audio_dir.clone()
            };
            std::fs::create_dir_all(&target_dir)
                .map_err(|e| format!("Failed to create playlist folder: {e}"))?;
        } else if is_video {
            target_dir = output_dir.to_path_buf();
        }

        let state_path = audio_dir.join(".playlist_sync.json");

        let needs_load = current_folder_state
            .as_ref()
            .map(|(p, _)| p != &state_path)
            .unwrap_or(true);

        if needs_load {
            let (mut state, mut upgraded) = if state_path.exists() {
                match std::fs::read_to_string(&state_path)
                    .ok()
                    .and_then(|c| serde_json::from_str(&c).ok())
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

            if !state.playlist_name.contains(" || ") {
                if let (Some(name), Some(url)) =
                    (meta.folder_name.as_deref(), meta.playlist_url.as_deref())
                {
                    if !name.is_empty() && !url.is_empty() {
                        state.playlist_name = format!("{} || {}", name, url);
                        upgraded = true;
                    }
                }
            }

            if upgraded {
                let _ = std::fs::write(&state_path, serde_json::to_string_pretty(&state).unwrap());
            }

            current_folder_state = Some((state_path.clone(), state));
        }

        let (_, state) = current_folder_state.as_mut().unwrap();

        let safe_title = meta
            .title
            .replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_");
        let safe_artist = meta
            .artists
            .join(", ")
            .replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_");

        let file_stem = format!("{} - {}", safe_title, safe_artist);

        let out_base = target_dir.join(&file_stem);
        let audio_base = audio_dir.join(&file_stem);

        if !is_video {
            let candidate_video_bases = vec![
                audio_dir.join(&file_stem),
                audio_dir.join("videos").join(&file_stem),
            ];

            let mut converted = false;
            for base in &candidate_video_bases {
                for ext in &["mp4", "webm", "mkv"] {
                    let video_path = base.with_extension(ext);
                    if !video_path.exists() {
                        continue;
                    }
                    let audio_path = audio_base.with_extension("mp3");
                    if audio_path.exists() && !overwrite_mode {
                        converted = true;
                        break;
                    }
                    let _ = app.emit(
                        "download-progress",
                        json!({
                            "percent": (i as f64 / total as f64) * 100.0,
                            "status": format!("Converting existing video to MP3: {}", meta.title)
                        }),
                    );
                    match convert_existing_video_to_audio(&video_path, &audio_path, &ffmpeg_path)
                        .await
                    {
                        Ok(()) => {
                            let key = sync_key(meta, "audio");
                            state.downloaded_tracks.insert(key);
                            let _ = std::fs::write(
                                &state_path,
                                serde_json::to_string_pretty(state).unwrap(),
                            );
                            let _ = app.emit(
                                "download-progress",
                                json!({
                                    "percent": ((i as f64 + 1.0) / total as f64) * 100.0,
                                    "status": format!("Converted (bandwidth saved): {}", meta.title)
                                }),
                            );
                            converted = true;
                            break;
                        }
                        Err(e) => {
                            let _ = app.emit(
                                "download-error",
                                json!({
                                    "error": format!("Video→audio conversion failed for {}: {}", meta.title, e)
                                }),
                            );
                        }
                    }
                }
                if converted {
                    break;
                }
            }

            if converted {
                continue;
            }
        }

        let possible_exts: &[&str] = if is_video {
            &["mp4", "webm", "mkv", "m4a"]
        } else {
            &["mp3", "m4a", "opus"]
        };

        if !overwrite_mode {
            let key = sync_key(meta, media_mode);
            if state.downloaded_tracks.contains(&key) {
                let _ = app.emit(
                    "download-progress",
                    json!({
                        "percent": ((i as f64 + 1.0) / total as f64) * 100.0,
                        "status": format!("Skipped (already downloaded): {}", meta.title)
                    }),
                );
                continue;
            }

            if let Some(existing) = possible_exts
                .iter()
                .map(|e| out_base.with_extension(e))
                .find(|p| p.exists())
            {
                state.downloaded_tracks.insert(key);
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
        } else {
            for ext in possible_exts {
                let p = out_base.with_extension(ext);
                if p.exists() {
                    let _ = std::fs::remove_file(&p);
                }
            }
            for ext in ["part", "ytdl", "temp.mp3", "temp.mp4"] {
                let p = out_base.with_extension(ext);
                if p.exists() {
                    let _ = std::fs::remove_file(&p);
                }
            }
        }

        let mut success = false;
        let mut attempts = 0u32;
        const MAX_ATTEMPTS: u32 = 3;
        let mut error_emitted = false;
        let mut bot_check_paused_once = false;

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
                    media_mode,
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
                        error_msg = Some(format!(
                            "Downloader sidecar exited with code {code} without a status message"
                        ));
                    }
                    None => {
                        error_msg = Some(
                            "Downloader sidecar exited without a status message".to_string(),
                        );
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
                                "error": format!(
                                    "YouTube still requires verification for \"{}\". Skipping this track. Watch a video and retry the batch later.",
                                    meta.title
                                )
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
                            if p.exists() {
                                let _ = std::fs::remove_file(&p);
                            }
                        }
                    }
                    attempts += 1;
                    continue;
                }

                let _ = app.emit(
                    "download-error",
                    json!({
                        "error": format!("Failed to download {}: {}", meta.title, err)
                    }),
                );
                error_emitted = true;
                break;
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
            let key = sync_key(meta, media_mode);
            state.downloaded_tracks.insert(key);
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
                        meta.title,
                        attempts
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

async fn tag_file(app: &AppHandle, _path: &Path, _meta: &TrackMeta) -> Result<(), String> {
    let _ = app;
    Ok(())
}

// -------------------- Internal tests --------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn spclient_diagnostic() {
        let client = reqwest::Client::new();
        for url in &[
            "https://api.spotify.com/v1/playlists/7JbGYbGThZDPLfLK2nPX1c/tracks?offset=0&limit=1",
            "https://spclient.wg.spotify.com/playlist/v2/playlist/7JbGYbGThZDPLfLK2nPX1c?from=0&length=1",
        ] {
            match client.get(*url).header("User-Agent", "Mozilla/5.0").send().await {
                Ok(r) => println!("[DIAG] {} → HTTP {}", url, r.status()),
                Err(e) => println!("[DIAG] {} → transport error: {}", url, e),
            }
        }
    }


    #[tokio::test]
    async fn spotify_playlist_fetches_all_tracks() {
        let url =
            "https://open.spotify.com/playlist/7JbGYbGThZDPLfLK2nPX1c?si=A_n5O3enQ3aizFwc6M86Ng";
        match fetch_spotify_playlist(url).await {
            Ok(tracks) => {
                println!("\n[TEST] Fetched {} tracks from playlist", tracks.len());
                for (i, t) in tracks.iter().take(3).enumerate() {
                    println!("[TEST]   #{}: {} - {}", i + 1, t.title, t.artists.join(", "));
                }
                println!("[TEST]   ...");
                if let Some(last) = tracks.last() {
                    println!(
                        "[TEST]   #{}: {} - {}",
                        tracks.len(),
                        last.title,
                        last.artists.join(", ")
                    );
                }
                assert!(
                    tracks.len() > 100,
                    "Expected >100 tracks (full playlist), got {}. Pagination failed.",
                    tracks.len()
                );
            }
            Err(e) => panic!("fetch_spotify_playlist failed: {e}"),
        }
    }
}
