@echo off
setlocal
rem Resolve paths from this script so double-clicking works from any directory.
cd /d "%~dp0"
if not exist "target\release\herdr.exe" (
    echo Agents Go release executable not found. Run "just build" first.
    pause
    exit /b 1
)
rem Use the same packaged release executable as the PowerShell herdr command.
set "HERDR_SESSION=agents-go"
set "HERDR_SOCKET_PATH=%APPDATA%\agents-go\sessions\agents-go\herdr.sock"
set "HERDR_CLIENT_SOCKET_PATH="
set "HERDR_CONFIG_PATH=%APPDATA%\agents-go\config.toml"

rem The executable starts its own background server when the session is stopped.
"target\release\herdr.exe" --session agents-go %*
if errorlevel 1 (
    goto failed
)
exit /b 0

:failed
echo Agents Go failed to start.
pause
exit /b 1
