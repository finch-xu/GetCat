#!/usr/bin/env bash
# 把应用 logo（SVG）渲染成 macOS 图标用的 1024×1024 PNG，写到
# crates/getcat-app/resources/macos/getcat-1024.png。
#
# 只在 logo 改动时由开发者在 macOS 上手动跑一次并提交 PNG；CI 不调用本脚本
# （runner 上没有 SVG 栅格化工具，qlmanage 的渲染结果也随系统版本漂移，
# 固定位图才可复现）。
#
# Apple 的图标网格要求圆角方块只占画布的 824/1024（≈80.5%），四周留透明边距，
# 否则在 Dock 里会比其他应用的图标大一圈。logo SVG 本身是铺满 64×64 viewBox 的
# 圆角矩形，这里用一个带 padding 的包装 SVG 把它缩进去再渲染。
#
# 用法：scripts/gen-macos-icon.sh
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
src="$repo_root/crates/getcat-app/assets/logo/getcat.svg"
out="$repo_root/crates/getcat-app/resources/macos/getcat-1024.png"

command -v qlmanage >/dev/null || { echo "需要 macOS 的 qlmanage" >&2; exit 1; }
command -v sips >/dev/null || { echo "需要 macOS 的 sips" >&2; exit 1; }

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

# 824/1024 = 0.8047；在 64 单位的 viewBox 里偏移 (64 - 64*0.8047)/2 = 6.25
scale=0.8047
offset=6.25

# 去掉原 SVG 的外层 <svg …> 与 </svg>，把内容放进缩放过的 <g>
inner="$(sed -e '1,/<svg[^>]*>/{/<svg[^>]*>/d;}' -e 's#</svg>##' "$src")"
cat >"$tmp/icon.svg" <<EOF
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64">
  <g transform="translate($offset $offset) scale($scale)">
$inner
  </g>
</svg>
EOF

qlmanage -t -s 1024 -o "$tmp" "$tmp/icon.svg" >/dev/null 2>&1
png="$tmp/icon.svg.png"
[ -f "$png" ] || { echo "qlmanage 没有产出 PNG" >&2; exit 1; }

w="$(sips -g pixelWidth "$png" | awk '/pixelWidth/ {print $2}')"
h="$(sips -g pixelHeight "$png" | awk '/pixelHeight/ {print $2}')"
if [ "$w" != "1024" ] || [ "$h" != "1024" ]; then
  echo "渲染尺寸不是 1024×1024：${w}×${h}" >&2
  exit 1
fi

mkdir -p "$(dirname "$out")"
cp "$png" "$out"
echo "已写入 $out（${w}×${h}）"
