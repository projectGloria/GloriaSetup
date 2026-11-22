param(
  [switch]$Build,
  [switch]$Pack
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Push-Location $root
try {
  $cargoVersion = (Select-String -Path 'Cargo.toml' -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value
  $npmPkg = Get-Content 'npm\package.json' -Raw | ConvertFrom-Json
  if ($npmPkg.version -ne $cargoVersion) {
    throw "Version mismatch: Cargo.toml has $cargoVersion but npm/package.json has $($npmPkg.version)"
  }

  if ($Build) {
    cargo build --release -p gloria-stub -p gloria-pack
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
  }

  $packExe = 'target\release\gloria-pack.exe'
  $stubExe = 'target\release\gloria-stub.exe'
  if (-not (Test-Path $packExe) -or -not (Test-Path $stubExe)) {
    throw 'Release binaries missing. Run with -Build or run: cargo build --release -p gloria-stub -p gloria-pack'
  }

  New-Item -ItemType Directory -Force 'npm\bin' | Out-Null
  Copy-Item $packExe, $stubExe 'npm\bin\' -Force

  Push-Location 'npm'
  try {
    npm test
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    if ($Pack) {
      npm pack
      if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    }
  } finally {
    Pop-Location
  }
} finally {
  Pop-Location
}
