# Zotero Search Bridge - build & package script
#
# Usage (from repo root):
#   powershell -ExecutionPolicy Bypass -File scripts\release.ps1
#   powershell -ExecutionPolicy Bypass -File scripts\release.ps1 -SkipBuild
#
# Artifacts go to target\dist\:
#   zsb-portable-v<version>-windows-x64.zip   portable (no-install) package
#   *.msi / *-setup.exe                       installers (if previously built via tauri build)
#
# Release: attach files under target\dist\ to a GitHub Release, or push a tag
# to trigger .github/workflows/release.yml.

param(
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
Set-Location $Root

# Build tools may not be on PATH (rustup / node default locations)
foreach ($p in @("$env:USERPROFILE\.cargo\bin", "C:\Program Files\nodejs")) {
    if ((Test-Path $p) -and ($env:PATH -notlike "*$p*")) { $env:PATH = "$p;$env:PATH" }
}

# Version from tauri.conf.json
$conf = Get-Content "apps\desktop\src-tauri\tauri.conf.json" -Raw | ConvertFrom-Json
$Version = $conf.version
Write-Host "==> Version: v$Version"

if (-not $SkipBuild) {
    Write-Host "==> Building frontend"
    Push-Location "apps\desktop"
    npm run build
    if ($LASTEXITCODE -ne 0) { throw "frontend build failed" }
    Pop-Location

    Write-Host "==> Building Rust release (zsb + zsb-desktop)"
    cargo build --release -p zsb -p zsb-desktop
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
}

$Dist = Join-Path $Root "target\dist"
$Stage = Join-Path $Root "target\portable\zsb-portable"
Remove-Item $Stage -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $Stage, $Dist | Out-Null

Write-Host "==> Assembling portable package"
Copy-Item "target\release\zsb.exe" $Stage
Copy-Item "target\release\zsb-desktop.exe" $Stage
# Static assets live in dist-portable\ (说明.txt, zsb-config.toml)
Copy-Item "dist-portable\*" $Stage -Recurse

$ZipName = "zsb-portable-v$Version-windows-x64.zip"
$ZipPath = Join-Path $Dist $ZipName
Remove-Item $ZipPath -Force -ErrorAction SilentlyContinue
Compress-Archive -Path $Stage -DestinationPath $ZipPath -CompressionLevel Optimal
$mb = [math]::Round((Get-Item $ZipPath).Length / 1MB, 1)
Write-Host "    $ZipPath  ($mb MB)"

# Installers (copy if present)
foreach ($sub in @("nsis", "msi")) {
    $bundle = "target\release\bundle\$sub"
    if (Test-Path $bundle) {
        Get-ChildItem "$bundle\*" -Include *.exe, *.msi -File |
            ForEach-Object { Copy-Item $_.FullName $Dist; Write-Host "    copied $($_.Name)" }
    }
}

Write-Host ""
Write-Host "Done. Artifacts:"
Get-ChildItem $Dist | ForEach-Object { Write-Host "  target\dist\$($_.Name)" }
Write-Host ""
Write-Host "Release: push a tag (git tag v$Version; git push --tags) to trigger GitHub Actions,"
Write-Host "or attach the files above to a GitHub Release manually."
