//! Job runner: metadata resolution, folder layout, playlist sync, prompts and
//! progress/event emission.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::oneshot;

use crate::auth;
use crate::downloader::{self, Binaries, DownloadError};
use crate::metadata::{self, Collection, CollectionKind, TrackMeta};
use crate::naming;
use crate::sync::{self, EntryStatus, RemovedTrack, SyncEntry};
use crate::{AppState, ConflictResolution};

const PROMPT_TIMEOUT_MS: u64 = 600_000;
const JOB_START_DELAY_MS: u64 = 200;

pub struct JobStart {
    pub job_id: String,
    pub mode: String,
    pub input_mode: String,
    pub urls: Vec<String>,
    pub output_dir: PathBuf,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ProgressPayload {
    job_id: String,
    phase: String,
    current: usize,
    total: usize,
    percent: Option<u8>,
    title: Option<String>,
    message: Option<String>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct LogPayload {
    job_id: String,
    level: String,
    message: String,
    timestamp: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct TrackFailedPayload {
    job_id: String,
    title: String,
    artist: String,
    error: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct RemovedTrackPayload {
    track_id: String,
    title: String,
    artist: String,
    local_filename: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct RemovedPayload {
    job_id: String,
    request_id: String,
    folder: String,
    playlist_name: String,
    tracks: Vec<RemovedTrackPayload>,
    timeout_ms: u64,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ConflictMatchPayload {
    dir: String,
    playlist_name: String,
    track_count: usize,
    updated_at: Option<String>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ConflictPayload {
    job_id: String,
    request_id: String,
    playlist_name: String,
    playlist_id: String,
    target_dir: String,
    matches: Vec<ConflictMatchPayload>,
    timeout_ms: u64,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct CompletePayload {
    job_id: String,
    status: String,
    downloaded: usize,
    skipped: usize,
    failed: usize,
    removed: usize,
    message: String,
}

enum Outcome {
    Completed,
    Cancelled,
}

enum ProcessError {
    Cancelled,
    Failed(String),
}

enum DeletionChoice {
    Delete,
    Keep,
}

pub fn spawn_job(app: AppHandle, start: JobStart) {
    tauri::async_runtime::spawn(async move {
        run_job(app, start).await;
    });
}

/// Restore the display name / refresh the token in the background at startup.
pub async fn restore_session(app: AppHandle) {
    let state = app.state::<AppState>();
    let client = { state.auth.lock().await.clone() };
    let Some(client) = client else {
        return;
    };

    // Refresh before touching the API: access tokens only live an hour, and a
    // job started later must not begin with a token that is about to die.
    match auth::refresh_session(&client).await {
        auth::SessionRefresh::Refreshed => eprintln!("Refreshed the Spotify access token."),
        auth::SessionRefresh::Rejected => {
            eprintln!("Spotify no longer accepts the stored session; signing out.");
            *state.auth.lock().await = None;
            *state.display_name.lock().unwrap() = None;
            let status = crate::commands::auth_status_snapshot(&state).await;
            let _ = app.emit("auth://status", status);
            return;
        }
        auth::SessionRefresh::Fresh => {}
        auth::SessionRefresh::Failed(error) => {
            // Keep the session: the access token may still be valid, and the job
            // path retries once on a 401.
            eprintln!("{error}");
        }
    }

    if let Some(name) = auth::fetch_display_name(&client).await {
        *state.display_name.lock().unwrap() = Some(name);
    }
    auth::persist_token(&client).await;

    let status = crate::commands::auth_status_snapshot(&state).await;
    let _ = app.emit("auth://status", status);
}

async fn run_job(app: AppHandle, start: JobStart) {
    tokio::time::sleep(std::time::Duration::from_millis(JOB_START_DELAY_MS)).await;

    let job_id = start.job_id.clone();
    let cancel = {
        let state = app.state::<AppState>();
        let slot = state.job.lock().unwrap();
        match slot.as_ref() {
            Some(job) if job.job_id == job_id => job.cancel.clone(),
            _ => Arc::new(AtomicBool::new(false)),
        }
    };

    let guard = JobGuard {
        app: app.clone(),
        job_id: job_id.clone(),
    };

    let mut runner = JobRunner {
        app: app.clone(),
        job_id,
        mode: start.mode.clone(),
        is_bulk: start.input_mode == "bulk",
        cancel,
        bins: None,
        total: 0,
        processed: 0,
        downloaded: 0,
        skipped: 0,
        failed: 0,
        removed: 0,
    };

    let result = runner.execute(&start).await;
    let (status, message) = match result {
        Ok(Outcome::Completed) => ("completed", runner.summary()),
        Ok(Outcome::Cancelled) => ("cancelled", "Cancelled.".to_string()),
        Err(message) => ("failed", message),
    };

    let level = match status {
        "completed" => "success",
        "failed" => "error",
        _ => "warn",
    };
    runner.emit_log(level, &message);
    runner.emit_complete(status, &message);

    drop(guard);
}

struct JobGuard {
    app: AppHandle,
    job_id: String,
}

impl Drop for JobGuard {
    fn drop(&mut self) {
        let state = self.app.state::<AppState>();
        {
            let mut slot = state.job.lock().unwrap();
            if slot
                .as_ref()
                .map(|job| job.job_id == self.job_id)
                .unwrap_or(false)
            {
                *slot = None;
            }
        }
        state.running.store(false, Ordering::SeqCst);
        state.pending_deletions.lock().unwrap().clear();
        state.pending_conflicts.lock().unwrap().clear();
        state.conflict_matches.lock().unwrap().clear();
    }
}

struct JobRunner {
    app: AppHandle,
    job_id: String,
    mode: String,
    is_bulk: bool,
    cancel: Arc<AtomicBool>,
    bins: Option<Binaries>,
    total: usize,
    processed: usize,
    downloaded: usize,
    skipped: usize,
    failed: usize,
    removed: usize,
}

impl JobRunner {
    fn state(&self) -> tauri::State<'_, AppState> {
        self.app.state::<AppState>()
    }

    fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }

    fn percent(&self) -> Option<u8> {
        if self.total == 0 {
            None
        } else {
            let value = self.processed as f64 / self.total as f64 * 100.0;
            Some(value.round().clamp(0.0, 100.0) as u8)
        }
    }

    fn emit_log(&self, level: &str, message: &str) {
        let _ = self.app.emit(
            "download://log",
            LogPayload {
                job_id: self.job_id.clone(),
                level: level.to_string(),
                message: message.to_string(),
                timestamp: chrono::Utc::now().to_rfc3339(),
            },
        );
    }

    fn emit_progress(
        &self,
        phase: &str,
        percent: Option<u8>,
        title: Option<String>,
        message: Option<String>,
    ) {
        let _ = self.app.emit(
            "download://progress",
            ProgressPayload {
                job_id: self.job_id.clone(),
                phase: phase.to_string(),
                current: self.processed,
                total: self.total,
                percent,
                title,
                message,
            },
        );
    }

    fn emit_track_failed(&self, meta: &TrackMeta, error: &str) {
        let _ = self.app.emit(
            "download://track_failed",
            TrackFailedPayload {
                job_id: self.job_id.clone(),
                title: meta.title.clone(),
                artist: naming::display_artist(&meta.artists),
                error: error.to_string(),
            },
        );
    }

    fn emit_complete(&self, status: &str, message: &str) {
        let _ = self.app.emit(
            "sync://complete",
            CompletePayload {
                job_id: self.job_id.clone(),
                status: status.to_string(),
                downloaded: self.downloaded,
                skipped: self.skipped,
                failed: self.failed,
                removed: self.removed,
                message: message.to_string(),
            },
        );
    }

    fn summary(&self) -> String {
        let mut message = format!(
            "Downloaded {}, skipped {}, failed {}",
            self.downloaded, self.skipped, self.failed
        );
        if self.removed > 0 {
            message.push_str(&format!(", removed {}", self.removed));
        }
        message.push('.');
        message
    }

    async fn execute(&mut self, start: &JobStart) -> Result<Outcome, String> {
        self.emit_log(
            "info",
            &format!(
                "Job started - {} mode, {} link{}.",
                start.mode,
                start.urls.len(),
                if start.urls.len() == 1 { "" } else { "s" }
            ),
        );
        self.emit_progress("preparing", None, None, Some("Preparing...".to_string()));

        let bins = downloader::discover_binaries().map_err(|error| {
            self.emit_log("error", &error);
            error
        })?;
        self.bins = Some(bins);

        let client = {
            let state = self.state();
            let guard = state.auth.lock().await;
            guard
                .clone()
                .ok_or_else(|| "Connect Spotify in Settings first.".to_string())?
        };

        // Cheap no-op when the token is fresh; avoids failing the first request
        // of a job that starts more than an hour after the app did.
        if let auth::SessionRefresh::Failed(error) = auth::refresh_session(&client).await {
            eprintln!("{error}");
        }

        self.emit_progress(
            "metadata",
            None,
            None,
            Some("Reading track lists from Spotify.".to_string()),
        );

        let mut collections: Vec<Collection> = Vec::new();
        let mut last_error: Option<String> = None;

        for raw in &start.urls {
            if self.is_cancelled() {
                return Ok(Outcome::Cancelled);
            }
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                continue;
            }
            let (kind, id) = match metadata::parse_spotify_url(trimmed) {
                Ok(parsed) => parsed,
                Err(error) => {
                    self.emit_log("warn", &format!("{error} ({trimmed})"));
                    last_error = Some(error);
                    continue;
                }
            };
            self.emit_log("info", &format!("Reading {trimmed}"));
            let mut fetched = metadata::fetch_collection(&client, kind, &id, trimmed).await;
            if let Err(error) = &fetched {
                if metadata::is_sign_in_error(error) && auth::refresh_now(&client).await {
                    // Spotify can reject an access token before its stated
                    // expiry; one forced refresh usually recovers the request.
                    fetched = metadata::fetch_collection(&client, kind, &id, trimmed).await;
                }
            }
            match fetched {
                Ok(collection) => {
                    self.emit_log(
                        "info",
                        &format!(
                            "Found \"{}\" with {} track{}.",
                            collection.name,
                            collection.tracks.len(),
                            if collection.tracks.len() == 1 {
                                ""
                            } else {
                                "s"
                            }
                        ),
                    );
                    collections.push(collection);
                }
                Err(error) => {
                    self.emit_log("error", &format!("{error}"));
                    last_error = Some(error);
                }
            }
        }

        auth::persist_token(&client).await;

        if collections.is_empty() {
            return Err(match last_error {
                Some(error) => error,
                None => "No Spotify links to process.".to_string(),
            });
        }

        if self.is_cancelled() {
            return Ok(Outcome::Cancelled);
        }

        let base_dir = if self.is_bulk {
            let stamp = chrono::Local::now().format("%Y-%m-%d %H-%M-%S").to_string();
            let master_name = naming::sanitize(&format!("Bulk Download {stamp}"), 64);
            let master = naming::unique_dir(&start.output_dir, &master_name);
            std::fs::create_dir_all(&master)
                .map_err(|error| format!("Could not create {}: {error}", master.display()))?;
            self.emit_log("info", &format!("Saving into {}", master.display()));
            master
        } else {
            start.output_dir.clone()
        };

        self.total = collections.iter().map(|item| item.tracks.len()).sum();
        self.emit_progress(
            "downloading",
            Some(0),
            None,
            Some(format!("{} track(s) queued.", self.total)),
        );

        for collection in &collections {
            if self.is_cancelled() {
                return Ok(Outcome::Cancelled);
            }

            let result = match collection.kind {
                CollectionKind::Track => self.process_track(collection, &base_dir).await,
                CollectionKind::Album => self.process_album(collection, &base_dir).await,
                CollectionKind::Playlist => self.process_playlist(collection, &base_dir).await,
            };

            match result {
                Ok(()) => {}
                Err(ProcessError::Cancelled) => {
                    if self.is_bulk {
                        self.emit_log("warn", &format!("Skipped \"{}\".", collection.name));
                        continue;
                    }
                    return Ok(Outcome::Cancelled);
                }
                Err(ProcessError::Failed(message)) => {
                    self.emit_log(
                        "error",
                        &format!("Could not process \"{}\": {message}", collection.name),
                    );
                    if !self.is_bulk {
                        return Err(message);
                    }
                }
            }
        }

        if self.is_cancelled() {
            Ok(Outcome::Cancelled)
        } else {
            Ok(Outcome::Completed)
        }
    }

    async fn process_track(
        &mut self,
        collection: &Collection,
        dir: &Path,
    ) -> Result<(), ProcessError> {
        let Some(meta) = collection.tracks.first() else {
            self.emit_log("warn", "This track link had no track metadata.");
            return Ok(());
        };

        let filename = naming::single_track_filename(meta);
        let final_path = dir.join(&filename);

        self.emit_progress(
            "downloading",
            self.percent(),
            Some(track_label(meta)),
            Some(if file_ok(&final_path) {
                "Already downloaded".to_string()
            } else {
                "Downloading audio".to_string()
            }),
        );

        if file_ok(&final_path) {
            self.skipped += 1;
            self.processed += 1;
            self.emit_log("info", &format!("Skipped existing file {filename}"));
            self.emit_progress("downloading", self.percent(), Some(track_label(meta)), None);
            return Ok(());
        }

        match self.download(meta, &final_path, dir).await {
            Ok(_) => {
                self.processed += 1;
                self.downloaded += 1;
                self.emit_log("success", &format!("Saved {filename}"));
                self.emit_progress("downloading", self.percent(), Some(track_label(meta)), None);
            }
            Err(DownloadError::Cancelled) => return Err(ProcessError::Cancelled),
            Err(DownloadError::Failed(error)) => {
                self.processed += 1;
                self.failed += 1;
                self.emit_track_failed(meta, &error);
                self.emit_log("error", &format!("Failed {}: {error}", track_label(meta)));
            }
        }

        Ok(())
    }

    async fn process_album(
        &mut self,
        collection: &Collection,
        dir: &Path,
    ) -> Result<(), ProcessError> {
        if collection.tracks.is_empty() {
            self.emit_log(
                "warn",
                &format!("Album \"{}\" has no downloadable tracks.", collection.name),
            );
            return Ok(());
        }

        let artist = collection
            .tracks
            .first()
            .map(|meta| naming::display_artist(&meta.artists))
            .unwrap_or_else(|| "Unknown Artist".to_string());
        let folder_name = naming::album_folder(&artist, &collection.name);
        let folder = naming::existing_or_unique_dir(dir, &folder_name);
        std::fs::create_dir_all(&folder).map_err(|error| {
            ProcessError::Failed(format!("Could not create {}: {error}", folder.display()))
        })?;
        self.emit_log("info", &format!("Album folder: {}", folder.display()));

        let multi_disc = collection.tracks.iter().any(|meta| meta.disc_number > 1);

        for meta in &collection.tracks {
            if self.is_cancelled() {
                return Err(ProcessError::Cancelled);
            }

            let filename = naming::album_track_filename(meta, multi_disc);
            let final_path = folder.join(&filename);

            if file_ok(&final_path) {
                self.skipped += 1;
                self.processed += 1;
                self.emit_progress(
                    "downloading",
                    self.percent(),
                    Some(track_label(meta)),
                    Some("Already downloaded".to_string()),
                );
                continue;
            }

            self.emit_progress(
                "downloading",
                self.percent(),
                Some(track_label(meta)),
                Some("Downloading audio".to_string()),
            );

            match self.download(meta, &final_path, &folder).await {
                Ok(_) => {
                    self.processed += 1;
                    self.downloaded += 1;
                    self.emit_log("success", &format!("Saved {filename}"));
                    self.emit_progress(
                        "downloading",
                        self.percent(),
                        Some(track_label(meta)),
                        None,
                    );
                }
                Err(DownloadError::Cancelled) => return Err(ProcessError::Cancelled),
                Err(DownloadError::Failed(error)) => {
                    self.processed += 1;
                    self.failed += 1;
                    self.emit_track_failed(meta, &error);
                    self.emit_log("error", &format!("Failed {}: {error}", track_label(meta)));
                }
            }
        }

        Ok(())
    }

    async fn process_playlist(
        &mut self,
        collection: &Collection,
        root: &Path,
    ) -> Result<(), ProcessError> {
        let folder_name = naming::playlist_folder(&collection.name);
        let candidate = root.join(&folder_name);

        let candidate_state = if sync::state_path(&candidate).is_file() {
            Some(sync::load_state(&candidate, &collection.id))
        } else {
            None
        };
        let candidate_is_target = candidate_state
            .as_ref()
            .map(|(state, _)| state.playlist_id == collection.id)
            .unwrap_or(false);
        // A corrupt state file was already backed up by `load_state`; keep syncing in that
        // folder instead of creating a duplicate one next to it.
        let candidate_corrupt = candidate_state
            .as_ref()
            .map(|(_, corrupt)| *corrupt)
            .unwrap_or(false);

        let mut target = candidate.clone();
        let mut force_new = false;

        if !candidate_is_target {
            let matches = sync::scan_for_playlist(root, &collection.id, &candidate);
            if !matches.is_empty() {
                self.emit_progress(
                    "awaiting confirmation",
                    self.percent(),
                    Some(collection.name.clone()),
                    Some("Found an existing folder for this playlist.".to_string()),
                );
                self.emit_log(
                    "warn",
                    &format!(
                        "Found {} existing folder(s) tracking this playlist.",
                        matches.len()
                    ),
                );
                match self.conflict(collection, &candidate, &matches).await? {
                    ConflictResolution::SyncHere(selected) => {
                        target = selected;
                    }
                    ConflictResolution::CreateNew => force_new = true,
                    ConflictResolution::Cancel => return Err(ProcessError::Cancelled),
                }
            }
        }

        if force_new
            || (target == candidate
                && candidate.exists()
                && !candidate_is_target
                && !candidate_corrupt)
        {
            target = naming::unique_dir(root, &folder_name);
        }

        std::fs::create_dir_all(&target).map_err(|error| {
            ProcessError::Failed(format!("Could not create {}: {error}", target.display()))
        })?;

        let (mut state, corrupt) = if target == candidate {
            match candidate_state {
                Some(loaded) => loaded,
                None => sync::load_state(&target, &collection.id),
            }
        } else {
            sync::load_state(&target, &collection.id)
        };
        if corrupt {
            self.emit_log(
                "warn",
                &format!(
                    "The playlist state in {} was corrupt; a backup was created and a fresh state started.",
                    target.display()
                ),
            );
        }
        if state.playlist_id != collection.id {
            state.playlist_id = collection.id.clone();
        }

        let plan = sync::plan_playlist(
            &target,
            &collection.name,
            &collection.source_url,
            &collection.tracks,
            &mut state,
        );

        self.emit_log(
            "info",
            &format!(
                "\"{}\": {} new, {} already present, {} removed from Spotify.",
                collection.name,
                plan.downloads.len(),
                plan.skipped + plan.kept_removed,
                plan.removals.len()
            ),
        );

        if plan.kept_removed > 0 {
            self.emit_log(
                "info",
                &format!(
                    "\"{}\": {} track(s) removed from Spotify earlier are still kept on disk (no prompt).",
                    collection.name, plan.kept_removed
                ),
            );
        }

        if !plan.renames.is_empty() {
            self.emit_progress(
                "downloading",
                self.percent(),
                Some(collection.name.clone()),
                Some("Updating file numbers to match the playlist".to_string()),
            );

            let total_tracks = collection.tracks.len() as u32;
            let outcome = sync::apply_renames(&target, &plan.renames, &mut state);
            let mut renumbered = 0usize;
            let mut moved_kept = 0usize;

            for rename in &plan.renames {
                let moved = state
                    .entries
                    .get(rename.entry_index)
                    .map(|entry| entry.local_filename == rename.to)
                    .unwrap_or(false);
                if !moved {
                    continue;
                }

                let Some(track_index) = rename.track_index else {
                    moved_kept += 1;
                    continue;
                };
                renumbered += 1;

                let meta = &collection.tracks[track_index];
                let position = if meta.position == 0 {
                    1
                } else {
                    meta.position as u32
                };
                if let Err(error) = downloader::retag_playlist_track(
                    &target.join(&rename.to),
                    meta,
                    position,
                    total_tracks,
                )
                .await
                {
                    self.emit_log(
                        "warn",
                        &format!("Could not update tags for {}: {error}", rename.to),
                    );
                }

                if let Some(entry) = state.entries.get_mut(rename.entry_index) {
                    entry.title = meta.title.clone();
                    entry.artist = naming::display_artist(&meta.artists);
                    entry.status = EntryStatus::Ok;
                }
            }

            if renumbered > 0 {
                self.emit_log(
                    "info",
                    &format!(
                        "\"{}\": {} file(s) renumbered to match the playlist order.",
                        collection.name, renumbered
                    ),
                );
            }
            if moved_kept > 0 {
                self.emit_log(
                    "info",
                    &format!(
                        "\"{}\": {} kept file(s) moved out of the playlist numbering.",
                        collection.name, moved_kept
                    ),
                );
            }
            if let Err(error) = outcome {
                self.emit_log("warn", &error);
            }

            if renumbered > 0 {
                self.skipped += renumbered;
                self.processed += renumbered;
            }
            if let Err(error) = sync::save_state(&target, &mut state) {
                self.emit_log("error", &error);
            }
        }

        let untracked = sync::untracked_files(&target, &state);
        if !untracked.is_empty() {
            let shown: Vec<&str> = untracked.iter().take(5).map(String::as_str).collect();
            self.emit_log(
                "info",
                &format!(
                    "\"{}\": {} untracked file(s) left untouched: {}{}",
                    collection.name,
                    untracked.len(),
                    shown.join(", "),
                    if untracked.len() > shown.len() {
                        ", ..."
                    } else {
                        ""
                    }
                ),
            );
        }

        if plan.skipped + plan.kept_removed > 0 {
            self.skipped += plan.skipped + plan.kept_removed;
            self.processed += plan.skipped;
            self.emit_progress(
                "downloading",
                self.percent(),
                Some(collection.name.clone()),
                Some(format!(
                    "{} track(s) already present",
                    plan.skipped + plan.kept_removed
                )),
            );
        }

        let mut deletion: Option<DeletionChoice> = None;
        if self.mode == "sync" && !plan.removals.is_empty() {
            self.emit_progress(
                "awaiting confirmation",
                self.percent(),
                Some(collection.name.clone()),
                Some("Waiting for your decision on removed tracks.".to_string()),
            );
            self.emit_log(
                "info",
                &format!(
                    "{} track(s) are no longer on Spotify - waiting for your decision.",
                    plan.removals.len()
                ),
            );
            deletion = Some(
                self.deletion_prompt(collection, &target, &plan.removals)
                    .await?,
            );
        }

        for planned in &plan.downloads {
            if self.is_cancelled() {
                return Err(ProcessError::Cancelled);
            }

            // Playlist folders are numbered by playlist position, so the tag
            // track number must follow the same order instead of the album one.
            let mut meta = collection.tracks[planned.track_index].clone();
            meta.track_number = if meta.position == 0 {
                1
            } else {
                meta.position as u32
            };
            meta.total_tracks = Some(collection.tracks.len() as u32);
            let meta = &meta;
            let final_path = target.join(&planned.filename);

            if file_ok(&final_path) {
                // Race: another process produced the file since planning.
                self.skipped += 1;
                self.processed += 1;
                continue;
            }

            self.emit_progress(
                "downloading",
                self.percent(),
                Some(track_label(meta)),
                Some("Downloading audio".to_string()),
            );

            match self.download(meta, &final_path, &target).await {
                Ok(_) => {
                    self.processed += 1;
                    self.downloaded += 1;

                    match planned.entry_index {
                        Some(index) => {
                            if let Some(entry) = state.entries.get_mut(index) {
                                entry.local_filename = planned.filename.clone();
                                entry.title = meta.title.clone();
                                entry.artist = naming::display_artist(&meta.artists);
                                entry.status = EntryStatus::Ok;
                            }
                        }
                        None => state.entries.push(SyncEntry {
                            track_id: meta.id.clone(),
                            local_filename: planned.filename.clone(),
                            title: meta.title.clone(),
                            artist: naming::display_artist(&meta.artists),
                            status: EntryStatus::Ok,
                        }),
                    }

                    if let Err(error) = sync::save_state(&target, &mut state) {
                        self.emit_log("error", &error);
                    }
                    self.emit_log("success", &format!("Saved {}", planned.filename));
                    self.emit_progress(
                        "downloading",
                        self.percent(),
                        Some(track_label(meta)),
                        None,
                    );
                }
                Err(DownloadError::Cancelled) => return Err(ProcessError::Cancelled),
                Err(DownloadError::Failed(error)) => {
                    self.processed += 1;
                    self.failed += 1;
                    self.emit_track_failed(meta, &error);
                    self.emit_log("error", &format!("Failed {}: {error}", track_label(meta)));
                }
            }
        }

        match deletion {
            Some(DeletionChoice::Delete) => {
                let mut indices: Vec<usize> = plan
                    .removals
                    .iter()
                    .map(|removed| removed.entry_index)
                    .collect();
                indices.sort_unstable();
                indices.reverse();

                for index in indices {
                    let Some(removed) = plan
                        .removals
                        .iter()
                        .find(|removed| removed.entry_index == index)
                    else {
                        continue;
                    };
                    let path = target.join(&removed.local_filename);
                    let mut can_remove_entry = true;
                    if !sync::is_safe_filename(&removed.local_filename) {
                        self.emit_log(
                            "warn",
                            &format!(
                                "Ignoring unsafe filename in playlist state: {}",
                                removed.local_filename
                            ),
                        );
                    } else if path.exists() {
                        match std::fs::remove_file(&path) {
                            Ok(()) => {}
                            Err(error) => {
                                can_remove_entry = false;
                                self.emit_log(
                                    "warn",
                                    &format!("Could not delete {}: {error}", path.display()),
                                );
                            }
                        }
                    }
                    if can_remove_entry && index < state.entries.len() {
                        state.entries.remove(index);
                        self.removed += 1;
                    }
                }

                if let Err(error) = sync::save_state(&target, &mut state) {
                    self.emit_log("error", &error);
                }
            }
            Some(DeletionChoice::Keep) => {
                for removed in &plan.removals {
                    if let Some(entry) = state.entries.get_mut(removed.entry_index) {
                        entry.status = EntryStatus::KeptRemoved;
                    }
                }
                if let Err(error) = sync::save_state(&target, &mut state) {
                    self.emit_log("error", &error);
                }
            }
            None => {}
        }

        if let Err(error) = sync::save_state(&target, &mut state) {
            self.emit_log("error", &error);
        }

        Ok(())
    }

    async fn download(
        &self,
        meta: &TrackMeta,
        final_path: &Path,
        dest_dir: &Path,
    ) -> Result<u64, DownloadError> {
        let bins = self.bins.as_ref().ok_or_else(|| {
            DownloadError::Failed("Download tools are not available.".to_string())
        })?;

        let app = self.app.clone();
        let job_id = self.job_id.clone();
        let total = self.total;
        let processed = self.processed;
        let title = track_label(meta);

        downloader::download_track(
            bins,
            meta,
            final_path,
            dest_dir,
            self.cancel.clone(),
            &move |fraction: u8| {
                let percent = if total == 0 {
                    None
                } else {
                    let value = (processed as f64 + fraction as f64 / 100.0) / total as f64 * 100.0;
                    Some(value.round().clamp(0.0, 100.0) as u8)
                };
                let _ = app.emit(
                    "download://progress",
                    ProgressPayload {
                        job_id: job_id.clone(),
                        phase: "downloading".to_string(),
                        current: processed,
                        total,
                        percent,
                        title: Some(title.clone()),
                        message: Some("Downloading audio".to_string()),
                    },
                );
            },
        )
        .await
    }

    async fn conflict(
        &self,
        collection: &Collection,
        target: &Path,
        matches: &[sync::ScanMatch],
    ) -> Result<ConflictResolution, ProcessError> {
        let request_id = uuid::Uuid::new_v4().to_string();
        let (sender, receiver) = oneshot::channel::<ConflictResolution>();

        {
            let state = self.state();
            state
                .pending_conflicts
                .lock()
                .unwrap()
                .insert(request_id.clone(), sender);
            state.conflict_matches.lock().unwrap().insert(
                request_id.clone(),
                matches.iter().map(|item| item.dir.clone()).collect(),
            );
        }

        let payload = ConflictPayload {
            job_id: self.job_id.clone(),
            request_id: request_id.clone(),
            playlist_name: collection.name.clone(),
            playlist_id: collection.id.clone(),
            target_dir: target.display().to_string(),
            matches: matches
                .iter()
                .map(|item| ConflictMatchPayload {
                    dir: item.dir.display().to_string(),
                    playlist_name: item.playlist_name.clone(),
                    track_count: item.track_count,
                    updated_at: item.updated_at.clone(),
                })
                .collect(),
            timeout_ms: PROMPT_TIMEOUT_MS,
        };
        let _ = self.app.emit("sync://conflict", payload);

        let resolution = match tokio::time::timeout(
            std::time::Duration::from_millis(PROMPT_TIMEOUT_MS),
            receiver,
        )
        .await
        {
            Ok(Ok(resolution)) => resolution,
            Ok(Err(_)) => ConflictResolution::Cancel,
            Err(_) => ConflictResolution::Cancel,
        };

        {
            let state = self.state();
            state.pending_conflicts.lock().unwrap().remove(&request_id);
            state.conflict_matches.lock().unwrap().remove(&request_id);
        }

        if self.is_cancelled() {
            return Err(ProcessError::Cancelled);
        }

        if matches!(resolution, ConflictResolution::Cancel) {
            self.emit_log(
                "warn",
                &format!(
                    "\"{}\" was skipped (folder conflict unresolved).",
                    collection.name
                ),
            );
        }

        Ok(resolution)
    }

    async fn deletion_prompt(
        &self,
        collection: &Collection,
        folder: &Path,
        removals: &[RemovedTrack],
    ) -> Result<DeletionChoice, ProcessError> {
        let request_id = uuid::Uuid::new_v4().to_string();
        let (sender, receiver) = oneshot::channel::<String>();
        self.state()
            .pending_deletions
            .lock()
            .unwrap()
            .insert(request_id.clone(), sender);

        let payload = RemovedPayload {
            job_id: self.job_id.clone(),
            request_id: request_id.clone(),
            folder: folder.display().to_string(),
            playlist_name: collection.name.clone(),
            tracks: removals
                .iter()
                .map(|removed| RemovedTrackPayload {
                    track_id: removed.track_id.clone(),
                    title: removed.title.clone(),
                    artist: removed.artist.clone(),
                    local_filename: removed.local_filename.clone(),
                })
                .collect(),
            timeout_ms: PROMPT_TIMEOUT_MS,
        };
        let _ = self.app.emit("sync://removed_tracks", payload);

        let result = tokio::time::timeout(
            std::time::Duration::from_millis(PROMPT_TIMEOUT_MS),
            receiver,
        )
        .await;

        self.state()
            .pending_deletions
            .lock()
            .unwrap()
            .remove(&request_id);

        if self.is_cancelled() {
            return Err(ProcessError::Cancelled);
        }

        match result {
            Ok(Ok(decision)) => {
                if decision == "delete" {
                    self.emit_log(
                        "info",
                        &format!(
                            "Deleting {} removed file(s) from \"{}\".",
                            removals.len(),
                            collection.name
                        ),
                    );
                    Ok(DeletionChoice::Delete)
                } else {
                    self.emit_log(
                        "info",
                        &format!("Keeping the local files in \"{}\".", collection.name),
                    );
                    Ok(DeletionChoice::Keep)
                }
            }
            Ok(Err(_)) => Err(ProcessError::Cancelled),
            Err(_) => {
                self.emit_log(
                    "warn",
                    "No answer to the deletion prompt - kept the local files.",
                );
                Ok(DeletionChoice::Keep)
            }
        }
    }
}

fn track_label(meta: &TrackMeta) -> String {
    format!("{} - {}", naming::display_artist(&meta.artists), meta.title)
}

fn file_ok(path: &Path) -> bool {
    std::fs::metadata(path)
        .map(|metadata| metadata.is_file() && metadata.len() > 0)
        .unwrap_or(false)
}
