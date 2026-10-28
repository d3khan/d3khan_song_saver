#!/bin/bash
set -e

echo "🔍 Checking prerequisites..."
if ! command -v cargo &> /dev/null; then echo "❌ Rust not installed."; exit 1; fi
if ! command -v node &> /dev/null; then echo "❌ Node.js not installed."; exit 1; fi
if ! command -v python3 &> /dev/null; then echo "❌ Python 3 not installed."; exit 1; fi

# --- STAGE 1: REPO LOCATION & GIT PULL ---
REPO_NAME="d3khan_song_saver"
if [ -f "package.json" ]; then
    echo "✅ Found repository in current directory."
elif [ -f "../package.json" ]; then
    echo "✅ Found repository in parent directory. Moving up."
    cd ..
elif [ -d "$REPO_NAME" ] && [ -f "$REPO_NAME/package.json" ]; then
    echo "✅ Found repository in subdirectory '$REPO_NAME'. Moving in."
    cd "$REPO_NAME"
else
    echo "📥 Repository not found. Cloning..."
    git clone https://github.com/d3khan/d3khan_song_saver.git
    cd "$REPO_NAME"
fi

# --- FIX: REMOVE WINDOWS TOOLCHAIN LOCK ---
if [ -f "rust-toolchain.toml" ]; then
    echo "🧹 Removing Windows-specific toolchain file for Linux build..."
    rm -f rust-toolchain.toml
fi

if [ -d ".git" ]; then
    echo "🔄 Pulling latest changes from Git..."
    git pull || echo "⚠️ Warning: Git pull failed. Continuing with local files."
fi

# --- STAGE 2: PYTHON DEPENDENCIES ---
echo "📦 Ensuring Python dependencies are installed..."
pip3 install -q yt-dlp mutagen requests pyinstaller

# --- STAGE 3: BUILD SIDECAR (SKIP IF UNCHANGED) ---
if [ ! -f "dist/downloader" ] || [ "downloader.py" -nt "dist/downloader" ]; then
    echo "🐍 Compiling Python sidecar (downloader.py)..."
    pyinstaller --noconfirm --onefile --name downloader downloader.py
else
    echo "✅ Python sidecar is up to date."
fi

# --- STAGE 4: PREPARE BINARIES & FFMPEG ---
mkdir -p src-tauri/binaries
TARGET=$(rustc -Vv | grep host | cut -d' ' -f2)

echo "🚚 Moving Python sidecar to Tauri binaries..."
cp dist/downloader "src-tauri/binaries/downloader-$TARGET"
chmod +x "src-tauri/binaries/downloader-$TARGET"

if [ ! -f "src-tauri/binaries/ffmpeg-$TARGET" ]; then
    echo "🔍 FFmpeg binary not found. Checking for existing archive..."
    ARCHIVE="ffmpeg-linux.tar.xz"
    
    if [ -f "$ARCHIVE" ]; then
        echo "   Found existing archive. Verifying integrity..."
        if ! tar -tf "$ARCHIVE" >/dev/null 2>&1; then
            echo "   ⚠️ Existing archive is corrupt. Deleting and restarting..."
            rm -f "$ARCHIVE"
        else
            echo "   ✅ Archive is valid."
        fi
    fi

    if [ ! -f "$ARCHIVE" ]; then
        echo "   📥 Downloading static FFmpeg (this may take a minute)..."
        curl -L -C - -o "$ARCHIVE" "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-linux64-gpl.tar.xz"
    fi

    echo "   📦 Extracting FFmpeg..."
    rm -rf "ffmpeg-master-latest-linux64-gpl"
    tar -xJf "$ARCHIVE"
    cp "ffmpeg-master-latest-linux64-gpl/bin/ffmpeg" "src-tauri/binaries/ffmpeg-$TARGET"
    chmod +x "src-tauri/binaries/ffmpeg-$TARGET"
    rm -rf "$ARCHIVE" "ffmpeg-master-latest-linux64-gpl"
else
    echo "✅ FFmpeg is already bundled."
fi

# --- STAGE 5: NODE & BUILD ---
if [ ! -d "node_modules" ]; then
    echo "📦 Installing Node dependencies..."
    npm install
else
    echo "✅ Node dependencies already installed."
fi

echo "🛠️ Building Tauri Application..."
npm run tauri build

echo "📁 Collecting build artifacts..."
mkdir -p dist_out
find src-tauri/target/release/bundle -type f \( -name "*.deb" -o -name "*.AppImage" -o -name "*.dmg" \) -exec cp {} dist_out/ \;
echo "✅ Build complete! Check the 'dist_out/' folder."
