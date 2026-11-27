"""Sidecar entry point 1. CLI contract is unchanged from the old downloader.py.

  downloader --search   <query> <limit>
  downloader --playlist <url>
  downloader --formats  <url> [quality]
  downloader <query> <out_base> <ffmpeg> [song] [artist] [mode] [duration_ms] [quality]
"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import yt_dlp

from core import ffmpeg, formats, tagging, util
from core.events import fail, pp_hook, progress_hook, success, warn
from providers import resolve_media, youtube


def cmd_search(query, limit):
    try:
        success(tracks=youtube.search(query, limit))
    except Exception as e:
        fail(e)


def cmd_playlist(url):
    try:
        name, tracks = youtube.playlist(url)
        success(playlist_name=name, tracks=tracks)
    except Exception as e:
        fail(e)


def cmd_formats(url, quality="best"):
    try:
        info = youtube.formats(url)
        dur = info.get("duration") or 0
        cap = formats.height_cap(quality)
        rows, seen = [], set()
        for f in info.get("formats") or []:
            h = f.get("height")
            if not h or f.get("vcodec") in (None, "none") or h > cap:
                continue
            key = (h, f.get("fps"))
            if key in seen:
                continue
            seen.add(key)
            rows.append({"height": h, "fps": f.get("fps"),
                         "vcodec": (f.get("vcodec") or "").split(".")[0],
                         "ext": f.get("ext"),
                         "size_bytes": formats.size_of(f, dur)})
        rows.sort(key=lambda r: r["height"], reverse=True)

        v = next((f for f in info.get("formats") or []
                  if (f.get("vcodec") or "").startswith("avc")
                  and (f.get("height") or 0) <= cap), None)
        a = next((f for f in info.get("formats") or []
                  if f.get("vcodec") in (None, "none")
                  and (f.get("acodec") or "") != "none"), None)
        success(title=info.get("title"), duration_s=dur,
                estimated_bytes=formats.size_of(v or {}, dur) + formats.size_of(a or {}, dur),
                qualities=rows)
    except Exception as e:
        fail(e)


def download(query, out_base, ffmpeg_path, song="", artist="",
             media_mode="audio", duration_ms=0, quality="best"):
    media_mode = (media_mode or "audio").lower()
    is_video = media_mode == "video"
    duration_s = duration_ms / 1000.0 if duration_ms else None
    target, is_direct = resolve_media(query, song, artist, media_mode, duration_s)

    opts = {
        "noplaylist": True,
        "writethumbnail": True,
        "convert_thumbnails": "jpg",
        "outtmpl": util.safe_outtmpl(out_base),
        "quiet": True, "noprogress": True,
        "socket_timeout": 30, "retries": 10,
        "force_ipv4": True, "nopart": True,
        "format": formats.build_format(media_mode, quality),
        "progress_hooks": [progress_hook],
        "postprocessor_hooks": [pp_hook],
    }
    if is_video:
        opts["merge_output_format"] = "mp4"
        opts["postprocessors"] = []
    else:
        opts["postprocessors"] = [
            {"key": "FFmpegExtractAudio", "preferredcodec": "mp3",
             "preferredquality": "192"},
        ]
    if not is_direct:
        opts["default_search"] = "ytsearch1"

    staged = ffmpeg.prepare(ffmpeg_path)
    if staged:
        opts["ffmpeg_location"] = staged

    try:
        with yt_dlp.YoutubeDL(opts) as ydl:
            info = ydl.extract_info(target, download=True)
            if not info:
                util.cleanup_leftovers(out_base)
                return fail("No info returned")
            base = os.path.splitext(ydl.prepare_filename(info))[0]
            exts = ([".mp4", ".webm", ".mkv", ".m4a"] if is_video
                    else [".mp3", ".m4a", ".opus", ".webm"])
            final = next((base + e for e in exts if os.path.exists(base + e)), None)
            if not final:
                util.cleanup_leftovers(out_base)
                return fail("Output file not found")
            size = os.path.getsize(final)
            if size < 10000:
                util.cleanup_leftovers(out_base)
                try:
                    os.remove(final)
                except Exception:
                    pass
                return fail("Output too small")
    except Exception as e:
        util.cleanup_leftovers(out_base)
        return fail(e)

    try:
        tagging.finalize(final, info, out_base, ffmpeg_path, song, artist)
    except Exception as e:
        warn(f"tagging failed: {e}")

    util.cleanup_leftovers(out_base)
    success(path=final, size_bytes=size)


def main():
    a = sys.argv[1:]
    if a and a[0] == "--resolve":
        from providers import resolve_input
        try:
            success(tracks=resolve_input(a[1] if len(a) > 1 else ""))
        except Exception as e:
            fail(e)
        return
    if a and a[0] == "--search":
        return cmd_search(a[1] if len(a) > 1 else "", int(a[2]) if len(a) > 2 else 10)
    if a and a[0] == "--playlist":
        return cmd_playlist(a[1] if len(a) > 1 else "")
    if a and a[0] == "--formats":
        return cmd_formats(a[1] if len(a) > 1 else "", a[2] if len(a) > 2 else "best")
    if len(a) >= 3:
        return download(
            a[0], a[1], a[2],
            a[3] if len(a) > 3 else "",
            a[4] if len(a) > 4 else "",
            a[5] if len(a) > 5 else "audio",
            int(a[6]) if len(a) > 6 else 0,
            a[7] if len(a) > 7 else "best",
        )
    print("Usage: downloader <query> <out_base> <ffmpeg> [song] [artist] [mode] [duration_ms] [quality]")
    print("       downloader --search <query> <limit>")
    print("       downloader --playlist <url>")
    print("       downloader --formats <url> [quality]")


if __name__ == "__main__":
    main()
