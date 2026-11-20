# Builds the GloriaSetup Dummy test setups for 0.1.0 (icon A) and 0.2.0 (icon B) into dist\dummy.
#   powershell -ExecutionPolicy Bypass -File examples\dummy\pack.ps1
# 0.1.0 ships an extra `old-notes.txt` that 0.2.0 drops, so an update shows obsolete-file removal.
$ErrorActionPreference = 'Stop'
$root = Resolve-Path "$PSScriptRoot\..\.."
Push-Location $root
try {
    cargo build --release -p gloria-stub -p gloria-pack -p gloria-dummy
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
    $rel = "$root\target\release"
    $out = "$root\dist\dummy"
    New-Item -ItemType Directory -Force $out | Out-Null
    foreach ($v in @(
            @{ version = '0.1.0'; icon = 'dummy-a.ico'; extra = $true },
            @{ version = '0.2.0'; icon = 'dummy-b.ico'; extra = $false })) {
        $app = "$root\target\dummy-app\$($v.version)"
        if (Test-Path $app) { Remove-Item -Recurse -Force $app }
        New-Item -ItemType Directory -Force "$app\docs" | Out-Null
        Copy-Item "$rel\dummy.exe" "$app\dummy.exe"
        Set-Content -Encoding ascii "$app\version.txt" $v.version
        Set-Content -Encoding ascii "$app\docs\readme.txt" "GloriaSetup Dummy $($v.version)"
        if ($v.extra) { Set-Content -Encoding ascii "$app\old-notes.txt" "only in 0.1.0" }
        & "$rel\gloria-pack.exe" --app $app --manifest "$PSScriptRoot\gloria.toml" --version $v.version `
            --icon "$PSScriptRoot\icons\$($v.icon)" --stub "$rel\gloria-stub.exe" --out $out
        if ($LASTEXITCODE -ne 0) { throw "gloria-pack failed for $($v.version)" }
    }
    Get-ChildItem $out -Filter *-setup.exe | Format-Table Name, Length
} finally {
    Pop-Location
}
