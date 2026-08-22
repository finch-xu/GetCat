#!/usr/bin/env bash
# 把 getcat 打成 macOS 的 GetCat.app 与 dmg：构建 → 组装 .app → 签名 →（可选）公证并 staple .app
# → 打 dmg → 签名 dmg →（可选）公证并 staple dmg。
#
# 全部参数走环境变量：
#   TARGET                  cargo --target（默认 aarch64-apple-darwin；x64 传 x86_64-apple-darwin）
#   ARCH_LABEL              产物文件名里的架构后缀（默认 arm64；x64 传 x64）
#   APPLE_SIGNING_IDENTITY  codesign 身份（默认 "-" 即 ad-hoc，本地无证书也能跑通）
#   NOTARIZE                1 = 公证 + staple（需要 APPLE_ID / APPLE_PASSWORD / APPLE_TEAM_ID）；默认 0
#   ENTITLEMENTS            非空时传给 codesign --entitlements；默认不用（gpui 的 Metal 运行时着色器
#                           不是可执行内存 JIT，hardened runtime 下不需要任何 entitlement）
#   OUT_DIR                 输出目录（默认 dist）
#   SKIP_BUILD              1 = 跳过 cargo build，复用已有二进制（本地反复调试打包时用）
#   MACOSX_DEPLOYMENT_TARGET 默认 11.0，与 Info.plist 的 LSMinimumSystemVersion 一致
#
# 产物：$OUT_DIR/$ARCH_LABEL/GetCat.app 与 $OUT_DIR/GetCat-macos-$ARCH_LABEL.dmg
#
# 为什么先公证 .app 再打 dmg、dmg 再公证一次：应用内更新器（gpui-updater）会从 dmg 里
# ditto 出 .app 直接替换正在运行的那份，之后 Gatekeeper 只看 .app 自带的 staple ticket，
# dmg 的 ticket 帮不上忙；而手动下载的用户双击 dmg 时，dmg 的 ticket 让它离线也能通过。
# 反过来（先打 dmg 再 staple 里面的 .app）做不到：dmg 只读，重打会让 dmg 的 ticket 失效。
set -euo pipefail

TARGET="${TARGET:-aarch64-apple-darwin}"
ARCH_LABEL="${ARCH_LABEL:-arm64}"
APPLE_SIGNING_IDENTITY="${APPLE_SIGNING_IDENTITY:--}"
NOTARIZE="${NOTARIZE:-0}"
ENTITLEMENTS="${ENTITLEMENTS:-}"
OUT_DIR="${OUT_DIR:-dist}"
SKIP_BUILD="${SKIP_BUILD:-0}"
export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-11.0}"

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

resources="crates/getcat-app/resources/macos"
app_name="GetCat"
bin_name="getcat"

log() { printf '\n==> %s\n' "$*"; }
die() { echo "错误：$*" >&2; exit 1; }

[ "$(uname -s)" = "Darwin" ] || die "只能在 macOS 上运行"
for tool in cargo codesign hdiutil iconutil sips plutil ditto; do
  command -v "$tool" >/dev/null || die "缺少工具 $tool"
done
if [ "$NOTARIZE" = "1" ]; then
  [ "$APPLE_SIGNING_IDENTITY" != "-" ] || die "NOTARIZE=1 需要真实签名身份，ad-hoc 签名无法公证"
  : "${APPLE_ID:?NOTARIZE=1 需要 APPLE_ID}"
  : "${APPLE_PASSWORD:?NOTARIZE=1 需要 APPLE_PASSWORD}"
  : "${APPLE_TEAM_ID:?NOTARIZE=1 需要 APPLE_TEAM_ID}"
fi

# ---------------------------------------------------------------------------
# 版本：单一来源是根 Cargo.toml 的 [workspace.package].version
# ---------------------------------------------------------------------------
version="$(cargo metadata --no-deps --format-version 1 \
  | python3 -c 'import json,sys; m=json.load(sys.stdin); print(next(p["version"] for p in m["packages"] if p["name"]=="getcat-app"))')"
[ -n "$version" ] || die "读不到 getcat-app 的版本号"
log "版本 ${version}，目标 ${TARGET}（${ARCH_LABEL}），签名身份：$APPLE_SIGNING_IDENTITY"

# ---------------------------------------------------------------------------
# 构建
# ---------------------------------------------------------------------------
binary="target/$TARGET/release/$bin_name"
if [ "$SKIP_BUILD" != "1" ]; then
  log "cargo build --release --target $TARGET"
  cargo build --release --locked -p getcat-app --target "$TARGET"
fi
[ -x "$binary" ] || die "找不到二进制 $binary"

# ---------------------------------------------------------------------------
# 组装 .app
# ---------------------------------------------------------------------------
stage="$OUT_DIR/$ARCH_LABEL"
app="$stage/$app_name.app"
contents="$app/Contents"
rm -rf "$stage"
mkdir -p "$contents/MacOS" "$contents/Resources"

log "组装 $app"
sed "s/@VERSION@/$version/g" "$resources/Info.plist" >"$contents/Info.plist"
plutil -lint "$contents/Info.plist" >/dev/null
printf 'APPL????' >"$contents/PkgInfo"
cp "$binary" "$contents/MacOS/$bin_name"
chmod 755 "$contents/MacOS/$bin_name"

# 图标：1024 PNG → iconset（10 张）→ icns
iconset="$stage/$app_name.iconset"
mkdir -p "$iconset"
png="$resources/getcat-1024.png"
[ -f "$png" ] || die "缺少 ${png}（用 scripts/gen-macos-icon.sh 生成）"
for size in 16 32 128 256 512; do
  double=$((size * 2))
  sips -z "$size" "$size" "$png" --out "$iconset/icon_${size}x${size}.png" >/dev/null
  sips -z "$double" "$double" "$png" --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$iconset" -o "$contents/Resources/$app_name.icns"
rm -rf "$iconset"

# ---------------------------------------------------------------------------
# 签名：先内层 Mach-O，再整个 bundle（Apple 已弃用 --deep，且这里只有一个可执行文件）
# ---------------------------------------------------------------------------
sign_flags=(--force --options runtime --sign "$APPLE_SIGNING_IDENTITY")
if [ "$APPLE_SIGNING_IDENTITY" != "-" ]; then
  sign_flags+=(--timestamp)     # ad-hoc 签名不能带时间戳
fi
if [ -n "$ENTITLEMENTS" ]; then
  sign_flags+=(--entitlements "$ENTITLEMENTS")
fi

log "codesign"
codesign "${sign_flags[@]}" "$contents/MacOS/$bin_name"
codesign "${sign_flags[@]}" "$app"
codesign --verify --deep --strict --verbose=2 "$app"

# ---------------------------------------------------------------------------
# 公证
# ---------------------------------------------------------------------------
notarize() {
  # $1 = 待提交文件（zip 或 dmg），$2 = 给 staple 的目标（.app 或 dmg）
  local submission="$1" staple_target="$2" result status id
  log "notarytool submit $submission"
  result="$(xcrun notarytool submit "$submission" \
    --apple-id "$APPLE_ID" --password "$APPLE_PASSWORD" --team-id "$APPLE_TEAM_ID" \
    --wait --timeout 45m --output-format json)"
  status="$(printf '%s' "$result" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("status",""))')"
  id="$(printf '%s' "$result" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("id",""))')"
  if [ "$status" != "Accepted" ]; then
    echo "公证失败（status=${status}，id=${id}），日志：" >&2
    [ -n "$id" ] && xcrun notarytool log "$id" \
      --apple-id "$APPLE_ID" --password "$APPLE_PASSWORD" --team-id "$APPLE_TEAM_ID" >&2 || true
    exit 1
  fi
  log "stapler staple $staple_target"
  xcrun stapler staple "$staple_target"
  xcrun stapler validate "$staple_target"
}

if [ "$NOTARIZE" = "1" ]; then
  zip="$stage/$app_name-notarize.zip"
  ditto -c -k --keepParent "$app" "$zip"
  notarize "$zip" "$app"
  rm -f "$zip"
fi

# ---------------------------------------------------------------------------
# dmg：根目录只放 .app 和指向 /Applications 的符号链接
#（gpui-updater 取挂载卷里第一个 *.app，符号链接不算）
# ---------------------------------------------------------------------------
dmg="$OUT_DIR/$app_name-macos-$ARCH_LABEL.dmg"
dmg_root="$stage/dmg-root"
rm -rf "$dmg_root" "$dmg"
mkdir -p "$dmg_root"
ditto "$app" "$dmg_root/$app_name.app"       # ditto 保留签名与 staple ticket
ln -s /Applications "$dmg_root/Applications"

log "hdiutil create $dmg"
hdiutil create -volname "$app_name" -srcfolder "$dmg_root" -ov -format UDZO -fs HFS+ "$dmg" >/dev/null
rm -rf "$dmg_root"

log "codesign dmg"
dmg_sign_flags=(--force --sign "$APPLE_SIGNING_IDENTITY")
[ "$APPLE_SIGNING_IDENTITY" != "-" ] && dmg_sign_flags+=(--timestamp)
codesign "${dmg_sign_flags[@]}" "$dmg"

if [ "$NOTARIZE" = "1" ]; then
  notarize "$dmg" "$dmg"
  log "spctl 评估 dmg"
  spctl -a -t open --context context:primary-signature -v "$dmg"
fi

log "完成"
ls -la "$dmg"
codesign -dv --verbose=1 "$app" 2>&1 | grep -E '^(Identifier|TeamIdentifier|Authority|flags)' || true
