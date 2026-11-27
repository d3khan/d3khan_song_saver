use std::path::{Path, PathBuf};

const BAD: [char; 9] = ['/', '\\', ':', '*', '?', '"', '<', '>', '|'];

pub fn safe(name: &str) -> String {
    name.replace(BAD, "_")
}

pub fn stem(title: &str, artist: &str) -> String {
    format!("{} - {}", safe(title), safe(artist))
}

/// Returns (audio_dir, target_dir). Videos nest under `videos/`.
pub fn dirs(output: &Path, folder: Option<&str>, is_video: bool) -> Result<(PathBuf, PathBuf), String> {
    let mut audio = output.to_path_buf();
    let mut target = output.to_path_buf();
    if let Some(f) = folder {
        audio = output.join(safe(f));
        target = if is_video { audio.join("videos") } else { audio.clone() };
        std::fs::create_dir_all(&target).map_err(|e| format!("Failed to create folder: {e}"))?;
    }
    Ok((audio, target))
}

pub fn find_bundled_binary(name: &str) -> Option<PathBuf> {
    let triple = env!("TAURI_ENV_TARGET_TRIPLE");
    let exe = std::env::current_exe().ok()?;
    let exe_dir = exe.parent()?;
    let direct_name = if cfg!(windows) { format!("{name}.exe") } else { name.to_string() };
    let direct = exe_dir.join(&direct_name);
    if direct.exists() {
        return Some(direct);
    }
    let suffixed = if cfg!(windows) { format!("{name}-{triple}.exe") } else { format!("{name}-{triple}") };
    for anc in exe_dir.ancestors() {
        let c = anc.join("binaries").join(&suffixed);
        if c.exists() {
            return Some(c);
        }
    }
    None
}

pub fn find_ffmpeg() -> Option<PathBuf> {
    find_bundled_binary("d3khan-ffmpeg").or_else(|| which::which("ffmpeg").ok())
}
