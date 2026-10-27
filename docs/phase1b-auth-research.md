# Phase 1b — Spotify Authorization Code + PKCE Auth Research

**Date:** 2026-10-26
**Scope:** rspotify 0.16.1 `AuthCodePkceSpotify`, keyring token storage, Tauri 2.12.5 static-frontend
hosting, browser-open strategy, loopback callback server, settings/env precedence.
**Method:** Every claim verified live against rspotify `v0.16.1` tag source (raw.githubusercontent.com),
crates.io API, Tauri `tauri-v2.12.5` source + official docs, Spotify developer docs, and Microsoft
wincred.h on 2026-10-26. No API was invented; every signature is quoted from source.
**Decision context:** rustifydl dropped (Phase 1); custom pipeline + full user login with redirect URI
fixed at `http://127.0.0.1:8888/callback`.

---

## 1. rspotify `AuthCodePkceSpotify` (v0.16.1)

### 1.1 Cargo features

Source: https://raw.githubusercontent.com/ramsayleung/rspotify/v0.16.1/Cargo.toml

```toml
[features]
default = ["client-reqwest", "reqwest-default-tls"]
cli = ["webbrowser"]          # enables prompt_for_token / get_code_from_user only
env-file = ["dotenvy"]        # enables Credentials::from_env / OAuth::from_env reading .env
client-reqwest = [...]        # async client (Tokio) — required for Tauri
reqwest-default-tls = [...]   # schannel/native TLS on Windows
```

**Recommended feature set for this app:**

```toml
rspotify = { version = "0.16.1", features = ["client-reqwest", "reqwest-default-tls"] }
```

- `cli` is **not** recommended (see §1.5): it pulls `webbrowser`, prints to the terminal, and blocks
  with no timeout. We implement our own listener (§5) instead. (`webbrowser` is added directly if we
  want it for opening the authorize URL — §4.)
- `env-file` is optional; we load `.env` ourselves via `dotenvy` (§6) for explicit path control.

### 1.2 Construction — `Credentials` and `OAuth`

Source: https://raw.githubusercontent.com/ramsayleung/rspotify/v0.16.1/src/lib.rs

```rust
pub struct Credentials {
    pub id: String,
    /// PKCE doesn't require a client secret
    pub secret: Option<String>,
}

impl Credentials {
    /// Initialization with just the client ID
    #[must_use]
    pub fn new_pkce(id: &str) -> Self {
        Self { id: id.to_owned(), secret: None }
    }
}
```

- **Secret is optional and unused in the PKCE token exchange.** `new_pkce` sets `secret: None`, and the
  token request body contains only `client_id` (verified in `request_token`, §1.4). Spotify's PKCE flow
  is designed for public clients — no secret needed. (`Credentials::new(id, secret)` exists if a
  confidential client is ever wanted; not needed here.)
- `Credentials::from_env()` reads `RSPOTIFY_CLIENT_ID` (required) and `RSPOTIFY_CLIENT_SECRET`
  (optional — `env::var(...).ok()`); with `env-file` it first runs `dotenvy::dotenv()`.

```rust
pub struct OAuth {
    pub redirect_uri: String,
    pub state: String,               // Default: random 16 chars via generate_random_string
    pub scopes: HashSet<String>,
    pub proxies: Option<String>,
}
```

- `state` is auto-randomized by `OAuth::default()`; it is embedded in the authorize URL and validated
  on callback by `parse_response_code`. Use `OAuth { redirect_uri: "...".into(), scopes: scopes!(...), ..Default::default() }`.
- Scope separator: **space**. `join_scopes` joins the `HashSet` with `" "` (`src/lib.rs`).
  The `scopes!` macro builds the set. Required scopes for this app (all confirmed present in
  https://developer.spotify.com/documentation/web-api/concepts/scopes):

```rust
use rspotify::scopes;
let scopes = scopes!(
    "playlist-read-private",
    "playlist-read-collaborative",
    "user-read-private",
    "user-read-email"
);
```

- Client construction:

```rust
let mut spotify = AuthCodePkceSpotify::new(creds, oauth);           // default Config
// or with Config { token_cached: false, token_refreshing: true, .. } via with_config / from_token_with_config
```

### 1.3 `get_authorize_url` — signature and verifier storage

Source: https://raw.githubusercontent.com/ramsayleung/rspotify/v0.16.1/src/auth_code_pkce.rs

```rust
pub fn get_authorize_url(&mut self, verifier_bytes: Option<usize>) -> ClientResult<String> {
    let scopes = join_scopes(&self.oauth.scopes);
    let verifier_bytes = verifier_bytes.unwrap_or(43);
    let (verifier, challenge) = Self::generate_codes(verifier_bytes);
    // The verifier will be needed later when requesting the token
    self.verifier = Some(verifier);
    // payload: client_id, response_type=code, redirect_uri,
    //          code_challenge_method=S256, code_challenge, state
    ...
}
```

- **The parameter is verifier length (`Option<usize>`, 43–128 bytes), NOT state.** State comes from
  `self.oauth.state` and is appended automatically. Pass `None` for the default 43-byte verifier.
- The verifier is stored in the **public field** `spotify.verifier: Option<String>`.
  It does **not** survive an app restart unless you persist it yourself (serde it into your settings
  file or in-memory state alongside `state` if you want resume-after-restart mid-flow).
- The returned URL is the full `https://accounts.spotify.com/authorize?...` URL — open it in the
  browser (§4).

### 1.4 `request_token` — automatic verifier, no secret, panic caveat

Source: https://raw.githubusercontent.com/ramsayleung/rspotify/v0.16.1/src/auth_code_pkce.rs (`impl OAuthClient for AuthCodePkceSpotify`)

```rust
/// Note that the code verifier must be set at this point, either manually
/// or with [`Self::get_authorize_url`]. Otherwise, this function will panic.
async fn request_token(&self, code: &str) -> ClientResult<()> {
    let verifier = self.verifier.as_ref().expect(
        "Unknown code verifier. Try calling `AuthCodePkceSpotify::get_authorize_url` first or setting it yourself.",
    );
    let mut data = Form::new();
    data.insert(params::CLIENT_ID, &self.creds.id);
    data.insert(params::GRANT_TYPE, params::GRANT_TYPE_AUTH_CODE);
    data.insert(params::CODE, code);
    data.insert(params::REDIRECT_URI, &self.oauth.redirect_uri);
    data.insert(params::CODE_VERIFIER, verifier);
    let token = self.fetch_access_token(&data, None).await?;
    // token_callback_fn invoked here (keyring hook — §2)
    if let Some(callback_fn) = &*self.get_config().token_callback_fn.clone() {
        callback_fn.0(token.clone())?;
    }
    *self.token.lock().await.unwrap() = Some(token);
    self.write_token_cache().await
}
```

- Verifier is included **automatically** from `self.verifier`.
- **Pitfall:** `request_token` **panics** if `get_authorize_url` was never called on this client
  instance in this process. Always call `get_authorize_url` before wiring the callback, and keep the
  same `AuthCodePkceSpotify` instance alive between URL generation and code exchange.
- Body contains only `client_id` (no secret) — confirmed PKCE/public-client behavior.
- `redirect_uri` in the token request must match the authorize request exactly (Spotify enforces this).

### 1.5 `prompt_for_token` — do NOT use in the Tauri app (or wrap carefully)

Source: https://raw.githubusercontent.com/ramsayleung/rspotify/v0.16.1/src/clients/oauth.rs

```rust
/// Note: this method requires the `cli` feature.
#[cfg(feature = "cli")]
#[maybe_async]
async fn prompt_for_token(&self, url: &str) -> ClientResult<()> {
    match self.read_token_cache(true).await { /* try cache first */ ... }
    // fallback:
    let code = self.get_code_from_user(url)?;   // blocks!
    self.request_token(&code).await?;
    self.write_token_cache().await
}
```

`get_code_from_user` (same file, `cli` feature):

```rust
match webbrowser::open(url) { ... }                       // opens system browser
match self.get_socket_address(&self.get_oauth().redirect_uri) {
    Some(addr) => self.get_authcode_listener(addr),       // loopback -> std TcpListener
    _ => { /* non-loopback: prompts user to paste URL via stdin */ }
}
```

`get_authcode_listener` (same file):

```rust
let listener = TcpListener::bind(socket_address).map_err(|e| ClientError::AuthCodeListenerBind { ... })?;
// The server will terminate itself after collecting the first code.
let mut stream = listener.incoming().flatten().next().ok_or(ClientError::AuthCodeTerminated)?;
// reads request line, builds redirect_full_url, parse_response_code validates `state`
```

Verified facts:

| Question | Answer (source-verified) |
|---|---|
| Local server? | Yes — plain **`std::net::TcpListener`** (not `tiny_http`). Binds the exact loopback `SocketAddr` parsed from the redirect URI (`get_socket_address` extracts host/port; for `http://127.0.0.1:8888/callback` → binds `127.0.0.1:8888`). |
| Blocks? | **Yes**, single blocking `accept()` with **no timeout**. If the user never completes login, it hangs forever. |
| Opens browser? | Yes, via **`webbrowser::open(url)`** (the `cli` feature pulls `webbrowser = "1.0"`). |
| Tokio-safe? | The listener is blocking std I/O. Calling `prompt_for_token(...).await` directly on Tauri's Tokio runtime will **block a runtime worker**. Must wrap in `tokio::task::spawn_blocking` (or `tauri::async_runtime::spawn_blocking`) and ideally a `tokio::time::timeout` around it. |
| Terminal I/O? | Yes — `println!`/`eprintln!`/stdin fallback. Unsuitable for a GUI app. |
| State validated? | Yes — `parse_response_code` checks `state` against `self.oauth.state`. |
| Path validated? | No — `parse_response_code` reads only the query string; the listener echoes any path. |

**Verdict:** skip the `cli` feature; implement the flow manually with `get_authorize_url` + our own
loopback listener (§5) for timeout control, a proper HTML response, and no terminal coupling.

### 1.6 Token access, persistence, auto-refresh

Source: `src/auth_code_pkce.rs`, `src/lib.rs`, `rspotify-model/src/auth.rs` (v0.16.1)

- Client field: `pub token: Arc<Mutex<Option<Token>>>` (rspotify's internal async `Mutex`) — public
  and readable/writable: `*spotify.token.lock().await.unwrap() = Some(token)`.
- `Token` is `#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]` with fields:
  `access_token: String`, `expires_in: Duration` (serde seconds), `expires_at: Option<DateTime<Utc>>`,
  `refresh_token: Option<String>`, `scopes: HashSet<String>`. It implements `write_cache(path)` /
  `from_cache(path)` and `is_expired()` (10-second safety margin).
- Restore at startup:

```rust
let token: Token = serde_json::from_str(&keyring_json)?;      // from keyring (§2)
let spotify = AuthCodePkceSpotify::from_token_with_config(
    token, creds, oauth,
    Config { token_cached: false, token_refreshing: true, ..Default::default() },
);
```

(`from_token(token)` also exists but uses default creds/oauth — prefer `from_token_with_config` so
refresh has the client_id and the redirect/scopes stay consistent.)
- **Auto-refresh:** `Config.token_refreshing` defaults to **`true`**. Every BaseClient request calls
  `auth_headers()` → `auto_reauth()`, which transparently refreshes an expired token via
  `refetch_token()`:

```rust
async fn refetch_token(&self) -> ClientResult<Option<Token>> {
    match self.token.lock().await.unwrap().as_ref() {
        Some(Token { refresh_token: Some(refresh_token), .. }) => {
            let mut data = Form::new();
            data.insert(params::GRANT_TYPE, params::GRANT_TYPE_REFRESH_TOKEN);
            data.insert(params::REFRESH_TOKEN, refresh_token);
            data.insert(params::CLIENT_ID, &self.creds.id);   // no secret for PKCE
            let token = self.fetch_access_token(&data, None).await?;
            if let Some(callback_fn) = &*self.get_config().token_callback_fn.clone() {
                callback_fn.0(token.clone())?;               // persist new token here
            }
            Ok(Some(token))
        }
        _ => Ok(None),
    }
}
```

  The refreshed token **replaces** the old one in memory (`auto_reauth` swaps it). A restored token
  keeps its `refresh_token` field — but see the rotation caveat below.
- **`Config.token_callback_fn: Arc<Option<TokenCallback>>`** where
  `TokenCallback(pub Box<dyn Fn(Token) -> Result<(), CallbackError> + Send + Sync>)` — invoked on
  **both** `request_token` and `refetch_token`. This is the cleanest hook to write the refreshed
  `Token` JSON into keyring after every refresh, with zero polling.
- **File cache:** `Config.token_cached` defaults to **`false`** (writes ignored); when enabled the
  default path is `DEFAULT_CACHE_PATH = ".spotify_token_cache.json"` — resolved **relative to CWD**,
  which under `tauri dev` is `src-tauri/`. Keep `token_cached: false`; keyring is the store.
- **PKCE refresh-token rotation (documented on the type):** *"the refresh token obtained with PKCE
  will only work to request the next one, after which it'll become invalid."* Persist the latest
  token (with its new refresh token) after **every** refresh — the `token_callback_fn` hook does this.
- **Refresh-token lifetime (Spotify, verified today):** *"Refresh tokens issued to apps registered in
  the Developer Dashboard have a lifetime of 6 months."* Refreshing does not extend it; after 6 months
  the user must re-authorize. Surface this as `Error`/`Disconnected` → re-login required.
  Source: https://developer.spotify.com/documentation/web-api/tutorials/refreshing-tokens

### 1.7 BaseClient availability & redirect URI matching

- `playlist()`, `playlist_items_manual()`, `album()`, `track()` (and the whole `BaseClient` surface)
  are implemented for `AuthCodePkceSpotify` via `impl BaseClient for AuthCodePkceSpotify` in
  `src/clients/base.rs` — the PKCE client is a full API client once a token exists. `OAuthClient`
  adds the user-authorization endpoints on top.
- **Redirect URI must exactly match the Spotify Dashboard entry** (except that loopback URIs may vary
  the port). Spotify rules (verified):
  - Use HTTPS unless loopback, where HTTP is permitted.
  - Loopback must be an explicit literal: `http://127.0.0.1:PORT` or `http://[::1]:PORT`.
  - **`localhost` is not allowed as a redirect URI.**
  Source: https://developer.spotify.com/documentation/web-api/concepts/redirect_uri
- The chosen `http://127.0.0.1:8888/callback` is **correct**: register that exact string in the
  Dashboard (with port 8888), and rspotify's listener will bind `127.0.0.1:8888`.

---

## 2. Token storage — `keyring` crate

### 2.1 Version & Windows backend (verified)

| Field | Value | Source |
|---|---|---|
| Latest version | **4.2.0** (updated 2026-08-29) | crates.io API |
| Default features | `v1`, `windows-native-keyring-store` (+ macOS/Linux stores) | docs.rs features page |
| Windows store | **Windows Credential Manager** via `windows-native-keyring-store` | `src/v1.rs` `set_credential_store` |
| Repo | https://github.com/open-source-cooperative/keyring-rs | crates.io |

`keyring 4.x` wraps `keyring-core`; the `v1` feature re-exports the classic API. Default features are
enough on Windows — no feature flags to fiddle with:

```toml
keyring = "4.2.0"
```

### 2.2 Exact API (quoted from v4.2.0 `src/v1.rs`)

```rust
pub struct Entry { pub inner: keyring_core::Entry }

impl Entry {
    /// Create a new entry in the platform-specific credential store.
    pub fn new(service: &str, username: &str) -> Result<Self>;

    pub fn set_password(&self, password: &str) -> Result<()>;
    pub fn get_password(&self) -> Result<String>;
    pub fn delete_credential(&self) -> Result<()>;
    // also: set_secret(&[u8]) / get_secret() -> Result<Vec<u8>>
}
```

- Missing entry error: **`keyring::Error::NoEntry`** (from `keyring-core` `src/error.rs`:
  *"This indicates that there is no underlying credential entry in the platform for this entry."*).
  Match it to distinguish "never logged in" from real failures:

```rust
match entry.get_password() {
    Ok(json) => Some(json),
    Err(keyring::Error::NoEntry) => None,
    Err(e) => return Err(e),
}
```

- Use e.g. `service = "spotify_downloader"`, `username = "spotify_token"` (or the client_id if you
  plan multi-account later).

### 2.3 Size limits & payload choice

- Windows credential blob limit: **`CRED_MAX_CREDENTIAL_BLOB_SIZE (5*512)` = 2560 bytes**
  (verified in Microsoft `wincred.h`: https://raw.githubusercontent.com/microsoft/win32metadata/main/generation/WinSDK/RecompiledIdlHeaders/um/wincred.h).
- rspotify `Token` JSON (`access_token` ~300 chars + `refresh_token` ~180 chars + expiry + scopes)
  is roughly **400–700 bytes** — comfortably under the limit.
- **Recommendation: store the full serialized `Token`** (`serde_json::to_string(&token)`), not just
  the refresh token. Reasons: (a) instant restore at startup with no network round-trip;
  (b) `from_token_with_config` restores scopes/expiry so `auto_reauth` behaves correctly;
  (c) lets the UI show token expiry / "re-login in N days". No encryption layer needed — the
  Credential Manager store is the OS-protected vault; do not over-engineer a local encrypted file.
- Thread-safety: `Entry` is a cheap handle over the platform store and the calls are fast/synchronous.
  No async wrapper needed; if you want to be extra cautious about blocking the runtime, run
  set/get in `spawn_blocking` (keyring calls are typically sub-millisecond on Windows — not required).

---

## 3. Tauri static frontend (`tauri dev` with no dev server)

### 3.1 Is `devUrl` optional? YES — verified in Tauri 2.12.5 source

Source: https://raw.githubusercontent.com/tauri-apps/tauri/tauri-v2.12.5/crates/tauri-utils/src/config.rs

```rust
pub struct BuildConfig {
    /// The URL to load in development.
    /// If you don't have a dev server or don't want to use one, ignore this option and use
    /// [`frontendDist`](BuildConfig::frontend_dist) and point to a web assets directory, and
    /// Tauri CLI will run its built-in dev server and provide a simple hot-reload experience.
    pub dev_url: Option<Url>,
    /// The path to the application assets ... When a path relative to the configuration file is
    /// provided, it is read recursively and all files are embedded in the application binary.
    pub frontend_dist: Option<FrontendDist>,
    pub before_dev_command: Option<BeforeDevCommand>,
    ...
}
```

- **`tauri dev` works with only `build.frontendDist: "../src"` — no `devUrl`, no `beforeDevCommand`,
  no Vite, no npm bundler.** The Tauri CLI serves the static directory itself with a simple
  hot-reload watcher. `frontendDist` is also what gets embedded at `tauri build` time.
- Minimal `src-tauri/tauri.conf.json` build section:

```json
{
  "build": {
    "frontendDist": "../src"
  },
  "app": {
    "withGlobalTauri": true
  }
}
```

(`devUrl` omitted on purpose. `beforeDevCommand` omitted — no JS toolchain.)

### 3.2 `withGlobalTauri` — what it exposes (verified in `bundle.global.js`)

Source: https://raw.githubusercontent.com/tauri-apps/tauri/tauri-v2.12.5/crates/tauri/scripts/bundle.global.js

The injected IIFE ends with:

```js
... return e.app = k, e.core = g, e.dpi = N, e.event = F, e.image = f, e.menu = K,
    e.mocks = ee, e.path = ne, e.tray = se, e.webview = Ae, e.webviewWindow = Re,
    e.window = ye, e}({}); window.__TAURI__ = __TAURI_IIFE__;
```

Official docs confirm the JS usage (https://v2.tauri.app/develop/calling-rust/):

```js
// When using the Tauri global script (if not using the npm package)
// Be sure to set `app.withGlobalTauri` in `tauri.conf.json` to true
const invoke = window.__TAURI__.core.invoke;
invoke('my_custom_command');
```

- **`window.__TAURI__.core.invoke` ✔ and `window.__TAURI__.event.listen` ✔** (modules `core`, `event`,
  `path`, `window`, … are all present on the global). No `@tauri-apps/api` npm import needed.
- Capabilities: with `withGlobalTauri` you still need **capabilities only for JS-invoked plugin
  commands** (e.g. `core:path:default` if you call `path` from JS). Plain `invoke` of your own Rust
  commands needs the default `core:default` capability. CSP: no extra CSP entries needed for
  `invoke`/`listen`; keep the default CSP unless you load remote content.

---

## 4. Opening the authorize URL from Rust

| Option | API (verified) | Notes |
|---|---|---|
| **`webbrowser` crate 1.2.5** (crates.io, updated 2026-10-22) | `webbrowser::open(url)?` | Pure Rust, no Tauri coupling, no registration. This is exactly what rspotify's `get_code_from_user` uses. On Windows it uses the shell `start`/ShellExecute — reliable for `https://accounts.spotify.com/...`. |
| **`tauri-plugin-opener` 2.7.2** | Rust: `use tauri_plugin_opener::OpenerExt; app.opener().open_url(url, None)?;` (also free fn `tauri_plugin_opener::open_url(url, with)`) | Plugin must be registered (`.plugin(tauri_plugin_opener::init())`). **Scope checks apply only to the IPC command** (`plugin:opener|open_url` — `commands.rs` builds a `Scope` and checks `is_url_allowed`); the docs in `lib.rs` state: *"The [`open_url`] and [`open_path`] free functions and the [`Opener`] methods themselves do not perform any scope check."* So a Rust-side call needs **no capability/permission**. |

**Recommendation:** use **`webbrowser`** for the auth URL — zero config, matches rspotify's own flow,
and the authorize URL contains the PKCE challenge (public data) so opener's URL-scope regex is
irrelevant anyway. Keep/`tauri-plugin-opener` only if you already use it for "reveal file in
explorer" (its `reveal_item_in_dir`). Either is reliable on Windows; opener's Rust path adds a
plugin dependency for no functional gain here.

---

## 5. Manual loopback callback server (`GET /callback?code=&state=`)

### 5.1 `tiny_http` status

| Field | Value |
|---|---|
| Latest version | **0.12.0** (last release 2022-10-06 — stale but stable; fine for a loopback HTTP listener, no TLS needed) |
| Repo | https://github.com/tiny-http/tiny-http |

Minimal pattern (blocking — run on a dedicated thread or `spawn_blocking`):

```rust
use tiny_http::{Server, Response};
let server = Server::http("127.0.0.1:8888").expect("bind failed — port in use?");
for request in server.incoming_requests() {          // single-accept then drop(server)
    let url = request.url().to_string();             // "/callback?code=...&state=..."
    // parse code/state, verify state, then request_token(code)
    request.respond(Response::from_string(
        "<html><body><h3>Login complete — you can close this tab.</h3></body></html>")
        .with_header("Content-Type: text/html"))?;
    break;                                            // one code then close
}
```

Alternatively a 15-line `std::net::TcpListener` (what rspotify itself uses, §1.5) avoids the
dependency entirely: `bind("127.0.0.1:8888")`, `set_read_timeout(Some(Duration::from_secs(...)))`,
read the request line, parse the query, write a canned HTTP/1.1 200 response. Either is fine;
`tiny_http` is less brittle around keep-alive/partial reads.

### 5.2 Pitfalls

- **Firewall prompt:** loopback (`127.0.0.1`) connections do not traverse the network interface the
  way LAN binds do — Windows does not typically show a firewall prompt for a loopback-only listener.
  (A prompt would only appear for `0.0.0.0`/LAN binds.)
- **Port already in use:** `Server::http` / `TcpListener::bind` returns an error if 8888 is taken.
  Map it to the `Error` auth state with a user-readable message ("port 8888 is in use — close the
  other app or change the port"). The redirect URI port must match the Dashboard entry, so if you
  make the port configurable you must also update the Dashboard entry (loopback URIs may vary the
  port per Spotify's rules, but the Dashboard entry still has to list a valid loopback URI).
- **IPv4 vs IPv6:** bind **IPv4 `127.0.0.1`** explicitly because the fixed redirect URI is
  `http://127.0.0.1:8888/...`. Do not bind `localhost` (resolves to `::1` or `127.0.0.1`
  nondeterministically, and Spotify forbids `localhost` redirect URIs anyway).
- **Timeout/cancellation:** rspotify's built-in listener has **none**. Our manual server must set a
  read/accept timeout (e.g. 5 minutes) and also honor a Tauri command `login_cancel` that drops the
  listener (wrap the accept loop so a `CancellationToken`/`AtomicBool` + short poll interval breaks
  the loop). On timeout/cancel → auth state back to `Disconnected`/`Error("timed out")`.
- **One code then exit:** Spotify sends the browser to the redirect exactly once per login; accept one
  request, respond with the friendly HTML page, then shut the listener down so the port is free for
  the next login.

### 5.3 Exact flow & state verification

1. `let url = spotify.get_authorize_url(None)?;` — this stores the PKCE verifier in `spotify.verifier`.
2. Persist a copy of `{ state: spotify.oauth.state, verifier: spotify.verifier }` if you want
   restart-resume mid-flow (optional).
3. `webbrowser::open(&url)?;`
4. Start the loopback listener on `127.0.0.1:8888` (in `spawn_blocking` / dedicated thread).
5. Parse `?code=` and `?state=` from the query. **Verify `state == spotify.oauth.state`** before
   proceeding (reject mismatches → `Error`). (rspotify's `parse_response_code` does this too if you
   use its helper.)
6. `spotify.request_token(&code).await?;` — verifier is already on the instance; token lands in
   `spotify.token` and your `token_callback_fn` writes it to keyring.
7. Emit `auth-status-changed` with `Authenticated`; shut the listener down.

**Recommendation:** manual listener (this section) over `prompt_for_token` — control over timeout,
HTML response, cancellation, and no terminal I/O. `prompt_for_token` remains a valid quick
alternative **only** if wrapped in `tauri::async_runtime::spawn_blocking` + `tokio::time::timeout`
and the `cli` feature is enabled (accepting its stdout prints).

---

## 6. Settings / env precedence & storage

### 6.1 Precedence

**User-saved Client ID/Secret (settings file) > `.env` > process env.** Rationale: the app is
distributed to end users who will paste their own Spotify Developer Dashboard credentials; `.env` is
a dev convenience; real env vars are the CI/system escape hatch.

Load order sketch:

```rust
// 1. settings file (authoritative if present)
let settings_path = app.path().app_config_dir()?.join("settings.json");
// 2. else .env (dev): dotenvy
// 3. else std::env::var("RSPOTIFY_CLIENT_ID")
```

### 6.2 Loading `.env` with `dotenvy` when CWD is `src-tauri`

- Latest: **dotenvy 0.15.7** (crates.io). `dotenvy::dotenv()` loads `.env` from the **current
  working directory only** (no upward search). Under `tauri dev`/`tauri build` the Rust binary's CWD
  is **`src-tauri/`**, so a project-root `.env` is *not* found by bare `dotenv()`.
- Fix: put `.env` in `src-tauri/` for dev, **or** use `dotenvy::from_path(path)` with an explicit
  path (e.g. resolve relative to `CARGO_MANIFEST_DIR` baked in at compile time, or search upward).
  Simplest reliable dev setup: `src-tauri/.env` + `dotenvy::dotenv().ok();` early in `run()`.
- Keys: `RSPOTIFY_CLIENT_ID` (and optionally `RSPOTIFY_CLIENT_SECRET` — unused for PKCE).

### 6.3 Where things live

| Data | Location | Why |
|---|---|---|
| Client ID / (optional) secret / output prefs | `app.path().app_config_dir()` → `settings.json` (`%APPDATA%\<identifier>\settings.json` on Windows) | Per-user, roaming, Tauri-sanctioned. |
| Access+refresh `Token` JSON | **keyring** (Credential Manager), service `spotify_downloader` | OS-protected at rest; survives CWD changes; §2. |
| PKCE verifier + state | In-memory only (optionally mirrored to settings for mid-flow resume) | Short-lived; single-use. |
| rspotify file cache | **Disabled** (`token_cached: false`) | Avoids plaintext `.spotify_token_cache.json` in CWD. |

### 6.4 Security notes

- **PKCE = public client:** no client secret is required or used in the token exchange. If the user
  pastes a secret anyway, never log it, never send it to the frontend, and don't store it in keyring
  unless you switch to the confidential flow (not needed).
- The **refresh token is the long-lived credential** (6-month Spotify lifetime for Dashboard apps).
  It lives only in keyring; never write it to logs, settings.json, or events sent to the frontend.
- Tell users: (1) they must create their own Spotify app in the Developer Dashboard and add
  `http://127.0.0.1:8888/callback` as a redirect URI; (2) login expires after ~6 months and they'll
  be asked to re-authorize; (3) the app only requests read scopes (playlists/profile).
- Frontend events should carry status enums/booleans, not token material.

---

## 7. Recommended auth state machine & commands

States: **`Disconnected` → `WaitingForBrowser` → `Authenticated`**, with **`Error(message)`** reachable
from any non-`Authenticated` state (timeout, port-in-use, state mismatch, refresh failure,
6-month expiry).

```
Disconnected --[login_start]--> WaitingForBrowser --[callback ok + request_token ok]--> Authenticated
     ^                              |        |
     |                        [login_cancel] [timeout / state mismatch / bind error]
     |                              v        v
     +--------------------------- Error <----+
Authenticated --[refresh fails w/ invalid_grant]--> Error/Disconnected (require re-login)
Authenticated --[logout]--> Disconnected  (keyring delete_credential + drop token)
```

Suggested Tauri commands (all `async`, return `Result<AuthStatusDto, String>`; emit
`auth-status-changed` on every transition):

| Command | Behavior |
|---|---|
| `get_auth_status` | On startup / on demand: try keyring restore → `Authenticated` (with expiry) or `Disconnected`. |
| `login_start` | Build client (`get_authorize_url`), open browser, start loopback listener with timeout; sets `WaitingForBrowser`. |
| `login_cancel` | Signal the listener to stop; back to `Disconnected`. |
| `logout` | `keyring delete_credential`; drop in-memory token; emit `Disconnected`. |

`AuthStatusDto`: `{ state: "disconnected"|"waiting"|"authenticated"|"error", message?: string, expires_at?: string }`.
The verifier/state live in a `tokio::sync::Mutex<AuthFlowState>` behind Tauri managed state so
`login_start`/the listener task/`login_cancel` share them.

---

## 8. Key findings & blockers (report)

1. **`AuthCodePkceSpotify` is a full fit.** `Credentials::new_pkce(id)` (secret optional/None),
   `OAuth { redirect_uri, state (auto-random), scopes, proxies }`, scopes joined with a **space**.
   `get_authorize_url(&mut self, verifier_bytes: Option<usize>)` — param is verifier length, not state;
   verifier stored in the public `verifier` field. `request_token(&code)` auto-embeds the verifier but
   **panics if `get_authorize_url` wasn't called** on the same instance.
2. **Skip rspotify's `cli` feature / `prompt_for_token`.** It opens the browser via `webbrowser`,
   then blocks forever on a bare `std::net::TcpListener` at the redirect loopback (`127.0.0.1:8888`
   ✔ binds correctly) with no timeout and terminal I/O — wrong for a GUI/Tokio app. Implement the
   manual loopback server (§5) with a timeout and `login_cancel`.
3. **Token handling is clean.** `token: Arc<Mutex<Option<Token>>>` is public; `Token` is
   Serialize/Deserialize; restore via `from_token_with_config`; auto-refresh is on by default
   (`token_refreshing: true`) and the **`Config.token_callback_fn`** hook fires on every
   request/refresh — use it to persist to keyring. Keep `token_cached: false` (default cache path is
   CWD-relative `.spotify_token_cache.json`).
4. **keyring 4.2.0** (Windows Credential Manager, default features) with
   `Entry::new/set_password/get_password/delete_credential` and `Error::NoEntry`. Windows blob limit
   is 2560 bytes; a full `Token` JSON (~400–700 bytes) fits — store the **full token**, not just the
   refresh token.
5. **Spotify constraints confirmed:** refresh tokens for Dashboard apps last **6 months** and PKCE
   refresh tokens rotate (must persist the newest after every refresh); redirect URI must exactly
   match the Dashboard; loopback must be literal `127.0.0.1`/`[::1]` (HTTP allowed);
   **`localhost` is not allowed** — `http://127.0.0.1:8888/callback` is the correct choice.
6. **Tauri static hosting is supported:** official 2.12.5 source says to omit `devUrl` and use
   `frontendDist` — the CLI runs its own dev server with hot reload. `withGlobalTauri: true` injects
   `window.__TAURI__` exposing `core.invoke` and `event.listen` (verified in `bundle.global.js`) — no
   npm `@tauri-apps/api` needed.
7. **Browser open:** `webbrowser::open` (pure Rust, what rspotify uses) recommended over
   `tauri-plugin-opener` for the auth URL; opener's Rust-side `app.opener().open_url` skips scope
   checks anyway, but adds a plugin dependency for no gain here.
8. **Settings precedence:** settings.json (in `app_config_dir()`) > `.env` (dotenvy; note CWD is
   `src-tauri` under `tauri dev`, so put `.env` there or use `from_path`) > process env. Refresh token
   in keyring only; never log secrets.

**Blockers: none.** All six research areas verified with source-level quotes. The only caveats to
carry into implementation: (a) `request_token` panics without a prior `get_authorize_url` on the same
instance; (b) persist the rotated refresh token after every refresh via `token_callback_fn`;
(c) `tiny_http` is stale (2022) but fine for loopback — a raw `TcpListener` is an equally valid
zero-dependency choice; (d) Spotify's 6-month refresh-token lifetime must be surfaced as a
re-login flow in the UI.
