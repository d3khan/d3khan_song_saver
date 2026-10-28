(function () {
  "use strict";

  const state = {
    isTauri: Boolean(window.__TAURI__ && window.__TAURI__.core),
    folder: null,
    selectedTracks: [],
    mode: "download",
  };

  const $ = (id) => document.getElementById(id);

  // ---- Activity Log ----
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

  // ---- Tauri Events ----
  if (window.__TAURI__ && window.__TAURI__.event) {
    window.__TAURI__.event.listen("download-progress", (event) => {
      const { current, total, title, status } = event.payload;

      const progressTitle = $("progress-title");
      if (progressTitle) progressTitle.textContent = `[${current}/${total}] ${title}: ${status}`;

      const barFill = $("bar-fill");
      if (barFill) {
        barFill.style.width = `${(current / total) * 100}%`;
        barFill.classList.remove("is-indeterminate");
      }

      if (status.includes("Waiting")) addLog(`Waiting 5s before downloading ${title}...`, "warn");
      if (status.includes("Downloading")) addLog(`Downloading ${title}...`, "info");
      if (status.includes("Tagging")) addLog(`Embedding metadata for ${title}...`, "info");
    });

    window.__TAURI__.event.listen("download-error", (event) => {
      addLog(`Error: ${event.payload}`, "error");
    });
  }

  // ---- Folder Picker ----
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

  // ---- Tabs ----
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

  // ---- Search ----
  async function triggerSearch() {
    const input = $("url-input");
    if (!input) return;

    const query = input.value.trim();
    if (!query) return;
    if (!state.isTauri) return alert("Search requires the Tauri app.");

    state.selectedTracks = [];
    checkReady();
    addLog(`Searching: "${query}"...`, "info");

    const container = $("search-results");
    if (!container) return;

    try {
      container.innerHTML = `<div style="padding: 12px; color: var(--text-dim); text-align: center;">Searching...</div>`;
      container.hidden = false;

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

  function renderResults(results) {
    const container = $("search-results");
    if (!container) return;

    container.innerHTML = "";

    results.forEach(track => {
      const item = document.createElement("div");
      item.className = "search-result-item";
      item.style.display = "flex";
      item.style.alignItems = "center";
      item.style.gap = "12px";
      item.style.padding = "10px 12px";
      item.style.cursor = "pointer";
      item.style.borderBottom = "1px solid var(--border)";

      const imgSrc = track.coverUrl || "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='48' height='48' viewBox='0 0 24 24' fill='none' stroke='%23666' stroke-width='2' stroke-linecap='round' stroke-linejoin='round'%3E%3Cpath d='M9 18V5l12-2v13'%3E%3C/path%3E%3Ccircle cx='6' cy='18' r='3'%3E%3C/circle%3E%3Ccircle cx='18' cy='16' r='3'%3E%3C/circle%3E%3C/svg%3E";

      let durStr = "";
      if (track.durationMs > 0) {
        const mins = Math.floor(track.durationMs / 60000);
        const secs = Math.floor((track.durationMs % 60000) / 1000);
        durStr = `${mins}:${secs.toString().padStart(2, "0")}`;
      }

      const metaParts = [track.artist];
      if (track.year) metaParts.push(track.year);
      if (durStr) metaParts.push(durStr);
      const metaStr = metaParts.join(" • ");

      item.innerHTML = `
        <img src="${imgSrc}" style="width:48px; height:48px; border-radius:4px; object-fit:cover;">
        <div style="flex:1; min-width:0;">
          <div style="font-weight:600; font-size:14px; white-space:nowrap; overflow:hidden; text-overflow:ellipsis;">${track.title}</div>
          <div style="font-size:12px; color:var(--text-dim); white-space:nowrap; overflow:hidden; text-overflow:ellipsis;">${metaStr}</div>
        </div>
      `;

      item.addEventListener("click", () => {
        Array.from(container.children).forEach(c => {
          c.style.background = "transparent";
          c.style.borderLeft = "none";
        });

        item.style.background = "var(--surface-3)";
        item.style.borderLeft = "3px solid var(--accent)";

        state.selectedTracks = [track];
        const input = $("url-input");
        if (input) input.value = `${track.title} - ${track.artist}`;

        addLog(`Selected: ${track.title}`, "success");
        checkReady();
      });

      container.appendChild(item);
    });

    container.hidden = false;
  }

  function hideResults() {
    const container = $("search-results");
    if (container) container.hidden = true;
  }

  // ---- Modes & Ready State ----
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
        textEl.textContent = "Downloads all selected files directly.";
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

  // ---- Start Download ----
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

    addLog(`Starting download for ${state.selectedTracks.length} track(s)...`, "info");

    try {
      // THIS NOW WAITS FOR THE ACTUAL DOWNLOAD TO FINISH
      await window.__TAURI__.core.invoke("download_track", {
        tracks: state.selectedTracks,
        outputDir: state.folder,
      });

      if (pill && pillText) {
        pill.dataset.status = "done";
        pillText.textContent = "Done";
      }
      const progressTitle = $("progress-title");
      if (progressTitle) progressTitle.textContent = "Download complete!";
      addLog("All downloads finished.", "success");

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

  // ---- Wire up ----
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
  });
})();