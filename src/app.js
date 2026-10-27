/* ==========================================================================
   d3khan song saver - frontend controller
   Vanilla JS. Talks to the Tauri v2 backend through window.__TAURI__ when
   present (withGlobalTauri: true); otherwise runs a mock engine so the whole
   UI — including both modals — can be inspected in a plain browser.
   ========================================================================== */
(function () {
  "use strict";

  /* ======================================================================
     1. State
     ====================================================================== */
  const state = {
    isTauri: Boolean(window.__TAURI__ && window.__TAURI__.core && window.__TAURI__.event),
    folder: null,                       // chosen output directory
    inputMode: "single",                // "single" | "bulk"
    mode: "sync",                       // "download" | "sync"
    running: false,
    jobId: null,
    cancelling: false,
    autoScroll: true,
    failedTracks: [],                   // from download://track_failed
    lastFocus: null,                    // element to restore focus to on modal close
    auth: {                             // mirrors auth_status / auth://status
      authenticated: false,
      configured: false,
      displayName: null,
      error: null,
      waiting: false,                   // browser round-trip in progress (UI-side only)
      waitStartedAt: null,
      waitTick: null,                   // elapsed-timer interval
      waitTimeout: null,                // 3-minute UI timeout (fallback)
      cancelRequested: false,           // user asked to cancel — suppress connect rejection
      cancelInFlight: false,            // auth_cancel invoke in flight
    },
  };

  const LOG_ICONS = { info: "i", warn: "!", error: "×", success: "✓" };
  const LOG_MAX_LINES = 400;
  const AUTH_WAIT_UI_TIMEOUT = 180000;  // 3 minutes, mirrors the backend wait

  /* ======================================================================
     2. Small utilities
     ====================================================================== */
  const $ = (id) => document.getElementById(id);

  function el(tag, className, text) {
    const node = document.createElement(tag);
    if (className) node.className = className;
    if (text != null) node.textContent = text;
    return node;
  }

  function truncateMiddle(str, max) {
    max = max || 52;
    if (!str || str.length <= max) return str || "";
    const keep = max - 1;
    const head = Math.ceil(keep * 0.42);
    const tail = Math.floor(keep * 0.58);
    return str.slice(0, head) + "…" + str.slice(str.length - tail);
  }

  function formatClock(ms) {
    const total = Math.max(0, Math.round(ms / 1000));
    const m = Math.floor(total / 60);
    const s = total % 60;
    return m + ":" + String(s).padStart(2, "0");
  }

  function formatTimestamp(ts) {
    if (!ts) return "";
    const d = new Date(ts);
    if (Number.isNaN(d.getTime())) return String(ts);
    return d.toTimeString().slice(0, 8);
  }

  function formatWhen(iso) {
    if (!iso) return "never synced";
    const d = new Date(iso);
    if (Number.isNaN(d.getTime())) return String(iso);
    const date = d.toISOString().slice(0, 10);
    const time = d.toTimeString().slice(0, 5);
    return "last synced " + date + " " + time;
  }

  function prettyPhase(phase) {
    if (!phase) return "Working";
    const spaced = String(phase).replace(/[_-]+/g, " ").replace(/([a-z0-9])([A-Z])/g, "$1 $2");
    return spaced.charAt(0).toUpperCase() + spaced.slice(1);
  }

  /* ======================================================================
     3. DOM references
     ====================================================================== */
  const dom = {};
  [
    "status-pill", "status-text", "account-chip", "account-name", "settings-btn",
    "auth-banner", "auth-banner-title", "auth-banner-detail", "auth-actions",
    "auth-connect-btn", "auth-wait", "auth-wait-timer", "auth-wait-cancel",
    "browse-btn", "folder-placeholder", "folder-value",
    "tab-single", "tab-bulk", "panel-single", "panel-bulk", "url-input", "url-note", "urls-input", "urls-note",
    "mode-download", "mode-sync", "what-happens", "what-happens-text",
    "start-btn", "cancel-btn",
    "phase-chip", "progress-counts", "progress-bar", "bar-fill", "progress-title",
    "summary-card", "summary-status", "summary-message", "summary-stats", "summary-failures",
    "log-block", "log-collapse-btn", "log-clear-btn", "log-body", "log-empty", "log-lines",
    "deletion-layer", "deletion-modal", "deletion-desc", "deletion-countdown", "deletion-list",
    "deletion-keep", "deletion-delete",
    "conflict-layer", "conflict-modal", "conflict-desc", "conflict-countdown", "conflict-matches",
    "conflict-sync", "conflict-create", "conflict-cancel",
    "settings-layer", "settings-account-dot", "settings-account-name", "settings-account-note",
    "settings-connect", "settings-logout", "settings-close",
    "toast-region",
  ].forEach((id) => { dom[id] = $(id); });

  /* ======================================================================
     4. Tauri bridge (real or mock)
     ====================================================================== */
  const bridge = {
    async invoke(cmd, args) {
      if (state.isTauri) return window.__TAURI__.core.invoke(cmd, args || {});
      return mock.invoke(cmd, args || {});
    },
    listen(eventName, handler) {
      if (state.isTauri) {
        const p = window.__TAURI__.event.listen(eventName, (event) => handler(event.payload));
        if (p && typeof p.catch === "function") p.catch(() => {});   // never throw unhandled
      } else {
        mock.listen(eventName, handler);
      }
    },
  };

  /* ======================================================================
     5. Mock engine (browser preview only)
     ====================================================================== */
  const mock = {
    handlers: {},
    timers: [],
    seq: 0,
    pending: {},               // requestId -> { resolve, kind }
    folders: [
      "C:\\Users\\You\\Music\\Spotify",
      "D:\\Audio\\Library\\Master",
    ],
    folderIdx: -1,
    auth: { authenticated: false, configured: true, displayName: null, error: null },
    authTimer: null,           // simulated browser round-trip (survives clearTimers)
    authConnectReject: null,   // pending auth_connect rejection, settled by auth_cancel

    listen(name, handler) {
      (this.handlers[name] = this.handlers[name] || []).push(handler);
    },
    emit(name, payload) {
      (this.handlers[name] || []).forEach((h) => h(payload));
    },
    later(fn, ms) {
      const t = setTimeout(() => {
        this.timers = this.timers.filter((x) => x !== t);
        fn();
      }, ms);
      this.timers.push(t);
      return t;
    },
    clearTimers() {
      this.timers.forEach(clearTimeout);
      this.timers = [];
    },
    nextId(prefix) {
      this.seq += 1;
      return prefix + "-mock-" + this.seq;
    },
    wait(requestId, kind, timeoutMs, onTimeout) {
      return new Promise((resolve) => {
        this.pending[requestId] = { resolve, kind };
        this.later(() => {
          if (this.pending[requestId]) {
            delete this.pending[requestId];
            onTimeout();
            resolve({ __timeout: true });
          }
        }, timeoutMs);
      });
    },
    settle(requestId, kind, value) {
      const entry = this.pending[requestId];
      if (!entry || entry.kind !== kind) return false;
      delete this.pending[requestId];
      entry.resolve(value);
      return true;
    },

    async invoke(cmd, args) {
      switch (cmd) {
        case "pick_folder": {
          this.folderIdx = (this.folderIdx + 1) % this.folders.length;
          return this.folders[this.folderIdx];
        }
        case "start_job": {
          const jobId = this.nextId("job");
          runMockScenario(jobId, args);
          return jobId;
        }
        case "cancel_job": {
          this.clearTimers();
          Object.keys(this.pending).forEach((k) => delete this.pending[k]);
          emitComplete({
            jobId: args && args.jobId ? args.jobId : "job-mock-0",
            status: "cancelled",
            downloaded: 0, skipped: 0, failed: 0, removed: 0,
            message: "Job cancelled.",
          });
          return null;
        }
        case "confirm_deletions": {
          const ok = this.settle(args.requestId, "deletion", args.decision);
          if (!ok) throw new Error("request-already-resolved");
          return null;
        }
        case "resolve_conflict": {
          const ok = this.settle(args.requestId, "conflict", { choice: args.choice, dir: args.dir || null });
          if (!ok) throw new Error("request-already-resolved");
          return null;
        }
        case "auth_status": {
          return Object.assign({}, this.auth);
        }
        case "auth_connect": {
          // simulates opening the system browser and waiting for the callback
          return new Promise((resolve, reject) => {
            clearTimeout(this.authTimer);
            this.authConnectReject = reject;
            this.authTimer = setTimeout(() => {
              mock.authConnectReject = null;
              mock.setAuth({ authenticated: true, configured: true, displayName: "Preview Listener", error: null });
              resolve(Object.assign({}, mock.auth));
            }, 4500);
          });
        }
        case "auth_cancel": {
          // stops the loopback wait: auth_connect rejects, status returns disconnected
          clearTimeout(this.authTimer);
          if (this.authConnectReject) {
            const reject = this.authConnectReject;
            this.authConnectReject = null;
            reject(new Error("Authorization cancelled."));
          }
          this.setAuth({ authenticated: false, displayName: null, error: null });
          return Object.assign({}, this.auth);
        }
        case "auth_logout": {
          clearTimeout(this.authTimer);
          this.setAuth({ authenticated: false, displayName: null, error: null });
          return Object.assign({}, this.auth);
        }
        default:
          throw new Error("Unknown command in mock mode: " + cmd);
      }
    },

    /* scripted scenario helpers */
    armDeletion(count) {
      const requestId = this.nextId("req");
      const names = [
        ["Get Lucky", "Daft Punk", "03 - Daft Punk - Get Lucky.mp3"],
        ["Instant Crush", "Daft Punk", "05 - Daft Punk - Instant Crush.mp3"],
        ["Contact", "Daft Punk", "13 - Daft Punk - Contact.mp3"],
      ];
      return {
        jobId: "job-mock-demo", requestId,
        folder: "C:\\Users\\You\\Music\\Spotify\\Chill Mix 2026",
        playlistName: "Chill Mix 2026",
        tracks: names.slice(0, count || 3).map((n) => ({
          trackId: "t" + n[2], title: n[0], artist: n[1], localFilename: n[2],
        })),
        timeoutMs: 120000,
      };
    },
    armConflict(matchCount) {
      const requestId = this.nextId("req");
      const matches = [
        { dir: "C:\\Users\\You\\Music\\Spotify\\Chill Mix 2026", playlistName: "Chill Mix 2026", trackCount: 47, updatedAt: "2026-10-24T18:20:00Z" },
        { dir: "D:\\Backup\\Old Name (renamed by you)", playlistName: "Chill Mix 2026", trackCount: 31, updatedAt: "2026-09-01T10:00:00Z" },
        { dir: "E:\\Portable\\Chill Mix", playlistName: "Chill Mix", trackCount: 12, updatedAt: "2026-06-14T09:12:00Z" },
      ];
      return {
        jobId: "job-mock-demo", requestId,
        playlistName: "Chill Mix 2026",
        playlistId: "37i9dQZF1DXcBWIGoYBM5M",
        targetDir: "C:\\Users\\You\\Music\\Spotify\\Chill Mix 2026 (new)",
        matches: matches.slice(0, matchCount || 1),
        timeoutMs: 120000,
      };
    },
    /* auth state helper (also used by the scripted sign-in flow) */
    setAuth(overrides) {
      this.auth = Object.assign({}, this.auth, overrides);
      this.emit("auth://status", Object.assign({}, this.auth));
    },
  };

  /* Scripted mock job: resolve → (conflict for sync) → download tracks →
     (removals for sync) → complete. Everything goes through the same
     event payloads the real backend emits. */
  function runMockScenario(jobId, args) {
    const isSync = args.mode === "sync";
    const isBulk = args.inputMode === "bulk";
    const urlCount = isBulk ? (args.urls || []).length : (args.url ? 1 : 0);
    const total = isBulk ? Math.max(urlCount, 4) : 8;
    let cancelled = false;

    mock.clearTimers();
    mock.later(() => emitLog(jobId, "info", "Job started — " + args.mode + " mode, " + urlCount + " link" + (urlCount === 1 ? "" : "s") + "."), 200);
    // deferred: the app stores the jobId only after start_job resolves
    mock.later(() => emitProgress(jobId, { phase: "resolvingMetadata", current: 0, total: 0, percent: null, title: null, message: "Reading track lists from Spotify…" }), 60);

    let step = 900;

    if (isSync && !isBulk) {
      mock.later(() => {
        emitLog(jobId, "info", "Found an existing folder for this playlist under the output tree.");
        const payload = mock.armConflict(1);
        emitEvent("sync://conflict", payload);
        mock.wait(payload.requestId, "conflict", 120000, () => {
          emitLog(jobId, "warn", "Conflict prompt timed out — job cancelled.");
        }).then((res) => {
          if (cancelled) return;
          if (res && res.__timeout) { abortJob("No answer to the folder conflict — cancelled."); return; }
          if (res.choice === "syncHere") emitLog(jobId, "info", "Syncing into existing folder: " + (res.dir || payload.matches[0].dir));
          else if (res.choice === "createNew") emitLog(jobId, "warn", "Creating a duplicate folder by user choice.");
          else { abortJob("Folder conflict cancelled — nothing was created."); return; }
          continueAfterConflict();
        });
      }, step);
      step += 400;
    } else {
      mock.later(continueAfterConflict, step);
    }

    function continueAfterConflict() {
      if (cancelled) return;
      mock.later(() => {
        emitLog(jobId, "info", isSync
          ? "Plan: " + (isBulk ? 2 : 3) + " new, " + (isBulk ? 6 : 12) + " already on disk, " + (isSync && !isBulk ? 2 : 0) + " removed."
          : "Resolved " + total + " track" + (total === 1 ? "" : "s") + ".");
        let i = 0;
        const tick = () => {
          if (cancelled) return;
          i += 1;
          const title = mockTrack(i);
          emitProgress(jobId, {
            phase: "downloading",
            current: i,
            total,
            percent: Math.round((i / total) * 100),
            title,
            message: "Downloading " + title,
          });
          if (i === 3) emitLog(jobId, "info", "Saved: " + title + ".mp3");
          if (i === Math.ceil(total / 2)) {
            emitEvent("download://track_failed", {
              jobId, title: "Harder, Better, Faster, Stronger", artist: "Daft Punk",
              error: "Source unavailable after 3 attempts",
            });
          }
          if (i < total) mock.later(tick, 520);
          else mock.later(finish, 700);
        };
        tick();
      }, 250);
    }

    function finish() {
      if (cancelled) return;
      if (isSync && !isBulk) {
        emitLog(jobId, "info", "2 tracks are no longer on Spotify — waiting for your decision.");
        const payload = mock.armDeletion(2);
        emitEvent("sync://removed_tracks", payload);
        mock.wait(payload.requestId, "deletion", 120000, () => {
          emitLog(jobId, "warn", "No answer — kept the local files.");
        }).then((res) => {
          if (cancelled) return;
          if (res === "delete") emitLog(jobId, "info", "Deleting removed files…");
          else emitLog(jobId, "info", "Keeping local files.");
          emitProgress(jobId, { phase: "finalizing", current: total, total, percent: 100, title: null, message: "Finalizing…" });
          mock.later(complete, 600);
        });
      } else {
        emitProgress(jobId, { phase: "finalizing", current: total, total, percent: 100, title: null, message: "Finalizing…" });
        mock.later(complete, 600);
      }
    }

    function abortJob(message) {
      emitComplete({ jobId, status: "cancelled", downloaded: 0, skipped: 0, failed: 0, removed: 0, message });
    }

    function complete() {
      if (cancelled) return;
      emitComplete({
        jobId,
        status: "completed",
        downloaded: isBulk ? total - 1 : 3,
        skipped: isSync ? (isBulk ? 6 : 12) : 0,
        failed: 1,
        removed: isSync && !isBulk ? 2 : 0,
        message: isSync ? "Playlist folder is up to date." : "All links processed.",
      });
    }

  }

  const MOCK_TRACKS = [
    "Daft Punk — Get Lucky",
    "Daft Punk — Instant Crush",
    "The Weeknd — Blinding Lights",
    "Dua Lipa — Levitating",
    "Tame Impala — The Less I Know The Better",
    "Daft Punk — Around the World",
    "Phoenix — Lisztomania",
    "Justice — D.A.N.C.E.",
  ];
  function mockTrack(i) { return MOCK_TRACKS[(i - 1) % MOCK_TRACKS.length]; }

  function emitProgress(jobId, p) {
    emitEvent("download://progress", Object.assign({ jobId }, p));
  }
  function emitLog(jobId, level, message) {
    emitEvent("download://log", { jobId, level, message, timestamp: new Date().toISOString() });
  }
  function emitComplete(payload) {
    emitEvent("sync://complete", payload);
  }
  function emitEvent(name, payload) {
    if (state.isTauri) return;                 // real backend emits its own
    mock.emit(name, payload);
  }

  /* ======================================================================
     6. Toasts
     ====================================================================== */
  function toast(message, kind) {
    const node = el("div", "toast");
    node.dataset.kind = kind || "info";
    node.appendChild(el("span", "toast-msg", message));
    const close = el("button", "toast-close", "×");
    close.type = "button";
    close.setAttribute("aria-label", "Dismiss notification");
    close.addEventListener("click", () => node.remove());
    node.appendChild(close);
    dom["toast-region"].appendChild(node);
    setTimeout(() => node.remove(), 6000);
  }

  /* ======================================================================
     7. Renderers
     ====================================================================== */
  function setStatus(kind, label) {
    dom["status-pill"].dataset.status = kind;
    dom["status-text"].textContent = label;
  }

  function renderFolder() {
    const has = Boolean(state.folder);
    dom["folder-placeholder"].hidden = has;
    dom["folder-value"].hidden = !has;
    if (has) {
      dom["folder-value"].textContent = truncateMiddle(state.folder, 56);
      dom["folder-value"].title = state.folder;
      dom["folder-value"].setAttribute("aria-label", "Chosen output folder: " + state.folder);
    }
  }

  function parseBulkUrls() {
    const raw = dom["urls-input"].value;
    const parts = raw.split(/[\s,]+/).map((s) => s.trim()).filter(Boolean);
    return Array.from(new Set(parts));
  }

  /* URL classification: used for warnings only — never a hard block,
     because the backend may resolve other sources. */
  function classifyUrl(u) {
    let parsed;
    try { parsed = new URL(u); } catch { return { kind: "invalid" }; }
    const host = parsed.hostname.toLowerCase();
    if (host === "spotify.link" || host.endsWith(".spotify.link")) return { kind: "short" };
    if (host !== "open.spotify.com" && !host.endsWith(".spotify.com")) return { kind: "nonspotify" };
    const m = parsed.pathname.match(/\/(?:intl-[a-z-]+\/)?(track|album|playlist|episode|show|artist|user)(?:\/|$)/i);
    if (m) return { kind: m[1].toLowerCase() };
    return { kind: "spotify-other" };
  }

  function urlNoteFor(u, mode) {
    const c = classifyUrl(u);
    switch (c.kind) {
      case "invalid":
        return { text: "This doesn't look like a valid URL.", warn: true };
      case "nonspotify":
        return { text: "Not a Spotify link — it may still be resolved.", warn: true };
      case "short":
        return { text: "Short link — it will be expanded automatically.", warn: false };
      case "playlist":
        return { text: "Looks like a playlist link.", warn: false, ok: true };
      case "track":
      case "album":
        return mode === "sync"
          ? { text: "Sync works on playlist folders — a " + c.kind + " link is fetched without a sync state.", warn: true }
          : { text: "Looks like a " + c.kind + " link.", warn: false, ok: true };
      case "spotify-other":
        return { text: "Spotify link, but not a track, album, or playlist — it may not work.", warn: true };
      default:
        return { text: "Paste a track, album, or playlist link.", warn: false };
    }
  }

  function renderInputNotes() {
    if (state.inputMode === "single") {
      const v = dom["url-input"].value.trim();
      const note = v ? urlNoteFor(v, state.mode) : { text: "Paste a track, album, or playlist link.", warn: false };
      dom["url-note"].textContent = note.text;
      dom["url-note"].className = "field-note" + (note.warn ? " is-warn" : note.ok ? " is-ok" : "");
    } else {
      const urls = parseBulkUrls();
      const rawCount = dom["urls-input"].value.split(/[\s,]+/).filter((s) => s.trim()).length;
      const questionable = urls.filter((u) => {
        const k = classifyUrl(u).kind;
        return k === "invalid" || k === "spotify-other" || k === "track" || k === "album";
      }).length;
      let text = urls.length
        ? urls.length + " link" + (urls.length === 1 ? "" : "s") + " ready"
        : "No links yet — one per line.";
      if (urls.length && rawCount > urls.length) text += " · " + (rawCount - urls.length) + " duplicate" + ((rawCount - urls.length) === 1 ? "" : "s") + " removed";
      if (questionable) text += " · " + questionable + " may not be playlist links (fine for Download)";
      dom["urls-note"].textContent = text;
      dom["urls-note"].className = "field-note" + (questionable ? " is-warn" : "");
    }
  }

  function renderWhatHappens() {
    const box = dom["what-happens"];
    const text = dom["what-happens-text"];
    let msg = "";
    let warn = false;

    if (state.mode === "sync" && state.inputMode === "bulk") {
      msg = "Sync runs on one playlist at a time. Switch to a single playlist link, or use Download for many links.";
      warn = true;
    } else if (state.mode === "sync") {
      msg = "Sync compares the playlist with the folder's saved state — new tracks are downloaded, existing files are skipped, and you're asked before any local files for removed tracks are deleted.";
    } else if (state.inputMode === "bulk") {
      msg = "Download fetches every link. Loose tracks land directly in the output folder; each album or playlist becomes its own subfolder. Playlist state is saved for later syncs; links split on newlines or spaces, and duplicates are removed.";
    } else {
      msg = "Download fetches whatever the link points to. A track lands directly in the output folder; an album or playlist gets its own subfolder. Playlist state is saved so the folder can be synced later, but nothing is compared or deleted.";
    }

    text.textContent = msg;
    box.classList.toggle("is-warn", warn);
  }

  function canStart() {
    if (state.running) return false;
    if (!state.auth.authenticated) return false;
    if (!state.folder) return false;
    if (state.mode === "sync" && state.inputMode === "bulk") return false;
    if (state.inputMode === "single") return dom["url-input"].value.trim().length > 0;
    return parseBulkUrls().length > 0;
  }

  function renderActions() {
    const start = dom["start-btn"];
    start.disabled = !canStart();
    start.textContent = state.mode === "sync" ? "Start sync" : "Start download";
    start.title = state.auth.authenticated
      ? (!state.folder
        ? "Choose an output folder first"
        : (state.inputMode === "bulk" && parseBulkUrls().length === 0
          ? "Paste at least one link"
          : ""))
      : (state.auth.configured
        ? "Connect your Spotify account first"
        : "This build is not configured for Spotify");

    dom["cancel-btn"].hidden = !state.running;
    dom["cancel-btn"].disabled = state.cancelling;
    dom["cancel-btn"].textContent = state.cancelling ? "Cancelling…" : "Cancel job";

    // lock inputs while a job runs
    const lock = state.running;
    dom["browse-btn"].disabled = lock;
    dom["url-input"].disabled = lock;
    dom["urls-input"].disabled = lock;
    dom["tab-single"].disabled = lock;
    dom["tab-bulk"].disabled = lock;
    // Bulk only supports Download, so Sync is locked; Download stays selectable
    // (disabling both made the active mode look broken).
    dom["mode-download"].disabled = lock;
    dom["mode-sync"].disabled = lock || state.inputMode === "bulk";
  }

  function renderAll() {
    renderFolder();
    renderInputNotes();
    renderWhatHappens();
    renderActions();
  }

  /* ---------- progress ---------- */
  function resetProgressUi() {
    dom["summary-card"].hidden = true;
    dom["summary-stats"].textContent = "";
    dom["summary-failures"].hidden = true;
    dom["summary-failures"].textContent = "";
    dom["bar-fill"].classList.remove("is-indeterminate");
    dom["bar-fill"].style.width = "0%";
    dom["progress-bar"].classList.remove("is-empty");
    dom["progress-bar"].removeAttribute("aria-valuenow");
    dom["progress-bar"].setAttribute("aria-busy", "true");
    dom["phase-chip"].textContent = "Starting";
    dom["phase-chip"].classList.add("is-live");
    dom["progress-counts"].textContent = "—";
    dom["progress-title"].textContent = "Preparing…";
    dom["progress-title"].classList.add("is-muted");
  }

  function onProgress(p) {
    if (!p || p.jobId !== state.jobId) return;
    dom["progress-bar"].classList.remove("is-empty");
    dom["phase-chip"].textContent = prettyPhase(p.phase);
    dom["phase-chip"].classList.add("is-live");

    const counts = (p.total ? p.current + " / " + p.total : "—");
    dom["progress-counts"].textContent = p.percent != null ? counts + "  ·  " + p.percent + "%" : counts;

    const fill = dom["bar-fill"];
    if (p.percent != null) {
      fill.classList.remove("is-indeterminate");
      fill.style.width = Math.max(0, Math.min(100, p.percent)) + "%";
      dom["progress-bar"].setAttribute("aria-valuenow", String(Math.round(p.percent)));
      dom["progress-bar"].setAttribute("aria-busy", "false");
    } else {
      fill.classList.add("is-indeterminate");
      dom["progress-bar"].removeAttribute("aria-valuenow");
      dom["progress-bar"].setAttribute("aria-busy", "true");
    }

    const title = p.title || p.message || "";
    dom["progress-title"].textContent = title;
    dom["progress-title"].classList.toggle("is-muted", !title);
    dom["progress-title"].title = title;
  }

  function onJobComplete(payload) {
    if (!payload || (state.jobId && payload.jobId !== state.jobId && payload.jobId !== "job-mock-demo")) return;
    state.running = false;
    state.cancelling = false;
    state.jobId = null;

    closeDeletionModal(true);
    closeConflictModal(true);

    const status = payload.status || "completed";
    setStatus(
      status === "completed" ? "done" : status === "failed" ? "error" : "cancelled",
      status === "completed" ? "Done" : status === "failed" ? "Failed" : "Cancelled"
    );
    dom["phase-chip"].textContent = status === "completed" ? "Complete" : status === "failed" ? "Failed" : "Cancelled";
    dom["phase-chip"].classList.toggle("is-live", status === "completed");
    dom["progress-bar"].setAttribute("aria-busy", "false");
    dom["progress-title"].textContent = payload.message || "";
    dom["progress-title"].classList.remove("is-muted");

    renderSummary(status, payload);
    appendLog(status === "completed" ? "success" : status === "failed" ? "error" : "warn",
      payload.message || ("Job " + status + "."), new Date().toISOString());
    renderActions();

    if (status === "failed") toast(payload.message || "Job failed.", "error");
  }

  function renderSummary(status, payload) {
    dom["summary-card"].hidden = false;
    dom["summary-status"].textContent = status;
    dom["summary-status"].dataset.kind = status;

    const stats = dom["summary-stats"];
    stats.textContent = "";
    [
      ["Downloaded", payload.downloaded || 0],
      ["Skipped", payload.skipped || 0],
      ["Removed", payload.removed || 0],
      ["Failed", payload.failed || 0],
    ].forEach(([label, num]) => {
      const box = el("div", "stat");
      box.appendChild(el("div", "stat-num", String(num)));
      box.appendChild(el("div", "stat-label", label));
      stats.appendChild(box);
    });

    dom["summary-message"].textContent = payload.message || "";

    const fails = dom["summary-failures"];
    fails.textContent = "";
    if (state.failedTracks.length) {
      fails.hidden = false;
      fails.appendChild(el("div", "sf-title", "Failed tracks"));
      state.failedTracks.slice(0, 8).forEach((t) => {
        fails.appendChild(el("div", null, t.title + " — " + t.artist + " · " + t.error));
      });
      if (state.failedTracks.length > 8) {
        fails.appendChild(el("div", null, "+ " + (state.failedTracks.length - 8) + " more — see the log."));
      }
    } else {
      fails.hidden = true;
    }
  }

  /* ---------- log ---------- */
  function appendLog(level, message, timestamp) {
    // The completion handler restates the backend's final log line; keep one copy.
    const lastMessage = dom["log-lines"].lastElementChild?.querySelector(".log-msg");
    if (lastMessage && lastMessage.textContent === message) return;

    const empty = dom["log-empty"];
    if (empty) empty.hidden = true;

    const line = el("div", "log-line log-" + (level || "info"));
    line.appendChild(el("span", "log-time", formatTimestamp(timestamp)));
    line.appendChild(el("span", "log-icon", LOG_ICONS[level] || "·"));
    line.appendChild(el("span", "log-msg", message));
    dom["log-lines"].appendChild(line);

    while (dom["log-lines"].children.length > LOG_MAX_LINES) {
      dom["log-lines"].removeChild(dom["log-lines"].firstChild);
    }
    if (state.autoScroll) dom["log-body"].scrollTop = dom["log-body"].scrollHeight;
  }

  function clearLog() {
    dom["log-lines"].textContent = "";
    dom["log-empty"].hidden = false;
  }

  /* ======================================================================
     8. Job lifecycle
     ====================================================================== */
  function buildArgs() {
    return {
      mode: state.mode,
      inputMode: state.inputMode,
      url: state.inputMode === "single" ? (dom["url-input"].value.trim() || null) : null,
      urls: state.inputMode === "bulk" ? parseBulkUrls() : null,
      outputDir: state.folder || "",
    };
  }

  async function startJob() {
    if (!canStart()) return;
    state.failedTracks = [];
    clearLog();
    try {
      const jobId = await bridge.invoke("start_job", buildArgs());
      state.jobId = jobId;
      state.running = true;
      state.cancelling = false;
      setStatus("running", "Running");
      resetProgressUi();
      renderActions();
    } catch (err) {
      setStatus("error", "Error");
      toast(String(err), "error");
      appendLog("error", "Could not start job: " + err, new Date().toISOString());
      state.running = false;
      renderActions();
    }
  }

  async function cancelJob() {
    if (!state.running || state.cancelling || !state.jobId) return;
    state.cancelling = true;
    renderActions();
    try {
      await bridge.invoke("cancel_job", { jobId: state.jobId });
    } catch (err) {
      state.cancelling = false;
      toast(String(err), "error");
      renderActions();
    }
  }

  /* ======================================================================
     8b. Spotify auth + account panel
     ====================================================================== */
  function applyAuthStatus(s) {
    if (!s) return;
    state.auth.authenticated = Boolean(s.authenticated);
    state.auth.configured = Boolean(s.configured);
    state.auth.displayName = s.displayName || null;
    state.auth.error = s.error || null;
    if (state.auth.authenticated) endAuthWait();
    renderAuth();
    renderSettingsAccount();
    renderAll();                     // Start depends on auth
  }

  function renderAuth() {
    const a = state.auth;

    dom["account-chip"].hidden = !a.authenticated;
    dom["account-name"].textContent = a.displayName || "Spotify connected";

    const banner = dom["auth-banner"];
    if (a.authenticated) { banner.hidden = true; return; }
    banner.hidden = false;

    const view = a.waiting ? "waiting"
      : !a.configured ? "unconfigured"
      : a.error ? "error"
      : "connect";
    banner.dataset.state = view;

    dom["auth-wait"].hidden = view !== "waiting";
    dom["auth-actions"].hidden = view === "waiting" || view === "unconfigured";
    dom["auth-connect-btn"].textContent = a.error ? "Try again" : "Connect Spotify";

    if (view === "waiting") {
      dom["auth-banner-title"].textContent = "Waiting for Spotify authorization…";
      dom["auth-banner-detail"].textContent = "Complete the sign-in in your browser.";
      updateAuthWaitTimer();
    } else if (view === "unconfigured") {
      dom["auth-banner-title"].textContent = "Spotify is not configured in this build";
      dom["auth-banner-detail"].textContent = "Reinstall the app or contact support to enable Spotify sign-in.";
    } else if (view === "error") {
      dom["auth-banner-title"].textContent = "Spotify sign-in failed";
      dom["auth-banner-detail"].textContent = a.error;
    } else {
      dom["auth-banner-title"].textContent = "Connect your Spotify account to load playlists";
      dom["auth-banner-detail"].textContent = "Signing in opens your browser and returns here.";
    }
  }

  function beginAuthWait() {
    state.auth.waiting = true;
    state.auth.waitStartedAt = Date.now();
    dom["auth-wait-cancel"].disabled = false;
    if (state.auth.waitTick) clearInterval(state.auth.waitTick);
    state.auth.waitTick = setInterval(updateAuthWaitTimer, 500);
    if (state.auth.waitTimeout) clearTimeout(state.auth.waitTimeout);
    // 3-minute fallback: if the browser round-trip never lands, stop the
    // backend wait via auth_cancel (best-effort) and surface a timeout
    state.auth.waitTimeout = setTimeout(async () => {
      if (!state.auth.waiting) return;
      state.auth.cancelRequested = true;   // the connect rejection is ours, not a failure
      endAuthWait();
      try { await bridge.invoke("auth_cancel"); } catch { /* best-effort */ }
      state.auth.error = "Timed out waiting for Spotify authorization. Try connecting again.";
      renderAuth();
      toast("Spotify sign-in timed out — try connecting again.", "warn");
    }, AUTH_WAIT_UI_TIMEOUT);
    renderAuth();
  }

  function endAuthWait() {
    state.auth.waiting = false;
    state.auth.waitStartedAt = null;
    if (state.auth.waitTick) clearInterval(state.auth.waitTick);
    if (state.auth.waitTimeout) clearTimeout(state.auth.waitTimeout);
    state.auth.waitTick = null;
    state.auth.waitTimeout = null;
    renderAuth();
  }

  function updateAuthWaitTimer() {
    if (!state.auth.waiting || state.auth.waitStartedAt == null) return;
    const elapsed = Date.now() - state.auth.waitStartedAt;
    dom["auth-wait-timer"].textContent =
      "Elapsed " + formatClock(elapsed) + " · gives up after " + formatClock(AUTH_WAIT_UI_TIMEOUT);
  }

  async function connectSpotify() {
    if (state.auth.waiting) return;
    state.auth.error = null;
    state.auth.cancelRequested = false;
    beginAuthWait();
    try {
      const status = await bridge.invoke("auth_connect");
      endAuthWait();
      if (status) applyAuthStatus(status);
      if (status && status.authenticated) toast("Connected to Spotify.", "success");
    } catch (err) {
      endAuthWait();
      if (state.auth.cancelRequested) return;   // our own auth_cancel — not a failure
      state.auth.error = String(err) || "Authorization failed.";
      renderAuth();
      toast(state.auth.error, "error");
    }
  }

  async function refreshAuthStatus() {
    try {
      const a = await bridge.invoke("auth_status");
      applyAuthStatus(a);
    } catch { /* keep the current view */ }
  }

  /* ---------- account panel ---------- */
  function renderSettingsAccount() {
    const authed = state.auth.authenticated;
    dom["settings-account-name"].textContent = authed
      ? (state.auth.displayName || "Spotify account")
      : "Not connected";
    dom["settings-account-note"].hidden = authed;
    dom["settings-account-dot"].classList.toggle("is-off", !authed);
    dom["settings-connect"].hidden = authed;
    dom["settings-logout"].hidden = !authed;
  }

  function openSettings() {
    renderSettingsAccount();
    const focusTarget = state.auth.authenticated ? dom["settings-logout"] : dom["settings-connect"];
    openLayer(dom["settings-layer"], focusTarget, (e) => {
      if (e.key === "Escape") {
        e.preventDefault();
        closeSettings();
        return;
      }
      trapTab(dom["settings-layer"], e);
    });
  }

  function closeSettings() {
    closeLayer(dom["settings-layer"]);
  }

  async function logout() {
    try {
      const s = await bridge.invoke("auth_logout");
      if (s) applyAuthStatus(s);
      else { renderAuth(); renderSettingsAccount(); }
      if (!dom["settings-layer"].hidden) dom["settings-connect"].focus();
      toast("Signed out of Spotify.", "info");
    } catch (err) {
      toast(String(err), "error");
    }
  }

  /* ======================================================================
     9. Modals — focus trap, countdowns, decisions
     ====================================================================== */
  const FOCUSABLE =
    'a[href], button:not([disabled]), input:not([disabled]), textarea:not([disabled]), select:not([disabled]), [tabindex]:not([tabindex="-1"])';

  function trapTab(layer, event) {
    if (event.key !== "Tab") return;
    const nodes = Array.from(layer.querySelectorAll(FOCUSABLE))
      .filter((n) => n.offsetParent !== null || n === document.activeElement);
    if (!nodes.length) return;
    const first = nodes[0];
    const last = nodes[nodes.length - 1];
    if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last.focus(); }
    else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first.focus(); }
  }

  function openLayer(layer, focusTarget, onKeydown) {
    state.lastFocus = document.activeElement;
    layer.hidden = false;
    layer._onKeydown = onKeydown;
    layer.addEventListener("keydown", onKeydown);
    const target = focusTarget || layer.querySelector(FOCUSABLE);
    if (target) target.focus();
  }

  function closeLayer(layer) {
    if (layer.hidden) return;
    layer.removeEventListener("keydown", layer._onKeydown);
    layer.hidden = true;
    if (state.lastFocus && document.contains(state.lastFocus)) state.lastFocus.focus();
  }

  function startCountdown(node, timeoutMs, prefix, onExpire) {
    stopCountdown(node);
    const startedAt = Date.now();
    const total = typeof timeoutMs === "number" && timeoutMs > 0 ? timeoutMs : 120000;
    const tick = () => {
      const left = total - (Date.now() - startedAt);
      if (left <= 0) {
        node.textContent = "";
        node._timer = null;
        onExpire();
        return;
      }
      node.textContent = prefix + " " + formatClock(left);
      node._timer = setTimeout(tick, 1000);
    };
    tick();
  }
  function stopCountdown(node) {
    if (node._timer) clearTimeout(node._timer);
    node._timer = null;
    node.textContent = "";
  }

  function isExpiredError(err) {
    const s = String(err || "").toLowerCase();
    return s.includes("already-resolved") || s.includes("unknown-request") || s.includes("worker-gone");
  }

  /* ---------- deletion modal ---------- */
  const deletion = { requestId: null, submitting: false };

  function openDeletionModal(payload) {
    if (!payload) return;
    deletion.requestId = payload.requestId;
    deletion.submitting = false;

    const n = (payload.tracks || []).length;
    const name = payload.playlistName || "this playlist";
    dom["deletion-desc"].textContent =
      n + " track" + (n === 1 ? "" : "s") + " in '" + name + "' " + (n === 1 ? "is" : "are") +
      " no longer on Spotify. Delete the downloaded file" + (n === 1 ? "" : "s") + " from disk?";

    const list = dom["deletion-list"];
    list.textContent = "";
    (payload.tracks || []).forEach((t) => {
      const row = el("div", "track-row");
      const head = el("div", "track-title-line");
      head.appendChild(el("span", "track-name", t.title || "(unknown title)"));
      row.appendChild(head);
      row.appendChild(el("span", "track-artist", t.artist || ""));
      row.appendChild(el("span", "track-file", t.localFilename || ""));
      list.appendChild(row);
    });

    dom["deletion-keep"].disabled = false;
    dom["deletion-delete"].disabled = false;

    openLayer(dom["deletion-layer"], dom["deletion-keep"], (e) => {
      // Escape intentionally does nothing: a silent keypress must never
      // resolve a destructive prompt. Only an explicit button decides.
      if (e.key === "Escape") { e.preventDefault(); return; }
      trapTab(dom["deletion-layer"], e);
    });

    startCountdown(dom["deletion-countdown"], payload.timeoutMs, "Auto-keeps in", () => {
      closeDeletionModal();
      toast("No response — files were kept.", "info");
    });
  }

  function closeDeletionModal(silent) {
    stopCountdown(dom["deletion-countdown"]);
    deletion.requestId = null;
    deletion.submitting = false;
    if (!silent) closeLayer(dom["deletion-layer"]);
    else { dom["deletion-layer"].hidden = true; dom["deletion-layer"].removeEventListener("keydown", dom["deletion-layer"]._onKeydown); }
  }

  async function submitDeletion(decision) {
    if (!deletion.requestId || deletion.submitting) return;
    deletion.submitting = true;
    dom["deletion-keep"].disabled = true;
    dom["deletion-delete"].disabled = true;
    const btn = decision === "delete" ? dom["deletion-delete"] : dom["deletion-keep"];
    btn.classList.add("is-loading");
    try {
      await bridge.invoke("confirm_deletions", { requestId: deletion.requestId, decision });
      stopCountdown(dom["deletion-countdown"]);
      closeDeletionModal();
      toast(decision === "delete" ? "Removing deleted tracks from disk." : "Keeping the local files.", "info");
    } catch (err) {
      btn.classList.remove("is-loading");
      if (isExpiredError(err)) {
        closeDeletionModal();
        toast("This prompt already expired or was answered.", "warn");
      } else {
        deletion.submitting = false;
        dom["deletion-keep"].disabled = false;
        dom["deletion-delete"].disabled = false;
        toast(String(err), "error");
      }
    } finally {
      btn.classList.remove("is-loading");
    }
  }

  /* ---------- conflict modal ---------- */
  const conflict = { requestId: null, submitting: false, matches: [] };

  function renderConflictModal(payload) {
    const name = payload.playlistName || "This playlist";
    const target = payload.targetDir || "";
    dom["conflict-desc"].textContent =
      "A folder for '" + name + "' was already found under the output tree. " +
      "Sync one of the existing folders, or create a new folder anyway.";
    dom["conflict-desc"].appendChild(document.createElement("br"));
    dom["conflict-desc"].appendChild(document.createTextNode("Planned new folder: "));
    dom["conflict-desc"].appendChild(el("code", null, target));

    const box = dom["conflict-matches"];
    box.textContent = "";
    conflict.matches.forEach((m, i) => {
      const row = el("label", "match-row");
      const input = document.createElement("input");
      input.type = "radio";
      input.name = "conflict-match";
      input.value = String(i);
      if (i === 0) input.checked = true;
      row.appendChild(input);
      const info = el("span", "match-info");
      info.appendChild(el("span", "match-dir", m.dir));
      info.appendChild(el("span", "match-meta",
        (m.playlistName && m.playlistName !== name ? "'" + m.playlistName + "' · " : "") +
        (m.trackCount != null ? m.trackCount + " tracks · " : "") +
        formatWhen(m.updatedAt)));
      row.appendChild(info);
      box.appendChild(row);
    });

    dom["conflict-sync"].disabled = conflict.matches.length === 0;
    dom["conflict-create"].disabled = false;
    dom["conflict-cancel"].disabled = false;

    openLayer(dom["conflict-layer"], conflict.matches.length ? dom["conflict-matches"].querySelector("input") : dom["conflict-create"], (e) => {
      if (e.key === "Escape") {
        e.preventDefault();
        submitConflict("cancel");
        return;
      }
      trapTab(dom["conflict-layer"], e);
    });

    startCountdown(dom["conflict-countdown"], payload.timeoutMs, "Auto-cancels in", () => {
      closeConflictModal();
      toast("No response — the job was cancelled.", "info");
    });
  }

  function openConflict(payload) {
    if (!payload) return;
    conflict.requestId = payload.requestId;
    conflict.submitting = false;
    conflict.matches = payload.matches || [];
    renderConflictModal(payload);
  }

  function closeConflictModal(silent) {
    stopCountdown(dom["conflict-countdown"]);
    conflict.requestId = null;
    conflict.submitting = false;
    if (silent) {
      dom["conflict-layer"].hidden = true;
      dom["conflict-layer"].removeEventListener("keydown", dom["conflict-layer"]._onKeydown);
    } else {
      closeLayer(dom["conflict-layer"]);
    }
  }

  async function submitConflict(choice) {
    if (!conflict.requestId || conflict.submitting) return;
    let dir = null;
    if (choice === "syncHere") {
      const checked = dom["conflict-matches"].querySelector("input:checked");
      if (!checked) { toast("Pick a folder to sync first.", "warn"); return; }
      dir = conflict.matches[Number(checked.value)] ? conflict.matches[Number(checked.value)].dir : null;
    }
    conflict.submitting = true;
    dom["conflict-sync"].disabled = true;
    dom["conflict-create"].disabled = true;
    dom["conflict-cancel"].disabled = true;
    const btn = choice === "syncHere" ? dom["conflict-sync"]
      : choice === "createNew" ? dom["conflict-create"] : dom["conflict-cancel"];
    btn.classList.add("is-loading");
    try {
      // `dir` names the chosen folder when several matches exist. It is an
      // additive argument: backends that only take (requestId, choice) ignore it.
      await bridge.invoke("resolve_conflict", { requestId: conflict.requestId, choice, dir });
      stopCountdown(dom["conflict-countdown"]);
      closeConflictModal();
    } catch (err) {
      if (isExpiredError(err)) {
        closeConflictModal();
        toast("This prompt already expired or was answered.", "warn");
      } else {
        conflict.submitting = false;
        dom["conflict-sync"].disabled = false;
        dom["conflict-create"].disabled = false;
        dom["conflict-cancel"].disabled = false;
        toast(String(err), "error");
      }
    } finally {
      btn.classList.remove("is-loading");
    }
  }

  /* ======================================================================
     10. Event wiring
     ====================================================================== */
  function setInputMode(mode) {
    state.inputMode = mode;
    dom["tab-single"].classList.toggle("is-active", mode === "single");
    dom["tab-bulk"].classList.toggle("is-active", mode === "bulk");
    dom["tab-single"].setAttribute("aria-selected", String(mode === "single"));
    dom["tab-bulk"].setAttribute("aria-selected", String(mode === "bulk"));
    dom["panel-single"].hidden = mode !== "single";
    dom["panel-bulk"].hidden = mode !== "bulk";
    if (mode === "bulk" && state.mode === "sync") {
      // Sync is single-playlist; fall back to Download and say why.
      dom["mode-download"].checked = true;
      setMode("download");
    }
    renderAll();
  }

  function setMode(mode) {
    state.mode = mode;
    dom["mode-sync"].closest(".mode-card").classList.toggle("is-selected", mode === "sync");
    dom["mode-download"].closest(".mode-card").classList.toggle("is-selected", mode === "download");
    renderAll();
  }

  function wireEvents() {
    dom["browse-btn"].addEventListener("click", async () => {
      try {
        const folder = await bridge.invoke("pick_folder");
        if (folder) { state.folder = folder; renderAll(); }
      } catch (err) {
        toast(String(err), "error");
      }
    });

    dom["tab-single"].addEventListener("click", () => setInputMode("single"));
    dom["tab-bulk"].addEventListener("click", () => setInputMode("bulk"));

    dom["url-input"].addEventListener("input", renderAll);
    dom["urls-input"].addEventListener("input", renderAll);

    dom["mode-download"].addEventListener("change", () => setMode("download"));
    dom["mode-sync"].addEventListener("change", () => setMode("sync"));

    dom["start-btn"].addEventListener("click", startJob);
    dom["cancel-btn"].addEventListener("click", cancelJob);

    dom["log-clear-btn"].addEventListener("click", clearLog);
    dom["log-collapse-btn"].addEventListener("click", () => {
      const collapsed = dom["log-block"].classList.toggle("is-collapsed");
      dom["log-collapse-btn"].textContent = collapsed ? "Expand" : "Collapse";
      dom["log-collapse-btn"].setAttribute("aria-expanded", String(!collapsed));
    });
    dom["log-body"].addEventListener("scroll", () => {
      const b = dom["log-body"];
      state.autoScroll = b.scrollTop + b.clientHeight >= b.scrollHeight - 24;
    });

    // deletion modal
    dom["deletion-keep"].addEventListener("click", () => submitDeletion("keep"));
    dom["deletion-delete"].addEventListener("click", () => submitDeletion("delete"));

    // conflict modal
    dom["conflict-sync"].addEventListener("click", () => submitConflict("syncHere"));
    dom["conflict-create"].addEventListener("click", () => submitConflict("createNew"));
    dom["conflict-cancel"].addEventListener("click", () => submitConflict("cancel"));

    // account + auth
    dom["settings-btn"].addEventListener("click", openSettings);
    dom["auth-connect-btn"].addEventListener("click", connectSpotify);
    dom["auth-wait-cancel"].addEventListener("click", async () => {
      if (state.auth.cancelInFlight) return;
      state.auth.cancelInFlight = true;
      state.auth.cancelRequested = true;      // auth_connect's rejection is intentional
      dom["auth-wait-cancel"].disabled = true;
      try {
        const s = await bridge.invoke("auth_cancel");
        endAuthWait();
        if (s) applyAuthStatus(s);
        toast("Spotify sign-in cancelled.", "info");
      } catch (err) {
        state.auth.cancelRequested = false;   // cancel failed — we're still waiting
        dom["auth-wait-cancel"].disabled = false;
        toast(String(err), "warn");
      } finally {
        state.auth.cancelInFlight = false;
      }
    });
    dom["settings-connect"].addEventListener("click", () => {
      closeSettings();
      connectSpotify();
    });
    dom["settings-close"].addEventListener("click", closeSettings);
    dom["settings-logout"].addEventListener("click", logout);
  }

  function subscribeToBackend() {
    bridge.listen("download://progress", onProgress);
    bridge.listen("download://log", (p) => {
      if (!p || (state.jobId && p.jobId !== state.jobId)) return;
      appendLog(p.level, p.message, p.timestamp);
    });
    bridge.listen("download://track_failed", (p) => {
      if (!p || (state.jobId && p.jobId !== state.jobId)) return;
      // The backend already emits a download://log line for this failure; only
      // track it here so the log does not show every failed track twice.
      state.failedTracks.push({ title: p.title, artist: p.artist, error: p.error });
    });
    bridge.listen("sync://removed_tracks", (p) => {
      if (!p || (state.jobId && p.jobId !== state.jobId)) return;
      openDeletionModal(p);
    });
    bridge.listen("sync://conflict", (p) => {
      if (!p || (state.jobId && p.jobId !== state.jobId)) return;
      openConflict(p);
    });
    bridge.listen("sync://complete", onJobComplete);
    bridge.listen("auth://status", applyAuthStatus);
  }

  /* ======================================================================
     11. Init
     ====================================================================== */
  function init() {
    wireEvents();
    subscribeToBackend();
    setMode(state.mode);
    setInputMode(state.inputMode);
    setStatus("idle", "Idle");
    renderAuth();
    renderSettingsAccount();
    renderAll();
    refreshAuthStatus();            // async; auth_status drives the banner and account panel
  }

  init();
})();
