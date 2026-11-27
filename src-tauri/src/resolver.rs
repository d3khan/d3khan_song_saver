//! Turns any user input into TrackMeta via the sidecar.
//! Results are cached to disk so repeat searches cost zero bandwidth.
use crate::metadata::TrackMeta;
use crate::sidecar;
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

type Cache = HashMap<String, Vec<TrackMeta>>;

fn cache_path(app: &AppHandle) -> PathBuf {
    let mut p = app
        .path()
        .app_local_data_dir()
        .unwrap_or_else(|_| PathBuf::from("."));
    p.push("search_cache.bin");
    p
}

fn load_cache(app: &AppHandle) -> Cache {
    std::fs::read(cache_path(app))
        .ok()
        .and_then(|d| bincode::deserialize(&d).ok())
        .unwrap_or_default()
}

fn save_cache(app: &AppHandle, cache: &Cache) {
    if let Ok(d) = bincode::serialize(cache) {
        let _ = std::fs::write(cache_path(app), d);
    }
}

fn map_tracks(arr: &[Value]) -> Vec<TrackMeta> {
    arr.iter()
        .map(|t| TrackMeta {
            id: t["id"].as_str().unwrap_or("").to_string(),
            title: t["title"].as_str().unwrap_or("Unknown Title").to_string(),
            artists: t["artists"]
                .as_array()
                .unwrap_or(&vec![])
                .iter()
                .filter_map(|a| a.as_str().map(String::from))
                .collect(),
            cover_url: t["cover_url"].as_str().map(String::from),
            duration_ms: t["duration_ms"].as_u64().unwrap_or(0),
            track_number: t["track_number"].as_u64().unwrap_or(0) as u32,
            folder_name: t["folder_name"].as_str().map(String::from),
            playlist_url: t["playlist_url"].as_str().map(String::from),
            ..Default::default()
        })
        .collect()
}

pub async fn resolve(app: &AppHandle, input: &str) -> Result<Vec<TrackMeta>, String> {
    let key = format!("resolve:{}", input.trim().to_lowercase());
    let mut cache = load_cache(app);
    if let Some(hit) = cache.get(&key) {
        return Ok(hit.clone());
    }
    let v = sidecar::call(app, "downloader", &["--resolve", input]).await?;
    let arr = v["tracks"].as_array().cloned().unwrap_or_default();
    let mapped = map_tracks(&arr);
    if mapped.is_empty() {
        return Err("No tracks found for that input.".into());
    }
    cache.insert(key, mapped.clone());
    save_cache(app, &cache);
    Ok(mapped)
}

pub async fn search(app: &AppHandle, query: &str, limit: u32) -> Result<Vec<TrackMeta>, String> {
    let key = format!("search:{}:{}", query.trim().to_lowercase(), limit);
    let mut cache = load_cache(app);
    if let Some(hit) = cache.get(&key) {
        return Ok(hit.clone());
    }
    let limit_str = limit.to_string();
    let v = sidecar::call(app, "downloader", &["--search", query, limit_str.as_str()]).await?;
    let arr = v["tracks"].as_array().cloned().unwrap_or_default();
    let mapped = map_tracks(&arr);
    if !mapped.is_empty() {
        cache.insert(key, mapped.clone());
        save_cache(app, &cache);
    }
    Ok(mapped)
}
