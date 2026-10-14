@echo off
setlocal
rem Resolve paths from this script so double-clicking works from any directory.
cd /d "%~dp0"
if not exist "target\release\herdr.exe" (
    echo Agents Go executable not found. Run "just build" in this directory first.
    pause
    exit /b 1
)
"target\release\herdr.exe" --session agents-go
if errorlevel 1 (
    echo Agents Go failed to start.
    pause
    exit /b 1
)
