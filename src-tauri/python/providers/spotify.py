"""Spotify metadata: playlist / album / track.

Albums previously failed because everything was sent to get_playlist().
"""
import re

from .base import Provider, make_track

ENTITY = re.compile(
    r"open\.spotify\.com/(?:intl-[a-z]+/)?(playlist|album|track)/([A-Za-z0-9]+)")


def parse(url):
    m = ENTITY.search(url or "")
    return (m.group(1), m.group(2)) if m else (None, None)


def canonical(kind, ident):
    return f"https://open.spotify.com/{kind}/{ident}"


def _dict(obj):
    if isinstance(obj, dict):
        return obj
    for attr in ("to_dict", "dict", "_asdict"):
        fn = getattr(obj, attr, None)
        if callable(fn):
            try:
                return fn()
            except Exception:
                pass
    if hasattr(obj, "__dict__"):
        return {k: v for k, v in vars(obj).items() if not k.startswith("_")}
    return {}


def _best_image(images):
    if not isinstance(images, list):
        return None
    usable = [i for i in images if isinstance(i, dict) and i.get("url")]
    if not usable:
        return None
    return max(usable, key=lambda x: x.get("width") or 0).get("url")


def _artists(raw):
    out = []
    for a in raw or []:
        if isinstance(a, str) and a:
            out.append(a)
            continue
        d = _dict(a)
        if d.get("name"):
            out.append(d["name"])
    return out


def _normalize(raw, fallback_cover, collection, source_url, index, album_name):
    d = _dict(raw)
    t = _dict(d.get("track")) or d          # playlists nest, albums are flat
    tid = t.get("id") or ""
    if not tid:
        return None
    album = _dict(t.get("album"))
    cover = (_best_image(t.get("images"))
             or _best_image(album.get("images"))
             or fallback_cover)
    return make_track(
        id=f"https://open.spotify.com/track/{tid}",
        title=t.get("name"),
        artists=_artists(t.get("artists")),
        cover_url=cover,
        duration_ms=t.get("duration_ms") or 0,
        track_number=t.get("track_number") or index + 1,
        folder_name=collection,
        playlist_url=source_url,
        album=album_name or album.get("name"),
        source_kind="matched",
    )


def tracks(url):
    from spotify_scraper import SpotifyClient

    kind, ident = parse(url)
    if not kind:
        raise ValueError(f"Not a recognised Spotify link: {url}")
    clean = canonical(kind, ident)          # drops ?si= tracking

    with SpotifyClient() as client:
        if kind == "playlist":
            obj = client.get_playlist(clean, max_tracks=10000)
        else:
            fn = getattr(client, f"get_{kind}", None)
            if fn is None:
                raise RuntimeError(
                    f"spotify_scraper has no get_{kind}(); upgrade the package")
            obj = fn(clean)

        data = _dict(obj)
        if kind == "track":
            album = _dict(data.get("album"))
            collection = album.get("name")
            fallback = _best_image(album.get("images"))
            album_name = album.get("name")
            raw_tracks = [obj]
        else:
            collection = data.get("name") or kind.capitalize()
            fallback = _best_image(data.get("images"))
            album_name = collection if kind == "album" else None
            raw_tracks = data.get("tracks") or []
            if isinstance(raw_tracks, dict):
                raw_tracks = raw_tracks.get("items") or []

        out = []
        for i, rt in enumerate(raw_tracks):
            n = _normalize(rt, fallback, collection, url, i, album_name)
            if n:
                out.append(n)
        if not out:
            raise RuntimeError(f"No tracks found in this Spotify {kind}.")
        return collection, out


class Spotify(Provider):
    name = "spotify"
    hosts = ("open.spotify.com",)
    role = "meta"

    def tracks(self, url):
        return tracks(url)[1]


PROVIDER = Spotify()
