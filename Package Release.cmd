@echo off
rem Builds dist\CSGO-Rewrite-<version>-windows-x64.zip for GitHub Releases.
cd /d "%~dp0"
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0tools\package-release.ps1" %*
if errorlevel 1 (
    echo Packaging failed. See the message above.
    pause
    exit /b 1
)
pause
