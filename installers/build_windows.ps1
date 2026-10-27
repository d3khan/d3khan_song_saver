$ErrorActionPreference = "Stop"

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
