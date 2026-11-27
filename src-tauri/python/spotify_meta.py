"""Sidecar entry point 2. Same contract: spotify_meta <url>."""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from core.events import fail, success
from providers import spotify


def main():
    if len(sys.argv) < 2:
        return fail("No URL provided")
    try:
        name, tracks = spotify.tracks(sys.argv[1])
        success(playlist_name=name, tracks=tracks)
    except Exception as e:
        fail(e)


if __name__ == "__main__":
    main()
