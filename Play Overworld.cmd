@echo off
setlocal
cd /d "%~dp0"
set "CSR_CARGO=%USERPROFILE%\.cargo\bin\cargo.exe"
"%CSR_CARGO%" build --offline --profile play -p launcher
if errorlevel 1 goto failed
"%~dp0target\play\csr.exe" play overworld --seed 42
if errorlevel 1 goto failed
exit /b 0
:failed
echo Overworld stopped with an error. See the message above.
pause
exit /b 1
