import sys
import json
import re

from spotify_scraper import SpotifyClient


def clean_url(url: str) -> str:
    # strip ?si=... tracking params — they break embed lookups
    return url.split("?")[0].strip().rstrip("/")


def entity(url: str):
    m = re.search(
        r"open\.spotify\.com/(?:intl-[a-z]+/)?(playlist|album|track)/([A-Za-z0-9]+)", url
    )
    return (m.group(1), m.group(2)) if m else (None, None)


def as_dict(obj):
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


def best_image(images):
    if not isinstance(images, list) or not images:
        return None
    usable = [i for i in images if isinstance(i, dict) and i.get("url")]
    if not usable:
        return None
    return max(usable, key=lambda x: x.get("width") or 0).get("url")


def artist_names(raw):
    out = []
    for a in raw or []:
        if isinstance(a, str) and a:
            out.append(a)
            continue
        d = as_dict(a)
        if d.get("name"):
            out.append(d["name"])
    return out


def normalize(raw_track, fallback_cover, collection, source_url, index):
    d = as_dict(raw_track)
    # playlists wrap entries as {"track": {...}}; albums/tracks are flat
    t = as_dict(d.get("track")) or d
    if not t:
        return None
    tid = t.get("id") or ""
    if not tid:
        return None
    album = as_dict(t.get("album"))
    cover = best_image(t.get("images")) or best_image(album.get("images")) or fallback_cover
    return {
        "id": f"https://open.spotify.com/track/{tid}",
        "title": t.get("name") or "Unknown Title",
        "artists": artist_names(t.get("artists")),
        "duration_ms": t.get("duration_ms") or 0,
        "cover_url": cover,
        "track_number": t.get("track_number") or index + 1,
        "folder_name": collection,
        "playlist_url": source_url,
    }


def main():
    if len(sys.argv) < 2:
        print(json.dumps({"status": "error", "message": "No URL provided"}))
        return

    original = sys.argv[1].strip()
    url = clean_url(original)
    kind, _id = entity(url)

    if not kind:
        print(json.dumps({"status": "error",
                          "message": f"Not a recognised Spotify link: {original}"}))
        return

    try:
        with SpotifyClient() as client:
            if kind == "playlist":
                obj = client.get_playlist(url, max_tracks=10000)
            elif kind == "album":
                fn = getattr(client, "get_album", None)
                if fn is None:
                    print(json.dumps({"status": "error",
                                      "message": "spotify_scraper has no get_album(); upgrade the package."}))
                    return
                obj = fn(url)
            else:
                fn = getattr(client, "get_track", None)
                if fn is None:
                    print(json.dumps({"status": "error",
                                      "message": "spotify_scraper has no get_track(); upgrade the package."}))
                    return
                obj = fn(url)

            data = as_dict(obj)

            if kind == "track":
                album = as_dict(data.get("album"))
                collection = album.get("name")
                fallback_cover = best_image(album.get("images"))
                raw_tracks = [obj]
            else:
                collection = data.get("name") or ("Album" if kind == "album" else "Playlist")
                fallback_cover = best_image(data.get("images"))
                raw_tracks = data.get("tracks") or []
                if isinstance(raw_tracks, dict):
                    raw_tracks = raw_tracks.get("items") or []

            tracks = []
            for i, rt in enumerate(raw_tracks):
                n = normalize(rt, fallback_cover, collection, original, i)
                if n:
                    tracks.append(n)

            if not tracks:
                print(json.dumps({"status": "error",
                                  "message": f"No tracks found in this Spotify {kind}."}))
                return

            print(json.dumps({"status": "success",
                              "playlist_name": collection,
                              "tracks": tracks}))

    except Exception as e:
        print(json.dumps({"status": "error", "message": str(e)}))


if __name__ == "__main__":
    main()