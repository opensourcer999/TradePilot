@echo off
REM Builds the packages for passing on (ZIP and, with Inno Setup, a setup program) in app\dist, also where .ps1
REM scripts are blocked by the ExecutionPolicy.
REM Usage:  package.cmd           build and package
REM         package.cmd -NoBuild  only package (existing release EXE)
REM         package.cmd -Mac      also build the macOS package with GitHub Actions
REM         package.cmd -MacOnly  only the macOS package
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0app\package.ps1" %*
exit /b %ERRORLEVEL%
