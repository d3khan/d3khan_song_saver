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
    nsisunz::UnzipToLog "$LOCALAPPDATA\com.d3khan.songsaver\ffmpeg.zip" "$LOCALAPPDATA\com.d3khan.songsaver\bin"
    Pop $R0
    ${If} $R0 == "success"
      Goto move_ffmpeg
    ${Else}
      DetailPrint "Archive is corrupt. Deleting and re-downloading..."
      Delete "$LOCALAPPDATA\com.d3khan.songsaver\ffmpeg.zip"
      Goto download_new
    ${EndIf}

  download_new:
    DetailPrint "Downloading FFmpeg (approx 100MB, this may take a moment)..."
    inetc::get /TIMEOUT 120000 \
      "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl.zip" \
      "$LOCALAPPDATA\com.d3khan.songsaver\ffmpeg.zip" /END
    Pop $R0
    ${If} $R0 != "OK"
      MessageBox MB_ICONSTOP "Failed to download FFmpeg: $R0"
      Abort
    ${EndIf}
    
    DetailPrint "Extracting FFmpeg..."
    nsisunz::UnzipToLog "$LOCALAPPDATA\com.d3khan.songsaver\ffmpeg.zip" "$LOCALAPPDATA\com.d3khan.songsaver\bin"
    Pop $R0
    ${If} $R0 != "success"
      MessageBox MB_ICONSTOP "Failed to extract FFmpeg."
      Abort
    ${EndIf}

  move_ffmpeg:
    DetailPrint "Finalizing FFmpeg installation..."
    ; The zip extracts to ffmpeg-master-latest-win64-gpl\bin\ffmpeg.exe
    Rename /REBOOTOK "$LOCALAPPDATA\com.d3khan.songsaver\bin\ffmpeg-master-latest-win64-gpl\bin\ffmpeg.exe" "$LOCALAPPDATA\com.d3khan.songsaver\bin\ffmpeg.exe"
    RMDir /r "$LOCALAPPDATA\com.d3khan.songsaver\bin\ffmpeg-master-latest-win64-gpl"
    
  ffmpeg_done:
    DetailPrint "FFmpeg is ready."
!macroend