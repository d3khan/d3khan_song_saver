import os

def create_build_scripts():
    output_dir = "build_scripts"
    os.makedirs(output_dir, exist_ok=True)
    print(f"📁 Created directory: {output_dir}/")

    # ==========================================
    # 1. LINUX / MAC SCRIPT
    # ==========================================
    linux_script = r"""#!/bin/bash
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
"""

    # ==========================================
    # 2. WINDOWS POWERSHELL SCRIPT
    # ==========================================
    ps1_script = r"""$ErrorActionPreference = "Stop"

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
"""

    # ==========================================
    # 3. WINDOWS CMD SCRIPT
    # ==========================================
    bat_script = r"""@echo off
setlocal enabledelayedexpansion

where cargo >nul 2>nul
if %errorlevel% neq 0 ( echo Rust not installed. & pause & exit /b 1 )
where node >nul 2>nul
if %errorlevel% neq 0 ( echo Node.js not installed. & pause & exit /b 1 )
where python >nul 2>nul
if %errorlevel% neq 0 ( echo Python not installed. & pause & exit /b 1 )

:: --- STAGE 1: REPO LOCATION & GIT PULL ---
set REPO_NAME=d3khan_song_saver
if exist "package.json" (
    echo Found repository in current directory.
) else if exist "..\package.json" (
    pushd ..
    echo Found repository in parent directory. Moved up.
) else if exist "%REPO_NAME%\package.json" (
    cd %REPO_NAME%
    echo Found repository in subdirectory. Moved in.
) else (
    echo Repository not found. Cloning...
    git clone https://github.com/d3khan/d3khan_song_saver.git
    cd %REPO_NAME%
)

if exist ".git" (
    echo Pulling latest changes from Git...
    git pull
)

:: --- STAGE 2: PYTHON DEPENDENCIES ---
echo Ensuring Python dependencies are installed...
call python -m pip install -q yt-dlp mutagen requests pyinstaller

:: --- STAGE 3: BUILD SIDECAR (SKIP IF UNCHANGED) ---
if not exist "dist\downloader.exe" (
    echo Compiling Python sidecar...
    call pyinstaller --noconfirm --onefile --name downloader downloader.py
) else (
    echo Python sidecar already exists. Skipping PyInstaller.
)

:: --- STAGE 4: PREPARE SIDECAR ---
if not exist "src-tauri\binaries" mkdir "src-tauri\binaries"
for /f "tokens=2" %%i in ('rustc -Vv ^| findstr host') do set TARGET=%%i

echo Moving Python sidecar to Tauri binaries...
copy /Y "dist\downloader.exe" "src-tauri\binaries\downloader-!TARGET!.exe" >nul

:: --- STAGE 5: NODE & BUILD ---
if not exist "node_modules" (
    echo Installing Node dependencies...
    call npm install
) else (
    echo Node dependencies already installed.
)

echo Building Tauri Application...
call npm run tauri build
if %errorlevel% neq 0 goto :error

if not exist "dist_out" mkdir "dist_out"
copy /Y "src-tauri\target\release\bundle\msi\*.msi" "dist_out\" >nul 2>nul
copy /Y "src-tauri\target\release\bundle\nsis\*.exe" "dist_out\" >nul 2>nul

echo.
echo Build complete! Check the 'dist_out/' folder.
pause
exit /b 0

:error
echo Build failed.
pause
exit /b 1
"""

    with open(os.path.join(output_dir, "build.sh"), "w", newline="\n", encoding="utf-8") as f:
        f.write(linux_script)
    print("✅ Generated build.sh")

    with open(os.path.join(output_dir, "build.ps1"), "w", newline="\n", encoding="utf-8") as f:
        f.write(ps1_script)
    print("✅ Generated build.ps1")

    with open(os.path.join(output_dir, "build.bat"), "w", newline="\n", encoding="utf-8") as f:
        f.write(bat_script)
    print("✅ Generated build.bat")

    if os.name != 'nt':
        try:
            os.chmod(os.path.join(output_dir, "build.sh"), 0o755)
            print("🔧 Made build.sh executable")
        except Exception:
            pass
            
    print("\n🎉 All done! You can find your scripts in the 'build_scripts' folder.")

if __name__ == "__main__":
    create_build_scripts()