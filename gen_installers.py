import os

def create_installer_scripts():
    # Create the directory if it doesn't exist
    output_dir = "installers"
    if not os.path.exists(output_dir):
        os.makedirs(output_dir)
        print(f"📁 Created folder: {output_dir}/")
    else:
        print(f"📁 Folder {output_dir}/ already exists.")

    # ==========================================
    # 1. LINUX SCRIPT (Bash)
    # ==========================================
    linux_script = r"""#!/bin/bash
set -e

echo "🔍 Checking prerequisites..."
if ! command -v cargo &> /dev/null; then echo "❌ Rust is not installed."; exit 1; fi
if ! command -v node &> /dev/null || ! command -v npm &> /dev/null; then echo "❌ Node.js is not installed."; exit 1; fi

# --- STAGE 1: REPO LOCATION ---
REPO_NAME="spotify_downloader"
if [ -f "package.json" ] && grep -q "\"name\": \"$REPO_NAME\"" package.json; then
    :
elif [ -f "../package.json" ] && grep -q "\"name\": \"$REPO_NAME\"" ../package.json; then
    cd ..
elif [ -d "$REPO_NAME" ] && [ -f "$REPO_NAME/package.json" ]; then
    cd "$REPO_NAME"
else
    echo "📥 Cloning repository..."
    git clone https://github.com/d3khan/spotify_downloader.git
    cd "$REPO_NAME"
fi

if [ -d ".git" ]; then
    echo "🔄 Pulling latest changes from Git..."
    git pull || echo "⚠️ Warning: Git pull failed. Continuing..."
fi

# --- STAGE 2: SYSTEM DEPENDENCIES ---
if command -v apt-get &> /dev/null; then
    if ! dpkg -l | grep -q "libwebkit2gtk-4.1-dev"; then
        echo "📦 Installing system dependencies (requires sudo)..."
        sudo apt-get update
        sudo apt-get install -y build-essential curl wget file libssl-dev libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev
    fi
fi

if [ -f "rust-toolchain.toml" ]; then rm -f rust-toolchain.toml; fi

# --- STAGE 3: SIDECAR BINARIES (FOOLPROOF) ---
mkdir -p src-tauri/binaries

# 1. yt-dlp
YTDLP_PATH="src-tauri/binaries/yt-dlp-x86_64-unknown-linux-gnu"
if [ ! -s "$YTDLP_PATH" ]; then
    echo "⬇️ Downloading yt-dlp..."
    curl -f -L --retry 10 --retry-delay 3 -C - -o "$YTDLP_PATH" "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp_linux"
    chmod +x "$YTDLP_PATH"
else
    echo "✅ yt-dlp already exists."
fi

# 2. FFmpeg & FFprobe
FFMPEG_PATH="src-tauri/binaries/ffmpeg-x86_64-unknown-linux-gnu"
FFPROBE_PATH="src-tauri/binaries/ffprobe-x86_64-unknown-linux-gnu"
ARCHIVE="ffmpeg-linux.tar.xz"

if [ ! -s "$FFMPEG_PATH" ] || [ ! -s "$FFPROBE_PATH" ]; then
    NEED_DOWNLOAD=0
    if [ -f "$ARCHIVE" ]; then
        if ! tar -tf "$ARCHIVE" >/dev/null 2>&1; then
            echo "⚠️ Existing FFmpeg archive is corrupt. Deleting and restarting..."
            rm -f "$ARCHIVE"
            NEED_DOWNLOAD=1
        fi
    else
        NEED_DOWNLOAD=1
    fi

    if [ "$NEED_DOWNLOAD" -eq 1 ]; then
        echo "⬇️ Downloading FFmpeg (this may take a minute)..."
        curl -f -L --retry 10 --retry-delay 3 -C - -o "$ARCHIVE" "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-linux64-gpl.tar.xz"
    fi

    echo "📦 Extracting FFmpeg..."
    rm -rf "ffmpeg-master-latest-linux64-gpl"
    tar -xJf "$ARCHIVE"
    
    cp "ffmpeg-master-latest-linux64-gpl/bin/ffmpeg" "$FFMPEG_PATH"
    cp "ffmpeg-master-latest-linux64-gpl/bin/ffprobe" "$FFPROBE_PATH"
    chmod +x "$FFMPEG_PATH" "$FFPROBE_PATH"
    
    rm -rf "$ARCHIVE" "ffmpeg-master-latest-linux64-gpl"
else
    echo "✅ FFmpeg and FFprobe already exist."
fi

# --- STAGE 4: BUILD ---
echo "📦 Installing Node dependencies..."
npm install
echo "🛠️ Building Tauri Application..."
npm run tauri build

# --- STAGE 5: COLLECT ---
mkdir -p dist
find src-tauri/target/release/bundle -type f \( -name "*.deb" -o -name "*.AppImage" \) -exec cp {} dist/ \;
echo "✅ Build complete! Check the 'dist/' folder."
"""

    # ==========================================
    # 2. POWERSHELL SCRIPT
    # ==========================================
    ps1_script = r"""$ErrorActionPreference = "Stop"

if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) { Write-Host "❌ Rust not installed."; exit 1 }
if (-not (Get-Command node -ErrorAction SilentlyContinue)) { Write-Host "❌ Node.js not installed."; exit 1 }

# --- STAGE 1: REPO LOCATION ---
$RepoName = "spotify_downloader"
$InsideRepo = $false
if (Test-Path "package.json") { if ((Get-Content "package.json" | ConvertFrom-Json).name -eq $RepoName) { $InsideRepo = $true } }
if (-not $InsideRepo -and (Test-Path "..\package.json")) { if ((Get-Content "..\package.json" | ConvertFrom-Json).name -eq $RepoName) { Set-Location ".."; $InsideRepo = $true } }
if (-not $InsideRepo -and (Test-Path "$RepoName\package.json")) { Set-Location $RepoName; $InsideRepo = $true }
if (-not $InsideRepo) { git clone https://github.com/d3khan/spotify_downloader.git; Set-Location $RepoName }
if (Test-Path ".git") { git pull }

# --- STAGE 2: SIDECAR BINARIES (FOOLPROOF) ---
New-Item -ItemType Directory -Force -Path "src-tauri\binaries" | Out-Null

# 1. yt-dlp
$YtDlpPath = "src-tauri\binaries\yt-dlp-x86_64-pc-windows-msvc.exe"
if (-not (Test-Path $YtDlpPath) -or (Get-Item $YtDlpPath).Length -eq 0) {
    Write-Host "⬇️ Downloading yt-dlp..."
    curl.exe -f -L --retry 10 --retry-delay 3 -C - -o $YtDlpPath "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe"
} else { Write-Host "✅ yt-dlp already exists." }

# 2. FFmpeg & FFprobe
$FfmpegPath = "src-tauri\binaries\ffmpeg-x86_64-pc-windows-msvc.exe"
$FfprobePath = "src-tauri\binaries\ffprobe-x86_64-pc-windows-msvc.exe"
$Archive = "ffmpeg-win.zip"
$TempDir = "ffmpeg-temp"

if (-not (Test-Path $FfmpegPath) -or -not (Test-Path $FfprobePath)) {
    $NeedDownload = $false
    
    if (Test-Path $Archive) {
        try {
            Add-Type -AssemblyName System.IO.Compression.FileSystem
            [System.IO.Compression.ZipFile]::OpenRead((Resolve-Path $Archive)).Dispose()
        } catch {
            Write-Host "⚠️ Existing FFmpeg archive is corrupt. Deleting and restarting..."
            Remove-Item -Force $Archive
            $NeedDownload = $true
        }
    } else {
        $NeedDownload = $true
    }

    if ($NeedDownload) {
        Write-Host "⬇️ Downloading FFmpeg (this may take a minute)..."
        curl.exe -f -L --retry 10 --retry-delay 3 -C - -o $Archive "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl.zip"
    }

    Write-Host "📦 Extracting FFmpeg..."
    if (Test-Path $TempDir) { Remove-Item -Recurse -Force $TempDir }
    Expand-Archive -Path $Archive -DestinationPath $TempDir -Force
    
    Copy-Item "$TempDir\ffmpeg-master-latest-win64-gpl\bin\ffmpeg.exe" -Destination $FfmpegPath -Force
    Copy-Item "$TempDir\ffmpeg-master-latest-win64-gpl\bin\ffprobe.exe" -Destination $FfprobePath -Force
    
    Remove-Item -Recurse -Force $TempDir
    Remove-Item -Force $Archive
} else { Write-Host "✅ FFmpeg and FFprobe already exist." }

# --- STAGE 3: BUILD ---
Write-Host "📦 Installing Node dependencies..."
npm install
Write-Host "🛠️ Building Tauri Application..."
npm run tauri build

# --- STAGE 4: COLLECT ---
New-Item -ItemType Directory -Force -Path "dist" | Out-Null
Copy-Item "src-tauri\target\release\bundle\msi\*.msi" -Destination "dist\" -ErrorAction SilentlyContinue
Copy-Item "src-tauri\target\release\bundle\nsis\*.exe" -Destination "dist\" -ErrorAction SilentlyContinue
Write-Host "✅ Build complete! Check the 'dist' folder."
"""

    # ==========================================
    # 3. WINDOWS CMD SCRIPT
    # ==========================================
    cmd_script = r"""@echo off
setlocal enabledelayedexpansion

where cargo >nul 2>nul
if %errorlevel% neq 0 ( echo Rust not installed. & pause & exit /b 1 )
where node >nul 2>nul
if %errorlevel% neq 0 ( echo Node.js not installed. & pause & exit /b 1 )

:: --- STAGE 1: REPO LOCATION ---
set REPO_NAME=spotify_downloader
set INSIDE_REPO=0
if exist "package.json" ( findstr /C:"spotify_downloader" package.json >nul && set INSIDE_REPO=1 )
if !INSIDE_REPO! equ 0 if exist "..\package.json" ( pushd .. & findstr /C:"spotify_downloader" package.json >nul && set INSIDE_REPO=1 || popd )
if !INSIDE_REPO! equ 0 if exist "%REPO_NAME%\package.json" ( cd %REPO_NAME% & set INSIDE_REPO=1 )
if !INSIDE_REPO! equ 0 ( git clone https://github.com/d3khan/spotify_downloader.git & cd %REPO_NAME% )
if exist ".git" ( git pull )

:: --- STAGE 2: SIDECAR BINARIES (FOOLPROOF) ---
if not exist "src-tauri\binaries" mkdir "src-tauri\binaries"

:: 1. yt-dlp
set YTDLP_PATH=src-tauri\binaries\yt-dlp-x86_64-pc-windows-msvc.exe
if not exist "%YTDLP_PATH%" (
    echo Downloading yt-dlp...
    curl.exe -f -L --retry 10 --retry-delay 3 -C - -o "%YTDLP_PATH%" "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe"
) else ( echo yt-dlp already exists. )

:: 2. FFmpeg & FFprobe
set FFMPEG_PATH=src-tauri\binaries\ffmpeg-x86_64-pc-windows-msvc.exe
set FFPROBE_PATH=src-tauri\binaries\ffprobe-x86_64-pc-windows-msvc.exe
set ARCHIVE=ffmpeg-win.zip
set TEMP_DIR=ffmpeg-temp

if not exist "%FFMPEG_PATH%" (
    set NEED_DOWNLOAD=0
    if exist "%ARCHIVE%" (
        powershell -Command "try { Add-Type -AssemblyName System.IO.Compression.FileSystem; [System.IO.Compression.ZipFile]::OpenRead((Resolve-Path '%ARCHIVE%')).Dispose(); exit 0 } catch { exit 1 }"
        if !errorlevel! neq 0 (
            echo Existing FFmpeg archive is corrupt. Deleting...
            del /F /Q "%ARCHIVE%"
            set NEED_DOWNLOAD=1
        )
    ) else (
        set NEED_DOWNLOAD=1
    )

    if !NEED_DOWNLOAD! equ 1 (
        echo Downloading FFmpeg (this may take a minute)...
        curl.exe -f -L --retry 10 --retry-delay 3 -C - -o "%ARCHIVE%" "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl.zip"
    )

    echo Extracting FFmpeg...
    if exist "%TEMP_DIR%" rmdir /S /Q "%TEMP_DIR%"
    powershell -Command "Expand-Archive -Path '%ARCHIVE%' -DestinationPath '%TEMP_DIR%' -Force"
    
    copy /Y "%TEMP_DIR%\ffmpeg-master-latest-win64-gpl\bin\ffmpeg.exe" "%FFMPEG_PATH%" >nul
    copy /Y "%TEMP_DIR%\ffmpeg-master-latest-win64-gpl\bin\ffprobe.exe" "%FFPROBE_PATH%" >nul
    
    rmdir /S /Q "%TEMP_DIR%"
    del /F /Q "%ARCHIVE%"
) else ( echo FFmpeg and FFprobe already exist. )

:: --- STAGE 3: BUILD ---
echo Installing Node dependencies...
call npm install
if %errorlevel% neq 0 goto :error

echo Building Tauri Application...
call npm run tauri build
if %errorlevel% neq 0 goto :error

:: --- STAGE 4: COLLECT ---
if not exist "dist" mkdir "dist"
copy /Y "src-tauri\target\release\bundle\msi\*.msi" "dist\" >nul 2>nul
copy /Y "src-tauri\target\release\bundle\nsis\*.exe" "dist\" >nul 2>nul

echo.
echo Build complete! Your installers are in the 'dist' folder.
pause
exit /b 0

:error
echo.
echo Build failed.
pause
exit /b 1
"""

    # Write the files
    with open(os.path.join(output_dir, "build_linux.sh"), "w", newline="\n", encoding="utf-8") as f:
        f.write(linux_script)
    print("✅ Generated build_linux.sh")

    with open(os.path.join(output_dir, "build_windows.ps1"), "w", newline="\n", encoding="utf-8") as f:
        f.write(ps1_script)
    print("✅ Generated build_windows.ps1")

    with open(os.path.join(output_dir, "build_windows.cmd"), "w", newline="\n", encoding="utf-8") as f:
        f.write(cmd_script)
    print("✅ Generated build_windows.cmd")

    # Make the Linux script executable (only works on Unix-like systems)
    if os.name != 'nt':
        try:
            os.chmod(os.path.join(output_dir, "build_linux.sh"), 0o755)
            print("🔧 Made build_linux.sh executable")
        except Exception as e:
            print(f"⚠️ Could not make script executable: {e}")
            
    print("\n🎉 All done! You can find your scripts in the 'installers' folder.")

if __name__ == "__main__":
    create_installer_scripts()