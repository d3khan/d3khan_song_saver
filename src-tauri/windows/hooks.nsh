!include "MUI2.nsh"
!include "LogicLib.nsh"

!macro NSIS_HOOK_POSTINSTALL
  ; FFmpeg is now bundled via Tauri's externalBin mechanism.
  ; No separate download is needed or performed.
  DetailPrint "Bundled dependencies are ready."
!macroend
