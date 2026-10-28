import sys
import os
import subprocess
import requests
import yt_dlp
import json
import re
from urllib.parse import quote

OK = "[OK]"
FAIL = "[FAILED]"

def parse_spotify_title(title):
    """Spotify oEmbed returns 'Artist - Song Name'. Split into artist + track."""
    if not title:
        return "", ""
    if " - " in title:
        artist, track = title.split(" - ", 1)
        return artist.strip(), track.strip()
    return "", title.strip()

def has_audio_stream(file_path, ffmpeg_dir=None):
    """Return (has_audio, duration) by probing the file with ffprobe."""
    ffprobe_cmd = "ffprobe"
    # If Rust provided an ffmpeg path, look for ffprobe in the same folder
    if ffmpeg_dir:
        potential_ffprobe = os.path.join(ffmpeg_dir, "ffprobe.exe" if os.name == 'nt' else "ffprobe")
        if os.path.exists(potential_ffprobe):
            ffprobe_cmd = potential_ffprobe

    try:
        result = subprocess.run(
            [
                ffprobe_cmd, "-v", "error",
                "-select_streams", "a:0",
                "-show_entries", "stream=codec_name,duration",
                "-of", "csv=p=0",
                file_path,
            ],
            capture_output=True, text=True, check=True,
        )
        line = result.stdout.strip()
        if not line:
            return False, 0
        parts = line.split(",")
        duration = float(parts[1]) if len(parts) > 1 and parts[1] else 0
        return True, duration
    except Exception:
        return False, 0

def download_track(search_query, out_dir, ffmpeg_path, song, artist):
    # Build ID3 metadata args so Spotify title/artist are embedded in the MP3
    metadata_args = []
    if song: metadata_args += ["-metadata", f"title={song}"]
    if artist: metadata_args += ["-metadata", f"artist={artist}"]

    pp_list = [
        {"key": "FFmpegExtractAudio", "preferredcodec": "mp3", "preferredquality": "192"},
        {"key": "FFmpegMetadata"},
        {"key": "EmbedThumbnail"},
    ]
    
    ydl_opts = {
        'format': 'bestaudio/best',
        'noplaylist': True,
        'writethumbnail': True,
        'outtmpl': os.path.join(out_dir, '%(title)s.%(ext)s'),
        'quiet': False,
        'default_search': 'ytsearch1',
        'postprocessors': pp_list,
        'socket_timeout': 30,
        'retries': 10,
        'nocheckcertificate': True,
    }
    
    if metadata_args:
        ydl_opts['postprocessor_args'] = {'FFmpegMetadata': metadata_args}

    if ffmpeg_path and os.path.exists(ffmpeg_path):
        ydl_opts['ffmpeg_location'] = ffmpeg_path

    print("\nSearching YouTube and downloading...")
    try:
        with yt_dlp.YoutubeDL(ydl_opts) as ydl:
            info = ydl.extract_info(search_query, download=True)
            
            # yt-dlp appends .mp3 after conversion
            final_file = ydl.prepare_filename(info)
            base, _ = os.path.splitext(final_file)
            final_file = base + ".mp3"
            
            if not os.path.exists(final_file):
                print(json.dumps({"status": "error", "message": "MP3 not found"}))
                return
                
    except Exception as e:
        print(json.dumps({"status": "error", "message": str(e)}))
        return

    # 4. Verify the final file actually contains audio
    ffmpeg_dir = os.path.dirname(ffmpeg_path) if ffmpeg_path else None
    has_audio, duration = has_audio_stream(final_file, ffmpeg_dir)

    if has_audio:
        print(json.dumps({"status": "success", "path": final_file}))
    else:
        print(json.dumps({"status": "error", "message": "No audio stream found in the file!"}))


def main():
    if len(sys.argv) >= 4:
        # --- CALLED BY RUST ---
        query = sys.argv[1]
        out_base = sys.argv[2]
        ffmpeg_path = sys.argv[3]
        song = sys.argv[4] if len(sys.argv) > 4 else ""
        artist = sys.argv[5] if len(sys.argv) > 5 else ""
        
        out_dir = os.path.dirname(out_base)
        os.makedirs(out_dir, exist_ok=True)
        
        download_track(query, out_dir, ffmpeg_path, song, artist)
        
    else:
        # --- MANUAL TESTING MODE ---
        print("=== Manual Test Mode ===")
        url = input("Enter Spotify URL: ").strip()
        ffmpeg = input("Enter FFmpeg Path (or leave empty for system PATH): ").strip()
        
        # 1. Fetch Track Metadata from Spotify's oEmbed
        print("Fetching metadata from Spotify...")
        encoded_url = quote(url)
        oembed_url = f"https://open.spotify.com/oembed?url={encoded_url}"
        headers = {"User-Agent": "Mozilla/5.0 (Windows NT 10.0; Win64; x64)"}
        
        response = requests.get(oembed_url, headers=headers, timeout=30)
        if response.status_code != 200:
            print(f"Failed to fetch Spotify metadata: HTTP {response.status_code}")
            return
            
        data = response.json()
        track_title = data.get("title") or ""
        print(f"Found track: {track_title}")
        
        artist, song = parse_spotify_title(track_title)
        search_query = f"{artist} {song}".strip() if artist else track_title
        
        out_dir = os.path.join(os.getcwd(), "TestDownloads")
        os.makedirs(out_dir, exist_ok=True)
        
        download_track(search_query, out_dir, ffmpeg, song, artist)

if __name__ == "__main__":
    main()