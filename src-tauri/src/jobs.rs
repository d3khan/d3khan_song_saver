//! Minimal job runner skeleton (SoundCloud pipeline).
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::naming;

pub struct JobStart {
    pub job_id: String,
    pub mode: String,
    pub input_mode: String,
    pub urls: Vec<String>,
    pub output_dir: PathBuf,
}

pub fn spawn_job(app: tauri::AppHandle, start: JobStart) {
    tauri::async_runtime::spawn(async move {
        // Minimal skeleton
        let _ = start;
    });
}

fn track_label(_meta: &crate::metadata::TrackMeta) -> String {
    String::new()
}
fn file_ok(_path: &std::path::Path) -> bool {
    false
}
