"""Provider registry.

Explicit imports (not dynamic discovery) so PyInstaller bundles every module.
Adding a source = new file + one import + one REGISTRY entry.
"""
from .base import Provider, make_track
from . import youtube, ytmusic, spotify

REGISTRY = [
    youtube.PROVIDER,
    spotify.PROVIDER,
    # deezer.PROVIDER, soundcloud.PROVIDER, audiomack.PROVIDER, boomplay.PROVIDER
]


def find(url):
    for p in REGISTRY:
        if p.matches(url):
            return p
    return None


def resolve_input(url):
    p = find(url)
    if p is not None:
        return p.tracks(url)
    return youtube.search(url, 10)


def resolve_media(query, song, artist, media_mode, duration_s=None):
    """Returns (target, is_direct_url)."""
    q = (query or "").strip()
    p = find(q)
    if p is not None and p.role == "direct":
        return q, True
    human = f"{artist} {song}".strip() if (artist or song) else q
    if media_mode == "video":
        return f"{human} official video", False
    vid = ytmusic.find_video_id(human, artist, song, duration_s)
    if vid:
        return f"https://music.youtube.com/watch?v={vid}", True
    return f"{human} official audio", False
