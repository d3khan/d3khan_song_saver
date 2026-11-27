"""YouTube Music catalogue matcher - prefers the official studio art track."""
from core.events import warn

try:
    from ytmusicapi import YTMusic
    AVAILABLE = True
except Exception as e:
    warn(f"ytmusicapi not available: {e}")
    AVAILABLE = False

BAD_WORDS = ("live", "remix", "cover", "sped up", "slowed", "instrumental")


def find_video_id(query, artist="", song="", duration_s=None):
    if not AVAILABLE or not query:
        return None
    try:
        results = YTMusic().search(query, filter="songs", limit=10)
    except Exception as e:
        warn(f"ytmusic search failed: {e}")
        return None
    if not results:
        return None

    def score(r):
        s = 0
        vt = r.get("videoType", "")
        s += 4 if vt == "MUSIC_VIDEO_TYPE_ATV" else (1 if vt == "MUSIC_VIDEO_TYPE_OMV" else 0)
        title = (r.get("title") or "").lower()
        if song and song.lower() in title:
            s += 2
        if any(w in title for w in BAD_WORDS):
            s -= 3
        names = " ".join(a.get("name", "") for a in (r.get("artists") or [])).lower()
        if artist and artist.split(",")[0].strip().lower() in names:
            s += 2
        ds = r.get("duration_seconds")
        if duration_s and ds and abs(ds - duration_s) <= 3:
            s += 2
        return s

    return max(results, key=score).get("videoId")
