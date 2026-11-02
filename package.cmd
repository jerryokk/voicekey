@echo off
cd /d "%~dp0"

rem Release build + assemble a piik-style portable folder:
rem   dist\VoiceKey\voicekey.exe
rem   dist\VoiceKey\runtime\tunnel\cloudflared.exe
rem Requires: user env var CMAKE (audiopus builds libopus from source), cargo, node.

where cargo >nul 2>nul
if errorlevel 1 set "PATH=D:\Program Files\rust\.cargo\bin;%PATH%"

cargo build --release -p voicekey
if errorlevel 1 (
  echo build failed
  pause
  exit /b 1
)

if not exist "dist\VoiceKey\runtime\tunnel" mkdir "dist\VoiceKey\runtime\tunnel"
copy /y "target\release\voicekey.exe" "dist\VoiceKey\" >nul
copy /y "app\src-tauri\binaries\cloudflared.exe" "dist\VoiceKey\runtime\tunnel\" >nul

echo.
echo Ready: dist\VoiceKey\voicekey.exe
pause
