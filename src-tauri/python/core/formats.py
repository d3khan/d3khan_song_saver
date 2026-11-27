"""Quality selection.

H.264 (avc1) is preferred and capped at 1080p so the result plays everywhere
and NEVER needs transcoding. YouTube only serves VP9/AV1 above 1080p.
"""
HEIGHTS = {"best": 1080, "1080": 1080, "720": 720,
           "480": 480, "360": 360, "240": 240}


def height_cap(quality: str) -> int:
    return HEIGHTS.get((quality or "best").lower(), 1080)


def build_format(media_mode: str, quality: str = "best") -> str:
    if media_mode == "audio":
        return "bestaudio[acodec^=mp4a]/bestaudio/best"
    cap = f"[height<={height_cap(quality)}]"
    return (
        f"bestvideo[vcodec^=avc1]{cap}+bestaudio[acodec^=mp4a]/"
        f"best[vcodec^=avc1]{cap}/"
        f"bestvideo[vcodec^=vp09]{cap}+bestaudio[acodec^=mp4a]/"
        f"best{cap}"
    )


def size_of(f: dict, duration) -> int:
    n = f.get("filesize") or f.get("filesize_approx")
    if n:
        return int(n)
    tbr = f.get("tbr") or f.get("vbr") or f.get("abr")
    if tbr and duration:
        return int(float(tbr) * 1000 / 8 * float(duration))
    return 0
