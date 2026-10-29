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
    if not title:
        return "", ""
    if " - " in title:
        artist, track = title.split(" - ", 1)
        return artist.strip(), track.strip()
    return "", title.strip()

def has_audio_stream(file_path, ffmpeg_dir=None):
    ffprobe_cmd = "ffprobe"
    if ffmpeg_dir:
        potential_ffprobe = os.path.join(ffmpeg_dir, "ffprobe.exe" if os.name == 'nt' else "ffprobe")
        if os.path.exists(potential_ffprobe):
            ffprobe_cmd = potential_ffprobe
    try:
        result = subprocess.run(
            [ffprobe_cmd, "-v", "error", "-select_streams", "a:0", "-show_entries", "stream=codec_name,duration", "-of", "csv=p=0", file_path],
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

def cleanup_leftovers(out_base):
    for ext in ['.webp', '.png', '.temp.mp3', '.temp.mp4']:
        leftover = out_base + ext
        if os.path.exists(leftover):
            try: os.remove(leftover)
            except Exception: pass

def download_track(search_query, out_base, ffmpeg_path, song, artist):
    is_youtube = search_query.startswith("http") and ("youtube.com" in search_query or "youtu.be" in search_query)
    
    if is_youtube:
        ydl_opts = {
            'format': 'bestvideo[ext=mp4]+bestaudio[ext=m4a]/best[ext=mp4]/best',
            'merge_output_format': 'mp4',
            'noplaylist': True,
            'writethumbnail': True,
            'convert_thumbnails': 'jpg',
            'outtmpl': out_base + '.%(ext)s',
            'quiet': True,
            'socket_timeout': 30,
            'retries': 10,
            'nocheckcertificate': True,
            'force_ipv4': True,
            'nopart': True,
            'extractor_args': {'youtube': {'player_client': ['android', 'ios']}},
            'postprocessors': [
                {"key": "EmbedThumbnail"},
                {"key": "FFmpegMetadata"},
            ]
        }
    else:
        metadata_args = []
        if song: metadata_args += ["-metadata", f"title={song}"]
        if artist: metadata_args += ["-metadata", f"artist={artist}"]

        ydl_opts = {
            'format': 'bestaudio/best',
            'noplaylist': True,
            'writethumbnail': True,
            'convert_thumbnails': 'jpg',
            'outtmpl': out_base + '.%(ext)s',
            'quiet': True,
            'socket_timeout': 30,
            'retries': 10,
            'nocheckcertificate': True,
            'force_ipv4': True,
            'nopart': True,
            'extractor_args': {'youtube': {'player_client': ['android', 'ios']}},
            'postprocessors': [
                {"key": "FFmpegExtractAudio", "preferredcodec": "mp3", "preferredquality": "192"},
                {"key": "FFmpegMetadata"},
                {"key": "EmbedThumbnail"},
            ]
        }
        if not search_query.startswith("http"):
            ydl_opts['default_search'] = 'ytsearch1'
        if metadata_args:
            ydl_opts['postprocessor_args'] = {'FFmpegMetadata': metadata_args}

    if ffmpeg_path and os.path.exists(ffmpeg_path):
        ydl_opts['ffmpeg_location'] = ffmpeg_path

    try:
        with yt_dlp.YoutubeDL(ydl_opts) as ydl:
            info = ydl.extract_info(search_query, download=True)
            
            filename = ydl.prepare_filename(info)
            base, ext = os.path.splitext(filename)
            final_file = base + (".mp4" if is_youtube else ".mp3")
            
            if not os.path.exists(final_file):
                cleanup_leftovers(out_base)
                return json.dumps({"status": "error", "message": f"File not found: {final_file}"})
                
    except Exception as e:
        cleanup_leftovers(out_base)
        return json.dumps({"status": "error", "message": str(e)})

    cleanup_leftovers(out_base)

    ffmpeg_dir = os.path.dirname(ffmpeg_path) if ffmpeg_path else None
    has_audio, duration = has_audio_stream(final_file, ffmpeg_dir)

    if has_audio:
        return json.dumps({"status": "success", "path": final_file})
    else:
        return json.dumps({"status": "error", "message": "No audio stream found in the file!"})

def main():
    if len(sys.argv) >= 4:
        query = sys.argv[1]
        out_base = sys.argv[2]
        ffmpeg_path = sys.argv[3]
        song = sys.argv[4] if len(sys.argv) > 4 else ""
        artist = sys.argv[5] if len(sys.argv) > 5 else ""
        download_track(query, out_base, ffmpeg_path, song, artist)
    else:
        print("=== Manual Test Mode ===")
        url = input("Enter URL: ").strip()
        ffmpeg = input("Enter FFmpeg Path: ").strip()
        encoded_url = quote(url)
        oembed_url = f"https://open.spotify.com/oembed?url={encoded_url}"
        headers = {"User-Agent": "Mozilla/5.0 (Windows NT 10.0; Win64; x64)"}
        response = requests.get(oembed_url, headers=headers, timeout=30)
        data = response.json()
        track_title = data.get("title") or ""
        artist, song = parse_spotify_title(track_title)
        search_query = f"{artist} {song}".strip() if artist else track_title
        out_dir = os.path.join(os.getcwd(), "TestDownloads")
        os.makedirs(out_dir, exist_ok=True)
        out_base = os.path.join(out_dir, f"{song} - {artist}")
        download_track(search_query, out_base, ffmpeg, song, artist)

if __name__ == "__main__":
    main()