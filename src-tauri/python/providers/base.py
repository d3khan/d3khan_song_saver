"""Provider contract.

role:
  direct  - URL gives tracks AND is downloadable (YouTube, SoundCloud...)
  meta    - URL gives metadata only (Spotify, Deezer, Boomplay...)
  media   - turns a track into a playable source (YouTube Music, YouTube)

To add a source: drop a new module here, expose PROVIDER, and add it to
REGISTRY in __init__.py. No Rust changes needed.
"""


def make_track(id, title, artists, cover_url=None, duration_ms=0,
               track_number=None, folder_name=None, playlist_url=None,
               album=None, source_kind="matched"):
    t = {
        "id": id,
        "title": title or "Unknown Title",
        "artists": [a for a in (artists or []) if a] or ["Unknown Artist"],
        "cover_url": cover_url,
        "duration_ms": int(duration_ms or 0),
        "source_kind": source_kind,
    }
    if track_number is not None:
        t["track_number"] = int(track_number)
    if folder_name:
        t["folder_name"] = folder_name
    if playlist_url:
        t["playlist_url"] = playlist_url
    if album:
        t["album"] = album
    return t


class Provider:
    name = "base"
    hosts = ()
    role = "meta"

    def matches(self, url: str) -> bool:
        u = (url or "").lower()
        return any(h in u for h in self.hosts)

    def tracks(self, url: str):
        raise NotImplementedError
