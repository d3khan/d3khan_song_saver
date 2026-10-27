!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "FileFunc.nsh"

; --- NSIS Hook: Download yt-dlp and ffmpeg ---
!macro NSIS_HOOK_POSTINSTALL
  DetailPrint "Downloading required media tools (approx. 150 MB)..."
  
  ; Define URLs for the official static builds
  StrCpy $0 "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe"
  StrCpy $1 "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl.zip"
  
  ; Define destination paths next to the installed app executable
  StrCpy $2 "$INSTDIR\yt-dlp.exe"
  StrCpy $3 "$INSTDIR\ffmpeg.zip"
  StrCpy $4 "$INSTDIR\ffmpeg.exe"
  StrCpy $5 "$INSTDIR\ffprobe.exe"

  ; Download yt-dlp.exe
  DetailPrint "Downloading yt-dlp..."
  inetc::get /TIMEOUT 30000 "$0" "$2" /END
  Pop $R0
  ${If} $R0 != "OK"
    MessageBox MB_ICONSTOP "Failed to download yt-dlp: $R0"
    Abort
  ${EndIf}

  ; Download FFmpeg ZIP
  DetailPrint "Downloading FFmpeg (this may take a minute)..."
  inetc::get /TIMEOUT 60000 "$1" "$3" /END
  Pop $R0
  ${If} $R0 != "OK"
    MessageBox MB_ICONSTOP "Failed to download FFmpeg: $R0"
    Abort
  ${EndIf}

  ; Extract FFmpeg and FFprobe from the ZIP
  DetailPrint "Extracting FFmpeg binaries..."
  nsisunz::UnzipToLog "$3" "$INSTDIR"
  
  ; The ZIP usually extracts to a folder like "ffmpeg-master-latest-win64-gpl\bin\"
  ; We need to move the .exe files up to $INSTDIR
  nsExec::ExecToLog 'cmd /c move /Y "$INSTDIR\ffmpeg-master-latest-win64-gpl\bin\ffmpeg.exe" "$4"'
  nsExec::ExecToLog 'cmd /c move /Y "$INSTDIR\ffmpeg-master-latest-win64-gpl\bin\ffprobe.exe" "$5"'
  
  ; Clean up
  Delete "$3"
  RMDir /r "$INSTDIR\ffmpeg-master-latest-win64-gpl"
!macroend