(function () {
  "use strict";

  const state = {
    isTauri: Boolean(window.__TAURI__ && window.__TAURI__.core),
    folder: null,
    searchTimer: null,
    selectedTrack: null,
    mode: "sync", // Default from HTML is sync
  };

  const $ = (id) => document.getElementById(id);

  // ---- 1. Folder Picker ----
  async function browseFolder() {
    if (!state.isTauri) return alert("Folder picker works only in the Tauri app.");
    try {
      const path = await window.__TAURI__.core.invoke("pick_folder");
      if (path) {
        state.folder = path;
        const placeholder = $("folder-placeholder");
        const value = $("folder-value");

        // Update the UI correctly based on your HTML
        if (placeholder && value) {
          placeholder.hidden = true;
          value.textContent = path;
          value.hidden = false;
        }
        checkReady();
      }
    } catch (e) {
      alert("Could not open folder picker: " + e);
    }
  }

  // ---- 2. Live Search Popup ----
  function onSearchInput(e) {
    let query = e.target.value.trim();
    clearTimeout(state.searchTimer);
    state.selectedTrack = null;
    checkReady();

    if (query.length < 2) {
      hideResults();
      return;
    }

    // --- "by" Parsing Logic ---
    // If the user types "Song Title by Artist", rearrange it to "Artist - Song Title" for a better search
    const byIndex = query.toLowerCase().indexOf(" by ");
    if (byIndex !== -1) {
      const title = query.substring(0, byIndex).trim();
      const artist = query.substring(byIndex + 4).trim();
      query = `${artist} - ${title}`; // Swap to "Artist - Title"
    }

    // Wait 300ms after typing stops to avoid spamming the API
    state.searchTimer = setTimeout(() => runSearch(query), 300);
  }

  async function runSearch(query) {
    if (!state.isTauri) return;
    const container = $("search-results");
    if (!container) return;

    try {
      container.innerHTML = `<div style="padding: 12px; color: var(--text-dim);">Searching...</div>`;
      container.hidden = false;

      const results = await window.__TAURI__.core.invoke("search_tracks", { query });

      if (!results || results.length === 0) {
        container.innerHTML = `<div style="padding: 12px; color: var(--text-dim);">No results found.</div>`;
        return;
      }
      renderResults(results);
    } catch (e) {
      console.error("Search failed:", e);
      container.innerHTML = `<div style="padding: 12px; color: var(--danger);">Search error: ${e}</div>`;
    }
  }

  function renderResults(results) {
    const container = $("search-results");
    if (!container) return;

    container.innerHTML = "";
    results.forEach((track) => {
      const item = document.createElement("div");
      item.className = "search-result-item";
      item.innerHTML = `
        <img class="search-result-cover" src="${track.coverUrl || ""}" alt="" onerror="this.style.display='none'">
        <div class="search-result-info">
          <div class="search-result-title">${track.title}</div>
          <div class="search-result-meta">${track.artist}${track.year ? " · " + track.year : ""}</div>
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

  // ---- 3. Tab Switching (Single vs Bulk) ----
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

  // ---- 4. Mode Switching (Download vs Sync) ----
  function setMode() {
    const isSync = $("mode-sync")?.checked;
    state.mode = isSync ? "sync" : "download";

    const textEl = $("what-happens-text");
    const startBtn = $("start-btn");

    if (textEl) {
      if (isSync) {
        textEl.textContent = "Compares local files with playlist, skips existing tracks, downloads new ones, and prompts to delete removed tracks. Note: This mode needs a link";
        if (startBtn) startBtn.textContent = "Start sync";
      } else {
        textEl.textContent = "Fetches new files and saves playlist state for a later sync — no comparisons or deletions.";
        if (startBtn) startBtn.textContent = "Start download";
      }
    }
    checkReady();
  }

  // ---- 5. Ready State & Start Job ----
  function checkReady() {
    const startBtn = $("start-btn");
    if (!startBtn) return;

    // Enable button only if a folder is picked AND a track is selected
    if (state.folder && state.selectedTrack) {
      startBtn.disabled = false;
    } else {
      startBtn.disabled = true;
    }
  }

  async function startJob() {
    if (!state.folder) return alert("Choose an output folder first.");
    if (!state.selectedTrack) return alert("Search for and pick a song first.");

    const btn = $("start-btn");
    if (btn) {
      btn.disabled = true;
      btn.textContent = "Processing...";
    }

    try {
      // Set status pill to running
      const pill = $("status-pill");
      const pillText = $("status-text");
      if (pill && pillText) {
        pill.dataset.status = "running";
        pillText.textContent = "Running";
      }

      const savedPath = await window.__TAURI__.core.invoke("download_track", {
        track: state.selectedTrack,
        outputDir: state.folder,
      });

      alert("Successfully saved to:\n" + savedPath);

      // Reset status
      if (pill && pillText) {
        pill.dataset.status = "done";
        pillText.textContent = "Done";
      }
    } catch (e) {
      alert("Download failed:\n" + e);
      const pill = $("status-pill");
      const pillText = $("status-text");
      if (pill && pillText) {
        pill.dataset.status = "error";
        pillText.textContent = "Error";
      }
    } finally {
      if (btn) {
        btn.disabled = false;
        setMode(); // Restore button text
      }
    }
  }

  // ---- Wire up everything on load ----
  document.addEventListener("DOMContentLoaded", () => {
    $("browse-btn")?.addEventListener("click", browseFolder);
    $("url-input")?.addEventListener("input", onSearchInput);
    $("start-btn")?.addEventListener("click", startJob);

    // Listen for radio button changes
    $("mode-download")?.addEventListener("change", setMode);
    $("mode-sync")?.addEventListener("change", setMode);

    setupTabs();
    setMode(); // Initialize "What happens" text

    // Hide search popup when clicking outside
    document.addEventListener("click", (e) => {
      const input = $("url-input");
      const results = $("search-results");
      if (input && results && !input.contains(e.target) && !results.contains(e.target)) {
        hideResults();
      }
    });
  });
})();