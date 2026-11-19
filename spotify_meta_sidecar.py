import sys
import json
from spotify_scraper import SpotifyClient

def main():
    if len(sys.argv) < 2:
        print(json.dumps({"status": "error", "message": "No URL provided"}))
        return

    url = sys.argv[1]
    try:
        with SpotifyClient() as client:
            # Fetch up to 10,000 tracks to guarantee we get massive playlists
            playlist = client.get_playlist(url, max_tracks=10000)
            
            tracks = []
            for t in playlist.tracks:
                t_dict = t.to_dict()
                
                # The actual track data is nested under the "track" key
                track_data = t_dict.get("track", {})
                
                # Safely extract artists
                artists_list = []
                for a in track_data.get("artists", []):
                    if isinstance(a, dict) and "name" in a:
                        artists_list.append(a["name"])
                    elif isinstance(a, str):
                        artists_list.append(a)
                
                # Safely extract the highest resolution cover URL
                cover_url = None
                images = track_data.get("images", [])
                if images and isinstance(images, list) and len(images) > 0:
                    # Pick the image with the maximum width
                    best_image = max(images, key=lambda x: x.get("width", 0))
                    cover_url = best_image.get("url")
                
                # Fallback to album images if track images are somehow missing
                if not cover_url:
                    album = track_data.get("album", {})
                    album_images = album.get("images", [])
                    if album_images and isinstance(album_images, list) and len(album_images) > 0:
                        best_album_image = max(album_images, key=lambda x: x.get("width", 0))
                        cover_url = best_album_image.get("url")

                tracks.append({
                    "id": f"https://open.spotify.com/track/{track_data.get('id', '')}",
                    "title": track_data.get("name", "Unknown Title"),
                    "artists": artists_list,
                    "duration_ms": track_data.get("duration_ms", 0),
                    "cover_url": cover_url,
                    "folder_name": playlist.name,
                    "playlist_url": url
                })
                
            print(json.dumps({"status": "success", "tracks": tracks}))
            
    except Exception as e:
        print(json.dumps({"status": "error", "message": str(e)}))

if __name__ == "__main__":
    main()