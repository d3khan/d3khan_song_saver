"""Private ffmpeg staging.

yt-dlp only discovers a binary literally named ffmpeg/ffmpeg.exe, so we stage
our bundled d3khan-ffmpeg under that name ONCE and reuse it.
Never stage into %TEMP%: writing an exe there and running it looks like a
dropper to antivirus heuristics.
"""
import os
import platform
import shutil
from typing import Optional

from .events import warn


def cache_dir() -> str:
    if platform.system() == "Windows":
        base = os.environ.get("LOCALAPPDATA") or os.path.expanduser("~")
        return os.path.join(base, "d3khan-song-saver", "ffmpeg")
    return os.path.join(os.path.expanduser("~/.cache"), "d3khan-song-saver")


def prepare(ffmpeg_path: str) -> Optional[str]:
    if not ffmpeg_path or not os.path.exists(ffmpeg_path):
        return None
    d = cache_dir()
    try:
        os.makedirs(d, exist_ok=True)
        name = "ffmpeg.exe" if os.name == "nt" else "ffmpeg"
        dest = os.path.join(d, name)
        if os.path.exists(dest) and os.path.getsize(dest) == os.path.getsize(ffmpeg_path):
            return d
        shutil.copy2(ffmpeg_path, dest)
        os.chmod(dest, 0o755)
        return d
    except Exception as e:
        warn(f"could not stage ffmpeg: {e}")
        return None


def exe(ffmpeg_path: str) -> str:
    """For our OWN subprocess calls the real filename is fine."""
    if ffmpeg_path and os.path.isfile(ffmpeg_path):
        return ffmpeg_path
    return "ffmpeg"
