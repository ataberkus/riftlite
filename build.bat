@echo off
rem Builds target\release\riftlite.exe, or with "fast" target\fast\riftlite.exe (see README "Build").
setlocal
cd /d "%~dp0"

set "PROFILE=release"
if /i "%~1"=="fast" (
    set "PROFILE=fast"
) else if not "%~1"=="" (
    echo Usage: build.bat [fast]
    exit /b 1
)

rem Windows cannot replace a running .exe, so the link step would fail; never close it from here.
tasklist /fi "imagename eq riftlite.exe" /nh 2>nul | find /i "riftlite.exe" >nul
if not errorlevel 1 (
    echo riftlite.exe is running. Quit it from the tray menu, then run build.bat again.
    exit /b 1
)

cargo build --profile %PROFILE%
if errorlevel 1 (
    echo Build failed.
    exit /b 1
)

echo Built %CD%\target\%PROFILE%\riftlite.exe
pause