$chromeKey = "HKCU:\Software\Google\Chrome\NativeMessagingHosts\dev.ldm.native_host"
$edgeKey = "HKCU:\Software\Microsoft\Edge\NativeMessagingHosts\dev.ldm.native_host"

if (Test-Path $chromeKey) {
  Remove-Item -Path $chromeKey -Force
}

if (Test-Path $edgeKey) {
  Remove-Item -Path $edgeKey -Force
}

Write-Output "Unregistered dev.ldm.native_host"
