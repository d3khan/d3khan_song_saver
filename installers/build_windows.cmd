@echo off
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
    for /f "delims=" %%v in ('powershell -Command "$h=(Invoke-WebRequest -Uri 'https://github.com/yt-dlp/yt-dlp/releases/latest' -UseBasicParsing).Content; if ($h -match 'tag/([0-9]+\.[0-9]+\.[0-9]+)') { $matches[1] }" 2^>nul') do set LATEST_YTDLP=%%v
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
    for /f "delims=" %%d in ('powershell -Command "$h=(Invoke-WebRequest -Uri 'https://github.com/BtbN/FFmpeg-Builds/releases/latest' -UseBasicParsing).Content; if ($h -match 'autobuild-([0-9]{4}-[0-9]{2}-[0-9]{2})') { $matches[1] }" 2^>nul') do set LATEST_DATE=%%d
    if "!LATEST_DATE!" gtr "!LOCAL_DATE!" (
        echo    FFmpeg is out of date ^(local: !LOCAL_DATE!, latest: !LATEST_DATE!^).
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
