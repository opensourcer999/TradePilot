@echo off
setlocal EnableDelayedExpansion
REM Builds TradePilot, also where .ps1 scripts are blocked by the ExecutionPolicy.
REM Usage:
REM   build.cmd              release build
REM   build.cmd -Run         release build, then start the app
REM   build.cmd --debug      release AND debug build (release first, then debug); -Debug works too
REM   build.cmd -SkipClippy  faster build without the Clippy pass before it
REM (Only the debug build: app\build.ps1 -Debug)
set "HERE=%~dp0"
set "WANT_DEBUG="
set "REST="
:next
if "%~1"=="" goto done
if /i "%~1"=="--debug" (set "WANT_DEBUG=1") else if /i "%~1"=="-debug" (set "WANT_DEBUG=1") else (set "REST=!REST! %1")
shift
goto next
:done
powershell -NoProfile -ExecutionPolicy Bypass -File "%HERE%app\build.ps1" !REST!
if %ERRORLEVEL% NEQ 0 exit /b %ERRORLEVEL%
if defined WANT_DEBUG (
    REM Clippy has run already; the app is not started a second time.
    powershell -NoProfile -ExecutionPolicy Bypass -File "%HERE%app\build.ps1" -Debug -SkipClippy
)
exit /b %ERRORLEVEL%
