; Installer hooks (bundle.windows.nsis.installerHooks).
; The browser keeps ldm-native-host.exe running, so the exe is locked; end it before install and uninstall.

!macro LdmStopNativeHost
  Push $0
  nsExec::Exec '"$SYSDIR\taskkill.exe" /F /IM ldm-native-host.exe'
  Pop $0 ; exit code, 128 if it was not running
  Pop $0
  ; The file is released shortly after the process ends.
  Sleep 300
!macroend

!macro NSIS_HOOK_PREINSTALL
  !insertmacro LdmStopNativeHost
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  !insertmacro LdmStopNativeHost
!macroend

; An uninstall (not an update) removes the registration the app made.
!macro NSIS_HOOK_POSTUNINSTALL
  ${If} $UpdateMode <> 1
    DeleteRegKey HKCU "Software\Google\Chrome\NativeMessagingHosts\dev.ldm.native_host"
    DeleteRegKey HKCU "Software\Microsoft\Edge\NativeMessagingHosts\dev.ldm.native_host"
    DeleteRegKey HKCU "Software\BraveSoftware\Brave-Browser\NativeMessagingHosts\dev.ldm.native_host"
    DeleteRegKey HKCU "Software\BraveSoftware\Brave-Origin\NativeMessagingHosts\dev.ldm.native_host"
    RMDir /r "$LOCALAPPDATA\LDM\native-host"
  ${EndIf}
!macroend
