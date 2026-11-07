# Bouwt de release, zet de inhoud klaar in installer\build en maakt de setup met Inno Setup 6.
# Versie komt uit [workspace.package] in Cargo.toml.
#
#   powershell -ExecutionPolicy Bypass -File installer\build-installer.ps1 [-OutDir <map>] [-NoBuild]
param(
    [string]$OutDir = '',
    [switch]$NoBuild,
    # Alleen voor een testinstallatie: andere naam voor de bureaubladsnelkoppeling.
    [string]$DesktopName = ''
)
$ErrorActionPreference = 'Stop'

$repo = Split-Path -Parent $PSScriptRoot
if (-not $OutDir) {
    $shared = if ($env:KBA_SHARED) { $env:KBA_SHARED } else { 'D:\dev\shared' }
    $OutDir = Join-Path $shared 'uitvoer\sheetdiff\installer'
}

$toml = [IO.File]::ReadAllText((Join-Path $repo 'Cargo.toml'))
$m = [regex]::Match($toml, '(?s)\[workspace\.package\].*?version\s*=\s*"([^"]+)"')
if (-not $m.Success) { throw 'geen versie gevonden in Cargo.toml' }
$version = $m.Groups[1].Value

$iscc = @(
    (Join-Path $env:LOCALAPPDATA 'Programs\Inno Setup 6\ISCC.exe'),
    'C:\Program Files (x86)\Inno Setup 6\ISCC.exe',
    'C:\Program Files\Inno Setup 6\ISCC.exe'
) | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $iscc) { throw 'Inno Setup 6 niet gevonden: winget install --id JRSoftware.InnoSetup -e' }

if (-not $NoBuild) {
    Push-Location $repo
    try {
        cargo build --release
        if ($LASTEXITCODE -ne 0) { throw "cargo build faalde ($LASTEXITCODE)" }
    } finally {
        Pop-Location
    }
}

$stage = Join-Path $PSScriptRoot 'build'
if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
New-Item -ItemType Directory -Force $stage | Out-Null
$files = @(
    @('target\release\sheetdiff.exe', 'sheetdiff.exe'),
    @('target\release\sheetdiff-cli.exe', 'sheetdiff-cli.exe'),
    @('vendor\pdfium\bin\pdfium.dll', 'pdfium.dll'),
    @('vendor\pdfium\LICENSE', 'LICENSE-pdfium.txt'),
    @('README.md', 'README.md'),
    @('LICENSE-MIT', 'LICENSE-MIT'),
    @('LICENSE-APACHE', 'LICENSE-APACHE')
)
foreach ($f in $files) {
    Copy-Item (Join-Path $repo $f[0]) (Join-Path $stage $f[1])
}

New-Item -ItemType Directory -Force $OutDir | Out-Null
$isccArgs = @("/DAppVersion=$version", "/O$OutDir")
if ($DesktopName) { $isccArgs += "/DDesktopName=$DesktopName" }
$isccArgs += (Join-Path $PSScriptRoot 'sheetdiff.iss')
& $iscc @isccArgs
if ($LASTEXITCODE -ne 0) { throw "ISCC faalde ($LASTEXITCODE)" }

$setup = Join-Path $OutDir "sheetdiff-setup-$version.exe"
if (-not (Test-Path $setup)) { throw "setup niet gevonden: $setup" }
Write-Host "gemaakt: $setup ($([math]::Round((Get-Item $setup).Length / 1MB, 1)) MB)"
