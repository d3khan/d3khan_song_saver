# Frontend - d3khan song saver

Vanilla HTML/CSS/JS frontend for the Tauri v2 desktop app. No frameworks, no
bundler, no npm, no CDN or web-font fetches — system fonts only.

## Files

| File | Purpose |
|---|---|
| `index.html` | App shell: search bar, matching popup, progress bar, and activity log |
| `styles.css` | Design tokens (CSS custom properties) and all styling; dark theme |
| `app.js` | State, Tauri bridge, render functions, event wiring |

## Preview in a browser (no build step)

Open `index.html` directly (double-click or drag into a browser). When
`window.__TAURI__` is absent, a **mock engine** answers the same commands as the
Rust backend (`pick_folder`, `resolve_input`, `download_track`) with scripted
progress, log, and result events. The mock is a quiet development aid for layout
work, and the shipped UI looks identical in both modes.

## Backend contract

Commands (args camelCase; Tauri v2 maps them to snake_case Rust params):

- `pick_folder()` → `string | null`
- `resolve_input({ input })` → `[{ id, title, artist, cover_url, year, duration_ms }]`
- `download_track({ tracks, outputDir })` → `void`

Events emitted by the backend:

- `download-progress` — `{ percent, status }`
- `download-log` — `{ message }`
- `download-error` — `{ error }`

## Accessibility & motion

Labels and `aria-describedby` on inputs, `aria-live` on progress and log,
`:focus-visible` rings throughout, and full `prefers-reduced-motion` support
(indeterminate bar becomes a static striped track, shimmer/spin/slide/pulse
animations disabled).
