use std::fs;
use std::path::Path;

fn main() {
    // Desktop builds ship without a runtime `.env`; bake the public PKCE client id
    // in at compile time so end users never have to configure credentials.
    // The client secret is intentionally never embedded.
    let env_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(".env");
    println!("cargo:rerun-if-changed={}", env_path.display());

    if let Ok(contents) = fs::read_to_string(&env_path) {
        for line in contents.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            if key.trim() != "SPOTIFY_CLIENT_ID" {
                continue;
            }
            let value = value.trim().trim_matches('"').trim_matches('\'').trim();
            if !value.is_empty() {
                println!("cargo:rustc-env=SPOTIFY_CLIENT_ID={value}");
            }
        }
    }

    tauri_build::build()
}
