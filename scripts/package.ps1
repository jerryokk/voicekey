# 免安装包：把 release 产物打成 dist\VoiceKey-<版本>-x64-portable.zip，解压即用
# 结构与安装版一致：VoiceKey\voicekey.exe 旁边是 runtime\tunnel\cloudflared.exe
#   -Bin  release 产物目录。本地是 target\release；CI 带 --target 编译，在 target\x86_64-pc-windows-msvc\release
param([string]$Bin = 'target\release')
$ErrorActionPreference = 'Stop'
Set-Location (Split-Path $PSScriptRoot)

# tauri-build 编译时把资源拷到产物旁边的 runtime\
foreach ($f in "$Bin\voicekey.exe", "$Bin\runtime\tunnel\cloudflared.exe") {
  if (-not (Test-Path $f)) { throw "缺 $f，先 release 构建" }
}

$ver = (Get-Content app\src-tauri\tauri.conf.json -Raw -Encoding utf8 | ConvertFrom-Json).version
$stage = 'dist\VoiceKey'
$zip = "dist\VoiceKey-$ver-x64-portable.zip"
if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
New-Item -ItemType Directory $stage | Out-Null
Copy-Item "$Bin\voicekey.exe" $stage
Copy-Item -Recurse "$Bin\runtime" $stage
Compress-Archive -Force -Path $stage -DestinationPath $zip
Remove-Item -Recurse -Force $stage

Write-Host "已生成 $zip（$([math]::Round((Get-Item $zip).Length / 1MB, 1)) MB）"
