#!/usr/bin/env bash
set -uo pipefail
cd "$(dirname "$0")" || exit 1
[ -f src/index.html ] || { echo "❌ run from repo root"; exit 1; }

python3 - <<'PY'
import pathlib, re
p = pathlib.Path('src/index.html')
s = p.read_text(encoding='utf-8')

# strip every earlier quality script/marker so we never double-bind
s = re.sub(r'<!--\s*d3khan-quality.*?-->\n?', '', s, flags=re.S)
s = re.sub(r'<script>\s*\(function\s*\(\)\s*\{[\s\S]*?populateQualityDropdown[\s\S]*?</script>\n?', '', s)
s = re.sub(r'<script>\s*//\s*Self-contained quality[\s\S]*?</script>\n?', '', s)
s = re.sub(r'<script>\s*window\.populateQualityDropdown[\s\S]*?</script>\n?', '', s)

v3 = '''<!-- d3khan-quality-v3 -->
<script>
(function () {
  var RATE = { "360": 100000, "480": 150000, "720": 325000, "1080": 600000, best: 600000 };
  window.__lastTracks = window.__lastTracks || [];

  function field() {
    return document.getElementById("quality-field") ||
           (document.getElementById("quality-select") || {}).parentElement || null;
  }
  function sel()  { return document.getElementById("quality-select"); }
  function hint() { return document.getElementById("quality-hint"); }

  function mediaButtons() {
    var out = [];
    var c = document.querySelectorAll('[data-media-mode], #media-mode-seg *, .seg button, .seg [role="radio"], .seg label');
    for (var i = 0; i < c.length; i++) {
      var t = (c[i].textContent || "").trim();
      if (/audio\\s*\\(mp3\\)|video\\s*\\(mp4\\)|^(audio|video)$/i.test(t)) out.push(c[i]);
    }
    return out.filter(function (el) {
      return !out.some(function (o) { return o !== el && el.contains(o); });
    });
  }

  function isVideo() {
    try { if (window.state && window.state.mediaMode) return window.state.mediaMode === "video"; } catch (e) {}
    var b = mediaButtons(), i;
    for (i = 0; i < b.length; i++) {
      var active = b[i].classList.contains("active") || b[i].classList.contains("selected") ||
        b[i].classList.contains("is-active") || b[i].getAttribute("aria-pressed") === "true" ||
        b[i].getAttribute("aria-selected") === "true" || b[i].hasAttribute("checked");
      if (active) return /video/i.test(b[i].textContent || "");
    }
    for (i = 0; i < b.length; i++) {           // fallback: the lit-up (white) button
      var m = (getComputedStyle(b[i]).backgroundColor || "").match(/rgba?\\((\\d+),\\s*(\\d+),\\s*(\\d+)/);
      if (m && (+m[1] + +m[2] + +m[3]) > 600) return /video/i.test(b[i].textContent || "");
    }
    return false;
  }

  function estimateTotal(tracks, quality) {
    var rate = RATE[String(quality === "best" ? 1080 : quality)] || RATE.best, bytes = 0;
    (tracks || []).forEach(function (t) { bytes += ((t.durationMs || t.duration_ms || 0) / 1000) * rate; });
    return bytes;
  }

  function refreshTotal() {
    var s = sel(), h = hint();
    if (!s || !h) return;
    var n = (window.__lastTracks || []).length;
    h.textContent = n
      ? n + " track(s) · ≈ " + (estimateTotal(window.__lastTracks, s.value) / 1048576).toFixed(1) +
        " MB total at " + (s.value === "best" ? "1080p" : s.value + "p")
      : "Select tracks to see sizes.";
  }

  function apply() {
    var f = field(), s = sel();
    if (!f) return;
    var video = isVideo();
    var has = (window.__lastTracks || []).length > 0;
    f.style.display = video ? "" : "none";      // VIDEO MODE ONLY
    if (s) s.disabled = !has;
    if (video && s && !s.disabled) refreshTotal();
  }
  window.__applyQualityVisibility = apply;

  window.populateQualityDropdown = function () {
    var s = sel();
    if (!s) return;
    if (!s.options.length || (s.options.length === 1 && s.options[0].value === "best")) {
      s.innerHTML =
        '<option value="best">Best (1080p, H.264)</option>' +
        '<option value="1080">1080p</option><option value="720">720p</option>' +
        '<option value="480">480p</option><option value="360">360p</option>';
    }
    apply();
  };

  // Intercept resolve/search (single AND bulk) so the dropdown fills + totals update.
  (function hook() {
    if (!window.__TAURI__ || !window.__TAURI__.core) { setTimeout(hook, 50); return; }
    var orig = window.__TAURI__.core.invoke.bind(window.__TAURI__.core);
    window.__TAURI__.core.invoke = function (cmd, args) {
      if (cmd === "download_track" && args && typeof args === "object" && isVideo()) {
        var s = sel();
        if (s && !s.disabled) args.quality = s.value;
      }
      return orig(cmd, args).then(function (res) {
        if ((cmd === "resolve_input" || cmd === "resolve_bulk_input" || cmd === "search_youtube") &&
            Array.isArray(res) && res.length) {
          window.__lastTracks = res;
          window.populateQualityDropdown(res[0]);
        }
        return res;
      });
    };
  })();

  if (typeof MutationObserver !== "undefined") {
    new MutationObserver(function () { apply(); }).observe(document.body, {
      subtree: true, childList: true, attributes: true,
      attributeFilter: ["class", "aria-pressed", "aria-selected", "checked"]
    });
  }
  document.addEventListener("click",  function () { setTimeout(apply, 0); }, true);
  document.addEventListener("change", function () { setTimeout(apply, 0); }, true);
  setInterval(apply, 800);
  apply();
})();
</script>
'''
s = s.replace('</body>', v3 + '</body>', 1)
p.write_text(s, encoding='utf-8')
print('✅ quality dropdown v3 installed (video-mode only, single + bulk)')
PY

git add -A
git commit -m "ui: quality dropdown shows only in Video mode, works in single + bulk tabs"
retry(){ for i in 1 2 3 4 5; do "$@" && return 0; echo "retry $i..."; sleep 4; done; return 1; }
retry git push
echo "✅ pushed — reload the app window (Ctrl+R); no recompile needed"