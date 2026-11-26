import sys
import os
import json
import shutil
import platform
from typing import Any, Optional
from urllib.parse import quote

import requests
import yt_dlp

try:
    from ytmusicapi import YTMusic
    _YTM_AVAILABLE = True
except Exception as e:
    print(f"WARNING: ytmusicapi not available: {e}", file=sys.stderr)
    _YTM_AVAILABLE = False


def parse_spotify_title(title: Optional[str]) -> tuple[str, str]:
    if not title:
        return "", ""
    if " - " in title:
        artist, track = title.split(" - ", 1)
        return artist.strip(), track.strip()
    return "", title.strip()


def cleanup_leftovers(out_base: str) -> None:
    for ext in [".webp", ".png", ".jpg", ".temp.mp3", ".temp.mp4", ".part", ".ytdl"]:
        leftover = out_base + ext
        if os.path.exists(leftover):
            try:
                os.remove(leftover)
            except Exception:
                pass


def get_ffmpeg_cache_dir() -> str:
    if platform.system() == "Windows":
        base = os.environ.get("TEMP", os.getcwd())
        return os.path.join(base, "d3khan-ffmpeg-cache")
    base = os.path.expanduser("~/.cache")
    os.makedirs(base, exist_ok=True)
    return os.path.join(base, "d3khan-song-saver")


def prepare_ffmpeg(ffmpeg_path: str) -> Optional[str]:
    if not ffmpeg_path or not os.path.exists(ffmpeg_path):
        return None
    cache_dir = get_ffmpeg_cache_dir()
    os.makedirs(cache_dir, exist_ok=True)
    target_name = "ffmpeg.exe" if os.name == "nt" else "ffmpeg"
    dest_path = os.path.join(cache_dir, target_name)
    if os.path.exists(dest_path) and os.path.getsize(dest_path) == os.path.getsize(ffmpeg_path):
        return cache_dir
    try:
        shutil.copy2(ffmpeg_path, dest_path)
        os.chmod(dest_path, 0o755)
        return cache_dir
    except Exception as e:
        print(f"Warning: Could not prepare bundled ffmpeg: {e}", file=sys.stderr)
        return None


def find_ytmusic_video_id(query: str, artist: str = "", song: str = "",
                          duration_s: float = None) -> Optional[str]:
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
        names = " ".join(a.get("name", "") for a in r.get("artists") or []).lower()
        if artist and artist.split(",")[0].strip().lower() in names:
            s += 2
        ds = r.get("duration_seconds")
        if duration_s and ds and abs(ds - duration_s) <= 3:
            s += 2
        return s

    return max(results, key=score).get("videoId")


def resolve_target(search_query: str, song: str, artist: str, media_mode: str,
                   duration_s: float = None) -> tuple[str, bool]:
    q = search_query.strip()
    is_url = q.startswith("http")
    is_yt = is_url and ("youtube.com" in q or "youtu.be" in q or "music.youtube.com" in q)
    if is_yt:
        return q, True
    human_query = f"{artist} {song}".strip() if (artist or song) else q
    if media_mode == "video":
        return f"{human_query} official video", False
    vid = find_ytmusic_video_id(human_query, artist, song, duration_s)
    if vid:
        return f"https://music.youtube.com/watch?v={vid}", True
    return f"{human_query} official audio", False


# ---------- NEW: search mode (uses frozen yt_dlp module, NO external binary) ----------
def cmd_search(query: str, limit: int) -> None:
    opts = {
        "quiet": True, "no_warnings": True, "extract_flat": True,
        "skip_download": True, "noplaylist": True,
        "socket_timeout": 30, "retries": 3, "force_ipv4": True,
    }
    target = f"ytsearch{limit}:{query} official audio"
    try:
        with yt_dlp.YoutubeDL(opts) as ydl:
            info = ydl.extract_info(target, download=False)
        tracks = []
        for v in (info or {}).get("entries") or []:
            if not v:
                continue
            vid = v.get("id") or ""
            if not vid:
                continue
            dur = v.get("duration") or 0
            tracks.append({
                "id": f"https://www.youtube.com/watch?v={vid}",
                "title": v.get("title") or "Unknown Title",
                "artists": [v.get("uploader") or v.get("channel") or "YouTube"],
                "cover_url": f"https://i.ytimg.com/vi/{vid}/hqdefault.jpg",
                "duration_ms": int(float(dur) * 1000),
            })
        print(json.dumps({"status": "success", "tracks": tracks}))
    except Exception as e:
        print(json.dumps({"status": "error", "message": str(e)}))


# ---------- NEW: playlist mode (uses frozen yt_dlp module, NO external binary) ----------
def cmd_playlist(url: str) -> None:
    opts = {
        "quiet": True, "no_warnings": True, "extract_flat": True,
        "skip_download": True, "yes_playlist": True, "ignoreerrors": True,
        "socket_timeout": 30, "retries": 3, "force_ipv4": True,
        "extractor_args": {"youtubetab": {"skip": ["webpage"]}},
    }
    try:
        with yt_dlp.YoutubeDL(opts) as ydl:
            info = ydl.extract_info(url, download=False)
        name = (info or {}).get("title") or "YouTube Playlist"
        tracks = []
        for i, v in enumerate((info or {}).get("entries") or []):
            if not v:
                continue
            vid = v.get("id") or ""
            if not vid:
                continue
            dur = v.get("duration") or 0
            tracks.append({
                "id": f"https://www.youtube.com/watch?v={vid}",
                "title": v.get("title") or "Unknown Title",
                "artists": [v.get("uploader") or v.get("channel") or "YouTube"],
                "cover_url": f"https://i.ytimg.com/vi/{vid}/hqdefault.jpg",
                "duration_ms": int(float(dur) * 1000),
                "track_number": i + 1,
                "folder_name": name,
                "playlist_url": url,
            })
        print(json.dumps({"status": "success", "playlist_name": name, "tracks": tracks}))
    except Exception as e:
        print(json.dumps({"status": "error", "message": str(e)}))


def download_track(search_query: str, out_base: str, ffmpeg_path: str, song: str,
                   artist: str, media_mode: str = "audio", duration_ms: int = 0) -> None:
    media_mode = (media_mode or "audio").lower()
    is_video = media_mode == "video"
    duration_s = duration_ms / 1000.0 if duration_ms else None
    target, is_direct_url = resolve_target(search_query, song, artist, media_mode, duration_s)

    ydl_opts: dict[str, Any] = {
        "noplaylist": True,
        "writethumbnail": True,
        "convert_thumbnails": "jpg",
        "outtmpl": out_base.replace("%", "%%") + ".%(ext)s",
        "quiet": True,
        "socket_timeout": 30,
        "retries": 10,
        "force_ipv4": True,
        "nopart": True,
    }

    if is_video:
        ydl_opts["format"] = "bestvideo[ext=mp4]+bestaudio[ext=m4a]/best[ext=mp4]/best"
        ydl_opts["merge_output_format"] = "mp4"
        ydl_opts["postprocessors"] = [{"key": "EmbedThumbnail"}, {"key": "FFmpegMetadata"}]
    else:
        metadata_args = []
        if song:
            metadata_args += ["-metadata", f"title={song}"]
        if artist:
            metadata_args += ["-metadata", f"artist={artist}"]
        ydl_opts["format"] = "bestaudio/best"
        ydl_opts["postprocessors"] = [
            {"key": "FFmpegExtractAudio", "preferredcodec": "mp3", "preferredquality": "192"},
            {"key": "FFmpegMetadata"},
            {"key": "EmbedThumbnail"},
        ]
        if metadata_args:
            ydl_opts["postprocessor_args"] = {"FFmpegMetadata": metadata_args}

    if not is_direct_url:
        ydl_opts["default_search"] = "ytsearch1"

    ffmpeg_cache = prepare_ffmpeg(ffmpeg_path)
    if ffmpeg_cache:
        ydl_opts["ffmpeg_location"] = ffmpeg_cache

    final_file = None
    try:
        with yt_dlp.YoutubeDL(ydl_opts) as ydl:
            info = ydl.extract_info(target, download=True)
            if not info:
                cleanup_leftovers(out_base)
                print(json.dumps({"status": "error", "message": "No info returned"}))
                return
            filename = ydl.prepare_filename(info)
            base = os.path.splitext(filename)[0]
            possible_exts = [".mp4", ".webm", ".mkv", ".m4a"] if is_video else [".mp3", ".m4a", ".opus", ".webm"]
            for ext in possible_exts:
                candidate = base + ext
                if os.path.exists(candidate):
                    final_file = candidate
                    break
            if not final_file:
                final_file = filename
            if not os.path.exists(final_file):
                cleanup_leftovers(out_base)
                print(json.dumps({"status": "error", "message": f"File not found: {final_file}"}))
                return
            if os.path.getsize(final_file) < 10_000:
                cleanup_leftovers(out_base)
                try:
                    os.remove(final_file)
                except Exception:
                    pass
                print(json.dumps({"status": "error", "message": "Output too small"}))
                return
    except Exception as e:
        cleanup_leftovers(out_base)
        print(json.dumps({"status": "error", "message": str(e)}))
        return

    cleanup_leftovers(out_base)
    print(json.dumps({"status": "success", "path": final_file}))


def main() -> None:
    args = sys.argv[1:]
    if args and args[0] == "--search":
        cmd_search(args[1] if len(args) > 1 else "", int(args[2]) if len(args) > 2 else 10)
        return
    if args and args[0] == "--playlist":
        cmd_playlist(args[1] if len(args) > 1 else "")
        return
    if len(args) >= 3:
        query, out_base, ffmpeg_path = args[0], args[1], args[2]
        song = args[3] if len(args) > 3 else ""
        artist = args[4] if len(args) > 4 else ""
        media_mode = args[5] if len(args) > 5 else "audio"
        duration_ms = int(args[6]) if len(args) > 6 else 0
        download_track(query, out_base, ffmpeg_path, song, artist, media_mode, duration_ms)
    else:
        print("Usage: downloader <query> <out_base> <ffmpeg> [song] [artist] [mode] [duration_ms]")
        print("       downloader --search <query> <limit>")
        print("       downloader --playlist <url>")


if __name__ == "__main__":
    main()