@echo off
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
