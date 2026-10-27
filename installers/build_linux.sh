#!/bin/bash
set -e

echo "🔍 Checking prerequisites..."
if ! command -v cargo &> /dev/null; then echo "❌ Rust is not installed."; exit 1; fi
if ! command -v node &> /dev/null || ! command -v npm &> /dev/null; then echo "❌ Node.js is not installed."; exit 1; fi

# --- STAGE 1: REPO LOCATION (FOOLPROOF) ---
REPO_NAME="spotify_downloader"
if [ -d "src-tauri" ] && [ -f "package.json" ]; then
    echo "   ✅ Found repository in current directory."
elif [ -d "../src-tauri" ] && [ -f "../package.json" ]; then
    echo "   ✅ Found repository in parent directory. Moving up."
    cd ..
elif [ -d "$REPO_NAME/src-tauri" ] && [ -f "$REPO_NAME/package.json" ]; then
    echo "   ✅ Found repository in subdirectory '$REPO_NAME'. Moving in."
    cd "$REPO_NAME"
else
    echo "   📥 Repository not found. Cloning..."
    git clone https://github.com/d3khan/spotify_downloader.git
    cd "$REPO_NAME"
fi

if [ -d ".git" ]; then
    echo "🔄 Pulling latest changes from Git..."
    git pull || echo "⚠️ Warning: Git pull failed. Continuing with local files."
fi

# --- STAGE 2: SYSTEM DEPENDENCIES ---
if command -v apt-get &> /dev/null; then
    if ! dpkg -l | grep -q "libwebkit2gtk-4.1-dev"; then
        echo "📦 Installing system dependencies (requires sudo)..."
        sudo apt-get update
        sudo apt-get install -y build-essential curl wget file libssl-dev libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev
    else
        echo "   ✅ System dependencies already satisfied."
    fi
fi

if [ -f "rust-toolchain.toml" ]; then
    echo "🧹 Removing Windows-specific toolchain file..."
    rm -f rust-toolchain.toml
fi

# --- STAGE 3: SIDECAR BINARIES (API-FREE VERSION CHECK) ---
echo "🎵 Fetching sidecar binaries..."
mkdir -p src-tauri/binaries

# 1. yt-dlp
YTDLP_PATH="src-tauri/binaries/yt-dlp-x86_64-unknown-linux-gnu"
if [ -s "$YTDLP_PATH" ]; then
    echo "   🔍 Checking yt-dlp version..."
    LOCAL_YTDLP=$("$YTDLP_PATH" --version 2>/dev/null || echo "0")
    # Uses HEAD request to get the redirect URL, bypassing API rate limits
    LATEST_YTDLP=$(curl -sI "https://github.com/yt-dlp/yt-dlp/releases/latest" | grep -i '^location:' | sed -E 's/.*\/tag\/([^ ]+).*/\1/' | tr -d '\r')
    
    if [ -z "$LATEST_YTDLP" ]; then
        echo "   ⚠️ Could not check for updates (offline?). Using existing version."
    elif [ "$LOCAL_YTDLP" != "$LATEST_YTDLP" ]; then
        echo "   ⚠️  yt-dlp is out of date (local: $LOCAL_YTDLP, latest: $LATEST_YTDLP)."
        read -p "   Do you want to update it? (y/n) " -n 1 -r
        echo
        if [[ $REPLY =~ ^[Yy]$ ]]; then
            echo "   ⬇️ Downloading yt-dlp..."
            curl -f -L --retry 10 --retry-delay 3 -C - -o "$YTDLP_PATH" "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp_linux"
            chmod +x "$YTDLP_PATH"
        else
            echo "   ⏭️ Skipping update."
        fi
    else
        echo "   ✅ yt-dlp is up to date ($LOCAL_YTDLP)."
    fi
else
    echo "   ⬇️ Downloading yt-dlp (not found)..."
    curl -f -L --retry 10 --retry-delay 3 -C - -o "$YTDLP_PATH" "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp_linux"
    chmod +x "$YTDLP_PATH"
fi

# 2. FFmpeg & FFprobe
FFMPEG_PATH="src-tauri/binaries/ffmpeg-x86_64-unknown-linux-gnu"
FFPROBE_PATH="src-tauri/binaries/ffprobe-x86_64-unknown-linux-gnu"
ARCHIVE="ffmpeg-linux.tar.xz"
NEED_DOWNLOAD=0

if [ -s "$FFMPEG_PATH" ] && [ -s "$FFPROBE_PATH" ]; then
    echo "   🔍 Checking FFmpeg version..."
    # Uses HEAD request to get the BtbN tag (e.g., autobuild-2026-10-26-14-05)
    LATEST_FFMPEG_TAG=$(curl -sI "https://github.com/BtbN/FFmpeg-Builds/releases/latest" | grep -i '^location:' | sed -E 's/.*\/tag\/([^ ]+).*/\1/' | tr -d '\r')
    LOCAL_FFMPEG_DATE=$(date -r "$FFMPEG_PATH" +"%Y-%m-%d" 2>/dev/null)
    
    if [ -z "$LATEST_FFMPEG_TAG" ] || [ -z "$LOCAL_FFMPEG_DATE" ]; then
        echo "   ⚠️ Could not check for updates. Using existing version."
    elif [[ "$LATEST_FFMPEG_TAG" > *"$LOCAL_FFMPEG_DATE"* ]]; then
        echo "   ⚠️  FFmpeg is out of date (local build: $LOCAL_FFMPEG_DATE, latest build: $LATEST_FFMPEG_TAG)."
        read -p "   Do you want to update FFmpeg & FFprobe? (y/n) " -n 1 -r
        echo
        if [[ $REPLY =~ ^[Yy]$ ]]; then
            NEED_DOWNLOAD=1
        else
            echo "   ⏭️ Skipping update."
        fi
    else
        echo "   ✅ FFmpeg is up to date."
    fi
else
    echo "   ⬇️ Downloading FFmpeg (not found)..."
    NEED_DOWNLOAD=1
fi

if [ "$NEED_DOWNLOAD" -eq 1 ]; then
    if [ -f "$ARCHIVE" ]; then
        if ! tar -tf "$ARCHIVE" >/dev/null 2>&1; then
            echo "   ⚠️ Existing FFmpeg archive is corrupt. Deleting and restarting..."
            rm -f "$ARCHIVE"
        fi
    fi

    if [ ! -f "$ARCHIVE" ]; then
        echo "   ⬇️ Downloading FFmpeg (this may take a minute)..."
        curl -f -L --retry 10 --retry-delay 3 -C - -o "$ARCHIVE" "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-linux64-gpl.tar.xz"
    fi

    echo "   📦 Extracting FFmpeg..."
    rm -rf "ffmpeg-master-latest-linux64-gpl"
    tar -xJf "$ARCHIVE"
    
    cp "ffmpeg-master-latest-linux64-gpl/bin/ffmpeg" "$FFMPEG_PATH"
    cp "ffmpeg-master-latest-linux64-gpl/bin/ffprobe" "$FFPROBE_PATH"
    chmod +x "$FFMPEG_PATH" "$FFPROBE_PATH"
    
    rm -rf "$ARCHIVE" "ffmpeg-master-latest-linux64-gpl"
fi

# --- STAGE 4: BUILD ---
if [ ! -d "node_modules" ]; then
    echo "📦 Installing Node dependencies (npm)..."
    npm install
else
    echo "   ✅ Node dependencies already installed."
fi

echo "🛠️ Building Tauri Application (this will only recompile changed files)..."
npm run tauri build

# --- STAGE 5: COLLECT ---
echo "📁 Collecting build artifacts..."
mkdir -p dist
find src-tauri/target/release/bundle -type f \( -name "*.deb" -o -name "*.AppImage" \) -exec cp {} dist/ \;

echo "✅ Build complete! Your installers are ready in the 'dist/' folder."
