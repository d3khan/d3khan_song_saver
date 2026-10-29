!include "MUI2.nsh"
!include "LogicLib.nsh"

!macro NSIS_HOOK_POSTINSTALL
  SetShellVarContext current
  CreateDirectory "$LOCALAPPDATA\com.d3khan.songsaver\bin"

  ; 1. Check if ffmpeg.exe is already installed
  IfFileExists "$LOCALAPPDATA\com.d3khan.songsaver\bin\ffmpeg.exe" ffmpeg_done

  ; 2. Check if we have a previously downloaded zip
  IfFileExists "$LOCALAPPDATA\com.d3khan.songsaver\ffmpeg.zip" try_extract download_new

  try_extract:
    DetailPrint "Found existing FFmpeg archive. Attempting extraction..."
    ; Use PowerShell to extract
    nsExec::ExecToLog 'powershell -NoProfile -ExecutionPolicy Bypass -Command "Expand-Archive -Path \"$LOCALAPPDATA\com.d3khan.songsaver\ffmpeg.zip\" -DestinationPath \"$LOCALAPPDATA\com.d3khan.songsaver\bin\" -Force"'
    Pop $R0
    ${If} $R0 == "0"
      Goto move_ffmpeg
    ${Else}
      DetailPrint "Archive is corrupt. Deleting and re-downloading..."
      Delete "$LOCALAPPDATA\com.d3khan.songsaver\ffmpeg.zip"
      Goto download_new
    ${EndIf}

  download_new:
    DetailPrint "Downloading FFmpeg (approx 100MB, this may take a moment)..."
    ; Use PowerShell to download
    nsExec::ExecToLog 'powershell -NoProfile -ExecutionPolicy Bypass -Command "Invoke-WebRequest -Uri \"https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl.zip\" -OutFile \"$LOCALAPPDATA\com.d3khan.songsaver\ffmpeg.zip\""'
    Pop $R0
    ${If} $R0 != "0"
      MessageBox MB_ICONSTOP "Failed to download FFmpeg."
      Abort
    ${EndIf}
    
    DetailPrint "Extracting FFmpeg..."
    nsExec::ExecToLog 'powershell -NoProfile -ExecutionPolicy Bypass -Command "Expand-Archive -Path \"$LOCALAPPDATA\com.d3khan.songsaver\ffmpeg.zip\" -DestinationPath \"$LOCALAPPDATA\com.d3khan.songsaver\bin\" -Force"'
    Pop $R0
    ${If} $R0 != "0"
      MessageBox MB_ICONSTOP "Failed to extract FFmpeg."
      Abort
    ${EndIf}

  move_ffmpeg:
    DetailPrint "Finalizing FFmpeg installation..."
    Rename /REBOOTOK "$LOCALAPPDATA\com.d3khan.songsaver\bin\ffmpeg-master-latest-win64-gpl\bin\ffmpeg.exe" "$LOCALAPPDATA\com.d3khan.songsaver\bin\ffmpeg.exe"
    Rename /REBOOTOK "$LOCALAPPDATA\com.d3khan.songsaver\bin\ffmpeg-master-latest-win64-gpl\bin\ffprobe.exe" "$LOCALAPPDATA\com.d3khan.songsaver\bin\ffprobe.exe"
    RMDir /r "$LOCALAPPDATA\com.d3khan.songsaver\bin\ffmpeg-master-latest-win64-gpl"
    
  ffmpeg_done:
    DetailPrint "FFmpeg is ready."
!macroend