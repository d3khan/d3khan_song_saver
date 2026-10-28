@echo off
setlocal enabledelayedexpansion

where cargo >nul 2>nul
if %errorlevel% neq 0 ( echo Rust not installed. & pause & exit /b 1 )
where node >nul 2>nul
if %errorlevel% neq 0 ( echo Node.js not installed. & pause & exit /b 1 )
where python >nul 2>nul
if %errorlevel% neq 0 ( echo Python not installed. & pause & exit /b 1 )

:: --- STAGE 1: REPO LOCATION & GIT PULL ---
set REPO_NAME=spotify_downloader
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
    git clone https://github.com/d3khan/spotify_downloader.git
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

:: --- STAGE 4: PREPARE BINARIES & FFMPEG ---
if not exist "src-tauri\binaries" mkdir "src-tauri\binaries"
for /f "tokens=2" %%i in ('rustc -Vv ^| findstr host') do set TARGET=%%i

echo Moving Python sidecar to Tauri binaries...
copy /Y "dist\downloader.exe" "src-tauri\binaries\downloader-!TARGET!.exe" >nul

if not exist "src-tauri\binaries\ffmpeg-!TARGET!.exe" (
    echo Downloading static FFmpeg...
    curl.exe -L -C - -o "ffmpeg.zip" "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl.zip"
    powershell -Command "Expand-Archive -Path 'ffmpeg.zip' -DestinationPath 'ffmpeg-temp' -Force"
    copy /Y "ffmpeg-temp\ffmpeg-master-latest-win64-gpl\bin\ffmpeg.exe" "src-tauri\binaries\ffmpeg-!TARGET!.exe" >nul
    rmdir /S /Q "ffmpeg-temp"
    del /F /Q "ffmpeg.zip"
) else (
    echo FFmpeg already bundled.
)

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
