"""Cover art + tags that file explorers actually render.

VLC reads WebP APIC frames; Windows Explorer / Nautilus / Dolphin do not.
So every cover is re-encoded to baseline JPEG and written as ID3v2.3.
MP4 uses mutagen's covr atom, so AtomicParsley is NOT required.
"""
import os
import subprocess
import tempfile

from .ffmpeg import exe

try:
    from mutagen.id3 import ID3, APIC, TIT2, TPE1, TALB, TRCK, ID3NoHeaderError
    from mutagen.mp4 import MP4, MP4Cover
    HAVE_MUTAGEN = True
except Exception:
    HAVE_MUTAGEN = False

THUMB_EXTS = (".jpg", ".jpeg", ".png", ".webp", ".avif", ".jpe")


def find_thumbnail(out_base: str):
    for ext in THUMB_EXTS:
        p = out_base + ext
        if os.path.exists(p):
            return p
    return None


def to_jpeg_bytes(src, ffmpeg_path):
    if not src or not os.path.exists(src):
        return None
    fd, dest = tempfile.mkstemp(suffix=".jpg", prefix="d3khan_cover_")
    os.close(fd)
    cmd = [exe(ffmpeg_path), "-y", "-hide_banner", "-loglevel", "error",
           "-i", src, "-frames:v", "1",
           "-vf", "scale=trunc(iw/2)*2:trunc(ih/2)*2",
           "-pix_fmt", "yuvj420p", "-c:v", "mjpeg", "-q:v", "2", dest]
    try:
        r = subprocess.run(cmd, stdout=subprocess.DEVNULL,
                           stderr=subprocess.DEVNULL, timeout=120)
        if r.returncode == 0 and os.path.getsize(dest) > 100:
            with open(dest, "rb") as f:
                return f.read()
    except Exception:
        pass
    finally:
        try:
            os.remove(dest)
        except Exception:
            pass
    return None


def finalize(path, info, out_base, ffmpeg_path, song="", artist="",
             album=None, track_number=None):
    """Never raises into the download path."""
    if not HAVE_MUTAGEN or not path or not os.path.exists(path):
        return False
    info = info or {}
    title = song or info.get("title") or ""
    artist_name = artist or info.get("artist") or info.get("uploader") or ""
    album_name = album or info.get("album") or ""
    track_no = track_number or info.get("track_number") or 0

    jpeg = to_jpeg_bytes(find_thumbnail(out_base), ffmpeg_path)
    ext = os.path.splitext(path)[1].lower()

    if ext == ".mp3":
        try:
            tags = ID3(path)
        except ID3NoHeaderError:
            tags = ID3()
        except Exception:
            tags = ID3()
        if title:
            tags.delall("TIT2"); tags.add(TIT2(encoding=3, text=[title]))
        if artist_name:
            tags.delall("TPE1"); tags.add(TPE1(encoding=3, text=[artist_name]))
        if album_name:
            tags.delall("TALB"); tags.add(TALB(encoding=3, text=[album_name]))
        if track_no:
            tags.delall("TRCK"); tags.add(TRCK(encoding=3, text=[str(track_no)]))
        if jpeg:
            tags.delall("APIC")
            tags.add(APIC(encoding=3, mime="image/jpeg", type=3,
                          desc="Cover", data=jpeg))
        tags.save(path, v2_version=3, v1=2)
        return True

    if ext in (".mp4", ".m4a", ".m4v"):
        f = MP4(path)
        if f.tags is None:
            f.add_tags()
        if title:
            f.tags["\xa9nam"] = [title]
        if artist_name:
            f.tags["\xa9ART"] = [artist_name]
            f.tags["aART"] = [artist_name]
        if album_name:
            f.tags["\xa9alb"] = [album_name]
        if track_no:
            f.tags["trkn"] = [(int(track_no), 0)]
        if jpeg:
            f.tags["covr"] = [MP4Cover(jpeg, imageformat=MP4Cover.FORMAT_JPEG)]
        f.save()
        return True

    return False
