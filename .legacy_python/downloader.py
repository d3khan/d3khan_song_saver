import sys
import os
import json
import shutil
import platform
from typing import Any, Optional

import yt_dlp

try:
    from cover_art import finalize_tags
except Exception:
    finalize_tags = None

try:
    from ytmusicapi import YTMusic
    _YTM_AVAILABLE = True
except Exception as e:
    print(f"WARNING: ytmusicapi not available: {e}", file=sys.stderr)
    _YTM_AVAILABLE = False

HEIGHTS = {"best": 1080, "1080": 1080, "720": 720, "480": 480, "360": 360, "240": 240}


def emit(obj):
    print(json.dumps(obj), flush=True)


def cleanup_leftovers(out_base: str) -> None:
    for ext in [".webp", ".png", ".jpg", ".temp.mp3", ".temp.mp4", ".part", ".ytdl"]:
        p = out_base + ext
        if os.path.exists(p):
            try:
                os.remove(p)
            except Exception:
                pass


def get_ffmpeg_cache_dir() -> str:
    # AV FIX: never stage an executable in %TEMP% — that pattern looks like a dropper.
    if platform.system() == "Windows":
        base = os.environ.get("LOCALAPPDATA") or os.path.expanduser("~")
        return os.path.join(base, "d3khan-song-saver", "ffmpeg")
    base = os.path.expanduser("~/.cache")
    os.makedirs(base, exist_ok=True)
    return os.path.join(base, "d3khan-song-saver")


def prepare_ffmpeg(ffmpeg_path: str) -> Optional[str]:
    if not ffmpeg_path or not os.path.exists(ffmpeg_path):
        return None
    cache_dir = get_ffmpeg_cache_dir()
    os.makedirs(cache_dir, exist_ok=True)
    name = "ffmpeg.exe" if os.name == "nt" else "ffmpeg"
    dest = os.path.join(cache_dir, name)
    if os.path.exists(dest) and os.path.getsize(dest) == os.path.getsize(ffmpeg_path):
        return cache_dir
    try:
        shutil.copy2(ffmpeg_path, dest)
        os.chmod(dest, 0o755)
        return cache_dir
    except Exception as e:
        print(f"Warning: could not stage ffmpeg: {e}", file=sys.stderr)
        return None


def build_format(media_mode: str, quality: str) -> str:
    if media_mode == "audio":
        return "bestaudio[acodec^=mp4a]/bestaudio/best"
    h = HEIGHTS.get((quality or "best").lower(), 1080)
    cap = f"[height<={h}]"
    # H.264 first = plays everywhere, NO transcoding needed.
    return (
        f"bestvideo[vcodec^=avc1]{cap}+bestaudio[acodec^=mp4a]/"
        f"best[vcodec^=avc1]{cap}/"
        f"bestvideo[vcodec^=vp09]{cap}+bestaudio[acodec^=mp4a]/"
        f"best{cap}"
    )


def find_ytmusic_video_id(query, artist="", song="", duration_s=None):
    if not _YTM_AVAILABLE or not query:
        return None
    try:
        results = YTMusic().search(query, filter="songs", limit=10)
    except Exception as e:
        print(f"ytmusic search failed: {e}", file=sys.stderr)
        return None
    if not results:
        return None

    def score(r):
        s = 0
        vt = r.get("videoType", "")
        s += 4 if vt == "MUSIC_VIDEO_TYPE_ATV" else (1 if vt == "MUSIC_VIDEO_TYPE_OMV" else 0)
        title = (r.get("title") or "").lower()
        if song and song.lower() in title:
            s += 2
        if any(w in title for w in ("live", "remix", "cover", "sped up", "slowed", "instrumental")):
            s -= 3
        names = " ".join(a.get("name", "") for a in (r.get("artists") or [])).lower()
        if artist and artist.split(",")[0].strip().lower() in names:
            s += 2
        ds = r.get("duration_seconds")
        if duration_s and ds and abs(ds - duration_s) <= 3:
            s += 2
        return s

    return max(results, key=score).get("videoId")


def resolve_target(search_query, song, artist, media_mode, duration_s=None):
    q = (search_query or "").strip()
    is_url = q.startswith("http")
    is_yt = is_url and ("youtube.com" in q or "youtu.be" in q or "music.youtube.com" in q)
    if is_yt:
        return q, True
    human = f"{artist} {song}".strip() if (artist or song) else q
    if media_mode == "video":
        return f"{human} official video", False
    vid = find_ytmusic_video_id(human, artist, song, duration_s)
    if vid:
        return f"https://music.youtube.com/watch?v={vid}", True
    return f"{human} official audio", False


def _entry(v, i=None, playlist=None, url=None):
    vid = v.get("id") or ""
    dur = v.get("duration") or 0
    t = {
        "id": f"https://www.youtube.com/watch?v={vid}",
        "title": v.get("title") or "Unknown Title",
        "artists": [v.get("uploader") or v.get("channel") or "YouTube"],
        "cover_url": f"https://i.ytimg.com/vi/{vid}/hqdefault.jpg",
        "duration_ms": int(float(dur) * 1000),
    }
    if i is not None:
        t["track_number"] = i + 1
        t["folder_name"] = playlist
        t["playlist_url"] = url
    return t


def cmd_search(query, limit):
    opts = {"quiet": True, "no_warnings": True, "extract_flat": True,
            "skip_download": True, "noplaylist": True,
            "socket_timeout": 30, "retries": 3, "force_ipv4": True}
    try:
        with yt_dlp.YoutubeDL(opts) as ydl:
            info = ydl.extract_info(f"ytsearch{limit}:{query} official audio", download=False)
        tracks = [_entry(v) for v in ((info or {}).get("entries") or []) if v and v.get("id")]
        emit({"status": "success", "tracks": tracks})
    except Exception as e:
        emit({"status": "error", "message": str(e)})


def cmd_playlist(url):
    opts = {"quiet": True, "no_warnings": True, "extract_flat": True,
            "skip_download": True, "yes_playlist": True, "ignoreerrors": True,
            "socket_timeout": 30, "retries": 3, "force_ipv4": True,
            "extractor_args": {"youtubetab": {"skip": ["webpage"]}}}
    try:
        with yt_dlp.YoutubeDL(opts) as ydl:
            info = ydl.extract_info(url, download=False)
        name = (info or {}).get("title") or "YouTube Playlist"
        entries = [v for v in ((info or {}).get("entries") or []) if v and v.get("id")]
        emit({"status": "success", "playlist_name": name,
              "tracks": [_entry(v, i, name, url) for i, v in enumerate(entries)]})
    except Exception as e:
        emit({"status": "error", "message": str(e)})


def _size_of(f, duration):
    n = f.get("filesize") or f.get("filesize_approx")
    if n:
        return int(n)
    tbr = f.get("tbr") or f.get("vbr") or f.get("abr")
    if tbr and duration:
        return int(float(tbr) * 1000 / 8 * float(duration))
    return 0


def cmd_formats(url, quality="best"):
    """Size + available qualities BEFORE downloading anything."""
    opts = {"quiet": True, "no_warnings": True, "skip_download": True,
            "noplaylist": True, "socket_timeout": 30, "force_ipv4": True}
    try:
        with yt_dlp.YoutubeDL(opts) as ydl:
            info = ydl.extract_info(url, download=False)
        dur = info.get("duration") or 0
        h = HEIGHTS.get((quality or "best").lower(), 1080)
        rows, seen = [], set()
        for f in info.get("formats") or []:
            height = f.get("height")
            if not height or f.get("vcodec") in (None, "none"):
                continue
            if height > h:
                continue
            key = (height, f.get("fps"))
            if key in seen:
                continue
            seen.add(key)
            rows.append({
                "height": height,
                "fps": f.get("fps"),
                "vcodec": (f.get("vcodec") or "").split(".")[0],
                "ext": f.get("ext"),
                "size_bytes": _size_of(f, dur),
            })
        rows.sort(key=lambda r: r["height"], reverse=True)

        best_v = next((f for f in info.get("formats") or []
                       if (f.get("vcodec") or "").startswith("avc")
                       and (f.get("height") or 0) <= h), None)
        best_a = next((f for f in info.get("formats") or []
                       if f.get("vcodec") in (None, "none")
                       and (f.get("acodec") or "") != "none"), None)
        est = _size_of(best_v or {}, dur) + _size_of(best_a or {}, dur)

        emit({"status": "success", "title": info.get("title"),
              "duration_s": dur, "estimated_bytes": est, "qualities": rows})
    except Exception as e:
        emit({"status": "error", "message": str(e)})


def download_track(search_query, out_base, ffmpeg_path, song, artist,
                   media_mode="audio", duration_ms=0, quality="best"):
    media_mode = (media_mode or "audio").lower()
    is_video = media_mode == "video"
    duration_s = duration_ms / 1000.0 if duration_ms else None
    target, is_direct = resolve_target(search_query, song, artist, media_mode, duration_s)

    def hook(d):
        if d.get("status") == "downloading":
            done = d.get("downloaded_bytes") or 0
            total = d.get("total_bytes") or d.get("total_bytes_estimate") or 0
            emit({"event": "progress",
                  "percent": round(done * 100.0 / total, 1) if total else 0,
                  "downloaded": done, "total": total,
                  "speed": d.get("speed") or 0, "eta": d.get("eta") or 0})
        elif d.get("status") == "finished":
            emit({"event": "stage", "message": "Download complete, processing…"})

    def pp_hook(d):
        if d.get("status") == "started":
            emit({"event": "stage", "message": f"Running {d.get('postprocessor')}…"})

    opts: dict[str, Any] = {
        "noplaylist": True,
        "writethumbnail": True,
        "convert_thumbnails": "jpg",
        "outtmpl": out_base.replace("%", "%%") + ".%(ext)s",
        "quiet": True, "noprogress": True,
        "socket_timeout": 30, "retries": 10,
        "force_ipv4": True, "nopart": True,
        "format": build_format(media_mode, quality),
        "progress_hooks": [hook],
        "postprocessor_hooks": [pp_hook],
    }

    if is_video:
        opts["merge_output_format"] = "mp4"
        opts["postprocessors"] = [ {"key": "FFmpegMetadata"}]
    else:
        meta_args = []
        if song:
            meta_args += ["-metadata", f"title={song}"]
        if artist:
            meta_args += ["-metadata", f"artist={artist}"]
        opts["postprocessors"] = [
            {"key": "FFmpegExtractAudio", "preferredcodec": "mp3", "preferredquality": "192"},
            {"key": "FFmpegMetadata"},
            
        ]
        if meta_args:
            opts["postprocessor_args"] = {"FFmpegMetadata": meta_args}

    if not is_direct:
        opts["default_search"] = "ytsearch1"

    cache = prepare_ffmpeg(ffmpeg_path)
    if cache:
        opts["ffmpeg_location"] = cache

    try:
        with yt_dlp.YoutubeDL(opts) as ydl:
            info = ydl.extract_info(target, download=True)
            if not info:
                cleanup_leftovers(out_base)
                emit({"status": "error", "message": "No info returned"})
                return
            base = os.path.splitext(ydl.prepare_filename(info))[0]
            exts = [".mp4", ".webm", ".mkv", ".m4a"] if is_video else [".mp3", ".m4a", ".opus", ".webm"]
            final = next((base + e for e in exts if os.path.exists(base + e)), None)
            if not final:
                cleanup_leftovers(out_base)
                emit({"status": "error", "message": "Output file not found"})
                return
            size = os.path.getsize(final)
            if size < 10000:
                cleanup_leftovers(out_base)
                try:
                    os.remove(final)
                except Exception:
                    pass
                emit({"status": "error", "message": "Output too small"})
                return
    except Exception as e:
        cleanup_leftovers(out_base)
        emit({"status": "error", "message": str(e)})
        return

    if finalize_tags:
        try:
            finalize_tags(final, info, out_base, ffmpeg_path, song, artist)
        except Exception as e:
            print(f"Warning: tagging failed: {e}", file=sys.stderr)
    cleanup_leftovers(out_base)
    emit({"status": "success", "path": final, "size_bytes": size})


def main():
    a = sys.argv[1:]
    if a and a[0] == "--search":
        cmd_search(a[1] if len(a) > 1 else "", int(a[2]) if len(a) > 2 else 10)
        return
    if a and a[0] == "--playlist":
        cmd_playlist(a[1] if len(a) > 1 else "")
        return
    if a and a[0] == "--formats":
        cmd_formats(a[1] if len(a) > 1 else "", a[2] if len(a) > 2 else "best")
        return
    if len(a) >= 3:
        download_track(
            a[0], a[1], a[2],
            a[3] if len(a) > 3 else "",
            a[4] if len(a) > 4 else "",
            a[5] if len(a) > 5 else "audio",
            int(a[6]) if len(a) > 6 else 0,
            a[7] if len(a) > 7 else "best",
        )
        return
    print("Usage: downloader <query> <out_base> <ffmpeg> [song] [artist] [mode] [duration_ms] [quality]")
    print("       downloader --search <query> <limit>")
    print("       downloader --playlist <url>")
    print("       downloader --formats <url> [quality]")


if __name__ == "__main__":
    main()