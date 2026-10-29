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

# --- STAGE 4: PREPARE SIDECAR ---
mkdir -p src-tauri/binaries
TARGET=$(rustc -vV | grep host | cut -d' ' -f2)
echo "   Detected Target: $TARGET"

echo "🚚 Moving Python sidecar to Tauri binaries..."
cp dist/downloader "src-tauri/binaries/downloader-$TARGET"
chmod +x "src-tauri/binaries/downloader-$TARGET"

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
find src-tauri/target/release/bundle -type f \( -name "*.deb" -o -name "*.AppImage" \) -exec cp {} dist_out/ \;
echo "✅ Build complete! Check the 'dist_out/' folder."
