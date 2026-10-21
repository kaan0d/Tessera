; Per-user uninstall: take the Explorer menu, autostart entry and Send To shortcut out
; (tessera.exe --remove-menu does all three), then delete the app data (settings, history,
; downloaded ffmpeg, menu icons).
!macro NSIS_HOOK_PREUNINSTALL
  nsExec::Exec '"$INSTDIR\tessera.exe" --remove-menu'
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  RMDir /r "$APPDATA\Tessera"
!macroend
