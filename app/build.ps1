<#
Builds TradePilot (Rust/egui) and can start the app afterwards.

Usage:
  .\build.ps1                # release build
  .\build.ps1 -Run           # release build, then start the app
  .\build.ps1 -Debug         # debug build (faster to build, slower to run)
  .\build.ps1 -Debug -Run    # debug build, then start the app
  .\build.ps1 -SkipClippy    # build without the Clippy pass before it
#>
param(
    [switch]$Run,
    [switch]$Debug,
    [switch]$SkipClippy
)

$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot
$totalTimer = [System.Diagnostics.Stopwatch]::StartNew()
$phaseTimer = [System.Diagnostics.Stopwatch]::StartNew()
function Write-Phase($name) {
    Write-Host ('  {0}: {1:N1} s' -f $name, $phaseTimer.Elapsed.TotalSeconds) -ForegroundColor DarkGray
    $phaseTimer.Restart()
}

$exeDir = if ($Debug) { "debug" } else { "release" }
# Cargo's output folder: CARGO_TARGET_DIR if it is set (building outside OneDrive/Downloads is much
# faster), otherwise target\ next to this script.
$targetRoot = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $PSScriptRoot "target" }
$outDir = Join-Path $targetRoot $exeDir
$exePath = Join-Path $outDir "trade_pilot.exe"
$pdbPath = Join-Path $outDir "trade_pilot.pdb"

Write-Host "Preparing the build..." -ForegroundColor Cyan
# Matches the raw "trade_pilot" binary name (Cargo package name, unchanged) and any versioned
# "TradePilot-V*" rename (the app's display name since the rename) - a still-running versioned
# .exe from a previous build used to survive this cleanup (exact-name match only) and silently
# lock the linker output of the next build (LNK1104: access denied).
Get-Process -ErrorAction SilentlyContinue | Where-Object { $_.ProcessName -like "trade_pilot*" -or $_.ProcessName -like "TradePilot*" } | Stop-Process -Force -ErrorAction SilentlyContinue
Start-Sleep -Milliseconds 200

if ($SkipClippy) {
    Write-Host "Clippy check skipped." -ForegroundColor Yellow
} else {
    Write-Host "Checking the code (clippy)..." -ForegroundColor Cyan
    $phaseTimer.Restart()
    cargo clippy --workspace --all-targets -- -D warnings
    Write-Phase "Clippy"
    if ($LASTEXITCODE -ne 0) {
        Write-Host "Clippy check failed (exit code $LASTEXITCODE) - build aborted." -ForegroundColor Red
        exit $LASTEXITCODE
    }
}

Write-Host "Building TradePilot ($(if ($Debug) { 'Debug' } else { 'Release' }))..." -ForegroundColor Cyan
Write-Host "The application is rebuilt on every call; dependencies stay cached." -ForegroundColor Cyan
$previousBuildInvocation = $env:TRADEPILOT_BUILD_INVOCATION
$previousRustFlags = $env:RUSTFLAGS
try {
    $env:TRADEPILOT_BUILD_INVOCATION = [Guid]::NewGuid().ToString("N")
    if ($Debug) {
        cargo build
    } else {
        # Release: the C runtime is linked into the exe, so it runs on a computer without the Visual C++ redistributable.
        # The exe must not carry the build machine's user name: Rust embeds source paths (panic messages,
        # debug info), so the project folder and the cargo/rustup homes are remapped to neutral names and the
        # linker stores only the file name of the .pdb instead of its full path.
        $cargoHome = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $env:USERPROFILE ".cargo" }
        $rustupHome = if ($env:RUSTUP_HOME) { $env:RUSTUP_HOME } else { Join-Path $env:USERPROFILE ".rustup" }
        $env:RUSTFLAGS = @(
            "-C target-feature=+crt-static",
            "--remap-path-prefix=$PSScriptRoot=src",
            "--remap-path-prefix=$cargoHome=cargo",
            "--remap-path-prefix=$rustupHome=rustup",
            "-C link-arg=/PDBALTPATH:%_PDB%"
        ) -join " "
        cargo build --release
    }
    $buildExitCode = $LASTEXITCODE
    Write-Phase "Cargo build"
} finally {
    $env:TRADEPILOT_BUILD_INVOCATION = $previousBuildInvocation
    $env:RUSTFLAGS = $previousRustFlags
}
if ($buildExitCode -ne 0) {
    Write-Host "Build failed (exit code $buildExitCode)." -ForegroundColor Red
    exit $buildExitCode
}

Write-Host "Build succeeded: $exePath" -ForegroundColor Green
$phaseTimer.Restart()

# Stage the new executable before replacing an existing version with the same name, so a failed
# copy or replacement does not destroy the previous build.
$outDir = (Get-Item -LiteralPath $outDir).FullName
$exePath = Join-Path $outDir "trade_pilot.exe"
$pdbPath = Join-Path $outDir "trade_pilot.pdb"
$finalExePath = $exePath
$productVersion = (Get-Item -LiteralPath $exePath).VersionInfo.ProductVersion
if ($productVersion) {
    $versionParts = $productVersion.Split(".")
    if ($versionParts.Count -lt 3) {
        throw "Invalid product version '$productVersion'; older versions are kept."
    }
    $versionedName = "TradePilot-V$($versionParts[0]).$($versionParts[1]).$($versionParts[2])$(if ($Debug) { '_debug' }).exe"
    $finalExePath = Join-Path $outDir $versionedName
    $stagedPath = Join-Path $outDir ($versionedName + "." + [Guid]::NewGuid().ToString("N") + ".tmp")
    try {
        Copy-Item -LiteralPath $exePath -Destination $stagedPath
        if (Test-Path -LiteralPath $finalExePath) {
            [System.IO.File]::Replace($stagedPath, $finalExePath, [System.Management.Automation.Language.NullString]::Value)
        } else {
            [System.IO.File]::Move($stagedPath, $finalExePath)
        }
    } finally {
        if (Test-Path -LiteralPath $stagedPath) {
            Remove-Item -LiteralPath $stagedPath -Force
        }
    }
    Write-Host "Versioned copy created: $finalExePath" -ForegroundColor Green
    $oldVersionedCopies = @(Get-ChildItem -LiteralPath $outDir -Filter "TradePilot-V*.exe" -File)
    $oldVersionedCopies += @(Get-ChildItem -LiteralPath $outDir -Filter "trade_pilot-V*.exe" -File)
    foreach ($oldFile in $oldVersionedCopies) {
        if ($oldFile.FullName -ne $finalExePath) {
            try {
                Remove-Item -LiteralPath $oldFile.FullName -Force -ErrorAction Stop
                Write-Host "  deleted: $($oldFile.FullName)" -ForegroundColor DarkGray
            } catch {
                Write-Host "  Note: could not delete $($oldFile.FullName) ($($_.Exception.Message)) - carrying on." -ForegroundColor Yellow
            }
        }
    }
    foreach ($unversioned in @($exePath, $pdbPath)) {
        if (Test-Path $unversioned) {
            try {
                Remove-Item $unversioned -Force -ErrorAction Stop
                Write-Host "Unversioned file removed: $unversioned" -ForegroundColor DarkGray
            } catch {
                Write-Host "Note: could not delete $unversioned ($($_.Exception.Message))." -ForegroundColor Yellow
            }
        }
    }
} else {
    Write-Host "No product version found; older versioned files are kept." -ForegroundColor Yellow
}

Write-Phase "Publish exe"
Write-Host ('Total: {0:N1} s' -f $totalTimer.Elapsed.TotalSeconds) -ForegroundColor Green

if ($Run) {
    if (Test-Path $finalExePath) {
        Write-Host "Starting the app..." -ForegroundColor Cyan
        Start-Process $finalExePath
    } else {
        Write-Host "Exe not found at $finalExePath" -ForegroundColor Red
        exit 1
    }
}
