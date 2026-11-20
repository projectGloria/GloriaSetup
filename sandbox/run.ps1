# Opens Windows Sandbox with this checkout's dist folder mapped read-only to Desktop\dist.
#   powershell -ExecutionPolicy Bypass -File sandbox\run.ps1
# Run examples\dummy\pack.ps1 first. Also copies the bare release stub in as dist\GloriaSetup-preview.exe
# (a stub with no payload opens the preview).
$ErrorActionPreference = 'Stop'
$root = Resolve-Path "$PSScriptRoot\.."
$dist = "$root\dist"
New-Item -ItemType Directory -Force $dist | Out-Null
$stub = "$root\target\release\gloria-stub.exe"
if (Test-Path $stub) { Copy-Item $stub "$dist\GloriaSetup-preview.exe" -Force }
if (-not (Get-Command WindowsSandbox.exe -ErrorAction SilentlyContinue)) {
    throw "Windows Sandbox is not installed. Turn on the 'Windows Sandbox' optional feature (needs admin and a reboot), see README."
}
$wsb = "$env:TEMP\GloriaSetup.wsb"
(Get-Content -Raw "$PSScriptRoot\GloriaSetup.wsb").Replace('HOST_DIST', $dist) | Set-Content -Encoding utf8 $wsb
Start-Process $wsb
