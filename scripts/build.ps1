# Release 构建：先出前端（界面在编译期嵌进 exe），再 cargo build --release
#   .\scripts\build.ps1            只编译，产物 target\release\voicekey.exe
#   .\scripts\build.ps1 -Package   编译完再打免安装包（见 package.ps1）
param([switch]$Package)
$ErrorActionPreference = 'Stop'
Set-Location (Split-Path $PSScriptRoot)

function Check([string]$what) {
  if ($LASTEXITCODE) { throw "$what 失败（退出码 $LASTEXITCODE）" }
}

if (-not (Test-Path app\src-tauri\binaries\cloudflared.exe)) {
  throw '缺 app\src-tauri\binaries\cloudflared.exe，按 README「从源码构建」下载'
}

Push-Location app
try { npm run build; Check '前端构建' } finally { Pop-Location }
cargo build --release --locked -p voicekey; Check 'cargo build --release'

if ($Package) { & "$PSScriptRoot\package.ps1" }
