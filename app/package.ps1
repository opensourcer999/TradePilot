<#
Builds TradePilot for passing on: a ZIP with the EXE (runs without installation) and, if Inno Setup is installed,
a setup program that installs everything on the new computer (Start menu, optional desktop icon, uninstaller).

Usage:
  .\package.ps1             # build the release (C runtime linked in), then create ZIP and setup in dist\
  .\package.ps1 -NoBuild    # use the existing release EXE, build nothing
  .\package.ps1 -Mac        # also have the macOS package (DMG, ZIP) built on GitHub and download it to dist\
  .\package.ps1 -MacOnly    # only the macOS package (needs gh: winget install GitHub.cli, then gh auth login)

The setup needs Inno Setup 6 (https://jrsoftware.org/isinfo.php, or: winget install JRSoftware.InnoSetup).
Without Inno Setup only the ZIP is created.

Note: build.ps1 ends running TradePilot processes. The package contains only the program, never the user's data
(%LOCALAPPDATA%\TradeWorkspace).
#>
param(
    [switch]$NoBuild,
    [switch]$Mac,
    [switch]$MacOnly
)

$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

# The macOS package is built by the "macos" job of the release workflow (it needs a Mac and Apple's SDK): this
# starts that job for the current branch, waits for it and downloads the DMG and ZIP into dist\.
function Invoke-MacBuild {
    if (-not (Get-Command gh -ErrorAction SilentlyContinue)) {
        Write-Host "GitHub CLI (gh) is missing: winget install GitHub.cli, then 'gh auth login'." -ForegroundColor Red
        return $false
    }
    $branch = (git rev-parse --abbrev-ref HEAD).Trim()
    $localHead = (git rev-parse HEAD).Trim()
    $remoteHead = (git ls-remote origin "refs/heads/$branch") -split "\s+" | Select-Object -First 1
    if ($remoteHead -ne $localHead) {
        Write-Host "The state of '$branch' is not on GitHub yet (push it first). The Mac build builds the pushed state." -ForegroundColor Red
        return $false
    }
    if (git status --porcelain) {
        Write-Host "Note: there are uncommitted changes; the Mac build contains only the pushed state." -ForegroundColor Yellow
    }
    $started = [DateTime]::UtcNow.AddSeconds(-5)
    Write-Host "Starting the macOS build on GitHub ($branch)..." -ForegroundColor Cyan
    gh workflow run release.yml --ref $branch
    if ($LASTEXITCODE -ne 0) { return $false }
    $runId = $null
    for ($attempt = 0; $attempt -lt 30 -and -not $runId; $attempt++) {
        Start-Sleep -Seconds 3
        $runs = gh run list --workflow release.yml --event workflow_dispatch --branch $branch --limit 5 --json databaseId,createdAt | ConvertFrom-Json
        $runId = $runs | Where-Object { [DateTime]::Parse($_.createdAt).ToUniversalTime() -ge $started } |
            Sort-Object createdAt -Descending | Select-Object -First 1 -ExpandProperty databaseId
    }
    if (-not $runId) {
        Write-Host "The started run was not found." -ForegroundColor Red
        return $false
    }
    gh run watch $runId --exit-status | Out-Null
    if ($LASTEXITCODE -ne 0) {
        Write-Host "The GitHub run failed: gh run view $runId --log-failed" -ForegroundColor Red
        return $false
    }
    $macDir = Join-Path $PSScriptRoot "dist"
    New-Item -ItemType Directory -Force -Path $macDir | Out-Null
    Get-ChildItem -LiteralPath $macDir -Filter "TradePilot-*-macos-*" -File -ErrorAction SilentlyContinue | Remove-Item -Force
    gh run download $runId --name TradePilot-macos --dir $macDir
    if ($LASTEXITCODE -ne 0) { return $false }
    Get-ChildItem -LiteralPath $macDir -Filter "TradePilot-*-macos-*" -File | ForEach-Object { Write-Host "macOS: $($_.FullName)" -ForegroundColor Green }
    return $true
}

if ($MacOnly) {
    if (-not (Invoke-MacBuild)) { exit 1 }
    exit 0
}

if (-not $NoBuild) {
    & (Join-Path $PSScriptRoot "build.ps1") -SkipClippy
    if ($LASTEXITCODE -ne 0) {
        Write-Host "Build failed - no package." -ForegroundColor Red
        exit $LASTEXITCODE
    }
}

$targetRoot = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $PSScriptRoot "target" }
$releaseDir = Join-Path $targetRoot "release"
$exe = Get-ChildItem -LiteralPath $releaseDir -Filter "TradePilot-V*.exe" -File -ErrorAction SilentlyContinue |
    Where-Object { $_.Name -notlike "*_debug*" } | Sort-Object LastWriteTime -Descending | Select-Object -First 1
if (-not $exe) {
    Write-Host "No release EXE found in $releaseDir. Build it first: .\build.ps1" -ForegroundColor Red
    exit 1
}

# The exe must not need the Visual C++ runtime DLL: it names it in its import table when the runtime is not linked in.
$bytes = [System.IO.File]::ReadAllBytes($exe.FullName)
$text = [System.Text.Encoding]::ASCII.GetString($bytes)
foreach ($dll in @("VCRUNTIME140.dll", "MSVCP140.dll")) {
    if ($text.IndexOf($dll, [System.StringComparison]::OrdinalIgnoreCase) -ge 0) {
        Write-Host "$($exe.Name) needs $dll - the C runtime is not linked in (build the release with build.ps1)." -ForegroundColor Red
        exit 1
    }
}

$version = (Get-Item -LiteralPath $exe.FullName).VersionInfo.ProductVersion
if (-not $version) { $version = "0.0.0" }
$dist = Join-Path $PSScriptRoot "dist"
New-Item -ItemType Directory -Force -Path $dist | Out-Null

# 1. The portable ZIP: the exe as TradePilot.exe, the file portable.txt (it switches the portable mode on: the data lives in Data next to the
#    exe, never in the user folder) and the short guide.
$staging = Join-Path $dist "staging"
if (Test-Path -LiteralPath $staging) { Remove-Item -LiteralPath $staging -Recurse -Force }
New-Item -ItemType Directory -Force -Path $staging | Out-Null
Copy-Item -LiteralPath $exe.FullName -Destination (Join-Path $staging "TradePilot.exe")
Copy-Item -LiteralPath (Join-Path $PSScriptRoot "installer\README.txt") -Destination $staging
Set-Content -LiteralPath (Join-Path $staging "portable.txt") -Encoding UTF8 -Value "This file switches the portable mode on: the data lives in the folder Data next to TradePilot.exe. Delete this file = data in the user folder (%LOCALAPPDATA%\TradeWorkspace)."
$zip = Join-Path $dist "TradePilot-$version-portable.zip"
if (Test-Path -LiteralPath $zip) { Remove-Item -LiteralPath $zip -Force }
Compress-Archive -Path (Join-Path $staging "*") -DestinationPath $zip
Remove-Item -LiteralPath $staging -Recurse -Force
Write-Host "ZIP: $zip" -ForegroundColor Green

# 2. The setup program, if Inno Setup is installed.
$candidates = @(
    (Get-Command ISCC.exe -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Source -ErrorAction SilentlyContinue),
    (Join-Path ${env:ProgramFiles(x86)} "Inno Setup 6\ISCC.exe"),
    (Join-Path $env:ProgramFiles "Inno Setup 6\ISCC.exe"),
    (Join-Path $env:LOCALAPPDATA "Programs\Inno Setup 6\ISCC.exe")
) | Where-Object { $_ -and (Test-Path -LiteralPath $_) }
$iscc = $candidates | Select-Object -First 1
if ($iscc) {
    & $iscc "/DAppVersion=$version" "/DSourceExe=$($exe.FullName)" "/O$dist" (Join-Path $PSScriptRoot "installer\TradePilot.iss")
    if ($LASTEXITCODE -ne 0) {
        Write-Host "Inno Setup reported an error (exit code $LASTEXITCODE)." -ForegroundColor Red
        exit $LASTEXITCODE
    }
    Write-Host "Setup: $(Join-Path $dist "TradePilot-$version-Setup.exe")" -ForegroundColor Green
} else {
    Write-Host "Inno Setup not found - only the ZIP was created. For the setup: winget install JRSoftware.InnoSetup" -ForegroundColor Yellow
}

if ($Mac) {
    if (-not (Invoke-MacBuild)) { exit 1 }
}
