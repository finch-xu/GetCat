#!/usr/bin/env bash
# CI 专用：把 Developer ID 证书导入一个临时 keychain，供 codesign 使用；结束后删掉。
#
#   scripts/ci/macos-keychain.sh import    # 需要 APPLE_CERTIFICATE（base64 p12）、
#                                          # APPLE_CERTIFICATE_PASSWORD、APPLE_SIGNING_IDENTITY
#   scripts/ci/macos-keychain.sh cleanup   # 删除临时 keychain（放在 if: always() 步骤里）
#
# 不用登录 keychain：runner 是一次性的，但临时 keychain 让「导入了什么、删了什么」
# 一目了然，也避免和 runner 镜像自带的证书混在一起。
set -euo pipefail

keychain="${KEYCHAIN_PATH:-$RUNNER_TEMP/getcat-signing.keychain-db}"
keychain_password="${KEYCHAIN_PASSWORD:-getcat-ci-$RANDOM$RANDOM}"

case "${1:-}" in
  import)
    : "${APPLE_CERTIFICATE:?缺少 APPLE_CERTIFICATE}"
    : "${APPLE_CERTIFICATE_PASSWORD:?缺少 APPLE_CERTIFICATE_PASSWORD}"
    : "${APPLE_SIGNING_IDENTITY:?缺少 APPLE_SIGNING_IDENTITY}"

    cert="$RUNNER_TEMP/certificate.p12"
    umask 077
    printf '%s' "$APPLE_CERTIFICATE" | base64 --decode >"$cert"

    security create-keychain -p "$keychain_password" "$keychain"
    # 6 小时内不锁定；公证排队可能很久
    security set-keychain-settings -lut 21600 "$keychain"
    security unlock-keychain -p "$keychain_password" "$keychain"
    security import "$cert" -P "$APPLE_CERTIFICATE_PASSWORD" -A -t cert -f pkcs12 -k "$keychain"
    # 允许 codesign 无交互地使用私钥
    security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$keychain_password" "$keychain" >/dev/null
    security list-keychain -d user -s "$keychain" login.keychain-db
    rm -f "$cert"

    if ! security find-identity -v -p codesigning "$keychain" | grep -Fq "$APPLE_SIGNING_IDENTITY"; then
      echo "::error::keychain 里找不到签名身份：$APPLE_SIGNING_IDENTITY" >&2
      security find-identity -v -p codesigning "$keychain" >&2 || true
      exit 1
    fi
    echo "已导入签名身份：$APPLE_SIGNING_IDENTITY"
    ;;
  cleanup)
    if [ -f "$keychain" ]; then
      security delete-keychain "$keychain" || true
      echo "已删除临时 keychain"
    fi
    ;;
  *)
    echo "用法：$0 import|cleanup" >&2
    exit 2
    ;;
esac
