# Windows 打包：构建 release 二进制，复制为固定命名的单文件 exe。
#
#   $env:OUT_DIR     输出目录（默认 dist）
#   $env:ARCH_LABEL  文件名里的架构后缀（默认 x64）
#   $env:SKIP_BUILD  1 = 跳过 cargo build
#
# 产物：$OUT_DIR/GetCat-windows-$ARCH_LABEL.exe。
# 不做 Authenticode 签名（没有证书）；应用内更新器按 .exe 后缀走「重命名旧文件、放入新文件」策略。
$ErrorActionPreference = 'Stop'

$outDir = if ($env:OUT_DIR) { $env:OUT_DIR } else { 'dist' }
$archLabel = if ($env:ARCH_LABEL) { $env:ARCH_LABEL } else { 'x64' }

$repoRoot = Resolve-Path (Join-Path $PSScriptRoot '..')
Set-Location $repoRoot

if ($env:SKIP_BUILD -ne '1') {
    cargo build --release --locked -p getcat-app
    if ($LASTEXITCODE -ne 0) { throw "cargo build 失败（exit $LASTEXITCODE）" }
}

$binary = Join-Path $repoRoot 'target\release\getcat.exe'
if (-not (Test-Path $binary)) { throw "找不到 $binary" }

New-Item -ItemType Directory -Force -Path $outDir | Out-Null
$target = Join-Path $outDir "GetCat-windows-$archLabel.exe"
Copy-Item -LiteralPath $binary -Destination $target -Force

Write-Host "已生成 $target"
Get-Item $target | Format-List Name, Length
