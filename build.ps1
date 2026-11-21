<#
Wrapper: calls app\build.ps1, so the build can also be started from the project root.

Usage (in the project root):
  .\build.ps1
  .\build.ps1 -Run
  .\build.ps1 -Debug -Run
  .\build.ps1 -SkipClippy
#>
param(
    [switch]$Run,
    [switch]$Debug,
    [switch]$SkipClippy
)

& (Join-Path $PSScriptRoot "app\build.ps1") @PSBoundParameters
exit $LASTEXITCODE
