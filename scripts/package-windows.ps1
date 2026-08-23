# Windows 打包：构建 release 二进制，产出两份东西 ——
#
#   $OUT_DIR/GetCat-windows-$ARCH_LABEL.exe   免安装单文件（名字不能改：老版本的更新器按它匹配）
#   $OUT_DIR/GetCat-windows-$ARCH_LABEL.msi   per-user 安装包（WiX v6，装到 %LOCALAPPDATA%\Programs\GetCat）
#
# 环境变量：
#   $env:OUT_DIR     输出目录（默认 dist）
#   $env:ARCH_LABEL  文件名里的架构后缀（默认 x64）
#   $env:SKIP_BUILD  1 = 跳过 cargo build
#   $env:SKIP_MSI    1 = 只出免安装 exe（本机没装 WiX 时的逃生口）
#
# 两份产物都不做 Authenticode 签名（没有证书），SmartScreen 会拦；MSI 因为是安装包，
# 拦得比免安装 exe 更凶。校验和与 minisign 签名由 release.yml 的 sign job 统一补。
#
# 应用内更新器按扩展名分派：.exe 走「重命名旧文件、放入新文件」，.msi 走「写 apply.cmd，
# 等应用退出后 msiexec /passive 再拉起」。装了 MSI 的用户靠安装目录里的 install-source.txt
# 被识别出来，从而下载 .msi 而不是裸 exe。
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

# ---------------------------------------------------------------------------
# 免安装版
# ---------------------------------------------------------------------------

$portable = Join-Path $outDir "GetCat-windows-$archLabel.exe"
Copy-Item -LiteralPath $binary -Destination $portable -Force
Write-Host "已生成 $portable"

if ($env:SKIP_MSI -eq '1') {
    Get-Item $portable | Format-List Name, Length
    return
}

# ---------------------------------------------------------------------------
# MSI
# ---------------------------------------------------------------------------

# 版本从 cargo metadata 拿，和 bundle-macos.sh 同源（都读 getcat-app 这个包）
$metaJson = cargo metadata --no-deps --format-version 1
if ($LASTEXITCODE -ne 0) { throw "cargo metadata 失败（exit $LASTEXITCODE）" }
$version = ($metaJson | ConvertFrom-Json).packages |
    Where-Object { $_.name -eq 'getcat-app' } |
    Select-Object -ExpandProperty version
if (-not $version) { throw 'cargo metadata 里找不到 getcat-app 的版本' }

# MSI 的 ProductVersion 只收 a.b.c[.d] 的纯数字，预发布后缀得剥掉。
# 副作用：0.3.2-rc.1 与 0.3.2 的 MSI 版本相同，rc 覆盖装正式版不会触发 MajorUpgrade。
# rc 只用于演练，可以接受；正式版之间的升级不受影响。
$msiVersion = ($version -split '-')[0]
Write-Host "MSI 版本：$msiVersion（包版本 $version）"

# 绝对路径，两个原因：WiX 的 Source 相对路径是相对 .wxs 所在目录解析的；
# 而 .NET 的 [System.IO.File] 用的是进程 CurrentDirectory，Set-Location 并不会同步它
$payloadDir = Join-Path (Join-Path $repoRoot $outDir) 'msi-payload'
if (Test-Path $payloadDir) { Remove-Item -LiteralPath $payloadDir -Recurse -Force }
New-Item -ItemType Directory -Force -Path $payloadDir | Out-Null

Copy-Item -LiteralPath $binary -Destination (Join-Path $payloadDir 'getcat.exe') -Force
Copy-Item -LiteralPath (Join-Path $repoRoot 'LICENSE') -Destination (Join-Path $payloadDir 'LICENSE') -Force

# 安装来源标记：免安装版没有这个文件，应用据此决定自动更新拉 .msi 还是裸 .exe。
# 写成不带 BOM 的 ASCII，Rust 端是 trim + 大小写不敏感比较。
[System.IO.File]::WriteAllText((Join-Path $payloadDir 'install-source.txt'), "msi`n")

$wxs = Join-Path $repoRoot 'crates\getcat-app\resources\windows\GetCat.wxs'
$icon = Join-Path $repoRoot 'crates\getcat-app\resources\windows\getcat.ico'
foreach ($required in @($wxs, $icon)) {
    if (-not (Test-Path $required)) { throw "找不到 $required" }
}

# dotnet tool install --global 装到 ~/.dotnet/tools，多数环境已在 PATH 里；没有就补上
if (-not (Get-Command wix -ErrorAction SilentlyContinue)) {
    $toolsDir = Join-Path $env:USERPROFILE '.dotnet\tools'
    if (Test-Path (Join-Path $toolsDir 'wix.exe')) {
        $env:PATH = "$toolsDir;$env:PATH"
    } else {
        throw '找不到 wix 命令。安装：dotnet tool install --global wix --version 6.*'
    }
}

$msi = Join-Path $outDir "GetCat-windows-$archLabel.msi"
wix build `
    -arch x64 `
    -ext WixToolset.Util.wixext `
    -d Version=$msiVersion `
    -d PayloadDir=$payloadDir `
    -d IconFile=$icon `
    -o $msi `
    $wxs
if ($LASTEXITCODE -ne 0) { throw "wix build 失败（exit $LASTEXITCODE）" }

Write-Host "已生成 $msi"
Get-Item $portable, $msi | Format-List Name, Length
