; Hooks for the NSIS installer that Tauri generates (bundle.windows.nsis.installerHooks in
; tauri.conf.json).
;
; The browser keeps ldm-native-host.exe running for as long as the extension holds its
; connection open, and Windows does not let a running executable be replaced or deleted.
; Tauri's own check only covers the desktop app, so an update right after a download stopped
; with "Error opening file for writing". Installer and uninstaller therefore end the native
; host first. Nothing is lost: it keeps no state, and the browser starts it again with the
; next download.

!macro LdmStopNativeHost
  Push $0
  nsExec::Exec '"$SYSDIR\taskkill.exe" /F /IM ldm-native-host.exe'
  Pop $0 ; taskkill's exit code; it is 128 when no native host runs, which is just as well
  Pop $0
  ; The file is released a moment after the process is gone.
  Sleep 300
!macroend

!macro NSIS_HOOK_PREINSTALL
  !insertmacro LdmStopNativeHost
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  !insertmacro LdmStopNativeHost
!macroend

; The desktop app registers the native host for the browsers on every start
; (src/native_host.rs). An uninstall takes that registration away again, so no browser is
; left pointing at a host that no longer exists. An update keeps it.
!macro NSIS_HOOK_POSTUNINSTALL
  ${If} $UpdateMode <> 1
    DeleteRegKey HKCU "Software\Google\Chrome\NativeMessagingHosts\dev.ldm.native_host"
    DeleteRegKey HKCU "Software\Microsoft\Edge\NativeMessagingHosts\dev.ldm.native_host"
    DeleteRegKey HKCU "Software\BraveSoftware\Brave-Browser\NativeMessagingHosts\dev.ldm.native_host"
    DeleteRegKey HKCU "Software\BraveSoftware\Brave-Origin\NativeMessagingHosts\dev.ldm.native_host"
    RMDir /r "$LOCALAPPDATA\LDM\native-host"
  ${EndIf}
!macroend
