pub mod convert;
pub mod paths;
pub mod state;

use crate::batch::BatchControl;
use crate::metadata::TrackMeta;
use crate::sidecar;
use serde_json::json;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter, Manager};
use tokio::time::{sleep, Duration};

const MAX_ATTEMPTS: u32 = 3;

fn is_bot(text: &str) -> bool {
    text.contains("Sign in to confirm") || text.contains("not a bot") || text.contains("LOGIN_REQUIRED")
}

fn is_retryable(err: &str) -> bool {
    err.contains("WinError 32") || err.contains("being used by another process") || err.contains("HTTP Error 416")
}

pub async fn download_batch(
    app: &AppHandle,
    tracks: Vec<TrackMeta>,
    output_dir: &Path,
    overwrite_mode: bool,
    media_mode: &str,
    quality: &str,
) -> Result<(), String> {
    let total = tracks.len();
    let is_video = media_mode == "video";
    let ffmpeg_path = paths::find_ffmpeg().ok_or_else(|| "FFmpeg not found.".to_string())?;
    let ffmpeg_str = ffmpeg_path.to_string_lossy().to_string();

    let mut folder_state: Option<(PathBuf, state::PlaylistState)> = None;
    let mut last_state_path = PathBuf::new();

    for (i, meta) in tracks.iter().enumerate() {
        let is_youtube = meta.id.starts_with("http")
            && (meta.id.contains("youtube.com")
                || meta.id.contains("music.youtube.com")
                || meta.id.contains("youtu.be"));
        let query = if is_youtube {
            meta.id.clone()
        } else {
            format!("{} {}", meta.title, meta.artists.join(", "))
        };

        let (audio_dir, target_dir) = paths::dirs(output_dir, meta.folder_name.as_deref(), is_video)?;
        let state_path = audio_dir.join(".playlist_sync.json");
        last_state_path = state_path.clone();

        let needs_load = folder_state.as_ref().map(|(p, _)| p != &state_path).unwrap_or(true);
        if needs_load {
            let st = state::load_or_new(&state_path, meta);
            folder_state = Some((state_path.clone(), st));
        }
        let (_, st) = folder_state.as_mut().unwrap();

        let file_stem = paths::stem(&meta.title, &meta.artists.join(", "));
        let out_base = target_dir.join(&file_stem);
        let audio_base = audio_dir.join(&file_stem);

        // ---- bandwidth saver: reuse local video, YouTube sources only ----
        if !is_video && is_youtube {
            let bases = vec![audio_dir.join(&file_stem), audio_dir.join("videos").join(&file_stem)];
            let mut converted = false;
            'outer: for base in &bases {
                for ext in &["mp4", "webm", "mkv"] {
                    let vp = base.with_extension(ext);
                    if !vp.exists() {
                        continue;
                    }
                    let ap = audio_base.with_extension("mp3");
                    if ap.exists() && !overwrite_mode {
                        converted = true;
                        break 'outer;
                    }
                    let _ = app.emit("download-progress", json!({
                        "percent": (i as f64 / total as f64) * 100.0,
                        "status": format!("Converting existing video to MP3: {}", meta.title)
                    }));
                    match convert::video_to_audio(&vp, &ap, &ffmpeg_path,
                        meta.cover_url.as_deref(), &meta.title, &meta.artists.join(", ")).await {
                        Ok(()) => {
                            let key = state::sync_key(meta, "audio");
                            st.downloaded_tracks.insert(key.clone());
                            st.manifest.insert(key, ap.to_string_lossy().to_string());
                            state::save(&state_path, st);
                            let _ = app.emit("download-progress", json!({
                                "percent": ((i as f64 + 1.0) / total as f64) * 100.0,
                                "status": format!("Converted (bandwidth saved): {}", meta.title)
                            }));
                            converted = true;
                            break 'outer;
                        }
                        Err(e) => {
                            let _ = app.emit("download-error", json!({
                                "error": format!("Video→audio conversion failed for {}: {}", meta.title, e)
                            }));
                        }
                    }
                }
            }
            if converted {
                continue;
            }
        }

        // ---- skip-if-present (sync mode) ----
        let exts: &[&str] = if is_video { &["mp4", "webm", "mkv", "m4a"] } else { &["mp3", "m4a", "opus", "webm"] };
        if !overwrite_mode {
            let key = state::sync_key(meta, media_mode);
            if st.downloaded_tracks.contains(&key) {
                let _ = app.emit("download-progress", json!({"percent": ((i as f64 + 1.0) / total as f64) * 100.0, "status": format!("Skipped: {}", meta.title)}));
                continue;
            }
            if let Some(ex) = exts.iter().map(|e| out_base.with_extension(e)).find(|p| p.exists()) {
                st.downloaded_tracks.insert(key.clone());
                st.manifest.insert(key, ex.to_string_lossy().to_string());
                state::save(&state_path, st);
                let _ = app.emit("download-progress", json!({"percent": ((i as f64 + 1.0) / total as f64) * 100.0, "status": format!("Skipped (exists): {}", meta.title)}));
                continue;
            }
        } else {
            for e in exts.iter().chain(["part", "ytdl", "temp.mp3", "temp.mp4"].iter()) {
                let p = out_base.with_extension(e);
                if p.exists() {
                    let _ = std::fs::remove_file(&p);
                }
            }
        }

        // ---- download with retries ----
        let mut success = false;
        let mut attempts = 0u32;
        let mut final_path: Option<PathBuf> = None;
        let mut error_emitted = false;
        let mut bot_paused = false;

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

            let args = vec![
                query.clone(),
                out_base.to_string_lossy().to_string(),
                ffmpeg_str.clone(),
                meta.title.clone(),
                meta.artists.join(", "),
                media_mode.to_string(),
                meta.duration_ms.to_string(),
                quality.to_string(),
            ];

            let app_evt = app.clone();
            let title = meta.title.clone();
            let base_pct = (i as f64 / total as f64) * 100.0;
            let outcome = sidecar::run_download(app, args, move |v| {
                match v.get("event").and_then(|e| e.as_str()) {
                    Some("progress") => {
                        let _ = app_evt.emit("download-bytes", json!({
                            "title": title,
                            "percent": v["percent"],
                            "downloaded": v["downloaded"],
                            "total": v["total"],
                            "speed": v["speed"],
                            "eta": v["eta"],
                        }));
                    }
                    Some("stage") => {
                        let _ = app_evt.emit("download-progress", json!({
                            "percent": base_pct,
                            "status": v["message"],
                        }));
                    }
                    _ => {}
                }
            })
            .await;

            match outcome {
                Ok(Some(p)) if p.exists() => {
                    success = true;
                    final_path = Some(p);
                }
                Ok(Some(_)) => attempts += 1,
                Ok(None) => attempts += 1,
                Err(err) => {
                    if is_bot(&err) {
                        if !bot_paused {
                            bot_paused = true;
                            let _ = app.emit("batch-paused", json!({
                                "reason": "bot-check",
                                "title": meta.title,
                                "message": "YouTube is asking for verification. Watch any short video in your browser, then click Continue."
                            }));
                            let ctrl = app.state::<BatchControl>();
                            ctrl.pause();
                            ctrl.wait_until_resumed().await;
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
                            if let Some(p) = final_path.as_ref() {
                                let _ = std::fs::remove_file(p);
                            }
                            for e in ["mp3", "mp4", "webm", "mkv", "m4a", "part", "ytdl", "temp.mp3", "temp.mp4"] {
                                let p = out_base.with_extension(e);
                                if p.exists() {
                                    let _ = std::fs::remove_file(&p);
                                }
                            }
                        }
                        attempts += 1;
                        continue;
                    }
                    let _ = app.emit("download-error", json!({"error": format!("Failed to download {}: {}", meta.title, err)}));
                    error_emitted = true;
                    break;
                }
            }
        }

        if success {
            let key = state::sync_key(meta, media_mode);
            st.downloaded_tracks.insert(key.clone());
            if let Some(ref p) = final_path {
                st.manifest.insert(key, p.to_string_lossy().to_string());
            }
            state::save(&state_path, st);
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

    // ---- sync deletion prompt ----
    if !overwrite_mode && !last_state_path.as_os_str().is_empty() {
        if let Some((_, st)) = folder_state.as_ref() {
            let current: HashSet<String> = tracks.iter().map(|m| state::sync_key(m, media_mode)).collect();
            let orph = state::orphans(st, &current);
            if !orph.is_empty() {
                let _ = app.emit("sync-deletion-prompt", json!({
                    "files": orph,
                    "timeout_ms": 10000,
                    "state_path": last_state_path.to_string_lossy().to_string()
                }));
            }
        }
    }

    let _ = app.emit("download-progress", json!({ "percent": 100.0, "status": "All done!" }));
    Ok(())
}
