//! Tauri commands exposed to the frozen frontend.

use std::path::PathBuf;
use std::sync::atomic::Ordering;

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_dialog::DialogExt;

use rspotify::prelude::OAuthClient;

use crate::auth;
use crate::downloader;
use crate::jobs::{self, JobStart};
use crate::{AppState, ConflictResolution, JobControl};

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AuthStatus {
    pub authenticated: bool,
    pub configured: bool,
    pub display_name: Option<String>,
    pub redirect_uri: String,
    pub error: Option<String>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SettingsView {
    pub client_id: Option<String>,
    pub has_secret: bool,
    pub redirect_uri: String,
    pub source: String,
}

pub async fn auth_status_snapshot(state: &AppState) -> AuthStatus {
    let settings = state.settings.lock().unwrap().clone();
    let authenticated = state.auth.lock().await.is_some();
    let display_name = state.display_name.lock().unwrap().clone();
    AuthStatus {
        authenticated,
        configured: settings.client_id.is_some(),
        display_name,
        redirect_uri: settings.redirect_uri,
        error: None,
    }
}

async fn push_auth_status(app: &AppHandle, state: &AppState) -> AuthStatus {
    let status = auth_status_snapshot(state).await;
    let _ = app.emit("auth://status", status.clone());
    status
}

fn settings_view(state: &AppState) -> SettingsView {
    let settings = state.settings.lock().unwrap().clone();
    SettingsView {
        client_id: settings.client_id,
        has_secret: auth::load_secret().is_some(),
        redirect_uri: settings.redirect_uri,
        source: settings.source,
    }
}

#[tauri::command]
pub async fn pick_folder(app: AppHandle) -> Result<Option<String>, String> {
    let picked = app.dialog().file().blocking_pick_folder();
    match picked {
        Some(path) => {
            let path = path
                .into_path()
                .map_err(|error| format!("Could not read the selected folder: {error}"))?;
            Ok(Some(path.to_string_lossy().into_owned()))
        }
        None => Ok(None),
    }
}

#[tauri::command]
pub async fn auth_status(state: State<'_, AppState>) -> Result<AuthStatus, String> {
    Ok(auth_status_snapshot(&state).await)
}

#[tauri::command]
pub async fn auth_connect(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<AuthStatus, String> {
    if state.auth_waiting.swap(true, Ordering::SeqCst) {
        return Err("A Spotify sign-in is already in progress.".to_string());
    }

    let result = connect_inner(&state).await;
    state.auth_waiting.store(false, Ordering::SeqCst);

    match result {
        Ok(status) => {
            let _ = app.emit("auth://status", status.clone());
            Ok(status)
        }
        Err(error) => {
            let mut status = auth_status_snapshot(&state).await;
            status.error = Some(error.clone());
            let _ = app.emit("auth://status", status);
            Err(error)
        }
    }
}

async fn connect_inner(state: &AppState) -> Result<AuthStatus, String> {
    let settings = state.settings.lock().unwrap().clone();
    let client_id = settings
        .client_id
        .clone()
        .ok_or_else(|| "Add your Spotify app Client ID in Settings first.".to_string())?;

    if state.auth.lock().await.is_some() {
        return Ok(auth_status_snapshot(state).await);
    }

    state.auth_cancel.store(false, Ordering::SeqCst);

    let mut client = auth::build_client(&client_id, &settings.redirect_uri);
    let port = auth::redirect_port(&settings.redirect_uri);

    // Bind before opening the browser so a busy port fails fast.
    let listener = auth::bind_callback(port)?;

    let url = client
        .get_authorize_url(None)
        .map_err(|error| format!("Could not start Spotify sign-in: {error}"))?;
    let expected_state = client.oauth.state.clone();

    if let Err(error) = webbrowser::open(&url) {
        return Err(format!(
            "Could not open your browser ({error}). Open this URL to sign in: {url}"
        ));
    }

    let code = auth::await_callback(listener, expected_state, state.auth_cancel.clone()).await?;

    client
        .request_token(&code)
        .await
        .map_err(|error| format!("Spotify sign-in failed: {error}"))?;

    auth::persist_token(&client).await;
    let display_name = auth::fetch_display_name(&client).await;

    *state.auth.lock().await = Some(client);
    *state.display_name.lock().unwrap() = display_name;

    Ok(auth_status_snapshot(state).await)
}

#[tauri::command]
pub async fn auth_cancel(app: AppHandle, state: State<'_, AppState>) -> Result<AuthStatus, String> {
    state.auth_cancel.store(true, Ordering::SeqCst);
    Ok(push_auth_status(&app, &state).await)
}

#[tauri::command]
pub async fn auth_logout(app: AppHandle, state: State<'_, AppState>) -> Result<AuthStatus, String> {
    state.auth_cancel.store(true, Ordering::SeqCst);
    auth::delete_token();
    *state.auth.lock().await = None;
    *state.display_name.lock().unwrap() = None;
    Ok(push_auth_status(&app, &state).await)
}

#[tauri::command]
pub async fn get_settings(state: State<'_, AppState>) -> Result<SettingsView, String> {
    Ok(settings_view(&state))
}

#[tauri::command(rename_all = "camelCase")]
pub async fn save_settings(
    app: AppHandle,
    client_id: String,
    client_secret: Option<String>,
    state: State<'_, AppState>,
) -> Result<SettingsView, String> {
    let client_id = client_id.trim().to_string();

    if client_id.is_empty() {
        auth::clear_settings_file(&state.settings_path)?;
        auth::delete_token();
        auth::delete_secret();
        *state.auth.lock().await = None;
        *state.display_name.lock().unwrap() = None;
        *state.settings.lock().unwrap() = auth::resolve_settings(&state.settings_path);
        push_auth_status(&app, &state).await;
        return Ok(settings_view(&state));
    }

    let active = { state.settings.lock().unwrap().client_id.clone() };
    if active.as_deref() != Some(client_id.as_str()) {
        // Tokens are issued per client id.
        auth::delete_token();
        *state.auth.lock().await = None;
        *state.display_name.lock().unwrap() = None;
    }

    auth::write_settings_file(&state.settings_path, &client_id)?;

    if let Some(secret) = client_secret {
        let secret = secret.trim();
        if !secret.is_empty() {
            auth::store_secret(secret)?;
        }
    }

    *state.settings.lock().unwrap() = auth::resolve_settings(&state.settings_path);
    push_auth_status(&app, &state).await;
    Ok(settings_view(&state))
}

#[tauri::command(rename_all = "camelCase")]
pub async fn start_job(
    app: AppHandle,
    state: State<'_, AppState>,
    mode: String,
    input_mode: String,
    url: Option<String>,
    urls: Option<Vec<String>>,
    output_dir: String,
) -> Result<String, String> {
    if mode != "download" && mode != "sync" {
        return Err("Unknown job mode.".to_string());
    }
    if input_mode != "single" && input_mode != "bulk" {
        return Err("Unknown input mode.".to_string());
    }
    if mode == "sync" && input_mode == "bulk" {
        return Err("Sync works with a single playlist link only.".to_string());
    }

    let output_dir = output_dir.trim();
    if output_dir.is_empty() {
        return Err("Choose an output folder first.".to_string());
    }
    let output_path = PathBuf::from(output_dir);
    std::fs::create_dir_all(&output_path)
        .map_err(|error| format!("Could not create {}: {error}", output_path.display()))?;
    if !output_path.is_dir() {
        return Err(format!("{} is not a folder.", output_path.display()));
    }

    let mut links: Vec<String> = Vec::new();
    if input_mode == "single" {
        if let Some(value) = url {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                links.push(trimmed.to_string());
            }
        }
    } else if let Some(values) = urls {
        for value in values {
            let trimmed = value.trim();
            if !trimmed.is_empty() && !links.iter().any(|existing| existing == trimmed) {
                links.push(trimmed.to_string());
            }
        }
    }

    if links.is_empty() {
        return Err("Paste at least one Spotify link.".to_string());
    }

    {
        let client = state.auth.lock().await;
        if client.is_none() {
            return Err("Connect Spotify in Settings first.".to_string());
        }
    }

    // Fail fast when the download tools are missing.
    downloader::discover_binaries()?;

    if state
        .running
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Err("A job is already running.".to_string());
    }

    let job_id = uuid::Uuid::new_v4().to_string();
    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    {
        let mut slot = state.job.lock().unwrap();
        *slot = Some(JobControl {
            job_id: job_id.clone(),
            cancel,
        });
    }

    jobs::spawn_job(
        app,
        JobStart {
            job_id: job_id.clone(),
            mode,
            input_mode,
            urls: links,
            output_dir: output_path,
        },
    );

    Ok(job_id)
}

#[tauri::command(rename_all = "camelCase")]
pub fn cancel_job(job_id: String, state: State<'_, AppState>) -> Result<(), String> {
    let matched = {
        let slot = state.job.lock().unwrap();
        match slot.as_ref() {
            Some(job) if job.job_id == job_id => {
                job.cancel.store(true, Ordering::SeqCst);
                true
            }
            _ => false,
        }
    };

    if matched {
        // Dropping the senders unblocks prompts immediately.
        state.pending_deletions.lock().unwrap().clear();
        state.pending_conflicts.lock().unwrap().clear();
        state.conflict_matches.lock().unwrap().clear();
    }

    Ok(())
}

#[tauri::command(rename_all = "camelCase")]
pub fn confirm_deletions(
    request_id: String,
    decision: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    if decision != "delete" && decision != "keep" {
        return Err("Unknown deletion decision.".to_string());
    }
    let sender = state
        .pending_deletions
        .lock()
        .unwrap()
        .remove(&request_id)
        .ok_or_else(|| "request-already-resolved".to_string())?;
    sender
        .send(decision)
        .map_err(|_| "worker-gone".to_string())?;
    Ok(())
}

#[tauri::command(rename_all = "camelCase")]
pub fn resolve_conflict(
    request_id: String,
    choice: String,
    dir: Option<String>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let resolution = match choice.as_str() {
        "syncHere" => {
            let Some(dir) = dir else {
                return Err("Choose a folder to sync into.".to_string());
            };
            let candidate = PathBuf::from(dir);
            let allowed = state
                .conflict_matches
                .lock()
                .unwrap()
                .get(&request_id)
                .map(|matches| matches.iter().any(|item| same_path(item, &candidate)))
                .unwrap_or(false);
            if !allowed {
                return Err("That folder is not one of the offered matches.".to_string());
            }
            ConflictResolution::SyncHere(candidate)
        }
        "createNew" => ConflictResolution::CreateNew,
        "cancel" => ConflictResolution::Cancel,
        _ => return Err("Unknown conflict choice.".to_string()),
    };

    let sender = state.pending_conflicts.lock().unwrap().remove(&request_id);
    let Some(sender) = sender else {
        state.conflict_matches.lock().unwrap().remove(&request_id);
        return Err("request-already-resolved".to_string());
    };
    state.conflict_matches.lock().unwrap().remove(&request_id);
    sender
        .send(resolution)
        .map_err(|_| "worker-gone".to_string())?;
    Ok(())
}

fn same_path(a: &std::path::Path, b: &std::path::Path) -> bool {
    if a == b {
        return true;
    }
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}
