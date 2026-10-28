!include "MUI2.nsh"
!include "LogicLib.nsh"

!macro NSIS_HOOK_POSTINSTALL
  SetShellVarContext current
  CreateDirectory "$LOCALAPPDATA\d3khan song saver\bin"

  DetailPrint "Downloading yt-dlp…"
  inetc::get /TIMEOUT 60000 \
    "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe" \
    "$LOCALAPPDATA\d3khan song saver\bin\yt-dlp.exe" /END
  Pop $R0
  ${If} $R0 != "OK"
    MessageBox MB_ICONSTOP "Failed to download yt-dlp: $R0"
    Abort
  ${EndIf}

  DetailPrint "Downloading FFmpeg…"
  inetc::get /TIMEOUT 120000 \
    "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl.zip" \
    "$TEMP\ffmpeg.zip" /END
  Pop $R0
  ${If} $R0 != "OK"
    MessageBox MB_ICONSTOP "Failed to download FFmpeg: $R0"
    Abort
  ${EndIf}

  DetailPrint "Extracting FFmpeg…"
  nsisunz::UnzipToLog "$TEMP\ffmpeg.zip" "$TEMP\ffmpeg-extract"
  nsExec::ExecToLog 'cmd /c move /Y "$TEMP\ffmpeg-extract\ffmpeg-master-latest-win64-gpl\bin\ffmpeg.exe" "$LOCALAPPDATA\d3khan song saver\bin\ffmpeg.exe"'
  nsExec::ExecToLog 'cmd /c move /Y "$TEMP\ffmpeg-extract\ffmpeg-master-latest-win64-gpl\bin\ffprobe.exe" "$LOCALAPPDATA\d3khan song saver\bin\ffprobe.exe"'

  Delete "$TEMP\ffmpeg.zip"
  RMDir /r "$TEMP\ffmpeg-extract"
!macroend