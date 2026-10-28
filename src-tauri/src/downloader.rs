use crate::metadata::TrackMeta;
use spotapi::SpotifySearch;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};
use tauri_plugin_shell::ShellExt;
use tauri_plugin_shell::process::CommandEvent;
use tokio::time::{Duration, sleep};

// Lofty Imports for Tagging
use lofty::config::WriteOptions;
use lofty::file::{AudioFile, TaggedFileExt};
use lofty::picture::Picture;
use lofty::tag::{Accessor, Tag, TagType};

// -------------------- Caching --------------------

fn get_cache_path(app: &AppHandle) -> PathBuf {
    let mut path = app
        .path()
        .app_local_data_dir()
        .unwrap_or_else(|_| PathBuf::from("."));
    path.push("search_cache.bin");
    path
}

fn load_cache(app: &AppHandle) -> HashMap<String, Vec<TrackMeta>> {
    let path = get_cache_path(app);
    if let Ok(data) = std::fs::read(path) {
        bincode::deserialize(&data).unwrap_or_default()
    } else {
        HashMap::new()
    }
}

fn save_cache(app: &AppHandle, cache: &HashMap<String, Vec<TrackMeta>>) {
    let path = get_cache_path(app);
    if let Ok(data) = bincode::serialize(cache) {
        let _ = std::fs::write(path, data);
    }
}

// -------------------- 1. Search & Metadata Resolution --------------------

pub async fn resolve_input(app: &AppHandle, input: &str) -> Result<Vec<TrackMeta>, String> {
    let trimmed = input.trim();

    if trimmed.contains("open.spotify.com/track/") {
        return fetch_single_track(trimmed).await;
    }

    if trimmed.contains("youtube.com") || trimmed.contains("youtu.be") {
        return Ok(vec![TrackMeta {
            id: trimmed.to_string(),
            title: "YouTube Link".into(),
            artists: vec!["User Provided".into()],
            ..Default::default()
        }]);
    }

    let cache_key = trimmed.to_lowercase();
    let mut cache = load_cache(app);

    if let Some(cached_results) = cache.get(&cache_key) {
        return Ok(cached_results.clone());
    }

    let mut search = SpotifySearch::new();
    let results = search
        .tracks(trimmed, 10)
        .await
        .map_err(|e| format!("Spotify search failed: {e}"))?;

    if !results.is_empty() {
        let mut mapped = Vec::new();
        for track in results {
            let track_id = track.uri.split(':').last().unwrap_or_default();
            let track_url = format!("https://open.spotify.com/track/{}", track_id);

            if let Ok(mut full_meta) = fetch_single_track(&track_url).await {
                if let Some(meta) = full_meta.pop() {
                    mapped.push(meta);
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

async fn fetch_single_track(spotify_url: &str) -> Result<Vec<TrackMeta>, String> {
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
        .map_err(|e| format!("Failed to read HTML: {e}"))?;

    let title_re = regex::Regex::new(r#"<meta property="og:title" content="([^"]+)""#).unwrap();
    let title = title_re
        .captures(&html)
        .map(|c| c[1].to_string())
        .ok_or("Could not extract title from Spotify link.")?;

    let desc_re =
        regex::Regex::new(r#"<meta property="og:description" content="([^"]+)""#).unwrap();
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

    let img_re = regex::Regex::new(r#"<meta property="og:image" content="([^"]+)""#).unwrap();
    let cover_url = img_re.captures(&html).map(|c| c[1].to_string());

    let dur_re = regex::Regex::new(r#"<meta property="music:duration" content="([^"]+)""#).unwrap();
    let duration_ms = dur_re
        .captures(&html)
        .and_then(|c| c[1].parse::<u64>().ok())
        .map(|secs| secs * 1000)
        .unwrap_or(0);

    return Ok(vec![TrackMeta {
        id: spotify_url.to_string(),
        title,
        artists: vec![artist],
        album: String::new(),
        release_date: year,
        cover_url,
        duration_ms,
        track_number: 1,
        ..Default::default()
    }]);
}

// -------------------- 2. Download & Tag --------------------

pub async fn download_batch(
    app: &AppHandle,
    tracks: Vec<TrackMeta>,
    output_dir: &Path,
    mut progress_cb: impl FnMut(usize, usize, &str, &str),
) -> Result<(), String> {
    let total = tracks.len();

    // Locate FFmpeg in AppData
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
        progress_cb(i, total, &meta.title, "Waiting 5s to avoid rate limits...");
        sleep(Duration::from_secs(5)).await;

        let safe_title = meta
            .title
            .replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_");
        let artist_str = meta.artists.join(", ");
        let safe_artist = if artist_str.is_empty() {
            "Unknown Artist".into()
        } else {
            artist_str.replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_")
        };

        let final_path = output_dir.join(format!("{} - {}.mp3", safe_title, safe_artist));
        let out_base = final_path.with_extension("");

        progress_cb(i, total, &meta.title, "Downloading...");

        let query = format!("{} {}", meta.title, artist_str);

        // Call Python sidecar
        let sidecar_command = app
            .shell()
            .sidecar("downloader")
            .map_err(|e| format!("Failed to find sidecar: {e}"))?
            .args([
                query.as_str(),
                out_base.to_string_lossy().as_ref(),
                ffmpeg_str.as_str(),
                meta.title.as_str(),
                artist_str.as_str(),
            ]);

        let (mut rx, _child) = sidecar_command
            .spawn()
            .map_err(|e| format!("Failed to spawn: {e}"))?;

        let mut success = false;
        while let Some(event) = rx.recv().await {
            if let CommandEvent::Stdout(line) = event {
                let text = String::from_utf8_lossy(&line);
                if text.contains("\"status\": \"success\"") {
                    success = true;
                    break;
                } else if text.contains("\"status\": \"error\"") {
                    eprintln!("Python download failed: {}", text);
                    break;
                }
            }
        }

        if success && final_path.exists() {
            progress_cb(i, total, &meta.title, "Adding remaining metadata...");
            if let Err(e) = tag_file(&final_path, meta).await {
                eprintln!("Failed to tag {}: {}", meta.title, e);
            }
        }
    }

    progress_cb(total, total, "Complete", "All done!");
    Ok(())
}

async fn tag_file(path: &Path, meta: &TrackMeta) -> Result<(), String> {
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    let mut tagged_file = lofty::read_from(&mut file).map_err(|e| e.to_string())?;

    // Force ID3v2.3 (Windows Explorer requires this for thumbnails)
    let tag = match tagged_file.tag_mut(TagType::Id3v2) {
        Some(t) => t,
        None => {
            tagged_file.insert_tag(Tag::new(TagType::Id3v2));
            tagged_file.tag_mut(TagType::Id3v2).unwrap()
        }
    };

    // Add Album, Year, Track Number, Genre
    tag.set_album(meta.album.clone());
    if let Some(year) = &meta.release_date {
        if year.len() >= 4 {
            if let Ok(y) = year[0..4].parse() {
                tag.set_year(y);
            }
        }
    }
    if meta.track_number > 0 {
        tag.set_track(meta.track_number);
    }
    if let Some(genre) = meta.genres.first() {
        tag.set_genre(genre.clone());
    }

    // SAFE COVER ART SWAP:
    // We only remove the old picture if we successfully download AND parse the new one.
    if let Some(cover_url) = &meta.cover_url {
        if let Ok(resp) = reqwest::get(cover_url).await {
            if let Ok(img) = resp.bytes().await {
                let mut cursor = std::io::Cursor::new(img.to_vec());
                if let Ok(pic) = Picture::from_reader(&mut cursor) {
                    // Success! Now it is safe to remove the YouTube thumbnail
                    tag.remove_picture_type(lofty::picture::PictureType::CoverFront);
                    tag.push_picture(pic);
                }
            }
        }
    }

    // Save with ID3v2.3
    let write_options = WriteOptions::new().use_id3v23(true);
    tagged_file
        .save_to_path(path, write_options)
        .map_err(|e| e.to_string())?;

    Ok(())
}
