# Release 构建：先出前端（界面在编译期嵌进 exe），再 cargo build --release
#   .\scripts\build.ps1            只编译，产物 target\release\voicekey.exe
#   .\scripts\build.ps1 -Package   编译完再打免安装包（见 package.ps1）
param([switch]$Package)
$ErrorActionPreference = 'Stop'
Set-Location (Split-Path $PSScriptRoot)

function Check([string]$what) {
  if ($LASTEXITCODE) { throw "$what 失败（退出码 $LASTEXITCODE）" }
}

# cloudflared 不进 git：没有就下载官方最新版（与 CI 同一个地址）。先落 .part 再改名，中断不会留下半个 exe
$cf = 'app\src-tauri\binaries\cloudflared.exe'
if (-not (Test-Path $cf)) {
  Write-Host '下载 cloudflared…'
  curl.exe -fL --retry 2 -o "$cf.part" https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-windows-amd64.exe
  Check '下载 cloudflared'
  Move-Item -Force "$cf.part" $cf
}

Push-Location app
try {
  # 刚 clone 或清理过：先按 lockfile 装依赖
  if (-not (Test-Path node_modules)) { npm ci; Check 'npm ci' }
  npm run build; Check '前端构建'
} finally { Pop-Location }
cargo build --release --locked -p voicekey; Check 'cargo build --release'

if ($Package) { & "$PSScriptRoot\package.ps1" }
