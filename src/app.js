(function () {
  "use strict";

  const state = {
    isTauri: Boolean(window.__TAURI__ && window.__TAURI__.core),
    folder: null,
    selectedTracks: [],
    mode: "download",
    mediaMode: "audio",
    isBulkRunning: false,
  };

  let syncDeleteCountdown = null;
  let pendingDeletionFiles = [];
  let pendingDeletionStatePath = "";

  const $ = (id) => document.getElementById(id);

  function addLog(message, type = "info") {
    const list = $("log-lines");
    const empty = $("log-empty");
    if (empty) empty.hidden = true;
    if (!list) return;
    const time = new Date().toLocaleTimeString([], { hour12: false });
    const line = document.createElement("div");
    line.className = `log-line log-${type}`;
    let icon = "i";
    if (type === "error") icon = "×";
    if (type === "success") icon = "✓";
    if (type === "warn") icon = "!";
    line.innerHTML = `${time} ${icon} ${message}`;
    list.appendChild(line);
    const body = $("log-body");
    if (body) body.scrollTop = body.scrollHeight;
  }

  if (window.__TAURI__ && window.__TAURI__.event) {
    window.__TAURI__.event.listen("download-progress", (event) => {
      const { percent, status } = event.payload;
      const barFill = $("bar-fill");
      if (barFill) { barFill.style.width = `${percent}%`; barFill.classList.remove("is-indeterminate"); }
      const progressTitle = $("progress-title");
      if (progressTitle) progressTitle.textContent = status;
      if (status.includes("Waiting")) addLog(status, "info");
      if (status.includes("Starting")) addLog(status, "info");
      if (status.includes("Retrying")) addLog(status, "warn");
      if (status.includes("Skipped")) addLog(status, "info");
      if (status.includes("Tagging")) addLog(status, "info");
      if (status.includes("Finished")) addLog(status, "success");
      if (status.includes("All done")) addLog(status, "success");
    });

    window.__TAURI__.event.listen("download-bytes", (event) => {
      const p = event.payload || {};
      const mb = (n) => ((Number(n) || 0) / 1048576).toFixed(1);
      const el = document.getElementById("progress-title");
      if (el) el.textContent = `${p.title || "Downloading"} — ${mb(p.downloaded)} / ${mb(p.total)} MB (${p.percent || 0}%)`;
    });

    window.__TAURI__.event.listen("download-error", (event) => {
      addLog(event.payload.error, "error");
    });

    window.__TAURI__.event.listen("download-log", (event) => {
      addLog(event.payload.message, "warn");
    });

    window.__TAURI__.event.listen("batch-paused", (event) => {
      const message = (event.payload && event.payload.message) ||
        "YouTube is asking for verification. Watch a short video, then click Continue.";
      const modal = document.getElementById("batch-pause-modal");
      const msgEl = document.getElementById("batch-pause-msg");
      if (msgEl) msgEl.textContent = message;
      if (modal) modal.style.display = "flex";
      addLog("⏸ Batch paused: " + message, "warn");
    });

    window.__TAURI__.event.listen("sync-deletion-prompt", (event) => {
      const { files, timeout_ms, state_path } = event.payload;
      pendingDeletionFiles = files;
      pendingDeletionStatePath = state_path;
      const modal = document.getElementById("sync-delete-modal");
      const msgEl = document.getElementById("sync-delete-msg");
      const timerEl = document.getElementById("sync-delete-timer");
      if (msgEl) msgEl.textContent = `${files.length} track(s) are no longer in the playlist. Delete them from your device?`;
      if (modal) modal.style.display = "flex";
      let seconds = Math.floor(timeout_ms / 1000);
      if (timerEl) timerEl.textContent = seconds;
      if (syncDeleteCountdown) clearInterval(syncDeleteCountdown);
      syncDeleteCountdown = setInterval(() => {
        seconds--;
        if (timerEl) timerEl.textContent = seconds;
        if (seconds <= 0) {
          clearInterval(syncDeleteCountdown);
          closeSyncDeleteModal();
          addLog("Sync: Kept old files (auto-dismissed).", "info");
        }
      }, 1000);
    });
  }

  function updateFolderUi(path) {
    const placeholder = $("folder-placeholder");
    const value = $("folder-value");
    if (placeholder && value) {
      placeholder.hidden = true;
      value.textContent = path;
      value.hidden = false;
    }
  }

  async function loadSettings() {
    if (!state.isTauri) return;
    try {
      const s = await window.__TAURI__.core.invoke("get_settings");
      if (s.outputDir) { state.folder = s.outputDir; updateFolderUi(s.outputDir); }
      if (s.mediaMode === "video") setMediaMode("video"); else setMediaMode("audio");
      if (typeof s.overwrite === "boolean") state.mode = s.overwrite ? "download" : "sync";
      const savedRadio = $(s.overwrite ? "mode-download" : "mode-sync");
      if (savedRadio) savedRadio.checked = true;
    } catch (e) {
      addLog("Could not load settings: " + e, "warn");
    }
  }

  async function persistSettings() {
    if (!state.isTauri) return;
    try {
      await window.__TAURI__.core.invoke("save_settings", {
        settings: { source: "none", outputDir: state.folder || null, mediaMode: state.mediaMode, quality: (document.getElementById("quality-select")?.value || "best"), overwrite: state.mode === "download" },
      });
    } catch (e) { console.warn("save_settings failed", e); }
  }

  async function browseFolder() {
    if (!state.isTauri) return alert("Folder picker works only in the Tauri app.");
    try {
      const path = await window.__TAURI__.core.invoke("pick_folder");
      if (path) { state.folder = path; updateFolderUi(path); checkReady(); persistSettings(); }
    } catch (e) { addLog("Could not open folder picker: " + e, "error"); }
  }

  function isBulkMode() { return !$("panel-bulk").hidden; }

  function setupTabs() {
    const tabSingle = $("tab-single");
    const tabBulk = $("tab-bulk");
    const panelSingle = $("panel-single");
    const panelBulk = $("panel-bulk");
    const syncCard = $("mode-sync-card");
    if (!tabSingle || !tabBulk) return;

    tabSingle.addEventListener("click", () => {
      tabSingle.classList.add("is-active");
      tabBulk.classList.remove("is-active");
      panelSingle.hidden = false;
      panelBulk.hidden = true;
      if (syncCard) syncCard.style.display = "";
    });

    tabBulk.addEventListener("click", () => {
      tabBulk.classList.add("is-active");
      tabSingle.classList.remove("is-active");
      panelBulk.hidden = false;
      panelSingle.hidden = true;
      if (syncCard) syncCard.style.display = "none";
      const dl = $("mode-download");
      if (dl) dl.checked = true;
      setMode();
    });
  }

  async function resolveSingle(query) {
    const container = $("search-results");
    if (!container) return [];
    const isUrl = /^https?:\/\//i.test(query);
    container.innerHTML = `<div style="padding: 16px;">Searching...</div>`;
    container.hidden = false;
    try {
      const results = isUrl
        ? await window.__TAURI__.core.invoke("resolve_input", { input: query })
        : await window.__TAURI__.core.invoke("search_youtube", { query, limit: 10 });
      if (!results || results.length === 0) {
        container.innerHTML = `<div style="padding: 16px;">No results found.</div>`;
        addLog("No results found.", "warn");
        return [];
      }
      addLog(`Found ${results.length} matches.`, "success");
      renderResults(results);
      return results;
    } catch (e) {
      container.innerHTML = `<div style="padding: 16px;">Search error: ${e}</div>`;
      addLog(`Search error: ${e}`, "error");
      return [];
    }
  }

  async function resolveBulk() {
    const textarea = $("urls-input");
    const container = $("search-results");
    if (!textarea || !container) return [];
    const rawText = textarea.value.trim();
    if (!rawText) return [];
    const links = rawText.split("\n").map((l) => l.trim()).filter((l) => l !== "");
    addLog(`Resolving ${links.length} bulk links...`, "info");
    container.innerHTML = `<div style="padding: 16px;">Resolving ${links.length} links...</div>`;
    container.hidden = false;
    try {
      const results = await window.__TAURI__.core.invoke("resolve_bulk_input", { inputs: links });
      if (!results || results.length === 0) {
        container.innerHTML = `<div style="padding: 16px;">No results found.</div>`;
        addLog("No results found for bulk links.", "warn");
        return [];
      }
      addLog(`Found ${results.length} total tracks across all links.`, "success");
      renderResults(results);
      return results;
    } catch (e) {
      container.innerHTML = `<div style="padding: 16px;">Error: ${e}</div>`;
      addLog(`Bulk search error: ${e}`, "error");
      return [];
    }
  }

  async function triggerSearch() {
    if (!state.isTauri) return alert("Search requires the Tauri app.");
    state.selectedTracks = [];
    checkReady();
    if (isBulkMode()) { await resolveBulk(); return; }
    const input = $("url-input");
    if (!input) return;
    const query = input.value.trim();
    if (!query) return;
    addLog(`Searching: "${query}"...`, "info");
    await resolveSingle(query);
  }

  function renderResults(results) {
    const container = $("search-results");
    if (!container) return;
    container.innerHTML = "";

    const bulk = isBulkMode();
    const isPlaylistSet = results.length > 0 && results.some((r) => Boolean(r.folderName || r.folder_name));

    results.forEach((track) => {
      const item = document.createElement("div");
      item.className = "search-result-item";
      item.style.display = "flex";
      item.style.alignItems = "center";
      item.style.gap = "12px";
      item.style.padding = "10px 12px";
      item.style.cursor = "pointer";
      item.style.borderBottom = "1px solid var(--border)";

      const imgSrc = track.coverUrl ||
        "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='48' height='48' viewBox='0 0 24 24' fill='none' stroke='%23666' stroke-width='2' stroke-linecap='round' stroke-linejoin='round'%3E%3Cpath d='M9 18V5l12-2v13'%3E%3C/path%3E%3Ccircle cx='6' cy='18' r='3'%3E%3C/circle%3E%3Ccircle cx='18' cy='16' r='3'%3E%3C/circle%3E%3C/svg%3E";

      const dur = track.durationMs
        ? `${Math.floor(track.durationMs / 60000)}:${String(Math.floor((track.durationMs % 60000) / 1000)).padStart(2, "0")}`
        : "";

      const metaParts = [track.artist];
      if (track.year) metaParts.push(track.year);
      if (dur) metaParts.push(dur);
      const metaStr = metaParts.filter(Boolean).join(" • ");

      item.innerHTML = `
        <img src="${imgSrc}" alt="" style="width:48px;height:48px;border-radius:8px;object-fit:cover;flex-shrink:0;">
        <div style="min-width:0;flex:1;">
          <div style="font-weight:600;white-space:nowrap;overflow:hidden;text-overflow:ellipsis;">${track.title}</div>
          <div style="color:var(--text-dim);font-size:12px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis;">${metaStr}</div>
        </div>
      `;

      item.addEventListener("click", () => {
        Array.from(container.children).forEach((c) => { c.style.background = "transparent"; c.style.borderLeft = "none"; });
        item.style.background = "var(--surface-3)";
        item.style.borderLeft = "3px solid var(--accent)";
        if (bulk || isPlaylistSet) state.selectedTracks = results; else state.selectedTracks = [track];
        const input = $("url-input");
        if (input) input.value = `${track.title} - ${track.artist}`;
        addLog(`Selected: ${track.title}`, "success");
        checkReady();
      });

      container.appendChild(item);
    });

    if (bulk || isPlaylistSet) {
      state.selectedTracks = results;
      addLog(`Selected all ${results.length} tracks for download.`, "success");
      checkReady();
    } else if (results.length === 1) {
      state.selectedTracks = results;
      checkReady();
    } else {
      addLog(`Found ${results.length} matches — click one to select it.`, "info");
    }

    container.hidden = false;
  }

  function hideResults() {
    const container = $("search-results");
    if (container) container.hidden = true;
  }

  function setMode() {
    const isSync = $("mode-sync")?.checked;
    state.mode = isSync ? "sync" : "download";
    const textEl = $("what-happens-text");
    const startBtn = $("start-btn");
    if (textEl) {
      if (isSync) {
        textEl.textContent = "Compares local files, skips existing tracks, downloads new ones.";
        if (startBtn) startBtn.textContent = "Start sync";
      } else {
        textEl.textContent = "Overwrites existing files and downloads every selected track.";
        if (startBtn) startBtn.textContent = "Start download";
      }
    }
    checkReady();
    persistSettings();
  }

  function setMediaMode(mode) {
    state.mediaMode = mode === "video" ? "video" : "audio";
    document.querySelectorAll("[data-media-mode]").forEach((btn) => {
      btn.classList.toggle("is-active", btn.dataset.mediaMode === state.mediaMode);
    });
    persistSettings();
  }

  function checkReady() {
    const startBtn = $("start-btn");
    if (!startBtn) return;
    startBtn.disabled = !(state.folder && state.selectedTracks.length > 0);
  }

  async function startJob() {
    if (!state.folder) return addLog("Choose an output folder first.", "error");

    // Resolve first if nothing selected yet (search-on-start behavior)
    if (state.selectedTracks.length === 0) {
      if (isBulkMode()) {
        await resolveBulk();
      } else {
        const input = $("url-input");
        const query = input ? input.value.trim() : "";
        if (!query) return addLog("Search for and pick a song first.", "error");
        await resolveSingle(query);
      }
    }

    if (state.selectedTracks.length === 0) {
      return addLog("Search for and pick a song first.", "error");
    }

    const btn = $("start-btn");
    if (btn) { btn.disabled = true; btn.textContent = "Processing..."; }

    const pill = $("status-pill");
    const pillText = $("status-text");
    if (pill && pillText) { pill.dataset.status = "running"; pillText.textContent = "Running"; }

    const bar = $("progress-bar");
    const barFill = $("bar-fill");
    if (bar && barFill) { bar.classList.remove("is-empty"); barFill.style.width = "0%"; }

    addLog(`Starting download for ${state.selectedTracks.length} track(s) [${state.mode} / ${state.mediaMode}]...`, "info");

    try {
      await window.__TAURI__.core.invoke("download_track", {
        tracks: state.selectedTracks,
        outputDir: state.folder,
        mode: state.mode,
        mediaMode: state.mediaMode,
      });
      if (pill && pillText) { pill.dataset.status = "done"; pillText.textContent = "Done"; }
    } catch (e) {
      addLog("Download failed: " + e, "error");
      if (pill && pillText) { pill.dataset.status = "error"; pillText.textContent = "Error"; }
    } finally {
      if (btn) { btn.disabled = false; setMode(); }
    }
  }

  function closeSyncDeleteModal() {
    const modal = document.getElementById("sync-delete-modal");
    if (modal) modal.style.display = "none";
    if (syncDeleteCountdown) { clearInterval(syncDeleteCountdown); syncDeleteCountdown = null; }
    pendingDeletionFiles = [];
    pendingDeletionStatePath = "";
  }

  document.addEventListener("DOMContentLoaded", () => {
    $("browse-btn")?.addEventListener("click", browseFolder);
    $("start-btn")?.addEventListener("click", startJob);
    $("find-btn")?.addEventListener("click", triggerSearch);

    $("url-input")?.addEventListener("keypress", (e) => {
      if (e.key === "Enter") triggerSearch();
    });

    $("mode-download")?.addEventListener("change", setMode);
    $("mode-sync")?.addEventListener("change", setMode);

    document.querySelectorAll("[data-media-mode]").forEach((btn) => {
      btn.addEventListener("click", () => setMediaMode(btn.dataset.mediaMode));
    });

    setupTabs();
    setMode();
    loadSettings();

    document.addEventListener("click", (e) => {
      const input = $("url-input");
      const results = $("search-results");
      const findBtn = $("find-btn");
      if (input && results && findBtn &&
        !input.contains(e.target) && !results.contains(e.target) && !findBtn.contains(e.target)) {
        hideResults();
      }
    });

    document.getElementById("batch-resume-btn")?.addEventListener("click", async () => {
      const modal = document.getElementById("batch-pause-modal");
      if (modal) modal.style.display = "none";
      try { await window.__TAURI__.core.invoke("resume_batch"); addLog("Resuming batch…", "info"); }
      catch (e) { addLog("Could not resume batch: " + e, "error"); }
    });

    document.getElementById("sync-delete-cancel-btn")?.addEventListener("click", () => {
      closeSyncDeleteModal();
      addLog("Sync: Kept old files.", "info");
    });

    document.getElementById("sync-delete-confirm-btn")?.addEventListener("click", async () => {
      if (pendingDeletionFiles.length === 0) return;
      try {
        await window.__TAURI__.core.invoke("delete_orphaned_files", {
          files: pendingDeletionFiles,
          statePath: pendingDeletionStatePath,
        });
        addLog(`Sync: Deleted ${pendingDeletionFiles.length} old file(s).`, "success");
      } catch (e) { addLog("Sync deletion failed: " + e, "error"); }
      closeSyncDeleteModal();
    });
  });
})();