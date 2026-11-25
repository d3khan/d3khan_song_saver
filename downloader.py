import sys
import os
import json
import tempfile
import shutil
from typing import Any, Optional
from urllib.parse import quote

import requests
import yt_dlp

try:
    from ytmusicapi import YTMusic
    _YTM_AVAILABLE = True
except Exception:
    _YTM_AVAILABLE = False


def parse_spotify_title(title: Optional[str]) -> tuple[str, str]:
    if not title:
        return "", ""
    if " - " in title:
        artist, track = title.split(" - ", 1)
        return artist.strip(), track.strip()
    return "", title.strip()


def cleanup_leftovers(out_base: str) -> None:
    for ext in [".webp", ".png", ".temp.mp3", ".temp.mp4", ".part", ".ytdl"]:
        leftover = out_base + ext
        if os.path.exists(leftover):
            try:
                os.remove(leftover)
            except Exception:
                pass


def find_ytmusic_video_id(query: str) -> Optional[str]:
    """Search the YouTube Music catalog for the best official track."""
    if not _YTM_AVAILABLE or not query:
        return None
    try:
        ytm = YTMusic()
        results = ytm.search(query, filter="songs", limit=5)
        if results:
            vid = results[0].get("videoId")
            if vid:
                return vid
    except Exception:
        return None
    return None


def resolve_target(search_query: str, song: str, artist: str, media_mode: str) -> tuple[str, bool]:
    """
    Returns (target, is_direct_url).
    - YouTube / YouTube Music links -> pass through directly.
    - Audio + non-YouTube -> try ytmusicapi for an official track.
    - Video + non-YouTube -> ALWAYS append 'official video'.
    - Fallback Audio -> ALWAYS append 'official audio'.
    """
    q = search_query.strip()
    is_url = q.startswith("http")
    is_yt = is_url and ("youtube.com" in q or "youtu.be" in q or "music.youtube.com" in q)

    if is_yt:
        return q, True

    # Build a human search string from song/artist when available
    human_query = f"{artist} {song}".strip() if (artist or song) else q

    if media_mode == "video":
        # ALWAYS append "official video" for video mode
        return f"{human_query} official video", False

    # Audio mode: prefer YouTube Music official track
    vid = find_ytmusic_video_id(human_query)
    if vid:
        return f"https://music.youtube.com/watch?v={vid}", True

    # ALWAYS append "official audio" to the fallback search to ensure accuracy
    return f"{human_query} official audio", False


def download_track(
    search_query: str,
    out_base: str,
    ffmpeg_path: str,
    song: str,
    artist: str,
    media_mode: str = "audio",
) -> None:
    media_mode = (media_mode or "audio").lower()
    is_video = media_mode == "video"

    target, is_direct_url = resolve_target(search_query, song, artist, media_mode)

    ydl_opts: dict[str, Any]

    if is_video:
        ydl_opts = {
            "format": "bestvideo[ext=mp4]+bestaudio[ext=m4a]/best[ext=mp4]/best",
            "merge_output_format": "mp4",
            "noplaylist": True,
            "writethumbnail": True,
            "convert_thumbnails": "jpg",
            "outtmpl": out_base + ".%(ext)s",
            "quiet": True,
            "socket_timeout": 30,
            "retries": 10,
            "nocheckcertificate": True,
            "force_ipv4": True,
            "nopart": True,
            "extractor_args": {"youtube": {"player_client": ["android", "ios"]}},
            "postprocessors": [
                {"key": "EmbedThumbnail"},
                {"key": "FFmpegMetadata"},
            ],
        }
    else:
        metadata_args: list[str] = []
        if song:
            metadata_args += ["-metadata", f"title={song}"]
        if artist:
            metadata_args += ["-metadata", f"artist={artist}"]

        ydl_opts = {
            "format": "bestaudio/best",
            "noplaylist": True,
            "writethumbnail": True,
            "convert_thumbnails": "jpg",
            "outtmpl": out_base + ".%(ext)s",
            "quiet": True,
            "socket_timeout": 30,
            "retries": 10,
            "nocheckcertificate": True,
            "force_ipv4": True,
            "nopart": True,
            "extractor_args": {"youtube": {"player_client": ["android", "ios"]}},
            "postprocessors": [
                {
                    "key": "FFmpegExtractAudio",
                    "preferredcodec": "mp3",
                    "preferredquality": "192",
                },
                {"key": "FFmpegMetadata"},
                {"key": "EmbedThumbnail"},
            ],
        }
        if metadata_args:
            ydl_opts["postprocessor_args"] = {"FFmpegMetadata": metadata_args}

    if not is_direct_url:
        ydl_opts["default_search"] = "ytsearch1"

    # FIX: yt-dlp strictly requires the binary to be named 'ffmpeg' or 'ffmpeg.exe'.
    # We copy our bundled binary to a temp folder with the correct name.
    if ffmpeg_path and os.path.exists(ffmpeg_path):
        temp_dir = tempfile.mkdtemp()
        target_name = "ffmpeg.exe" if os.name == "nt" else "ffmpeg"
        dest_path = os.path.join(temp_dir, target_name)
        
        try:
            shutil.copy2(ffmpeg_path, dest_path)
            os.chmod(dest_path, 0o755)  # Ensure it's executable
            ydl_opts["ffmpeg_location"] = temp_dir
        except Exception as e:
            print(f"Warning: Could not prepare bundled ffmpeg: {e}", file=sys.stderr)
            pass

    final_file = None
    try:
        with yt_dlp.YoutubeDL(ydl_opts) as ydl:  # type: ignore[arg-type]
            info = ydl.extract_info(target, download=True)
            if not info:
                cleanup_leftovers(out_base)
                print(json.dumps({"status": "error", "message": "No info returned from yt-dlp"}))
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

            size = os.path.getsize(final_file)
            if size < 10_000:
                cleanup_leftovers(out_base)
                try:
                    os.remove(final_file)
                except Exception:
                    pass
                print(json.dumps({"status": "error", "message": f"Output too small ({size} bytes) — likely a failed download"}))
                return
    except Exception as e:
        cleanup_leftovers(out_base)
        print(json.dumps({"status": "error", "message": str(e)}))
        return

    cleanup_leftovers(out_base)
    print(json.dumps({"status": "success", "path": final_file}))


def main() -> None:
    if len(sys.argv) >= 4:
        query = sys.argv[1]
        out_base = sys.argv[2]
        ffmpeg_path = sys.argv[3]
        song = sys.argv[4] if len(sys.argv) > 4 else ""
        artist = sys.argv[5] if len(sys.argv) > 5 else ""
        media_mode = sys.argv[6] if len(sys.argv) > 6 else "audio"
        download_track(query, out_base, ffmpeg_path, song, artist, media_mode)
    else:
        print("=== Manual Test Mode ===")
        url = input("Enter URL: ").strip()
        ffmpeg = input("Enter FFmpeg Path: ").strip()
        encoded_url = quote(url)
        oembed_url = f"https://open.spotify.com/oembed?url={encoded_url}"
        headers = {"User-Agent": "Mozilla/5.0 (Windows NT 10.0; Win64; x64)"}
        response = requests.get(oembed_url, headers=headers, timeout=30)
        data = response.json()
        track_title = data.get("title") or ""
        artist, song = parse_spotify_title(track_title)
        search_query = f"{artist} {song}".strip() if artist else track_title
        out_dir = os.path.join(os.getcwd(), "TestDownloads")
        os.makedirs(out_dir, exist_ok=True)
        out_base = os.path.join(out_dir, f"{song} - {artist}")
        download_track(search_query, out_base, ffmpeg, song, artist, "audio")


if __name__ == "__main__":
    main()