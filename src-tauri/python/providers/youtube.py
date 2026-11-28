"""YouTube / YouTube Music: direct links, search, playlists.

Uses the frozen yt_dlp MODULE only - never an external yt-dlp binary.
"""
import yt_dlp

from .base import Provider, make_track

FLAT = {
    "quiet": True, "no_warnings": True, "extract_flat": True,
    "skip_download": True, "socket_timeout": 30, "retries": 3,
    "force_ipv4": True,
}


def _entry(v, i=None, playlist=None, url=None):
    vid = v.get("id") or ""
    dur = v.get("duration") or 0
    return make_track(
        id=f"https://www.youtube.com/watch?v={vid}",
        title=v.get("title"),
        artists=[v.get("uploader") or v.get("channel") or "YouTube"],
        cover_url=f"https://i.ytimg.com/vi/{vid}/hqdefault.jpg",
        duration_ms=int(float(dur) * 1000),
        track_number=(i + 1) if i is not None else None,
        folder_name=playlist,
        playlist_url=url,
        source_kind="direct",
    )


def search(query: str, limit: int = 20):
    opts = dict(FLAT, noplaylist=True)
    with yt_dlp.YoutubeDL(opts) as ydl:
        info = ydl.extract_info(f"ytsearch{limit}:{query} official audio",
                                download=False)
    return [_entry(v) for v in ((info or {}).get("entries") or [])
            if v and v.get("id")]


def playlist(url: str):
    opts = dict(FLAT, yes_playlist=True, ignoreerrors=True,
                extractor_args={"youtubetab": {"skip": ["webpage"]}})
    with yt_dlp.YoutubeDL(opts) as ydl:
        info = ydl.extract_info(url, download=False)
    name = (info or {}).get("title") or "YouTube Playlist"
    entries = [v for v in ((info or {}).get("entries") or []) if v and v.get("id")]
    return name, [_entry(v, i, name, url) for i, v in enumerate(entries)]


def single(url: str):
    opts = dict(FLAT, noplaylist=True)
    with yt_dlp.YoutubeDL(opts) as ydl:
        info = ydl.extract_info(url, download=False)
    if not info:
        return []
    if info.get("entries"):
        name, tracks = playlist(url)
        return tracks
    return [_entry(info)]


def formats(url: str):
    opts = {"quiet": True, "no_warnings": True, "skip_download": True,
            "noplaylist": True, "socket_timeout": 30, "force_ipv4": True}
    with yt_dlp.YoutubeDL(opts) as ydl:
        return ydl.extract_info(url, download=False)


class YouTube(Provider):
    name = "youtube"
    hosts = ("youtube.com", "youtu.be", "music.youtube.com")
    role = "direct"

    def tracks(self, url):
        if "list=" in url and "watch" not in url:
            return playlist(url)[1]
        return single(url)


PROVIDER = YouTube()
