use futures_util::StreamExt;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Command;
use tauri::{AppHandle, Emitter, Manager};
use tokio::io::AsyncWriteExt;

#[derive(Clone, Serialize)]
pub struct DependencyStatus {
    pub name: String,
    pub status: String,
    pub progress: f32,
    pub message: String,
}

fn emit(app: &AppHandle, payload: DependencyStatus) {
    let _ = app.emit("dependency-status", payload);
}

pub fn bin_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_local_data_dir()
        .map_err(|e| e.to_string())?
        .join("bin");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

pub fn yt_dlp_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = bin_dir(app)?;
    let name = if cfg!(target_os = "windows") {
        "yt-dlp.exe"
    } else {
        "yt-dlp"
    };
    Ok(dir.join(name))
}

pub fn ffmpeg_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = bin_dir(app)?;
    let name = if cfg!(target_os = "windows") {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    };
    Ok(dir.join(name))
}

fn binary_version(path: &Path, arg: &str) -> Option<String> {
    let output = Command::new(path).arg(arg).output().ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    text.lines().next().map(|s| s.trim().to_string())
}

async fn latest_tag(url: &str) -> Result<String, String> {
    let client = reqwest::Client::new();
    let resp = client
        .get(url)
        .header("User-Agent", "d3khan-song-saver/0.2")
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let json: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    json.get("tag_name")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "No tag_name".to_string())
}

async fn download_resumable(
    app: &AppHandle,
    url: &str,
    dest: &Path,
    name: &str,
) -> Result<(), String> {
    let tmp = dest.with_extension("tmp");
    let mut start = 0u64;
    if tmp.exists() {
        start = std::fs::metadata(&tmp).map(|m| m.len()).unwrap_or(0);
    }

    let client = reqwest::Client::new();
    let mut req = client
        .get(url)
        .header("User-Agent", "d3khan-song-saver/0.2");
    if start > 0 {
        req = req.header("Range", format!("bytes={}-", start));
    }
    let resp = req.send().await.map_err(|e| e.to_string())?;
    let status = resp.status();
    if status == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
        std::fs::rename(&tmp, dest).map_err(|e| e.to_string())?;
        return Ok(());
    }
    if !status.is_success() {
        return Err(format!("HTTP {}", status));
    }

    let total = resp.content_length().unwrap_or(0) + start;
    let mut stream = resp.bytes_stream();
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&tmp)
        .await
        .map_err(|e| e.to_string())?;

    let mut written = start;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| e.to_string())?;
        file.write_all(&chunk).await.map_err(|e| e.to_string())?;
        written += chunk.len() as u64;
        let progress = if total > 0 {
            written as f32 / total as f32
        } else {
            0.0
        };
        emit(
            app,
            DependencyStatus {
                name: name.to_string(),
                status: "downloading".into(),
                progress,
                message: format!("Downloading {}… {:.0}%", name, progress * 100.0),
            },
        );
    }
    file.flush().await.map_err(|e| e.to_string())?;
    drop(file);
    std::fs::rename(&tmp, dest).map_err(|e| e.to_string())?;
    Ok(())
}

async fn extract_ffmpeg(app: &AppHandle, archive: &Path) -> Result<(), String> {
    let bin_dir = bin_dir(app)?;

    let result = (|| -> Result<(), String> {
        if cfg!(target_os = "windows") {
            let file = std::fs::File::open(archive).map_err(|e| e.to_string())?;
            let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
            for i in 0..zip.len() {
                let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
                let name = entry.name().to_string();
                if name.ends_with("ffmpeg.exe") || name.ends_with("ffprobe.exe") {
                    let out = bin_dir.join(Path::new(&name).file_name().unwrap());
                    let mut outfile = std::fs::File::create(&out).map_err(|e| e.to_string())?;
                    std::io::copy(&mut entry, &mut outfile).map_err(|e| e.to_string())?;
                }
            }
        } else {
            let file = std::fs::File::open(archive).map_err(|e| e.to_string())?;
            let decompressor = xz2::read::XzDecoder::new(file);
            let mut tar = tar::Archive::new(decompressor);
            for entry in tar.entries().map_err(|e| e.to_string())? {
                let mut entry = entry.map_err(|e| e.to_string())?;
                let path = entry.path().map_err(|e| e.to_string())?.to_path_buf();
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if name == "ffmpeg" || name == "ffprobe" {
                        let out = bin_dir.join(name);
                        let mut outfile = std::fs::File::create(&out).map_err(|e| e.to_string())?;
                        std::io::copy(&mut entry, &mut outfile).map_err(|e| e.to_string())?;
                    }
                }
            }
        }
        Ok(())
    })();

    if result.is_err() {
        // If extraction failed, the archive is likely corrupt. Delete it so it can be re-downloaded.
        let _ = std::fs::remove_file(archive);
    }

    result
}

pub async fn ensure_dependencies(app: AppHandle) {
    // 1. yt-dlp (Keep checking for updates because YouTube breaks often)
    if let Ok(path) = yt_dlp_path(&app) {
        let local = binary_version(&path, "--version").unwrap_or_default();
        let latest = latest_tag("https://api.github.com/repos/yt-dlp/yt-dlp/releases/latest")
            .await
            .unwrap_or_default();
        if latest.is_empty() || local != latest {
            emit(
                &app,
                DependencyStatus {
                    name: "yt-dlp".into(),
                    status: "updating".into(),
                    progress: 0.0,
                    message: "Updating yt-dlp…".into(),
                },
            );
            let url = if cfg!(target_os = "windows") {
                "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe"
            } else {
                "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp_linux"
            };
            if let Err(e) = download_resumable(&app, url, &path, "yt-dlp").await {
                emit(
                    &app,
                    DependencyStatus {
                        name: "yt-dlp".into(),
                        status: "error".into(),
                        progress: 0.0,
                        message: e,
                    },
                );
                return;
            }
            #[cfg(not(target_os = "windows"))]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755));
            }
        }
    }

    // 2. ffmpeg (Only download if the bins are missing, extract if zip exists)
    if let Ok(path) = ffmpeg_path(&app) {
        let ffprobe = bin_dir(&app).unwrap().join(if cfg!(target_os = "windows") {
            "ffprobe.exe"
        } else {
            "ffprobe"
        });

        // Only proceed if the binaries don't exist
        if !path.exists() || !ffprobe.exists() {
            emit(
                &app,
                DependencyStatus {
                    name: "ffmpeg".into(),
                    status: "updating".into(),
                    progress: 0.0,
                    message: "Installing ffmpeg…".into(),
                },
            );

            let archive_path = bin_dir(&app).unwrap().join(if cfg!(target_os = "windows") {
                "ffmpeg.zip"
            } else {
                "ffmpeg.tar.xz"
            });

            let mut need_download = true;

            // Check if we already have a valid zip file
            if archive_path.exists() {
                if cfg!(target_os = "windows") {
                    if let Ok(f) = std::fs::File::open(&archive_path) {
                        if zip::ZipArchive::new(f).is_ok() {
                            need_download = false;
                        }
                    }
                } else {
                    if std::fs::File::open(&archive_path).is_ok() {
                        need_download = false;
                    }
                }
            }

            // Download only if needed
            if need_download {
                let url = if cfg!(target_os = "windows") {
                    "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl.zip"
                } else {
                    "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-linux64-gpl.tar.xz"
                };
                if let Err(e) = download_resumable(&app, url, &archive_path, "ffmpeg").await {
                    emit(
                        &app,
                        DependencyStatus {
                            name: "ffmpeg".into(),
                            status: "error".into(),
                            progress: 0.0,
                            message: e,
                        },
                    );
                    return;
                }
            } else {
                emit(
                    &app,
                    DependencyStatus {
                        name: "ffmpeg".into(),
                        status: "extracting".into(),
                        progress: 0.5,
                        message: "Extracting existing ffmpeg archive…".into(),
                    },
                );
            }

            // Extract (this function no longer deletes the archive on success)
            if let Err(e) = extract_ffmpeg(&app, &archive_path).await {
                emit(
                    &app,
                    DependencyStatus {
                        name: "ffmpeg".into(),
                        status: "error".into(),
                        progress: 0.0,
                        message: e,
                    },
                );
                return;
            }
        }
    }

    emit(
        &app,
        DependencyStatus {
            name: "all".into(),
            status: "ready".into(),
            progress: 1.0,
            message: "Dependencies ready".into(),
        },
    );
}
