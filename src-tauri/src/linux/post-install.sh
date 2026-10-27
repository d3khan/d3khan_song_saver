#!/bin/bash
set -e

APP_DIR="/usr/lib/d3khan-song-saver"
mkdir -p "$APP_DIR"

echo "Downloading yt-dlp and FFmpeg for d3khan song saver..."

# Download yt-dlp Linux binary
curl -L -o "$APP_DIR/yt-dlp" "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp_linux"
chmod +x "$APP_DIR/yt-dlp"

# Download and extract FFmpeg for Linux
curl -L -o /tmp/ffmpeg.tar.xz "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-linux64-gpl.tar.xz"
tar -xJf /tmp/ffmpeg.tar.xz -C /tmp
cp /tmp/ffmpeg-master-latest-linux64-gpl/bin/ffmpeg "$APP_DIR/ffmpeg"
cp /tmp/ffmpeg-master-latest-linux64-gpl/bin/ffprobe "$APP_DIR/ffprobe"
chmod +x "$APP_DIR/ffmpeg" "$APP_DIR/ffprobe"
rm -rf /tmp/ffmpeg.tar.xz /tmp/ffmpeg-master-latest-linux64-gpl

echo "Download complete."