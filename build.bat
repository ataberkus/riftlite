@echo off
rem Builds target\release\riftlite.exe (see README "Build").
setlocal
cd /d "%~dp0"

rem Windows cannot replace a running .exe, so the link step would fail; never close it from here.
tasklist /fi "imagename eq riftlite.exe" /nh 2>nul | find /i "riftlite.exe" >nul
if not errorlevel 1 (
    echo riftlite.exe is running. Quit it from the tray menu, then run build.bat again.
    exit /b 1
)

cargo build --release
if errorlevel 1 (
    echo Build failed.
    exit /b 1
)

echo Built %CD%\target\release\riftlite.exe
pause