@echo off
chcp 65001 >nul
cd /d "%~dp0"

where cargo >nul 2>nul
if errorlevel 1 set "PATH=D:\Program Files\rust\.cargo\bin;%PATH%"
rem audiopus_sys 要从源码编 libopus，需要 cmake
set "PATH=D:\Program Files\WinPython\python\Lib\site-packages\cmake\data\bin;%PATH%"

echo 正在编译（首次需要几分钟）...
cargo build --release -p voicekey-probe
if errorlevel 1 (
  echo.
  echo 编译失败，看上面的报错。
  pause
  exit /b 1
)

echo.
echo 启动后终端会打印公网地址和二维码，关掉窗口即停止服务与隧道。
echo.
rem workspace 共用根目录的 target
..\..\target\release\voicekey-probe.exe
echo.
echo 服务已停止。
pause
