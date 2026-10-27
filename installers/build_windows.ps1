$ErrorActionPreference = "Stop"

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
