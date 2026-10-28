#!/bin/bash
set -e

APP_DIR="/usr/lib/d3khan-song-saver"
BIN_DIR="$APP_DIR/bin"
mkdir -p "$BIN_DIR"

if [ ! -x "$BIN_DIR/yt-dlp" ]; then
  echo "Downloading yt-dlp…"
  curl -L -o "$BIN_DIR/yt-dlp" \
    "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp_linux"
  chmod +x "$BIN_DIR/yt-dlp"
fi

if [ ! -x "$BIN_DIR/ffmpeg" ]; then
  echo "Downloading FFmpeg…"
  TMP=$(mktemp -d)
  curl -L -o "$TMP/ffmpeg.tar.xz" \
    "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-linux64-gpl.tar.xz"
  tar -xJf "$TMP/ffmpeg.tar.xz" -C "$TMP"
  cp "$TMP/ffmpeg-master-latest-linux64-gpl/bin/ffmpeg" "$BIN_DIR/ffmpeg"
  cp "$TMP/ffmpeg-master-latest-linux64-gpl/bin/ffprobe" "$BIN_DIR/ffprobe"
  chmod +x "$BIN_DIR/ffmpeg" "$BIN_DIR/ffprobe"
  rm -rf "$TMP"
fi

echo "d3khan song saver dependencies installed."