//! Minimal job runner skeleton (SoundCloud pipeline).
use std::path::PathBuf;

pub struct JobStart {
    pub job_id: String,
    pub mode: String,
    pub input_mode: String,
    pub urls: Vec<String>,
    pub output_dir: PathBuf,
}

pub fn spawn_job(_app: tauri::AppHandle, _start: JobStart) {
    tauri::async_runtime::spawn(async move {
        // Minimal skeleton
        // When you are ready to implement background jobs, you will trigger
        // downloader::download_track() in a loop here.
    });
}
