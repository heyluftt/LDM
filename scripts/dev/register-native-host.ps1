param(
  [Parameter(Mandatory = $true)]
  [string]$HostExe,

  # The id of the LDM extension, which follows from the key in its manifest. Pass another id
  # only for an extension build with a different key.
  [string]$ExtensionId = "dpfifoelihelgibggmccbedplhjpfmdc"
)

$resolvedHost = Resolve-Path -LiteralPath $HostExe
$manifestDir = Join-Path $env:LOCALAPPDATA "LDM\native-host"
$manifestPath = Join-Path $manifestDir "dev.ldm.native_host.json"

New-Item -ItemType Directory -Force -Path $manifestDir | Out-Null

$manifest = @{
  name = "dev.ldm.native_host"
  description = "LDM native messaging host"
  path = $resolvedHost.Path
  type = "stdio"
  allowed_origins = @("chrome-extension://$ExtensionId/")
} | ConvertTo-Json -Depth 4

Set-Content -LiteralPath $manifestPath -Value $manifest -Encoding UTF8

$chromeKey = "HKCU:\Software\Google\Chrome\NativeMessagingHosts\dev.ldm.native_host"
$edgeKey = "HKCU:\Software\Microsoft\Edge\NativeMessagingHosts\dev.ldm.native_host"

New-Item -Force -Path $chromeKey | Out-Null
New-Item -Force -Path $edgeKey | Out-Null
(Get-Item -Path $chromeKey).SetValue("", $manifestPath)
(Get-Item -Path $edgeKey).SetValue("", $manifestPath)

Write-Output "Registered dev.ldm.native_host"
