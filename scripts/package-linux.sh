#!/usr/bin/env bash
# Linux 打包：构建 release 二进制，strip 后连同 LICENSE 打成 tar.gz。
#
#   OUT_DIR     输出目录（默认 dist）
#   ARCH_LABEL  文件名里的架构后缀（默认 x64）
#   SKIP_BUILD  1 = 跳过 cargo build
#
# 产物：$OUT_DIR/GetCat-linux-$ARCH_LABEL.tar.gz，内含 getcat 与 LICENSE（无子目录）。
# 二进制必须叫 getcat：应用内更新器解包后按当前可执行文件名找新文件来替换。
set -euo pipefail

OUT_DIR="${OUT_DIR:-dist}"
ARCH_LABEL="${ARCH_LABEL:-x64}"
SKIP_BUILD="${SKIP_BUILD:-0}"

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

if [ "$SKIP_BUILD" != "1" ]; then
  cargo build --release --locked -p getcat-app
fi
binary="target/release/getcat"
[ -x "$binary" ] || { echo "找不到 $binary" >&2; exit 1; }

stage="$OUT_DIR/linux"
rm -rf "$stage"
mkdir -p "$stage"
cp "$binary" "$stage/getcat"
cp LICENSE "$stage/LICENSE"
strip "$stage/getcat"
chmod 755 "$stage/getcat"

tarball="$OUT_DIR/GetCat-linux-$ARCH_LABEL.tar.gz"
rm -f "$tarball"
tar -C "$stage" -czf "$tarball" getcat LICENSE
rm -rf "$stage"

echo "已生成 ${tarball}："
tar -tzvf "$tarball"
