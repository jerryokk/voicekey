@echo off
chcp 65001 >nul
cd /d "%~dp0"

rem audiopus_sys 要从源码编 libopus，必须有 cmake。本机这份是 pip 装的，不在 PATH 里
set "PATH=D:\Program Files\WinPython\python\Lib\site-packages\cmake\data\bin;%PATH%"

if not exist "app\src-tauri\binaries\cloudflared.exe" (
  echo 缺少 cloudflared.exe，下载一份放到 app\src-tauri\binaries\ 下：
  echo   https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-windows-amd64.exe
  pause
  exit /b 1
)

if not exist "app\node_modules" (
  echo 首次运行，安装前端依赖…
  pushd app
  call npm ci
  popd
  if errorlevel 1 (
    echo 依赖安装失败。
    pause
    exit /b 1
  )
)

echo 正在编译并启动，首次需要几分钟…
echo 窗口出现后，用手机扫「手机接入」里的二维码。
echo.
rem --no-default-features：关掉 custom-protocol，让界面走 vite 热更新；
rem 不加这个参数的话界面是编译期嵌进 exe 的，改前端不会实时生效。
pushd app
call npx tauri dev --no-default-features
popd
pause
