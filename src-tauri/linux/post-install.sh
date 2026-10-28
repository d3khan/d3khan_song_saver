#!/bin/bash
set -e
echo "Setting up d3khan song saver dependencies..."

APP_DIR="/usr/lib/d3khan-song-saver"
BIN_DIR="$APP_DIR/bin"
mkdir -p "$BIN_DIR"

# Only check for ffmpeg, since Python sidecar handles yt-dlp internally
if [ ! -x "$BIN_DIR/ffmpeg" ]; then
  echo "Downloading FFmpeg (this may take a minute)..."
  
  # Create a temp directory for the download
  TMP_DIR=$(mktemp -d)
  ARCHIVE="$TMP_DIR/ffmpeg.tar.xz"
  
  # -C - allows resuming if the download gets interrupted
  curl -L -C - -o "$ARCHIVE" \
    "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-linux64-gpl.tar.xz"
  
  echo "Extracting FFmpeg..."
  tar -xJf "$ARCHIVE" -C "$TMP_DIR"
  
  # Move binaries to the app's bin folder
  cp "$TMP_DIR/ffmpeg-master-latest-linux64-gpl/bin/ffmpeg" "$BIN_DIR/ffmpeg"
  cp "$TMP_DIR/ffmpeg-master-latest-linux64-gpl/bin/ffprobe" "$BIN_DIR/ffprobe"
  chmod +x "$BIN_DIR/ffmpeg" "$BIN_DIR/ffprobe"
  
  # Cleanup
  rm -rf "$TMP_DIR"
else
  echo "FFmpeg is already installed."
fi

echo "d3khan song saver dependencies installed successfully."