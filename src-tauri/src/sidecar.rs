//! The ONLY module that talks to the Python sidecars.
//! Rust stays provider-agnostic: it never names Spotify/YouTube/Deezer.
use serde_json::Value;
use std::path::PathBuf;
use tauri::AppHandle;
use tauri_plugin_shell::process::CommandEvent;
use tauri_plugin_shell::ShellExt;

pub fn last_json(stdout: &str) -> Value {
    stdout
        .lines()
        .rev()
        .find(|l| l.trim().starts_with('{'))
        .and_then(|l| serde_json::from_str(l.trim()).ok())
        .unwrap_or_default()
}

/// One-shot call (resolve / search / formats).
pub async fn call(app: &AppHandle, name: &str, args: &[&str]) -> Result<Value, String> {
    let cmd = app
        .shell()
        .sidecar(name)
        .map_err(|e| format!("Sidecar error: {e}"))?;
    let output = cmd
        .args(args)
        .output()
        .await
        .map_err(|e| format!("{name} failed: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "{name} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let v = last_json(&stdout);
    if v["status"] == "error" {
        return Err(v["message"].as_str().unwrap_or("sidecar error").to_string());
    }
    Ok(v)
}

/// Streaming download call. `on_event` receives progress/stage payloads.
/// Ok(Some(path)) = success, Ok(None) = no status (retryable), Err = hard error.
pub async fn run_download<F>(
    app: &AppHandle,
    args: Vec<String>,
    mut on_event: F,
) -> Result<Option<PathBuf>, String>
where
    F: FnMut(Value),
{
    let cmd = app
        .shell()
        .sidecar("downloader")
        .map_err(|e| format!("Sidecar error: {e}"))?;
    let (mut rx, _child) = cmd
        .args(args.iter().map(String::as_str).collect::<Vec<&str>>())
        .spawn()
        .map_err(|e| format!("Spawn failed: {e}"))?;

    let mut path: Option<PathBuf> = None;
    let mut err: Option<String> = None;
    let mut code: Option<i32> = None;

    while let Some(event) = rx.recv().await {
        match event {
            CommandEvent::Stdout(line) => {
                let text = String::from_utf8_lossy(&line);
                let t = text.trim();
                if !t.starts_with('{') {
                    continue;
                }
                if let Ok(v) = serde_json::from_str::<Value>(t) {
                    let ev = v.get("event").and_then(|e| e.as_str()).unwrap_or("").to_string();
                    if ev == "progress" || ev == "stage" {
                        on_event(v);
                        continue;
                    }
                    if v["status"] == "success" {
                        if let Some(p) = v["path"].as_str() {
                            path = Some(PathBuf::from(p));
                        }
                        break;
                    } else if v["status"] == "error" {
                        err = Some(v["message"].as_str().unwrap_or("Unknown error").to_string());
                        break;
                    }
                }
            }
            CommandEvent::Stderr(line) => {
                let text = String::from_utf8_lossy(&line);
                if text.contains("ERROR:") || text.contains("Traceback") {
                    err = Some(text.trim().to_string());
                }
            }
            CommandEvent::Terminated(p) => code = p.code,
            _ => {}
        }
    }

    if let Some(e) = err {
        return Err(e);
    }
    if path.is_some() {
        return Ok(path);
    }
    let _ = code;
    Ok(None)
}
