# Makes a throwaway self-signed code-signing certificate as a .pfx, for testing gloria-pack --sign-pfx only.
# Windows doesn't trust it, so a setup signed with it still shows "Unknown publisher". The certificate is removed
# from the user's store right after the export; only the .pfx file is left.
#
#   $env:GLORIA_SIGN_PASSWORD = 'test'
#   powershell -ExecutionPolicy Bypass -File tools\make-test-cert.ps1 -Out target\test-sign.pfx
#   $env:GLORIA_TEST_SIGN_PFX = (Resolve-Path target\test-sign.pfx)
#   cargo test -p gloria-pack --test e2e signed

param(
    [Parameter(Mandatory = $true)][string]$Out,
    [string]$Password = $env:GLORIA_SIGN_PASSWORD
)
$ErrorActionPreference = 'Stop'
if (-not $Password) { throw 'set GLORIA_SIGN_PASSWORD or pass -Password' }

$cert = New-SelfSignedCertificate -Type CodeSigningCert -Subject 'CN=GloriaSetup Test Signing' `
    -CertStoreLocation Cert:\CurrentUser\My -KeyExportPolicy Exportable -NotAfter (Get-Date).AddDays(7)
try {
    $dir = Split-Path -Parent $Out
    if ($dir) { New-Item -ItemType Directory -Force $dir | Out-Null }
    $pw = ConvertTo-SecureString $Password -AsPlainText -Force
    Export-PfxCertificate -Cert $cert -FilePath $Out -Password $pw | Out-Null
} finally {
    Remove-Item "Cert:\CurrentUser\My\$($cert.Thumbprint)" -DeleteKey
}
Write-Host "test certificate written to $Out (expires in 7 days)"
