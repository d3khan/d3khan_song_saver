//! Spotify authentication: PKCE client construction, loopback redirect
//! listener, keyring token storage and settings/env resolution.
//!
//! Diagnostics use `eprintln!`. Tokens and secrets are never printed, logged
//! or returned to the frontend.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use rspotify::prelude::*;
use rspotify::{
    AuthCodePkceSpotify, CallbackError, Config, Credentials, OAuth, Token, TokenCallback, scopes,
};

/// Refresh the access token this long before it expires, so a job never starts
/// with a token that dies mid-flight.
const REFRESH_MARGIN_SECS: i64 = 300;

/// Windows Credential Manager service name shared by all app credentials.
pub const SERVICE: &str = "spotify_downloader";
/// Keyring user name for the serialized `rspotify::Token`.
pub const TOKEN_USER: &str = "spotify_token";
/// Keyring user name for the optional client secret.
pub const SECRET_USER: &str = "spotify_secret";
/// Redirect URI used when `SPOTIFY_REDIRECT_URI` is not set.
pub const DEFAULT_REDIRECT_URI: &str = "http://127.0.0.1:8888/callback";

/// Listener port used when the redirect URI does not carry one.
const DEFAULT_PORT: u16 = 8888;

const CALLBACK_PATH: &str = "/callback";
const CALLBACK_TIMEOUT: Duration = Duration::from_secs(180);
const ACCEPT_POLL: Duration = Duration::from_millis(100);
const READ_POLL: Duration = Duration::from_millis(100);
/// How long a single client may take to send its request head. Keeps browser
/// pre-connects from stalling the accept loop, and keeps cancellation snappy.
const CONNECTION_BUDGET: Duration = Duration::from_secs(2);

const CANCELLED: &str = "Sign-in cancelled";
const TIMED_OUT: &str = "Sign-in timed out";

/// Client-id + provenance resolved from `settings.json` and the environment.
///
/// Deliberately has no secret field: the optional client secret lives in the
/// keyring and is never part of `Settings`.
#[derive(Clone, Debug, Default)]
pub struct Settings {
    pub client_id: Option<String>,
    /// `"settings"`, `"env"` or `"none"`.
    pub source: String,
    pub redirect_uri: String,
}

/// Loads `.env` candidates (first one found wins). Idempotent: dotenvy never
/// overwrites variables that are already present in the process environment.
pub fn load_env_files() {
    let mut candidates: Vec<PathBuf> = vec![
        PathBuf::from(".env"),
        PathBuf::from("../.env"),
        PathBuf::from("../../.env"),
    ];
    if let Some(manifest_dir) = option_env!("CARGO_MANIFEST_DIR") {
        candidates.push(Path::new(manifest_dir).join("..").join(".env"));
    }

    for candidate in candidates {
        if candidate.is_file() && dotenvy::from_path(&candidate).is_ok() {
            break;
        }
    }
}

/// Reads `settings.json` (if present) and falls back to the environment.
/// Never panics.
pub fn resolve_settings(settings_path: &Path) -> Settings {
    let redirect_uri = redirect_uri();

    if let Some(client_id) = read_settings_client_id(settings_path) {
        return Settings {
            client_id: Some(client_id),
            source: "settings".to_string(),
            redirect_uri,
        };
    }

    match env_client_id() {
        Some(client_id) => Settings {
            client_id: Some(client_id),
            source: "env".to_string(),
            redirect_uri,
        },
        None => Settings {
            client_id: None,
            source: "none".to_string(),
            redirect_uri,
        },
    }
}

fn read_settings_client_id(settings_path: &Path) -> Option<String> {
    let contents = match std::fs::read_to_string(settings_path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
        Err(error) => {
            eprintln!(
                "Could not read settings file {}: {error}",
                settings_path.display()
            );
            return None;
        }
    };

    let value: serde_json::Value = match serde_json::from_str(&contents) {
        Ok(value) => value,
        Err(error) => {
            eprintln!(
                "Ignoring invalid settings file {}: {error}",
                settings_path.display()
            );
            return None;
        }
    };

    let client_id = value.get("client_id")?.as_str()?.trim();
    if client_id.is_empty() {
        None
    } else {
        Some(client_id.to_string())
    }
}

/// Redirect URI from `SPOTIFY_REDIRECT_URI`, else [`DEFAULT_REDIRECT_URI`].
pub fn redirect_uri() -> String {
    match std::env::var("SPOTIFY_REDIRECT_URI") {
        Ok(value) if !value.trim().is_empty() => value.trim().to_string(),
        _ => DEFAULT_REDIRECT_URI.to_string(),
    }
}

/// Port of the loopback redirect URI (defaults to 8888).
pub fn redirect_port(uri: &str) -> u16 {
    url::Url::parse(uri)
        .ok()
        .and_then(|parsed| parsed.port())
        .unwrap_or(DEFAULT_PORT)
}

/// `SPOTIFY_CLIENT_ID`, falling back to `RSPOTIFY_CLIENT_ID`.
pub fn env_client_id() -> Option<String> {
    std::env::var("SPOTIFY_CLIENT_ID")
        .ok()
        .or_else(|| std::env::var("RSPOTIFY_CLIENT_ID").ok())
        // `build.rs` bakes the public PKCE client id into packaged builds.
        .or_else(|| option_env!("SPOTIFY_CLIENT_ID").map(str::to_string))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// `SPOTIFY_CLIENT_SECRET`, falling back to `RSPOTIFY_CLIENT_SECRET`.
///
/// PKCE does not use a secret; this exists for parity with the `.env` contract
/// and is never sent to the frontend.
pub fn env_client_secret() -> Option<String> {
    std::env::var("SPOTIFY_CLIENT_SECRET")
        .ok()
        .or_else(|| std::env::var("RSPOTIFY_CLIENT_SECRET").ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// Writes `{ "client_id": "..." }` as pretty JSON, creating parent folders.
pub fn write_settings_file(path: &Path, client_id: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "Could not create settings folder {}: {error}",
                    parent.display()
                )
            })?;
        }
    }

    let contents = serde_json::to_string_pretty(&serde_json::json!({ "client_id": client_id }))
        .map_err(|error| format!("Could not serialize settings: {error}"))?;

    std::fs::write(path, contents)
        .map_err(|error| format!("Could not write settings file {}: {error}", path.display()))
}

/// Removes `settings.json`; succeeds when the file is already gone.
pub fn clear_settings_file(path: &Path) -> Result<(), String> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "Could not remove settings file {}: {error}",
            path.display()
        )),
    }
}

fn keyring_entry(user: &str) -> Result<keyring::Entry, String> {
    keyring::Entry::new(SERVICE, user)
        .map_err(|error| format!("Could not open Windows Credential Manager: {error}"))
}

/// Stores the full `rspotify::Token` JSON in the Credential Manager.
pub fn store_token(token: &Token) -> Result<(), String> {
    let json = serde_json::to_string(token)
        .map_err(|error| format!("Could not serialize the Spotify token: {error}"))?;

    keyring_entry(TOKEN_USER)?
        .set_password(&json)
        .map_err(|error| format!("Could not save the Spotify token: {error}"))
}

fn has_refresh_token(token: &Token) -> bool {
    token
        .refresh_token
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty())
}

/// Spotify's refresh response usually omits `refresh_token` ("if it does not,
/// continue using the existing token"). rspotify stores the response as-is, so
/// merge the previous token back in before persisting.
fn with_previous_refresh_token(mut token: Token, previous: Option<&Token>) -> Token {
    if !has_refresh_token(&token) {
        if let Some(previous) = previous.filter(|previous| has_refresh_token(previous)) {
            token.refresh_token = previous.refresh_token.clone();
        }
    }
    token
}

fn store_token_preserving_refresh(token: &Token) -> Result<(), String> {
    let previous = load_token();
    store_token(&with_previous_refresh_token(
        token.clone(),
        previous.as_ref(),
    ))
}

/// Loads a previously stored token; `None` when absent or unreadable.
pub fn load_token() -> Option<Token> {
    let entry = match keyring_entry(TOKEN_USER) {
        Ok(entry) => entry,
        Err(error) => {
            eprintln!("{error}");
            return None;
        }
    };

    match entry.get_password() {
        Ok(json) => match serde_json::from_str::<Token>(&json) {
            Ok(token) => Some(token),
            Err(error) => {
                eprintln!("Stored Spotify token is invalid and was ignored: {error}");
                None
            }
        },
        Err(keyring::Error::NoEntry) => None,
        Err(error) => {
            eprintln!("Could not read the Spotify token: {error}");
            None
        }
    }
}

/// Deletes the stored token. Missing entries are fine; other errors are logged.
pub fn delete_token() {
    delete_credential(TOKEN_USER, "token");
}

/// Stores the optional client secret in the Credential Manager.
pub fn store_secret(secret: &str) -> Result<(), String> {
    keyring_entry(SECRET_USER)?
        .set_password(secret)
        .map_err(|error| format!("Could not save the Spotify client secret: {error}"))
}

/// Loads the optional client secret; `None` when absent or unreadable.
pub fn load_secret() -> Option<String> {
    let entry = match keyring_entry(SECRET_USER) {
        Ok(entry) => entry,
        Err(error) => {
            eprintln!("{error}");
            return None;
        }
    };

    match entry.get_password() {
        Ok(secret) => Some(secret),
        Err(keyring::Error::NoEntry) => None,
        Err(error) => {
            eprintln!("Could not read the Spotify client secret: {error}");
            None
        }
    }
}

/// Deletes the stored secret. Missing entries are fine; other errors are logged.
pub fn delete_secret() {
    delete_credential(SECRET_USER, "client secret");
}

fn delete_credential(user: &str, label: &str) {
    let entry = match keyring_entry(user) {
        Ok(entry) => entry,
        Err(error) => {
            eprintln!("{error}");
            return;
        }
    };

    match entry.delete_credential() {
        Ok(()) => {}
        Err(keyring::Error::NoEntry) => {}
        Err(error) => eprintln!("Could not delete the stored Spotify {label}: {error}"),
    }
}

fn oauth_for(redirect_uri: &str) -> OAuth {
    OAuth {
        redirect_uri: redirect_uri.to_string(),
        scopes: scopes!(
            "playlist-read-private",
            "playlist-read-collaborative",
            "user-read-private",
            "user-read-email"
        ),
        ..Default::default()
    }
}

fn shared_config() -> Config {
    let callback = TokenCallback(Box::new(|token: Token| {
        store_token_preserving_refresh(&token).map_err(CallbackError::CustomizedError)
    }));

    Config {
        token_cached: false,
        // rspotify refreshes on its own only when the token is within 10 s of
        // expiry, and it replaces the token with Spotify's response — which
        // omits `refresh_token`, signing the user out an hour later. All
        // refreshes go through `refresh_session`/`refresh_now` instead, which
        // carry the refresh token forward.
        token_refreshing: false,
        token_callback_fn: Arc::new(Some(callback)),
        ..Default::default()
    }
}

/// Builds a fresh PKCE client. The token callback persists every token
/// (including rotated refresh tokens) to the Credential Manager.
pub fn build_client(client_id: &str, redirect_uri: &str) -> AuthCodePkceSpotify {
    AuthCodePkceSpotify::with_config(
        Credentials::new_pkce(client_id),
        oauth_for(redirect_uri),
        shared_config(),
    )
}

/// Restores a client from a stored token, keeping auto-refresh and persistence.
pub fn restore_client(client_id: &str, redirect_uri: &str, token: Token) -> AuthCodePkceSpotify {
    AuthCodePkceSpotify::from_token_with_config(
        token,
        Credentials::new_pkce(client_id),
        oauth_for(redirect_uri),
        shared_config(),
    )
}

/// Persists the current in-memory token. No-op when there is no token.
pub async fn persist_token(client: &AuthCodePkceSpotify) {
    let token = client.token.lock().await.unwrap().clone();
    if let Some(token) = token {
        if let Err(error) = store_token(&token) {
            eprintln!("Could not persist the Spotify token: {error}");
        }
    }
}

/// Result of the proactive session refresh done at startup and before a job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionRefresh {
    /// The access token is valid for longer than the refresh margin.
    Fresh,
    /// A new access token was obtained.
    Refreshed,
    /// Spotify no longer accepts the refresh token; the stored session is gone.
    Rejected,
    /// Refresh failed for a reason that may be temporary (network, keyring, ...).
    Failed(String),
}

/// True when `token` is expired, has no expiry, or expires within the margin.
fn needs_refresh(token: &Token) -> bool {
    token.expires_at.is_none_or(|expiry| {
        expiry <= chrono::Utc::now() + chrono::TimeDelta::seconds(REFRESH_MARGIN_SECS)
    })
}

/// The refresh token from the in-memory session, falling back to the stored
/// credential (older builds dropped it from memory on automatic refresh).
async fn session_refresh_token(client: &AuthCodePkceSpotify) -> Option<String> {
    let in_memory = match client.token.lock().await {
        Ok(guard) => match guard.as_ref() {
            Some(token) => token.refresh_token.clone(),
            // Without an in-memory session there is nothing to refresh.
            None => return None,
        },
        Err(_) => return None,
    };

    if in_memory
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty())
    {
        return in_memory;
    }

    load_token()
        .and_then(|token| token.refresh_token)
        .filter(|value| !value.trim().is_empty())
}

/// Puts `refresh` back into the in-memory token when it is missing, so
/// rspotify's refresh request can use it.
async fn adopt_refresh_token(client: &AuthCodePkceSpotify, refresh: &str) {
    if let Ok(mut guard) = client.token.lock().await {
        if let Some(token) = guard.as_mut() {
            if !has_refresh_token(token) {
                token.refresh_token = Some(refresh.to_string());
            }
        }
    }
}

/// Spotify answers a revoked or 6-month-old refresh token with `400 invalid_grant`.
fn refresh_was_rejected(error: &rspotify::ClientError) -> bool {
    matches!(
        error,
        rspotify::ClientError::Http(http_error)
            if matches!(
                http_error.as_ref(),
                rspotify::http::HttpError::StatusCode(response)
                    if matches!(response.status().as_u16(), 400 | 401)
            )
    )
}

async fn refresh_token(client: &AuthCodePkceSpotify) -> SessionRefresh {
    // rspotify replaces the in-memory token with Spotify's response, which
    // omits `refresh_token`; keep the old one so the next refresh still works.
    let previous_refresh = match client.token.lock().await {
        Ok(guard) => guard
            .as_ref()
            .and_then(|token| token.refresh_token.clone())
            .filter(|value| !value.trim().is_empty()),
        Err(_) => None,
    };

    match client.refresh_token().await {
        Ok(()) => {
            if let Ok(mut guard) = client.token.lock().await {
                if let Some(token) = guard.as_mut() {
                    if !has_refresh_token(token) {
                        token.refresh_token = previous_refresh;
                    }
                }
            }
            SessionRefresh::Refreshed
        }
        Err(error) => {
            let message = format!("Could not refresh the Spotify session: {error}");
            eprintln!("{message}");
            if refresh_was_rejected(&error) {
                // The refresh token cannot be recovered; keeping it would leave
                // the UI claiming a session that no longer exists.
                delete_token();
                SessionRefresh::Rejected
            } else {
                SessionRefresh::Failed(message)
            }
        }
    }
}

/// Refreshes the access token when it is missing, expired, or about to expire.
///
/// Access tokens last one hour; the refresh token is what keeps the account
/// signed in between runs (Spotify allows it to live for six months from
/// authorization). Refreshing up front makes a job's first request
/// deterministic instead of relying on a last-second refresh.
pub async fn refresh_session(client: &AuthCodePkceSpotify) -> SessionRefresh {
    let needs = {
        let Ok(guard) = client.token.lock().await else {
            return SessionRefresh::Failed("Could not read the Spotify token.".to_string());
        };
        match guard.as_ref() {
            Some(token) => needs_refresh(token),
            None => return SessionRefresh::Rejected,
        }
    };

    if !needs {
        return SessionRefresh::Fresh;
    }

    let Some(refresh) = session_refresh_token(client).await else {
        delete_token();
        return SessionRefresh::Rejected;
    };
    adopt_refresh_token(client, &refresh).await;
    refresh_token(client).await
}

/// Forces one refresh now, used to recover from an unexpected `401` even
/// though the stored access token had not expired yet.
///
/// Returns `true` when a new access token was obtained, `false` when the
/// session is gone or the refresh failed (the caller should not retry).
pub async fn refresh_now(client: &AuthCodePkceSpotify) -> bool {
    let Some(refresh) = session_refresh_token(client).await else {
        delete_token();
        return false;
    };
    adopt_refresh_token(client, &refresh).await;

    matches!(refresh_token(client).await, SessionRefresh::Refreshed)
}

/// Best-effort display name for the signed-in user (`display_name`, else `id`).
pub async fn fetch_display_name(client: &AuthCodePkceSpotify) -> Option<String> {
    let user = client.current_user().await.ok()?;
    match user.display_name {
        Some(name) if !name.trim().is_empty() => Some(name.trim().to_string()),
        _ => Some(user.id.id().to_string()),
    }
}

/// Binds `127.0.0.1:<port>` before the browser is opened so a busy port fails
/// fast with a readable error.
pub fn bind_callback(port: u16) -> Result<TcpListener, String> {
    TcpListener::bind(("127.0.0.1", port)).map_err(|error| {
        format!(
            "Port {port} is already in use. Close the other application or choose a different \
             port. ({error})"
        )
    })
}

/// Waits for the browser to hit `/callback` and returns the authorization code.
///
/// Polls every 100 ms, gives up after three minutes, and returns
/// `Err("Sign-in cancelled")` within ~200 ms when `cancel` is set.
pub async fn await_callback(
    listener: TcpListener,
    expected_state: String,
    cancel: Arc<AtomicBool>,
) -> Result<String, String> {
    tokio::task::spawn_blocking(move || wait_for_callback(listener, &expected_state, &cancel))
        .await
        .map_err(|error| format!("Sign-in listener task failed: {error}"))?
}

fn wait_for_callback(
    listener: TcpListener,
    expected_state: &str,
    cancel: &AtomicBool,
) -> Result<String, String> {
    listener
        .set_nonblocking(true)
        .map_err(|error| format!("Could not configure the sign-in listener: {error}"))?;

    let deadline = Instant::now() + CALLBACK_TIMEOUT;

    loop {
        if cancel.load(Ordering::SeqCst) {
            return Err(CANCELLED.to_string());
        }
        if Instant::now() >= deadline {
            return Err(TIMED_OUT.to_string());
        }

        match listener.accept() {
            Ok((stream, _peer)) => match handle_connection(stream, expected_state, cancel) {
                Ok(Some(code)) => return Ok(code),
                // Unrelated request (favicon, pre-connect, ...): keep waiting.
                Ok(None) => {}
                Err(error) => return Err(error),
            },
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::TimedOut
                        | std::io::ErrorKind::Interrupted
                ) =>
            {
                std::thread::sleep(ACCEPT_POLL);
            }
            Err(error) => return Err(format!("Sign-in listener failed: {error}")),
        }
    }
}

/// Handles one HTTP request. `Ok(Some(code))` on a valid callback, `Ok(None)`
/// when the request is unrelated (already answered with 404), `Err` on a
/// callback-specific failure (already answered with an error page).
fn handle_connection(
    mut stream: TcpStream,
    expected_state: &str,
    cancel: &AtomicBool,
) -> Result<Option<String>, String> {
    let Some(head) = read_request_head(&mut stream, cancel) else {
        return Ok(None);
    };

    let request_line = head.lines().next().unwrap_or_default();
    let target = request_line.split_whitespace().nth(1).unwrap_or("");
    let (path, query) = match target.split_once('?') {
        Some((path, query)) => (path, query),
        None => (target, ""),
    };

    if path != CALLBACK_PATH {
        let _ = respond_html(
            &mut stream,
            404,
            "Not Found",
            &page("Not found", "This address only handles Spotify sign-in."),
        );
        return Ok(None);
    }

    let params = parse_query(query);

    if let Some(error) = params.get("error") {
        if error == "access_denied" {
            let _ = respond_html(
                &mut stream,
                200,
                "OK",
                &page(
                    "Sign-in cancelled",
                    "You cancelled the Spotify sign-in. You can close this tab.",
                ),
            );
            return Err(CANCELLED.to_string());
        }

        let _ = respond_html(
            &mut stream,
            200,
            "OK",
            &page(
                "Sign-in failed",
                "Spotify returned an error while signing you in. Please try again.",
            ),
        );
        return Err(format!("Sign-in failed: {error}"));
    }

    match params.get("state") {
        Some(state) if state == expected_state => {}
        Some(state) => {
            let _ = respond_html(
                &mut stream,
                400,
                "Bad Request",
                &page(
                    "Sign-in failed",
                    "The security state did not match. Please start again from the app.",
                ),
            );
            return Err(format!(
                "State mismatch: expected {expected_state}, got {state}"
            ));
        }
        None => {
            let _ = respond_html(
                &mut stream,
                400,
                "Bad Request",
                &page(
                    "Sign-in failed",
                    "The sign-in callback was missing its security state.",
                ),
            );
            return Err("Missing state in the sign-in callback".to_string());
        }
    }

    match params.get("code") {
        Some(code) if !code.is_empty() => {
            let _ = respond_html(
                &mut stream,
                200,
                "OK",
                &page(
                    "Sign-in complete",
                    "You are signed in. You can close this tab and return to the app.",
                ),
            );
            Ok(Some(code.clone()))
        }
        _ => {
            let _ = respond_html(
                &mut stream,
                400,
                "Bad Request",
                &page(
                    "Sign-in failed",
                    "Spotify did not return an authorization code. Please try again.",
                ),
            );
            Err("No authorization code returned by Spotify".to_string())
        }
    }
}

/// Reads the request head (request line + headers) with small read timeouts so
/// cancellation stays responsive and idle pre-connects do not block the loop.
fn read_request_head(stream: &mut TcpStream, cancel: &AtomicBool) -> Option<String> {
    if let Err(error) = stream.set_read_timeout(Some(READ_POLL)) {
        eprintln!("Could not set a timeout on the sign-in connection: {error}");
        return None;
    }

    let deadline = Instant::now() + CONNECTION_BUDGET;
    let mut data: Vec<u8> = Vec::new();
    let mut buffer = [0u8; 1024];

    loop {
        if cancel.load(Ordering::SeqCst) || Instant::now() >= deadline {
            return None;
        }

        match stream.read(&mut buffer) {
            Ok(0) => return None,
            Ok(read) => {
                data.extend_from_slice(&buffer[..read]);
                if has_complete_headers(&data) || data.len() > 32 * 1024 {
                    return Some(String::from_utf8_lossy(&data).into_owned());
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::TimedOut
                        | std::io::ErrorKind::Interrupted
                ) => {}
            Err(error) => {
                eprintln!("Could not read the sign-in request: {error}");
                return None;
            }
        }
    }
}

fn has_complete_headers(data: &[u8]) -> bool {
    data.windows(4).any(|window| window == b"\r\n\r\n")
        || data.windows(2).any(|window| window == b"\n\n")
}

fn respond_html(
    stream: &mut TcpStream,
    status: u16,
    reason: &str,
    body: &str,
) -> std::io::Result<()> {
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\n\
         Content-Type: text/html; charset=utf-8\r\n\
         Content-Length: {}\r\n\
         Cache-Control: no-store\r\n\
         Connection: close\r\n\
         \r\n{body}",
        body.as_bytes().len()
    );

    // Write the whole response before returning, then half-close so the browser
    // sees a complete body and the connection closes immediately.
    stream.write_all(response.as_bytes())?;
    stream.flush()?;
    let _ = stream.shutdown(Shutdown::Write);
    Ok(())
}

fn page(title: &str, message: &str) -> String {
    format!(
        "<!DOCTYPE html>\n\
<html lang=\"en\"><head><meta charset=\"utf-8\"><title>{title}</title><style>\n\
body{{margin:0;min-height:100vh;display:flex;align-items:center;justify-content:center;\
background:#121212;color:#f5f5f5;font-family:'Segoe UI',system-ui,sans-serif}}\n\
main{{max-width:30rem;padding:2rem;text-align:center}}\n\
h1{{font-size:1.35rem;margin:0 0 .75rem}}p{{color:#b3b3b3;line-height:1.5;margin:0}}\n\
</style></head><body><main><h1>{title}</h1><p>{message}</p></main></body></html>"
    )
}

fn parse_query(query: &str) -> HashMap<String, String> {
    let mut params = HashMap::new();

    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (key, value) = match pair.split_once('=') {
            Some((key, value)) => (key, value),
            None => (pair, ""),
        };
        params.insert(percent_decode(key), percent_decode(value));
    }

    params
}

/// Tiny `application/x-www-form-urlencoded` decoder (`%HH` and `+`).
fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut decoded: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut index = 0;

    while index < bytes.len() {
        match bytes[index] {
            b'%' if index + 2 < bytes.len() => {
                let high = (bytes[index + 1] as char).to_digit(16);
                let low = (bytes[index + 2] as char).to_digit(16);
                match (high, low) {
                    (Some(high), Some(low)) => {
                        decoded.push((high * 16 + low) as u8);
                        index += 3;
                    }
                    _ => {
                        decoded.push(bytes[index]);
                        index += 1;
                    }
                }
            }
            b'+' => {
                decoded.push(b' ');
                index += 1;
            }
            byte => {
                decoded.push(byte);
                index += 1;
            }
        }
    }

    String::from_utf8_lossy(&decoded).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "spotify_downloader_auth_{label}_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        dir
    }

    #[test]
    fn settings_file_round_trip_takes_precedence() {
        let dir = scratch_dir("settings");
        let path = dir.join("settings.json");

        // Missing file is fine to clear.
        assert!(clear_settings_file(&path).is_ok());
        assert!(!path.exists());

        write_settings_file(&path, "settings-client-id").expect("write settings");

        // File contains exactly the client id, never the secret.
        let raw = std::fs::read_to_string(&path).expect("read settings");
        let parsed: serde_json::Value = serde_json::from_str(&raw).expect("parse settings");
        assert_eq!(
            parsed,
            serde_json::json!({ "client_id": "settings-client-id" })
        );
        assert!(!raw.contains("secret"));

        let settings = resolve_settings(&path);
        assert_eq!(settings.client_id.as_deref(), Some("settings-client-id"));
        assert_eq!(settings.source, "settings");
        assert_eq!(settings.redirect_uri, redirect_uri());

        clear_settings_file(&path).expect("clear settings");
        assert!(!path.exists());

        let after = resolve_settings(&path);
        assert_ne!(after.source, "settings");
        assert_ne!(after.client_id.as_deref(), Some("settings-client-id"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_settings_file_creates_parent_directories() {
        let dir = scratch_dir("create");
        let path = dir.join("nested").join("settings.json");

        write_settings_file(&path, "abc123").expect("write settings");
        assert!(path.is_file());
        assert!(clear_settings_file(&path).is_ok());
        assert!(clear_settings_file(&path).is_ok());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn redirect_port_parsing() {
        assert_eq!(redirect_port(DEFAULT_REDIRECT_URI), 8888);
        assert_eq!(redirect_port("http://127.0.0.1:9123/callback"), 9123);
        assert_eq!(redirect_port("http://127.0.0.1:1/callback"), 1);
        // No explicit port, or garbage, falls back to the default.
        assert_eq!(redirect_port("http://127.0.0.1/callback"), DEFAULT_PORT);
        assert_eq!(redirect_port("not a uri"), DEFAULT_PORT);
    }

    #[test]
    fn percent_decoding_query_values() {
        assert_eq!(percent_decode("AQD%2Fabc-123_%7E"), "AQD/abc-123_~");
        assert_eq!(percent_decode("hello+world"), "hello world");
        assert_eq!(percent_decode("caf%C3%A9"), "café");
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%zz"), "%zz");
        assert_eq!(percent_decode(""), "");
    }

    #[test]
    fn parses_callback_query() {
        let params = parse_query("code=AQD%2Fxyz&state=abc123");
        assert_eq!(params.get("code").map(String::as_str), Some("AQD/xyz"));
        assert_eq!(params.get("state").map(String::as_str), Some("abc123"));
    }

    #[test]
    fn refresh_response_keeps_the_previous_refresh_token() {
        let previous = Token {
            refresh_token: Some("previous-refresh".to_string()),
            ..Default::default()
        };
        let refreshed = Token {
            access_token: "new-access".to_string(),
            refresh_token: None,
            ..Default::default()
        };

        let merged = with_previous_refresh_token(refreshed, Some(&previous));
        assert_eq!(merged.access_token, "new-access");
        assert_eq!(merged.refresh_token.as_deref(), Some("previous-refresh"));
    }

    #[test]
    fn a_rotated_refresh_token_wins_over_the_previous_one() {
        let previous = Token {
            refresh_token: Some("old-refresh".to_string()),
            ..Default::default()
        };
        let refreshed = Token {
            refresh_token: Some("new-refresh".to_string()),
            ..Default::default()
        };

        let merged = with_previous_refresh_token(refreshed, Some(&previous));
        assert_eq!(merged.refresh_token.as_deref(), Some("new-refresh"));
    }

    #[test]
    fn blank_refresh_tokens_are_not_usable() {
        let blank = Token {
            refresh_token: Some("   ".to_string()),
            ..Default::default()
        };
        assert!(!has_refresh_token(&blank));
        assert!(!has_refresh_token(&Token::default()));

        let merged = with_previous_refresh_token(blank, None);
        assert!(merged.refresh_token.is_some());
    }

    #[test]
    fn tokens_expiring_within_the_margin_are_refreshed() {
        let mut token = Token::default();
        token.expires_at = Some(chrono::Utc::now() + chrono::TimeDelta::minutes(30));
        assert!(!needs_refresh(&token));

        token.expires_at =
            Some(chrono::Utc::now() + chrono::TimeDelta::seconds(REFRESH_MARGIN_SECS - 30));
        assert!(needs_refresh(&token));

        token.expires_at = None;
        assert!(needs_refresh(&token));
    }
}
