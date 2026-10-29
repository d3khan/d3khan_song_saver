$ErrorActionPreference = "Stop"

Write-Host "🔍 Checking prerequisites..." -ForegroundColor Cyan
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) { Write-Host "❌ Rust not installed."; exit 1 }
if (-not (Get-Command node -ErrorAction SilentlyContinue)) { Write-Host "❌ Node.js not installed."; exit 1 }
if (-not (Get-Command python -ErrorAction SilentlyContinue)) { Write-Host "❌ Python not installed."; exit 1 }

# --- STAGE 1: REPO LOCATION & GIT PULL ---
$RepoName = "d3khan_song_saver"
if (Test-Path "package.json") {
    Write-Host "✅ Found repository in current directory." -ForegroundColor Green
} elseif (Test-Path "..\package.json") {
    Set-Location ".."; Write-Host "✅ Found repository in parent directory. Moved up." -ForegroundColor Green
} elseif (Test-Path "$RepoName\package.json") {
    Set-Location $RepoName; Write-Host "✅ Found repository in subdirectory. Moved in." -ForegroundColor Green
} else {
    Write-Host "📥 Repository not found. Cloning..." -ForegroundColor Yellow
    git clone https://github.com/d3khan/d3khan_song_saver.git
    Set-Location $RepoName
}

if (Test-Path ".git") {
    Write-Host "🔄 Pulling latest changes from Git..." -ForegroundColor Cyan
    git pull
}

# --- STAGE 2: PYTHON DEPENDENCIES ---
Write-Host "📦 Ensuring Python dependencies are installed..." -ForegroundColor Cyan
python -m pip install -q yt-dlp mutagen requests pyinstaller

# --- STAGE 3: BUILD SIDECAR (SKIP IF UNCHANGED) ---
$NeedPyBuild = $false
if (-not (Test-Path "dist\downloader.exe")) { $NeedPyBuild = $true }
elseif ((Get-Item "downloader.py").LastWriteTime -gt (Get-Item "dist\downloader.exe").LastWriteTime) { $NeedPyBuild = $true }

if ($NeedPyBuild) {
    Write-Host "🐍 Compiling Python sidecar (downloader.py)..." -ForegroundColor Cyan
    pyinstaller --noconfirm --onefile --name downloader downloader.py
} else {
    Write-Host "✅ Python sidecar is up to date." -ForegroundColor Green
}

# --- STAGE 4: PREPARE SIDECAR ---
New-Item -ItemType Directory -Force -Path "src-tauri\binaries" | Out-Null
$TargetLine = (rustc -Vv | Select-String "host").ToString()
$Target = $TargetLine.Split(" ")[1]

Write-Host "🚚 Moving Python sidecar to Tauri binaries..." -ForegroundColor Cyan
Copy-Item -Path "dist\downloader.exe" -Destination "src-tauri\binaries\downloader-$Target.exe" -Force

# --- STAGE 5: NODE & BUILD ---
if (-not (Test-Path "node_modules")) {
    Write-Host "📦 Installing Node dependencies..." -ForegroundColor Cyan
    npm install
} else {
    Write-Host "✅ Node dependencies already installed." -ForegroundColor Green
}

Write-Host "🛠️ Building Tauri Application..." -ForegroundColor Cyan
npm run tauri build

Write-Host "📁 Collecting build artifacts..." -ForegroundColor Cyan
New-Item -ItemType Directory -Force -Path "dist_out" | Out-Null
Copy-Item "src-tauri\target\release\bundle\msi\*.msi" -Destination "dist_out\" -ErrorAction SilentlyContinue
Copy-Item "src-tauri\target\release\bundle\nsis\*.exe" -Destination "dist_out\" -ErrorAction SilentlyContinue
Write-Host "✅ Build complete! Check the 'dist_out/' folder." -ForegroundColor Green
