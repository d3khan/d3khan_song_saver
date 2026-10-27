import os

def create_installer_scripts():
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
"""

    # ==========================================
    # 2. POWERSHELL SCRIPT
    # ==========================================
    ps1_script = r"""$ErrorActionPreference = "Stop"

Write-Host "🔍 Checking prerequisites..." -ForegroundColor Cyan
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) { Write-Host "❌ Rust not installed."; exit 1 }
if (-not (Get-Command node -ErrorAction SilentlyContinue)) { Write-Host "❌ Node.js not installed."; exit 1 }

# --- STAGE 1: REPO LOCATION ---
$RepoName = "spotify_downloader"
$InsideRepo = $false
if ((Test-Path "src-tauri") -and (Test-Path "package.json")) { $InsideRepo = $true; Write-Host "   ✅ Found repository in current directory." -ForegroundColor Green }
elseif ((Test-Path "..\src-tauri") -and (Test-Path "..\package.json")) { Set-Location ".."; $InsideRepo = $true; Write-Host "   ✅ Found repository in parent directory. Moved up." -ForegroundColor Green }
elseif ((Test-Path "$RepoName\src-tauri") -and (Test-Path "$RepoName\package.json")) { Set-Location $RepoName; $InsideRepo = $true; Write-Host "   ✅ Found repository in subdirectory. Moved in." -ForegroundColor Green }
if (-not $InsideRepo) { Write-Host "   📥 Repository not found. Cloning..." -ForegroundColor Yellow; git clone https://github.com/d3khan/spotify_downloader.git; Set-Location $RepoName }
if (Test-Path ".git") { Write-Host "🔄 Pulling latest changes from Git..." -ForegroundColor Cyan; git pull }

# --- STAGE 2: SIDECAR BINARIES (API-FREE VERSION CHECK) ---
Write-Host "🎵 Fetching sidecar binaries..." -ForegroundColor Cyan
New-Item -ItemType Directory -Force -Path "src-tauri\binaries" | Out-Null

# 1. yt-dlp
$YtDlpPath = "src-tauri\binaries\yt-dlp-x86_64-pc-windows-msvc.exe"
if ((Test-Path $YtDlpPath) -and (Get-Item $YtDlpPath).Length -gt 0) {
    Write-Host "   🔍 Checking yt-dlp version..."
    $LocalYtDlp = & $YtDlpPath --version 2>$null
    $LatestYtDlp = $null
    try {
        # Uses HEAD request via curl to avoid API rate limits
        $headers = curl.exe -sI "https://github.com/yt-dlp/yt-dlp/releases/latest"
        $loc = $headers | Select-String -Pattern "location:" | ForEach-Object { $_.Line }
        if ($loc -match "/tag/([^\s]+)") { $LatestYtDlp = $matches[1].Trim() }
    } catch {}

    if ($LatestYtDlp -and $LocalYtDlp -ne $LatestYtDlp) {
        Write-Host "   ⚠️ yt-dlp is out of date (local: $LocalYtDlp, latest: $LatestYtDlp)." -ForegroundColor Yellow
        $response = Read-Host "   Do you want to update it? (y/n)"
        if ($response -match "^[Yy]") {
            Write-Host "   ⬇️ Downloading yt-dlp..."
            curl.exe -f -L --retry 10 --retry-delay 3 -C - -o $YtDlpPath "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe"
        } else { Write-Host "   ⏭️ Skipping update." }
    } elseif ($LatestYtDlp) { Write-Host "   ✅ yt-dlp is up to date ($LocalYtDlp)." }
} else {
    Write-Host "   ⬇️ Downloading yt-dlp (not found)..."
    curl.exe -f -L --retry 10 --retry-delay 3 -C - -o $YtDlpPath "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe"
}

# 2. FFmpeg & FFprobe
$FfmpegPath = "src-tauri\binaries\ffmpeg-x86_64-pc-windows-msvc.exe"
$FfprobePath = "src-tauri\binaries\ffprobe-x86_64-pc-windows-msvc.exe"
$Archive = "ffmpeg-win.zip"
$TempDir = "ffmpeg-temp"
$NeedDownload = $false

if ((Test-Path $FfmpegPath) -and (Test-Path $FfprobePath) -and (Get-Item $FfmpegPath).Length -gt 0) {
    Write-Host "   🔍 Checking FFmpeg version..."
    $LocalDate = (Get-Item $FfmpegPath).LastWriteTime.ToString("yyyy-MM-dd")
    $LatestTag = $null
    try {
        $headers = curl.exe -sI "https://github.com/BtbN/FFmpeg-Builds/releases/latest"
        $loc = $headers | Select-String -Pattern "location:" | ForEach-Object { $_.Line }
        if ($loc -match "/tag/([^\s]+)") { $LatestTag = $matches[1].Trim() }
    } catch {}

    if ($LatestTag -and $LatestTag -match $LocalDate) {
        Write-Host "   ✅ FFmpeg is up to date." -ForegroundColor Green
    } elseif ($LatestTag) {
        Write-Host "   ⚠️ FFmpeg is out of date (local build: $LocalDate, latest build: $LatestTag)." -ForegroundColor Yellow
        $response = Read-Host "   Do you want to update FFmpeg & FFprobe? (y/n)"
        if ($response -match "^[Yy]") { $NeedDownload = $true } else { Write-Host "   ⏭️ Skipping update." }
    }
} else {
    Write-Host "   ⬇️ Downloading FFmpeg (not found)..."
    $NeedDownload = $true
}

if ($NeedDownload) {
    if (Test-Path $Archive) {
        try {
            Add-Type -AssemblyName System.IO.Compression.FileSystem
            [System.IO.Compression.ZipFile]::OpenRead((Resolve-Path $Archive)).Dispose()
        } catch { Remove-Item -Force $Archive }
    }
    if (-not (Test-Path $Archive)) {
        Write-Host "   ⬇️ Downloading FFmpeg..."
        curl.exe -f -L --retry 10 --retry-delay 3 -C - -o $Archive "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl.zip"
    }
    Write-Host "   📦 Extracting FFmpeg..."
    if (Test-Path $TempDir) { Remove-Item -Recurse -Force $TempDir }
    Expand-Archive -Path $Archive -DestinationPath $TempDir -Force
    Copy-Item "$TempDir\ffmpeg-master-latest-win64-gpl\bin\ffmpeg.exe" -Destination $FfmpegPath -Force
    Copy-Item "$TempDir\ffmpeg-master-latest-win64-gpl\bin\ffprobe.exe" -Destination $FfprobePath -Force
    Remove-Item -Recurse -Force $TempDir
    Remove-Item -Force $Archive
}

# --- STAGE 3: BUILD ---
if (-not (Test-Path "node_modules")) { Write-Host "📦 Installing Node dependencies (npm)..." -ForegroundColor Cyan; npm install } else { Write-Host "   ✅ Node dependencies already installed." -ForegroundColor Green }
Write-Host "🛠️ Building Tauri Application..." -ForegroundColor Cyan
npm run tauri build

# --- STAGE 4: COLLECT ---
Write-Host "📁 Collecting build artifacts..." -ForegroundColor Cyan
New-Item -ItemType Directory -Force -Path "dist" | Out-Null
Copy-Item "src-tauri\target\release\bundle\msi\*.msi" -Destination "dist\" -ErrorAction SilentlyContinue
Copy-Item "src-tauri\target\release\bundle\nsis\*.exe" -Destination "dist\" -ErrorAction SilentlyContinue
Write-Host "✅ Build complete! Your installers are ready in the 'dist' folder." -ForegroundColor Green
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
if exist "src-tauri" if exist "package.json" ( set INSIDE_REPO=1 & echo    Found repository in current directory. )
if !INSIDE_REPO! equ 0 if exist "..\src-tauri" if exist "..\package.json" ( pushd .. & set INSIDE_REPO=1 & echo    Found repository in parent directory. Moved up. )
if !INSIDE_REPO! equ 0 if exist "%REPO_NAME%\src-tauri" if exist "%REPO_NAME%\package.json" ( cd %REPO_NAME% & set INSIDE_REPO=1 & echo    Found repository in subdirectory. Moved in. )
if !INSIDE_REPO! equ 0 ( echo    Repository not found. Cloning... & git clone https://github.com/d3khan/spotify_downloader.git & cd %REPO_NAME% )
if exist ".git" ( echo Pulling latest changes from Git... & git pull )

:: --- STAGE 2: SIDECAR BINARIES ---
echo Fetching sidecar binaries...
if not exist "src-tauri\binaries" mkdir "src-tauri\binaries"

:: 1. yt-dlp
set YTDLP_PATH=src-tauri\binaries\yt-dlp-x86_64-pc-windows-msvc.exe
set NEED_YTDLP_UPDATE=0
if exist "%YTDLP_PATH%" (
    echo    Checking yt-dlp version...
    for /f "delims=" %%v in ('"%YTDLP_PATH%" --version 2^>nul') do set LOCAL_YTDLP=%%v
    for /f "delims=" %%v in ('powershell -Command "(curl.exe -sI 'https://github.com/yt-dlp/yt-dlp/releases/latest' | Select-String 'location:').ToString().Split('/tag/')[1].Trim()" 2^>nul') do set LATEST_YTDLP=%%v
    if "!LOCAL_YTDLP!" neq "!LATEST_YTDLP!" (
        echo    yt-dlp is out of date ^(local: !LOCAL_YTDLP!, latest: !LATEST_YTDLP!^).
        set /p UPDATE_YTDLP="   Do you want to update it? (y/n) "
        if /i "!UPDATE_YTDLP!"=="y" set NEED_YTDLP_UPDATE=1
    ) else ( echo    yt-dlp is up to date ^(!LOCAL_YTDLP!^). )
) else ( echo    Downloading yt-dlp ^(not found^)... & set NEED_YTDLP_UPDATE=1 )
if !NEED_YTDLP_UPDATE! equ 1 ( curl.exe -f -L --retry 10 --retry-delay 3 -C - -o "%YTDLP_PATH%" "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe" )

:: 2. FFmpeg & FFprobe
set FFMPEG_PATH=src-tauri\binaries\ffmpeg-x86_64-pc-windows-msvc.exe
set FFPROBE_PATH=src-tauri\binaries\ffprobe-x86_64-pc-windows-msvc.exe
set ARCHIVE=ffmpeg-win.zip
set TEMP_DIR=ffmpeg-temp
set NEED_FFMPEG_UPDATE=0
if exist "%FFMPEG_PATH%" (
    echo    Checking FFmpeg version...
    for /f "delims=" %%d in ('powershell -Command "(Get-Item '%FFMPEG_PATH%').LastWriteTime.ToString('yyyy-MM-dd')" 2^>nul') do set LOCAL_DATE=%%d
    for /f "delims=" %%d in ('powershell -Command "(curl.exe -sI 'https://github.com/BtbN/FFmpeg-Builds/releases/latest' | Select-String 'location:').ToString().Split('/tag/')[1].Trim()" 2^>nul') do set LATEST_TAG=%%d
    echo !LATEST_TAG! | findstr /C:"!LOCAL_DATE!" >nul
    if !errorlevel! neq 0 (
        echo    FFmpeg is out of date ^(local build: !LOCAL_DATE!, latest build: !LATEST_TAG!^).
        set /p UPDATE_FFMPEG="   Do you want to update FFmpeg & FFprobe? (y/n) "
        if /i "!UPDATE_FFMPEG!"=="y" set NEED_FFMPEG_UPDATE=1
    ) else ( echo    FFmpeg is up to date. )
) else ( echo    Downloading FFmpeg ^(not found^)... & set NEED_FFMPEG_UPDATE=1 )
if !NEED_FFMPEG_UPDATE! equ 1 (
    if exist "%ARCHIVE%" ( powershell -Command "try { Add-Type -AssemblyName System.IO.Compression.FileSystem; [System.IO.Compression.ZipFile]::OpenRead((Resolve-Path '%ARCHIVE%')).Dispose(); exit 0 } catch { exit 1 }" & if !errorlevel! neq 0 ( del /F /Q "%ARCHIVE%" ) )
    if not exist "%ARCHIVE%" ( curl.exe -f -L --retry 10 --retry-delay 3 -C - -o "%ARCHIVE%" "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl.zip" )
    echo    Extracting FFmpeg...
    if exist "%TEMP_DIR%" rmdir /S /Q "%TEMP_DIR%"
    powershell -Command "Expand-Archive -Path '%ARCHIVE%' -DestinationPath '%TEMP_DIR%' -Force"
    copy /Y "%TEMP_DIR%\ffmpeg-master-latest-win64-gpl\bin\ffmpeg.exe" "%FFMPEG_PATH%" >nul
    copy /Y "%TEMP_DIR%\ffmpeg-master-latest-win64-gpl\bin\ffprobe.exe" "%FFPROBE_PATH%" >nul
    rmdir /S /Q "%TEMP_DIR%"
    del /F /Q "%ARCHIVE%"
)

:: --- STAGE 3: BUILD ---
if not exist "node_modules" ( echo Installing Node dependencies... & call npm install & if %errorlevel% neq 0 goto :error ) else ( echo    Node dependencies already installed. )
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

    with open(os.path.join(output_dir, "build_linux.sh"), "w", newline="\n", encoding="utf-8") as f:
        f.write(linux_script)
    print("✅ Generated build_linux.sh")

    with open(os.path.join(output_dir, "build_windows.ps1"), "w", newline="\n", encoding="utf-8") as f:
        f.write(ps1_script)
    print("✅ Generated build_windows.ps1")

    with open(os.path.join(output_dir, "build_windows.cmd"), "w", newline="\n", encoding="utf-8") as f:
        f.write(cmd_script)
    print("✅ Generated build_windows.cmd")

    if os.name != 'nt':
        try:
            os.chmod(os.path.join(output_dir, "build_linux.sh"), 0o755)
            print("🔧 Made build_linux.sh executable")
        except Exception as e:
            print(f"⚠️ Could not make script executable: {e}")
            
    print("\n🎉 All done! You can find your scripts in the 'installers' folder.")

if __name__ == "__main__":
    create_installer_scripts()