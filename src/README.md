# Frontend - d3khan song saver

Vanilla HTML/CSS/JS frontend for the Tauri v2 desktop app. No frameworks, no
bundler, no npm, no CDN or web-font fetches — system fonts only.

## Files

| File | Purpose |
|---|---|
| `index.html` | App shell: header + status pill/account chip/gear, auth banner, job setup panel, progress/log panel, settings + both prompt modals, toast region |
| `styles.css` | Design tokens (CSS custom properties) and all styling; dark theme, 8px rhythm |
| `app.js` | State, Tauri bridge, render functions, event wiring, modal controllers, auth/settings controllers, mock engine |

## Preview in a browser (no build step)

Open `index.html` directly (double-click or drag into a browser). When
`window.__TAURI__` is absent, a **mock engine** answers the same commands as the
Rust backend (`pick_folder`, `start_job`, `cancel_job`, `confirm_deletions`,
`resolve_conflict`, and the `auth_*` commands) with scripted progress, log,
conflict, removal, and auth events. There is no preview badge or debug strip:
the mock is a quiet development aid for layout work, and the shipped UI looks
identical in both modes.

A sync job in preview exercises the full flow: resolve → conflict prompt →
per-track progress → removal prompt → summary. The mock starts connected so the
Start button is reachable; use **Log out** in Settings to see the disconnected
banner.

## Backend contract assumptions (fixed)

Commands (args camelCase; Tauri v2 maps them to snake_case Rust params):

- `pick_folder()` → `string | null`
- `start_job({ mode: "download"|"sync", inputMode: "single"|"bulk", url, urls, outputDir })` → `jobId`
- `cancel_job({ jobId })`
- `confirm_deletions({ requestId, decision: "delete"|"keep" })`
- `resolve_conflict({ requestId, choice: "syncHere"|"createNew"|"cancel", dir? })` — `dir` is **optional**:
  which existing folder to sync into when several matches are offered
- `auth_status()` → `{ authenticated, configured, displayName, redirectUri, error }`
- `auth_connect()` → `AuthStatus` (long-running; opens the system browser and
  waits for the `http://127.0.0.1:8888/callback` round-trip, up to ~3 min)
- `auth_cancel()` → `AuthStatus` (stops the loopback wait; returns the
  disconnected/cancelled status, and makes a pending `auth_connect` reject)
- `auth_logout()` → `AuthStatus`

There are no settings commands and no credential fields: the Client ID is
compiled into the binary by `build.rs`, so the account modal only shows the
signed-in name with **Log out** and **Close**.

Events: `download://progress`, `download://log`, `download://track_failed`,
`sync://removed_tracks`, `sync://conflict`, `sync://complete`,
`auth://status` `{ authenticated, configured, displayName, error }` — payloads
exactly as specified in the phase briefs (camelCase; `percent: null` =
indeterminate).

Contract notes:

1. **`resolve_conflict` optional `dir`.** When several folders match, the choice
   `syncHere` alone doesn't say *which* one. The frontend passes an additional
   `dir` argument (the selected match's path). It is additive: a backend that
   only declares `(request_id, choice)` ignores the extra key, and the frontend
   then falls back to whatever the backend picks.
2. **Deletion prompt has only Keep/Delete** (per the brief). "Keep files"
   answers `decision: "keep"`; there is no third "stop syncing" option here.
3. **Timeouts** (`timeoutMs` in both prompt payloads) run client-side: the
   deletion modal closes with the toast *"No response — files were kept."*, the
   conflict modal closes as cancel with *"No response — the job was cancelled."*
   The backend auto-keeps / auto-cancels on its own timer regardless.
4. **Escape on the deletion modal does nothing** (only an explicit button
   resolves it); Escape on the conflict modal resolves as *cancel*; Escape on
   Settings closes it (but is ignored while a sign-in or logout is in flight).
5. **Invalid-URL handling is warning-only.** Links that don't look like
   `open.spotify.com/{track,album,playlist}` show an inline note but never
   block Start, because the backend may resolve other sources.
6. **Bulk mode** splits on newlines/whitespace/commas and de-dupes; Sync is
   disabled in bulk mode (it runs on a single playlist) and the UI falls back
   to Download with an explanation.
7. **Start is gated on auth.** Start stays disabled until Spotify is
   authenticated (plus the usual URL + folder requirements); the banner explains
   what is missing.
8. **The account modal has no credential editing.** It shows the signed-in
   display name (when authenticated) plus **Log out** and **Close**, nothing
   else.
9. **`auth_cancel` stops the wait.** The waiting UI's Cancel invokes
   `auth_cancel`, applies the returned status, and toasts *"Spotify sign-in
   cancelled."*; the rejection of a cancelled `auth_connect` is suppressed (not
   shown as an error). The 3-minute UI timeout remains as a fallback and also
   calls `auth_cancel` (best-effort) before showing the timeout message.

## Accessibility & motion

Labels and `aria-describedby` on inputs, `role="dialog" aria-modal` with focus
trap + focus restore on all three modals (Settings scrolls internally down to
720×600), `aria-live` on progress, log, and the auth banner,
`role="radiogroup"` for mode cards and conflict matches, `:focus-visible`
rings throughout, and full `prefers-reduced-motion` support (indeterminate bar
becomes a static striped track, shimmer/spin/slide/pulse animations disabled).
