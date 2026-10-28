(function () {
  "use strict";

  const state = {
    isTauri: Boolean(window.__TAURI__ && window.__TAURI__.core),
    folder: null,
    selectedTrack: null,
    mode: "sync",
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

  function showDependencyBanner(text, isError = false) {
    let el = $("dependency-banner");
    if (!el) {
      el = document.createElement("div");
      el.id = "dependency-banner";
      el.style.position = "fixed";
      el.style.bottom = "16px";
      el.style.left = "50%";
      el.style.transform = "translateX(-50%)";
      el.style.background = "#1a1a1a";
      el.style.border = "1px solid #3a3a3a";
      el.style.borderRadius = "8px";
      el.style.padding = "10px 16px";
      el.style.color = "#f0f0f0";
      el.style.fontSize = "13px";
      el.style.zIndex = "9999";
      el.style.boxShadow = "0 8px 24px rgba(0,0,0,0.6)";
      document.body.appendChild(el);
    }
    el.textContent = text;
    el.style.borderColor = isError ? "#ff6b6b" : "#3a3a3a";
    el.style.display = "block";
    if (!isError) {
      clearTimeout(el._hideTimer);
      el._hideTimer = setTimeout(() => { el.style.display = "none"; }, 6000);
    }
  }

  if (window.__TAURI__ && window.__TAURI__.event) {
    window.__TAURI__.event.listen("dependency-status", (event) => {
      const p = event.payload;
      if (p.status === "downloading" || p.status === "updating") showDependencyBanner(p.message);
      else if (p.status === "ready") showDependencyBanner(p.message);
      else if (p.status === "error") showDependencyBanner(p.message, true);
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

  async function triggerSearch() {
    const input = $("url-input");
    if (!input) return;

    const query = input.value.trim();
    if (!query) return;

    if (!state.isTauri) return alert("Search requires the Tauri app.");

    state.selectedTrack = null;
    checkReady();

    const container = $("search-results");
    if (!container) return;

    try {
      container.innerHTML = `<div style="padding: 12px; color: var(--text-dim); text-align: center;">Searching...</div>`;
      container.hidden = false;

      const results = await window.__TAURI__.core.invoke("resolve_input", { input: query });

      if (!results || results.length === 0) {
        container.innerHTML = `<div style="padding: 12px; color: var(--text-dim); text-align: center;">No results found.</div>`;
        return;
      }
      renderResults(results);
    } catch (e) {
      console.error("Search failed:", e);
      container.innerHTML = `<div style="padding: 12px; color: var(--danger); text-align: center;">Search error: ${e}</div>`;
    }
  }

  function renderResults(results) {
    const container = $("search-results");
    if (!container) return;

    container.innerHTML = "";
    results.forEach((track) => {
      const item = document.createElement("div");
      item.className = "search-result-item";

      const dur = track.durationMs
        ? `${Math.floor(track.durationMs / 60000)}:${String(Math.floor((track.durationMs % 60000) / 1000)).padStart(2, "0")}`
        : "";

      const metaParts = [track.artist];
      if (track.year) metaParts.push(track.year);
      if (dur) metaParts.push(dur);
      const metaStr = metaParts.join(" · ");

      // Fallback image if cover_url is null
      const imgSrc = track.coverUrl || "https://i.ytimg.com/vi/default/hqdefault.jpg";

      item.innerHTML = `
        <img class="search-result-cover" src="${imgSrc}" alt="Cover" onerror="this.src='https://i.ytimg.com/vi/default/hqdefault.jpg'">
        <div class="search-result-info">
          <div class="search-result-title">${track.title}</div>
          <div class="search-result-meta">${metaStr}</div>
        </div>
      `;
      item.addEventListener("click", () => selectTrack(track));
      container.appendChild(item);
    });
    container.hidden = false;
  }

  function selectTrack(track) {
    state.selectedTrack = track;
    const input = $("url-input");
    if (input) input.value = `${track.title} - ${track.artist}`;
    hideResults();
    checkReady();
  }

  function hideResults() {
    const container = $("search-results");
    if (container) container.hidden = true;
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
      tabSingle.setAttribute("aria-selected", "true");
      tabBulk.setAttribute("aria-selected", "false");
      panelSingle.hidden = false;
      panelBulk.hidden = true;
    });

    tabBulk.addEventListener("click", () => {
      tabBulk.classList.add("is-active");
      tabSingle.classList.remove("is-active");
      tabBulk.setAttribute("aria-selected", "true");
      tabSingle.setAttribute("aria-selected", "false");
      panelBulk.hidden = false;
      panelSingle.hidden = true;
    });
  }

  function setMode() {
    const isSync = $("mode-sync")?.checked;
    state.mode = isSync ? "sync" : "download";

    const textEl = $("what-happens-text");
    const startBtn = $("start-btn");

    if (textEl) {
      if (isSync) {
        textEl.textContent = "Compares local files with YouTube, skips existing tracks, downloads new ones, and prompts to delete removed tracks.";
        if (startBtn) startBtn.textContent = "Start sync";
      } else {
        textEl.textContent = "Fetches new files and saves playlist state for a later sync — no comparisons or deletions.";
        if (startBtn) startBtn.textContent = "Start download";
      }
    }
    checkReady();
  }

  function checkReady() {
    const startBtn = $("start-btn");
    if (!startBtn) return;
    startBtn.disabled = !(state.folder && state.selectedTrack);
  }

  async function startJob() {
    if (!state.folder) return addLog("Choose an output folder first.", "error");
    if (!state.selectedTrack) return addLog("Search for and pick a song first.", "error");

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
      barFill.classList.add("is-indeterminate");
    }

    const progressTitle = $("progress-title");
    if (progressTitle) progressTitle.textContent = `Processing ${state.selectedTrack.title}...`;

    addLog(`Attempting: ${state.selectedTrack.title}`, "info");

    try {
      const result = await window.__TAURI__.core.invoke("download_track", {
        track: state.selectedTrack,
        outputDir: state.folder,
      });

      if (result.status === "skipped") {
        addLog(`Skipped: ${result.message} (${result.path})`, "warn");

        if (pill && pillText) {
          pill.dataset.status = "done";
          pillText.textContent = "Skipped";
        }

        if (bar && barFill) {
          barFill.classList.remove("is-indeterminate");
          barFill.style.width = "100%";
        }

        if (progressTitle) progressTitle.textContent = "File already existed.";
      } else {
        addLog(`Successfully saved: ${result.path}`, "success");

        if (pill && pillText) {
          pill.dataset.status = "done";
          pillText.textContent = "Done";
        }

        if (bar && barFill) {
          barFill.classList.remove("is-indeterminate");
          barFill.style.width = "100%";
        }

        if (progressTitle) progressTitle.textContent = "Download complete!";
      }

    } catch (e) {
      addLog(`Download failed: ${e}`, "error");

      if (pill && pillText) {
        pill.dataset.status = "error";
        pillText.textContent = "Error";
      }

      if (bar && barFill) {
        barFill.classList.remove("is-indeterminate");
        barFill.style.width = "0%";
      }

      if (progressTitle) progressTitle.textContent = "Download failed.";

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
  });
})();