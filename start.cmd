@echo off
setlocal
rem Resolve paths from this script so double-clicking works from any directory.
cd /d "%~dp0"
if not exist "target\debug\herdr.exe" (
    echo Agents Go debug executable not found. Run "cargo build --locked" first.
    pause
    exit /b 1
)
rem The debug build has its own data directory. Keep using the existing session.
set "HERDR_SESSION=agents-go"
set "HERDR_SOCKET_PATH=%APPDATA%\agents-go\sessions\agents-go\herdr.sock"
set "HERDR_CLIENT_SOCKET_PATH="
set "HERDR_CONFIG_PATH=%APPDATA%\agents-go\config.toml"

rem Let the release server restore the original session if it is not running.
"target\debug\herdr.exe" api snapshot >nul 2>&1
if errorlevel 1 (
    if not exist "target\release\herdr.exe" (
        echo Agents Go server executable not found. Run "just build" first.
        pause
        exit /b 1
    )
    powershell.exe -NoProfile -Command "Start-Process -FilePath (Join-Path $PWD 'target\release\herdr.exe') -ArgumentList '--session','agents-go','server' -WindowStyle Hidden"
    if errorlevel 1 goto failed
    powershell.exe -NoProfile -Command "for ($attempt = 0; $attempt -lt 100; $attempt++) { & '.\target\release\herdr.exe' --session agents-go api snapshot *> $null; if ($LASTEXITCODE -eq 0) { exit 0 }; Start-Sleep -Milliseconds 100 }; exit 1"
    if errorlevel 1 goto failed
)

rem Explicit --session would override the socket path with the debug directory.
"target\debug\herdr.exe"
if errorlevel 1 (
    goto failed
)
exit /b 0

:failed
echo Agents Go failed to start.
pause
exit /b 1
