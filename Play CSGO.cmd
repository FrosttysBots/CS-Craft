@echo off
setlocal
cd /d "%~dp0"
set "CSR_CARGO=%USERPROFILE%\.cargo\bin\cargo.exe"
if not exist "%CSR_CARGO%" (
    echo Rust Cargo was not found at "%CSR_CARGO%".
    echo Install Rust or use cargo run --profile play -p launcher -- play de_dust2.
    pause
    exit /b 1
)
echo.
echo Building the current source. The first build can take several minutes.
"%CSR_CARGO%" build --profile play -p launcher
if errorlevel 1 goto failed
"%~dp0target\play\csr.exe" play de_dust2
if errorlevel 1 goto failed
exit /b 0
:failed
echo.
echo The build or game stopped with an error. See the message above.
pause
exit /b 1
