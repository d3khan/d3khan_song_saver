//! Download pipeline: yt-dlp (search + download) -> FFmpeg (MP3 conversion) -> lofty (ID3 tags).
//!
//! The public surface is fixed by `docs/phase4-interfaces.md`:

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use lofty::config::WriteOptions;
use lofty::picture::{MimeType, Picture, PictureType};
use lofty::prelude::*;
use lofty::tag::{ItemKey, Tag, TagType};
use tokio::io::{AsyncBufReadExt, BufReader};

use crate::metadata::TrackMeta;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

/// Prevents console windows from flashing when we spawn yt-dlp/ffmpeg.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

const YT_DLP_MISSING: &str = "yt-dlp not found. Reinstall the app or place yt-dlp.exe next to the executable (or in src-tauri/binaries/ during development).";
const FFMPEG_MISSING: &str = "ffmpeg not found. Reinstall the app or place ffmpeg.exe next to the executable (or in src-tauri/binaries/ during development).";

const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const CANCEL_POLL_INTERVAL: Duration = Duration::from_millis(250);
const RETRY_DELAY: Duration = Duration::from_secs(2);
const MAX_ERROR_CHARS: usize = 800;
const STDERR_TAIL_LINES: usize = 20;
const STDERR_BUFFER_LINES: usize = 25;

/// yt-dlp reports a search with no hits as a successful run that downloads zero
/// items, so this exact message is the only signal that the *query* found nothing.
const NO_AUDIO_MESSAGE: &str = "yt-dlp finished but produced no audio file";

pub struct Binaries {
    pub yt_dlp: PathBuf,
    pub ffmpeg_dir: Option<PathBuf>,
}

#[derive(Debug)]
pub enum DownloadError {
    Cancelled,
    Failed(String),
}

impl std::fmt::Display for DownloadError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DownloadError::Cancelled => write!(formatter, "Cancelled"),
            DownloadError::Failed(message) => write!(formatter, "{message}"),
        }
    }
}

impl std::error::Error for DownloadError {}

// ---------------------------------------------------------------------------
// Binary discovery
// ---------------------------------------------------------------------------

/// Locates `yt-dlp`/`ffmpeg`, probing each candidate with `--version`.
///
/// Discovery order (first hit wins):
/// yt-dlp: `YTDLP_PATH` env, `<exe_dir>/binaries/yt-dlp.exe`, `<cwd>/binaries/yt-dlp.exe`,
/// `<cwd>/../src-tauri/binaries/yt-dlp.exe`, `CARGO_MANIFEST_DIR/binaries/yt-dlp.exe`, `yt-dlp` on PATH.
/// ffmpeg follows the same order (`FFMPEG_PATH`, exe dir, cwd, `../src-tauri`, manifest dir, PATH).
///
/// `ffmpeg_dir` is the parent directory of a resolved ffmpeg file, or `None` when ffmpeg was only
/// found on `PATH` (in that case yt-dlp resolves it itself and we pass no `--ffmpeg-location`).
pub fn discover_binaries() -> Result<Binaries, String> {
    let yt_dlp = find_yt_dlp().ok_or_else(|| YT_DLP_MISSING.to_string())?;
    let (_, ffmpeg_dir) = find_ffmpeg().ok_or_else(|| FFMPEG_MISSING.to_string())?;
    Ok(Binaries { yt_dlp, ffmpeg_dir })
}

fn find_yt_dlp() -> Option<PathBuf> {
    for candidate in candidate_paths("yt-dlp.exe", "YTDLP_PATH") {
        if probe_binary(&candidate) {
            return Some(candidate);
        }
    }
    let on_path = PathBuf::from("yt-dlp");
    probe_binary(&on_path).then_some(on_path)
}

fn find_ffmpeg() -> Option<(PathBuf, Option<PathBuf>)> {
    for candidate in candidate_paths("ffmpeg.exe", "FFMPEG_PATH") {
        if probe_binary(&candidate) {
            let dir = candidate.parent().map(Path::to_path_buf);
            return Some((candidate, dir));
        }
    }
    let on_path = PathBuf::from("ffmpeg");
    probe_binary(&on_path).then_some((on_path, None))
}

fn candidate_paths(file_name: &str, env_key: &str) -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    if let Some(value) = std::env::var_os(env_key) {
        let value = PathBuf::from(value);
        if !value.as_os_str().is_empty() {
            candidates.push(value);
        }
    }

    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            // Packaged builds place the bundled tools directly next to the app.
            candidates.push(exe_dir.join(file_name));
            candidates.push(exe_dir.join("binaries").join(file_name));
        }
    }

    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("binaries").join(file_name));
        candidates.push(
            cwd.join("..")
                .join("src-tauri")
                .join("binaries")
                .join(file_name),
        );
    }

    candidates.push(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("binaries")
            .join(file_name),
    );

    candidates
}

/// Spawns the program with a version flag and waits up to ~5 s for a successful exit.
/// yt-dlp uses `--version`; FFmpeg only understands `-version`.
fn probe_binary(program: &Path) -> bool {
    ["--version", "-version"]
        .iter()
        .any(|flag| probe_binary_with(program, flag))
}

fn probe_binary_with(program: &Path, flag: &str) -> bool {
    let mut command = std::process::Command::new(program);
    command
        .arg(flag)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());

    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);

    let Ok(mut child) = command.spawn() else {
        return false;
    };

    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return false;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Per-track download
// ---------------------------------------------------------------------------

/// Downloads a YouTube search hit for `meta`, converts it to MP3, writes ID3 text tags (cover art
/// best-effort) and moves it onto `final_path`.
///
/// Returns the number of bytes written. On any error no (partial) file is left at `final_path`:
/// the temp file is tagged first and only then renamed.
pub async fn download_track(
    bins: &Binaries,
    meta: &TrackMeta,
    final_path: &Path,
    dest_dir: &Path,
    cancel: Arc<AtomicBool>,
    on_progress: &(dyn Fn(u8) + Send + Sync),
) -> Result<u64, DownloadError> {
    let queries = search_queries(meta);
    let uuid = uuid::Uuid::new_v4();
    let temp_prefix = format!(".{uuid}.");
    // Absolute (when dest_dir is) so yt-dlp never writes into the process CWD.
    let temp_template = dest_dir
        .join(format!(".{uuid}.%(ext)s"))
        .to_string_lossy()
        .to_string();
    let cancel = cancel.as_ref();
    let total = queries.len();

    let mut primary_note = None;
    for (index, query) in queries.iter().enumerate() {
        let args = build_yt_dlp_args(query, &temp_template, bins.ffmpeg_dir.as_deref());
        match attempt_with_retry(
            bins,
            meta,
            final_path,
            dest_dir,
            &args,
            &temp_prefix,
            cancel,
            on_progress,
        )
        .await
        {
            Ok(bytes) => return Ok(bytes),
            Err(DownloadError::Cancelled) => return Err(DownloadError::Cancelled),
            Err(DownloadError::Failed(message)) => {
                // Exit code 0 with no file means YouTube returned zero hits for that
                // query (it silently drops some words, e.g. profanity). Any other
                // failure is a real download problem, so report it right away.
                let no_hits = message == NO_AUDIO_MESSAGE;
                if !no_hits || index + 1 == total {
                    if no_hits && total > 1 {
                        let primary = &queries[0];
                        return Err(DownloadError::Failed(format!(
                            "No YouTube results for {} ({} query variants tried)",
                            primary.strip_prefix("ytsearch1:").unwrap_or(primary).trim(),
                            total
                        )));
                    }
                    return Err(DownloadError::Failed(message));
                }
                primary_note = Some(message);
            }
        }
    }

    Err(DownloadError::Failed(
        primary_note.unwrap_or_else(|| NO_AUDIO_MESSAGE.to_string()),
    ))
}

/// Runs one yt-dlp attempt, retrying once for transient failures (403s, rate
/// limits, flaky connections).
#[allow(clippy::too_many_arguments)]
async fn attempt_with_retry(
    bins: &Binaries,
    meta: &TrackMeta,
    final_path: &Path,
    dest_dir: &Path,
    args: &[String],
    temp_prefix: &str,
    cancel: &AtomicBool,
    on_progress: &(dyn Fn(u8) + Send + Sync),
) -> Result<u64, DownloadError> {
    match run_attempt(
        bins,
        meta,
        final_path,
        dest_dir,
        args,
        temp_prefix,
        cancel,
        on_progress,
    )
    .await
    {
        Ok(bytes) => Ok(bytes),
        Err(AttemptError::Cancelled) => Err(DownloadError::Cancelled),
        Err(AttemptError::Failed { message, raw }) => {
            if is_transient(&raw) && !cancel.load(Ordering::Relaxed) {
                tokio::time::sleep(RETRY_DELAY).await;
                if cancel.load(Ordering::Relaxed) {
                    return Err(DownloadError::Cancelled);
                }
                match run_attempt(
                    bins,
                    meta,
                    final_path,
                    dest_dir,
                    args,
                    temp_prefix,
                    cancel,
                    on_progress,
                )
                .await
                {
                    Ok(bytes) => Ok(bytes),
                    Err(AttemptError::Cancelled) => Err(DownloadError::Cancelled),
                    Err(AttemptError::Failed { message, .. }) => {
                        Err(DownloadError::Failed(message))
                    }
                }
            } else {
                Err(DownloadError::Failed(message))
            }
        }
    }
}

/// YouTube's search endpoint answers with zero results when the query contains
/// certain words (verified with `yt-dlp "ytsearch1:Rustage - FUCK AI"`, which
/// downloads 0 items). These fallbacks keep the search usable for such titles.
fn search_queries(meta: &TrackMeta) -> Vec<String> {
    let artist = meta.artists.join(", ");
    let artist = artist.trim();
    let title = meta.title.trim();
    let mut queries: Vec<String> = Vec::new();
    let mut push = |query: String| {
        let query = query.trim().to_string();
        if !queries.iter().any(|existing| *existing == query) {
            queries.push(format!("ytsearch1:{query}"));
        }
    };

    if artist.is_empty() {
        push(title.to_string());
    } else {
        push(format!("{artist} - {title}"));
        push(format!("{artist} {title}"));
    }

    if let Some(clean) = title_without_filtered_words(title) {
        if artist.is_empty() {
            push(clean);
        } else {
            push(format!("{artist} - {clean}"));
            push(format!("{artist} {clean}"));
        }
    }

    queries
}

/// Lowercased letters/digits only, so `F*CK`, `F.U.C.K` and `FUCK` compare equal.
fn search_word_key(word: &str) -> String {
    word.chars()
        .flat_map(char::to_lowercase)
        .filter(|c| c.is_alphanumeric())
        .collect()
}

fn is_search_filtered(word: &str) -> bool {
    const FILTERED: [&str; 18] = [
        "fuck",
        "fck",
        "fk",
        "fuk",
        "fking",
        "fucking",
        "fucker",
        "motherfucker",
        "shit",
        "sh1t",
        "bitch",
        "cunt",
        "dick",
        "pussy",
        "asshole",
        "nigga",
        "nigger",
        "whore",
    ];
    let key = search_word_key(word);
    !key.is_empty() && FILTERED.contains(&key.as_str())
}

/// Drops the words YouTube refuses to search for, keeping the rest of the title
/// intact. Returns `None` when nothing would change or nothing would remain.
fn title_without_filtered_words(title: &str) -> Option<String> {
    let words: Vec<&str> = title.split_whitespace().collect();
    let kept: Vec<&str> = words
        .iter()
        .copied()
        .filter(|word| !is_search_filtered(word))
        .collect();
    if kept.is_empty() || kept.len() == words.len() {
        return None;
    }
    let cleaned = kept
        .join(" ")
        .trim_matches(|c: char| c.is_whitespace() || matches!(c, '-' | '–' | '—' | ',' | ':'))
        .to_string();
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned)
    }
}

enum WaitOutcome {
    Exited(std::process::ExitStatus),
    Cancelled,
    TimedOut,
    Error(String),
}

enum AttemptError {
    Cancelled,
    Failed {
        message: String,
        /// Untruncated output used for transient-error classification.
        raw: String,
    },
}

#[allow(clippy::too_many_arguments)]
async fn run_attempt(
    bins: &Binaries,
    meta: &TrackMeta,
    final_path: &Path,
    dest_dir: &Path,
    args: &[String],
    temp_prefix: &str,
    cancel: &AtomicBool,
    on_progress: &(dyn Fn(u8) + Send + Sync),
) -> Result<u64, AttemptError> {
    let mut command = tokio::process::Command::new(&bins.yt_dlp);
    command
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            cleanup_temp_files(dest_dir, temp_prefix).await;
            return Err(AttemptError::Failed {
                message: format!("Could not start yt-dlp: {error}"),
                raw: String::new(),
            });
        }
    };

    // Drain both pipes concurrently; either one filling up would deadlock the child.
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let stderr_lines = std::sync::Mutex::new(VecDeque::<String>::new());

    let read_stdout = async {
        if let Some(pipe) = stdout {
            let mut reader = BufReader::new(pipe);
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line).await {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        if let Some(percent) = parse_progress(line.trim_end()) {
                            // 100% only means the bytes are in; keep the callback below completion.
                            on_progress(percent.min(99));
                        }
                    }
                }
            }
        }
    };

    let read_stderr = async {
        if let Some(pipe) = stderr {
            let mut reader = BufReader::new(pipe);
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line).await {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        let text = line.trim_end().to_string();
                        if !text.is_empty() {
                            let mut buffer = stderr_lines.lock().unwrap();
                            buffer.push_back(text);
                            while buffer.len() > STDERR_BUFFER_LINES {
                                buffer.pop_front();
                            }
                        }
                    }
                }
            }
        }
    };

    let wait = async {
        let deadline = tokio::time::Instant::now() + DOWNLOAD_TIMEOUT;
        loop {
            if cancel.load(Ordering::Relaxed) {
                let _ = child.kill().await;
                let _ = child.wait().await;
                return WaitOutcome::Cancelled;
            }

            match child.try_wait() {
                Ok(Some(status)) => return WaitOutcome::Exited(status),
                Ok(None) => {}
                Err(error) => {
                    let _ = child.kill().await;
                    let _ = child.wait().await;
                    return WaitOutcome::Error(format!("Could not wait for yt-dlp: {error}"));
                }
            }

            if tokio::time::Instant::now() >= deadline {
                let _ = child.kill().await;
                let _ = child.wait().await;
                return WaitOutcome::TimedOut;
            }

            tokio::time::sleep(CANCEL_POLL_INTERVAL).await;
        }
    };

    let (_, _, outcome) = tokio::join!(read_stdout, read_stderr, wait);

    let stderr_tail = {
        let buffer = stderr_lines.lock().unwrap();
        let skip = buffer.len().saturating_sub(STDERR_TAIL_LINES);
        buffer
            .iter()
            .skip(skip)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    };

    match outcome {
        WaitOutcome::Cancelled => {
            cleanup_temp_files(dest_dir, temp_prefix).await;
            Err(AttemptError::Cancelled)
        }
        WaitOutcome::TimedOut => {
            cleanup_temp_files(dest_dir, temp_prefix).await;
            Err(AttemptError::Failed {
                message: "yt-dlp timed out after 10 minutes".to_string(),
                // Our own timeout message must never trigger a transient retry.
                raw: String::new(),
            })
        }
        WaitOutcome::Error(message) => {
            cleanup_temp_files(dest_dir, temp_prefix).await;
            Err(AttemptError::Failed {
                message,
                raw: stderr_tail,
            })
        }
        WaitOutcome::Exited(status) => {
            if !status.success() {
                cleanup_temp_files(dest_dir, temp_prefix).await;
                let message = build_failure_message(status.code(), &stderr_tail);
                return Err(AttemptError::Failed {
                    message,
                    raw: stderr_tail,
                });
            }

            let Some(temp_path) = find_temp_file(dest_dir, temp_prefix).await else {
                cleanup_temp_files(dest_dir, temp_prefix).await;
                return Err(AttemptError::Failed {
                    message: NO_AUDIO_MESSAGE.to_string(),
                    raw: stderr_tail,
                });
            };

            // Tag the temp file *before* it becomes visible at `final_path`.
            if let Err(message) = tag_file(&temp_path, meta).await {
                cleanup_temp_files(dest_dir, temp_prefix).await;
                return Err(AttemptError::Failed {
                    message,
                    raw: String::new(),
                });
            }

            // Overwriting a redownload: drop the stale destination first, then rename.
            if tokio::fs::metadata(final_path).await.is_ok() {
                let _ = tokio::fs::remove_file(final_path).await;
            }

            if let Err(error) = tokio::fs::rename(&temp_path, final_path).await {
                cleanup_temp_files(dest_dir, temp_prefix).await;
                return Err(AttemptError::Failed {
                    message: format!(
                        "Could not move downloaded file to {}: {error}",
                        final_path.display()
                    ),
                    raw: String::new(),
                });
            }

            cleanup_temp_files(dest_dir, temp_prefix).await;

            match tokio::fs::metadata(final_path).await {
                Ok(metadata) if metadata.len() > 0 => Ok(metadata.len()),
                Ok(_) => {
                    let _ = tokio::fs::remove_file(final_path).await;
                    Err(AttemptError::Failed {
                        message: format!("Downloaded file is empty: {}", final_path.display()),
                        raw: String::new(),
                    })
                }
                Err(error) => Err(AttemptError::Failed {
                    message: format!(
                        "Downloaded file missing after move ({}): {error}",
                        final_path.display()
                    ),
                    raw: String::new(),
                }),
            }
        }
    }
}

fn build_yt_dlp_args(query: &str, temp_template: &str, ffmpeg_dir: Option<&Path>) -> Vec<String> {
    let mut args: Vec<String> = [
        "--no-playlist",
        "--newline",
        "--no-warnings",
        "--ignore-config",
        "-f",
        "bestaudio/best",
        "-x",
        "--audio-format",
        "mp3",
        "--audio-quality",
        "0",
    ]
    .iter()
    .map(|arg| (*arg).to_string())
    .collect();

    if let Some(dir) = ffmpeg_dir {
        args.push("--ffmpeg-location".to_string());
        args.push(dir.to_string_lossy().to_string());
    }

    args.push("-o".to_string());
    args.push(temp_template.to_string());
    args.push(query.to_string());
    args
}

/// Parses yt-dlp progress lines: `[download]  42.3% of ...` -> `42`.
fn parse_progress(line: &str) -> Option<u8> {
    let rest = line.trim().strip_prefix("[download]")?;
    let rest = rest.trim_start();
    let percent_end = rest.find('%')?;
    let value: f32 = rest[..percent_end].trim().parse().ok()?;
    if !value.is_finite() || !(0.0..=100.0).contains(&value) {
        return None;
    }
    Some(value as u8)
}

/// True when the failure text matches a known transient condition (retry once).
fn is_transient(text: &str) -> bool {
    const PATTERNS: [&str; 6] = [
        "403",
        "429",
        "timed out",
        "connection reset",
        "unable to download",
        "temporary failure",
    ];

    let lowered = text.to_ascii_lowercase();
    PATTERNS.iter().any(|pattern| lowered.contains(pattern))
}

fn build_failure_message(code: Option<i32>, stderr_tail: &str) -> String {
    let prefix = match code {
        Some(code) => format!("yt-dlp failed (exit code {code})"),
        None => "yt-dlp failed".to_string(),
    };
    let message = if stderr_tail.is_empty() {
        prefix
    } else {
        format!("{prefix}: {stderr_tail}")
    };
    truncate_chars(&message, MAX_ERROR_CHARS)
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut truncated: String = text.chars().take(max_chars.saturating_sub(3)).collect();
    truncated.push_str("...");
    truncated
}

/// Finds the file yt-dlp produced for our temp template, preferring the converted `.mp3`.
async fn find_temp_file(dest_dir: &Path, prefix: &str) -> Option<PathBuf> {
    let mut entries = tokio::fs::read_dir(dest_dir).await.ok()?;
    let mut mp3 = Vec::new();
    let mut other = Vec::new();

    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = entry.file_name();
        let name = name.to_string_lossy().to_string();
        if !name.starts_with(prefix) {
            continue;
        }
        if name.ends_with(".part") || name.ends_with(".ytdl") {
            continue;
        }
        let Ok(file_type) = entry.file_type().await else {
            continue;
        };
        if !file_type.is_file() {
            continue;
        }
        let path = entry.path();
        if name.to_ascii_lowercase().ends_with(".mp3") {
            mp3.push(path);
        } else {
            other.push(path);
        }
    }

    mp3.into_iter().next().or_else(|| other.into_iter().next())
}

/// Removes every leftover file for this download's temp prefix (`.part` files included).
async fn cleanup_temp_files(dest_dir: &Path, prefix: &str) {
    let Ok(mut entries) = tokio::fs::read_dir(dest_dir).await else {
        return;
    };

    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = entry.file_name();
        let name = name.to_string_lossy().to_string();
        if !name.starts_with(prefix) {
            continue;
        }
        let is_file = entry
            .file_type()
            .await
            .map(|file_type| file_type.is_file())
            .unwrap_or(false);
        if is_file {
            let _ = tokio::fs::remove_file(entry.path()).await;
        }
    }
}

// ---------------------------------------------------------------------------
// ID3 tagging
// ---------------------------------------------------------------------------

/// Writes the full ID3v2.3 tag set; cover art is best-effort and never fails the
/// track. ID3v2.3 is deliberate: Windows Explorer's thumbnail handler does not
/// reliably read APIC frames from v2.4 tags, while every player reads v2.3.
async fn tag_file(path: &Path, meta: &TrackMeta) -> Result<(), String> {
    let mut tag = Tag::new(TagType::Id3v2);
    tag.set_title(meta.title.clone());
    tag.set_artist(join_names(&meta.artists));
    tag.set_album(meta.album.clone());
    tag.set_track(meta.track_number);
    tag.set_disk(meta.disc_number);

    if let Some(total) = meta.total_tracks.filter(|total| *total > 0) {
        tag.set_track_total(total);
    }
    if !meta.album_artists.is_empty() {
        tag.insert_text(ItemKey::AlbumArtist, join_names(&meta.album_artists));
    }
    if let Some(date) = &meta.release_date {
        tag.insert_text(ItemKey::RecordingDate, date.clone());
    }
    if !meta.genres.is_empty() {
        tag.set_genre(meta.genres.join(", "));
    }
    if let Some(isrc) = &meta.isrc {
        tag.insert_text(ItemKey::Isrc, isrc.clone());
    }
    if let Some(label) = &meta.label {
        tag.insert_text(ItemKey::Label, label.clone());
    }
    if let Some(copyright) = &meta.copyright {
        tag.insert_text(ItemKey::CopyrightMessage, copyright.clone());
    }
    if let Some(url) = &meta.spotify_url {
        tag.set_comment(url.clone());
    }
    if meta.explicit {
        tag.insert_text(ItemKey::ParentalAdvisory, "1".to_string());
    }

    if let Some(url) = meta.cover_url.as_deref() {
        if let Err(error) = attach_cover(&mut tag, url).await {
            eprintln!("Cover art skipped for {}: {error}", path.display());
        }
    }

    tag.save_to_path(path, WriteOptions::new().use_id3v23(true))
        .map_err(|error| format!("Could not write MP3 tags: {error}"))
}

/// Rewrites the tags of an existing playlist MP3 in place after a renumber,
/// keeping its embedded artwork. `position`/`total_tracks` are the current
/// playlist order, so TRCK matches the numbered file name.
pub async fn retag_playlist_track(
    path: &Path,
    meta: &TrackMeta,
    position: u32,
    total_tracks: u32,
) -> Result<(), String> {
    let mut tagged =
        lofty::read_from_path(path).map_err(|error| format!("Could not read MP3 tags: {error}"))?;

    let has_primary = tagged.primary_tag_mut().is_some();
    let tag = if has_primary {
        tagged.primary_tag_mut().expect("primary tag checked above")
    } else {
        tagged
            .first_tag_mut()
            .ok_or_else(|| "MP3 has no tag to update".to_string())?
    };

    tag.set_title(meta.title.clone());
    tag.set_artist(join_names(&meta.artists));
    tag.set_album(meta.album.clone());
    tag.set_track(position.max(1));
    tag.set_track_total(total_tracks);
    tag.set_disk(meta.disc_number);

    if !meta.album_artists.is_empty() {
        tag.insert_text(ItemKey::AlbumArtist, join_names(&meta.album_artists));
    }
    if let Some(date) = &meta.release_date {
        tag.insert_text(ItemKey::RecordingDate, date.clone());
    }
    if !meta.genres.is_empty() {
        tag.set_genre(meta.genres.join(", "));
    }
    if let Some(isrc) = &meta.isrc {
        tag.insert_text(ItemKey::Isrc, isrc.clone());
    }
    if let Some(label) = &meta.label {
        tag.insert_text(ItemKey::Label, label.clone());
    }
    if let Some(copyright) = &meta.copyright {
        tag.insert_text(ItemKey::CopyrightMessage, copyright.clone());
    }
    if let Some(url) = &meta.spotify_url {
        tag.set_comment(url.clone());
    }
    if meta.explicit {
        tag.insert_text(ItemKey::ParentalAdvisory, "1".to_string());
    }

    tag.save_to_path(path, WriteOptions::new().use_id3v23(true))
        .map_err(|error| format!("Could not write MP3 tags: {error}"))
}

fn join_names(names: &[String]) -> String {
    names
        .iter()
        .map(|name| name.trim())
        .filter(|name| !name.is_empty())
        .collect::<Vec<_>>()
        .join(", ")
}

async fn attach_cover(tag: &mut Tag, url: &str) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|error| format!("could not build HTTP client: {error}"))?;

    let response = client
        .get(url)
        .send()
        .await
        .map_err(|error| format!("request failed: {error}"))?
        .error_for_status()
        .map_err(|error| format!("request failed: {error}"))?;

    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);

    let data = response
        .bytes()
        .await
        .map_err(|error| format!("download failed: {error}"))?
        .to_vec();

    if data.is_empty() {
        return Err("empty image".to_string());
    }

    let mime = cover_mime_type(content_type.as_deref(), &data)
        .ok_or_else(|| "unsupported image type".to_string())?;

    let picture = Picture::unchecked(data)
        .pic_type(PictureType::CoverFront)
        .mime_type(mime)
        .build();
    tag.push_picture(picture);

    Ok(())
}

/// Content-type wins when it is JPEG/PNG; otherwise sniff the magic bytes.
fn cover_mime_type(content_type: Option<&str>, data: &[u8]) -> Option<MimeType> {
    if let Some(value) = content_type {
        let base = value.split(';').next().unwrap_or("").trim();
        let mime = MimeType::from_str(base);
        if matches!(mime, MimeType::Jpeg | MimeType::Png) {
            return Some(mime);
        }
    }

    if data.starts_with(&[0xFF, 0xD8]) {
        return Some(MimeType::Jpeg);
    }
    if data.starts_with(&[0x89, b'P', b'N', b'G']) {
        return Some(MimeType::Png);
    }

    None
}

// ---------------------------------------------------------------------------
// Tests (pure helpers only; no network or subprocesses)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_download_progress_lines() {
        assert_eq!(
            parse_progress("[download]  42.3% of 3.50MiB at 1.20MiB/s ETA 00:02"),
            Some(42)
        );
        assert_eq!(
            parse_progress("[download] 100% of 3.50MiB in 00:02"),
            Some(100)
        );
        assert_eq!(parse_progress("[download]   0.0% of ~3.50MiB"), Some(0));
        assert_eq!(parse_progress("[download] Destination: track.mp3"), None);
        assert_eq!(parse_progress("[info] Downloading 42.3%"), None);
        assert_eq!(parse_progress(""), None);
    }

    #[test]
    fn detects_transient_errors() {
        assert!(is_transient(
            "ERROR: unable to download video data: HTTP Error 403: Forbidden"
        ));
        assert!(is_transient("HTTP Error 429: Too Many Requests"));
        assert!(is_transient("The read operation timed out"));
        assert!(is_transient("Connection reset by peer"));
        assert!(is_transient("Unable to download webpage"));
        assert!(is_transient("Temporary failure in name resolution"));
        assert!(!is_transient("ERROR: Video unavailable"));
        assert!(!is_transient("ERROR: requested format not available"));
        assert!(!is_transient(""));
    }

    #[test]
    fn search_queries_fall_back_to_a_title_without_filtered_words() {
        let mut meta = TrackMeta::default();
        meta.title = "FUCK AI".to_string();
        meta.artists = vec!["Rustage".to_string()];

        assert_eq!(
            search_queries(&meta),
            vec![
                "ytsearch1:Rustage - FUCK AI",
                "ytsearch1:Rustage FUCK AI",
                "ytsearch1:Rustage - AI",
                "ytsearch1:Rustage AI",
            ]
        );
    }

    #[test]
    fn search_queries_keep_clean_titles_to_one_pair() {
        let mut meta = TrackMeta::default();
        meta.title = "Monkeys Spinning Monkeys".to_string();
        meta.artists = vec!["Kevin MacLeod".to_string()];

        assert_eq!(
            search_queries(&meta),
            vec![
                "ytsearch1:Kevin MacLeod - Monkeys Spinning Monkeys",
                "ytsearch1:Kevin MacLeod Monkeys Spinning Monkeys",
            ]
        );
    }

    #[test]
    fn masked_profanity_still_yields_a_clean_fallback() {
        let mut meta = TrackMeta::default();
        meta.title = "F*CK AI".to_string();
        meta.artists = vec!["Rustage".to_string()];

        let queries = search_queries(&meta);
        assert!(queries.contains(&"ytsearch1:Rustage - AI".to_string()));
        assert!(queries.contains(&"ytsearch1:Rustage AI".to_string()));
    }

    #[test]
    fn titles_made_only_of_filtered_words_have_no_clean_variant() {
        let mut meta = TrackMeta::default();
        meta.title = "FUCK".to_string();
        meta.artists = vec!["Rustage".to_string()];

        assert_eq!(
            search_queries(&meta),
            vec!["ytsearch1:Rustage - FUCK", "ytsearch1:Rustage FUCK"]
        );
    }

    #[test]
    fn search_queries_work_without_an_artist() {
        let mut meta = TrackMeta::default();
        meta.title = "FUCK AI".to_string();

        assert_eq!(
            search_queries(&meta),
            vec!["ytsearch1:FUCK AI", "ytsearch1:AI"]
        );
    }

    #[test]
    fn filtered_words_compare_by_letters_only() {
        assert!(is_search_filtered("FUCK"));
        assert!(is_search_filtered("F*CK"));
        assert!(is_search_filtered("F.U.C.K"));
        assert!(!is_search_filtered("Assassin"));
        assert!(!is_search_filtered("Truck"));
        assert!(!is_search_filtered(""));
    }

    #[test]
    fn builds_yt_dlp_args_without_ffmpeg_dir() {
        let args = build_yt_dlp_args("ytsearch1:Artist - Title", "/tmp/.abc.%(ext)s", None);

        assert_eq!(args[0], "--no-playlist");
        assert!(args.contains(&"--newline".to_string()));
        assert!(args.contains(&"--no-warnings".to_string()));
        assert!(args.contains(&"--ignore-config".to_string()));
        assert!(
            args.windows(2)
                .any(|pair| pair[0] == "-f" && pair[1] == "bestaudio/best")
        );
        assert!(args.contains(&"-x".to_string()));
        assert!(
            args.windows(2)
                .any(|pair| pair[0] == "--audio-format" && pair[1] == "mp3")
        );
        assert!(
            args.windows(2)
                .any(|pair| pair[0] == "--audio-quality" && pair[1] == "0")
        );
        assert!(!args.contains(&"--ffmpeg-location".to_string()));
        assert!(
            args.windows(2)
                .any(|pair| pair[0] == "-o" && pair[1] == "/tmp/.abc.%(ext)s")
        );

        // The query is always the final argument.
        assert_eq!(
            args.last().map(String::as_str),
            Some("ytsearch1:Artist - Title")
        );
    }

    #[test]
    fn builds_yt_dlp_args_with_ffmpeg_dir() {
        let args = build_yt_dlp_args(
            "ytsearch1:Artist - Title",
            "temp.%(ext)s",
            Some(Path::new("ffmpeg-dir")),
        );

        let position = args
            .iter()
            .position(|arg| arg == "--ffmpeg-location")
            .expect("--ffmpeg-location present");
        assert_eq!(args[position + 1], "ffmpeg-dir");
        assert_eq!(
            args.last().map(String::as_str),
            Some("ytsearch1:Artist - Title")
        );
    }
}
