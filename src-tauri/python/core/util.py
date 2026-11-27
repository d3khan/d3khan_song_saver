import os

LEFTOVER_EXTS = (".webp", ".png", ".jpg", ".jpeg", ".jpe",
                 ".temp.mp3", ".temp.mp4", ".part", ".ytdl")


def cleanup_leftovers(out_base: str) -> None:
    for ext in LEFTOVER_EXTS:
        p = out_base + ext
        if os.path.exists(p):
            try:
                os.remove(p)
            except Exception:
                pass


def is_url(s: str) -> bool:
    return (s or "").strip().startswith("http")


def safe_outtmpl(out_base: str) -> str:
    # a literal % in a title would otherwise break yt-dlp's template
    return out_base.replace("%", "%%") + ".%(ext)s"
