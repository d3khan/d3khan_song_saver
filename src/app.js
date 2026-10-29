(function () {
  "use strict";

  const state = {
    isTauri: Boolean(window.__TAURI__ && window.__TAURI__.core),
    folder: null,
    selectedTracks: [],
    mode: "download",
    isBulkRunning: false,
  };

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

    line.innerHTML = `<span class="log-time">${time}</span> <span class="log-icon">${icon}</span> <span class="log-msg">${message}</span>`;
    list.appendChild(line);

    const body = $("log-body");
    if (body) body.scrollTop = body.scrollHeight;
  }

  if (window.__TAURI__ && window.__TAURI__.event) {
    window.__TAURI__.event.listen("download-progress", (event) => {
      const { percent, status } = event.payload;

      const barFill = $("bar-fill");
      if (barFill) {
        barFill.style.width = `${percent}%`;
        barFill.classList.remove("is-indeterminate");
      }

      const progressTitle = $("progress-title");
      if (progressTitle) {
        progressTitle.textContent = status;
      }

      if (status.includes("Waiting")) addLog(status, "info");
      if (status.includes("Starting")) addLog(status, "info");
      if (status.includes("Retrying")) addLog(status, "warn");
      if (status.includes("Skipped")) addLog(status, "info");
      if (status.includes("Tagging")) addLog(status, "info");
      if (status.includes("Finished")) addLog(status, "success");
      if (status.includes("All done")) addLog(status, "success");
    });

    window.__TAURI__.event.listen("download-error", (event) => {
      addLog(event.payload.error, "error");
    });

    window.__TAURI__.event.listen("download-log", (event) => {
      addLog(event.payload.message, "warn");
    });

    window.__TAURI__.event.listen("batch-paused", (event) => {
      const message = (event.payload && event.payload.message)
        || "YouTube is asking for verification. Watch a short video, then click Continue.";
      const modal = document.getElementById("batch-pause-modal");
      const msgEl = document.getElementById("batch-pause-msg");
      if (msgEl) msgEl.textContent = message;
      if (modal) modal.style.display = "flex";
      addLog("⏸ Batch paused: " + message, "warn");
    });
  }

  async function browseFolder() {
    if (!state.isTauri) return alert("Folder picker works only in the Tauri app.");
    try {
      const path = await window.__TAURI__.core.invoke("pick_folder");
      if (path) {
        state.folder = path;
        const placeholder = $("folder-placeholder");
        const value = $("folder-value");
        if (placeholder && value) {
          placeholder.hidden = true;
          value.textContent = path;
          value.hidden = false;
        }
        checkReady();
      }
    } catch (e) {
      addLog("Could not open folder picker: " + e, "error");
    }
  }

  function setupTabs() {
    const tabSingle = $("tab-single");
    const tabBulk = $("tab-bulk");
    const panelSingle = $("panel-single");
    const panelBulk = $("panel-bulk");

    if (!tabSingle || !tabBulk) return;

    tabSingle.addEventListener("click", () => {
      tabSingle.classList.add("is-active");
      tabBulk.classList.remove("is-active");
      panelSingle.hidden = false;
      panelBulk.hidden = true;
    });

    tabBulk.addEventListener("click", () => {
      tabBulk.classList.add("is-active");
      tabSingle.classList.remove("is-active");
      panelBulk.hidden = false;
      panelSingle.hidden = true;
    });
  }

  async function triggerSearch() {
    if (!state.isTauri) return alert("Search requires the Tauri app.");

    state.selectedTracks = [];
    checkReady();

    const container = $("search-results");
    if (!container) return;

    const isBulkMode = !$("panel-bulk").hidden;

    if (isBulkMode) {
      const textarea = $("urls-input");
      if (!textarea) return;

      const rawText = textarea.value.trim();
      if (!rawText) return;

      const links = rawText.split("\n").map(l => l.trim()).filter(l => l !== "");

      addLog(`Resolving ${links.length} bulk links...`, "info");
      container.innerHTML = `<div style="padding: 12px; color: var(--text-dim); text-align: center;">Resolving ${links.length} links...</div>`;
      container.hidden = false;

      try {
        const results = await window.__TAURI__.core.invoke("resolve_bulk_input", { inputs: links });

        if (!results || results.length === 0) {
          container.innerHTML = `<div style="padding: 12px; color: var(--text-dim); text-align: center;">No results found.</div>`;
          addLog("No results found for bulk links.", "warn");
          return;
        }

        addLog(`Found ${results.length} total tracks across all links.`, "success");
        renderResults(results);
      } catch (e) {
        console.error("Bulk search failed:", e);
        container.innerHTML = `<div style="padding: 12px; color: var(--danger); text-align: center;">Error: ${e}</div>`;
        addLog(`Bulk search error: ${e}`, "error");
      }
    } else {
      const input = $("url-input");
      if (!input) return;

      const query = input.value.trim();
      if (!query) return;

      addLog(`Searching: "${query}"...`, "info");
      container.innerHTML = `<div style="padding: 12px; color: var(--text-dim); text-align: center;">Searching...</div>`;
      container.hidden = false;

      try {
        const results = await window.__TAURI__.core.invoke("resolve_input", { input: query });

        if (!results || results.length === 0) {
          container.innerHTML = `<div style="padding: 12px; color: var(--text-dim); text-align: center;">No results found.</div>`;
          addLog("No results found.", "warn");
          return;
        }

        addLog(`Found ${results.length} matches.`, "success");
        renderResults(results);
      } catch (e) {
        console.error("Search failed:", e);
        container.innerHTML = `<div style="padding: 12px; color: var(--danger); text-align: center;">Search error: ${e}</div>`;
        addLog(`Search error: ${e}`, "error");
      }
    }
  }

  function renderResults(results) {
    const container = $("search-results");
    if (!container) return;

    container.innerHTML = "";

    const isBulkMode = !$("panel-bulk").hidden;

    const isPlaylistSet =
      results.length > 0 &&
      results.some((r) => Boolean(r.folderName || r.folder_name));

    results.forEach((track) => {
      const item = document.createElement("div");
      item.className = "search-result-item";
      item.style.display = "flex";
      item.style.alignItems = "center";
      item.style.gap = "12px";
      item.style.padding = "10px 12px";
      item.style.cursor = "pointer";
      item.style.borderBottom = "1px solid var(--border)";

      const imgSrc = track.coverUrl || "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='48' height='48' viewBox='0 0 24 24' fill='none' stroke='%23666' stroke-width='2' stroke-linecap='round' stroke-linejoin='round'%3E%3Cpath d='M9 18V5l12-2v13'%3E%3C/path%3E%3Ccircle cx='6' cy='18' r='3'%3E%3C/circle%3E%3Ccircle cx='18' cy='16' r='3'%3E%3C/circle%3E%3C/svg%3E";

      const dur = track.durationMs
        ? `${Math.floor(track.durationMs / 60000)}:${String(Math.floor((track.durationMs % 60000) / 1000)).padStart(2, "0")}`
        : "";

      const metaParts = [track.artist];
      if (track.year) metaParts.push(track.year);
      if (dur) metaParts.push(dur);
      const metaStr = metaParts.filter(Boolean).join(" • ");

      item.innerHTML = `
        <img src="${imgSrc}" style="width:48px; height:48px; border-radius:4px; object-fit:cover;">
        <div style="flex:1; min-width:0;">
          <div style="font-weight:600; font-size:14px; white-space:nowrap; overflow:hidden; text-overflow:ellipsis;">${track.title}</div>
          <div style="font-size:12px; color:var(--text-dim); white-space:nowrap; overflow:hidden; text-overflow:ellipsis;">${metaStr}</div>
        </div>
      `;

      item.addEventListener("click", () => {
        Array.from(container.children).forEach((c) => {
          c.style.background = "transparent";
          c.style.borderLeft = "none";
        });

        item.style.background = "var(--surface-3)";
        item.style.borderLeft = "3px solid var(--accent)";

        if (isBulkMode || isPlaylistSet) {
          state.selectedTracks = results;
        } else {
          state.selectedTracks = [track];
        }

        const input = $("url-input");
        if (input) input.value = `${track.title} - ${track.artist}`;

        addLog(`Selected: ${track.title}`, "success");
        checkReady();
      });

      container.appendChild(item);
    });

    if (isBulkMode || isPlaylistSet) {
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

    if (isBulkMode && results.length > 0 && state.folder && !state.isBulkRunning) {
      state.isBulkRunning = true;
      Promise.resolve(startJob())
        .catch((e) => addLog("Auto-start failed: " + e, "error"))
        .finally(() => {
          state.isBulkRunning = false;
        });
    }
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
  }

  function checkReady() {
    const startBtn = $("start-btn");
    if (!startBtn) return;
    startBtn.disabled = !(state.folder && state.selectedTracks.length > 0);
  }

  async function startJob() {
    if (!state.folder) return addLog("Choose an output folder first.", "error");
    if (state.selectedTracks.length === 0) return addLog("Search for and pick a song first.", "error");

    const btn = $("start-btn");
    if (btn) {
      btn.disabled = true;
      btn.textContent = "Processing...";
    }

    const pill = $("status-pill");
    const pillText = $("status-text");
    if (pill && pillText) {
      pill.dataset.status = "running";
      pillText.textContent = "Running";
    }

    const bar = $("progress-bar");
    const barFill = $("bar-fill");
    if (bar && barFill) {
      bar.classList.remove("is-empty");
      barFill.style.width = "0%";
    }

    addLog(`Starting download for ${state.selectedTracks.length} track(s) [${state.mode} mode]...`, "info");

    try {
      await window.__TAURI__.core.invoke("download_track", {
        tracks: state.selectedTracks,
        outputDir: state.folder,
        mode: state.mode,   // "download" | "sync"
      });

      if (pill && pillText) {
        pill.dataset.status = "done";
        pillText.textContent = "Done";
      }

    } catch (e) {
      addLog("Download failed: " + e, "error");
      if (pill && pillText) {
        pill.dataset.status = "error";
        pillText.textContent = "Error";
      }
    } finally {
      if (btn) {
        btn.disabled = false;
        setMode();
      }
    }
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

    setupTabs();
    setMode();

    document.addEventListener("click", (e) => {
      const input = $("url-input");
      const results = $("search-results");
      const findBtn = $("find-btn");
      if (input && results && findBtn && !input.contains(e.target) && !results.contains(e.target) && !findBtn.contains(e.target)) {
        hideResults();
      }
    });

    document.getElementById("batch-resume-btn")?.addEventListener("click", async () => {
      const modal = document.getElementById("batch-pause-modal");
      if (modal) modal.style.display = "none";
      try {
        await window.__TAURI__.core.invoke("resume_batch");
        addLog("Resuming batch…", "info");
      } catch (e) {
        addLog("Could not resume batch: " + e, "error");
      }
    });

    let bulkSearchTimer = null;
    document.getElementById("urls-input")?.addEventListener("input", () => {
      clearTimeout(bulkSearchTimer);
      bulkSearchTimer = setTimeout(() => {
        const ta = document.getElementById("urls-input");
        if (ta && ta.value.trim()) triggerSearch();
      }, 800);
    });
  });
})();