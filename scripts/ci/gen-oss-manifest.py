#!/usr/bin/env python3
"""生成阿里云 OSS 镜像的静态更新 manifest（latest.json）。

由 release.yml 的 mirror-to-oss job 调用。输入是 `gh release download` 拉下来的
全部发布资产（8 个安装包 + SHA256SUMS + 各自的 .minisig），输出一份
gpui-updater `StaticManifestSource` 能读的 JSON：每个资产带 name / url / size /
sha256 / signature_url，客户端用编译期内置的 minisign 公钥验签——镜像被篡改
也过不了 Strict 校验。

自检从严：资产白名单缺一、SHA256SUMS 缺行、sha256 对不上、.minisig 缺失，
一律报错退出非零，宁可这次不镜像也不发一份坏 manifest。

用法：
    gen-oss-manifest.py --tag v0.2.0 --assets-dir assets \
        --base-url https://d.mirror.catonthe.top/GetCat --out assets/latest.json
"""

import argparse
import hashlib
import json
import sys
from pathlib import Path

# 与 release.yml sign job 的资产白名单一份约定，改名要两边同步。
# (文件名, os, arch, format)；format 值与 gpui-updater 文档的常用值一致。
EXPECTED_ASSETS = [
    ("GetCat-macos-arm64.dmg", "macos", "arm64", "dmg"),
    ("GetCat-macos-x64.dmg", "macos", "x86_64", "dmg"),
    ("GetCat-linux-x64.tar.gz", "linux", "x86_64", "tar.gz"),
    ("GetCat-linux-arm64.tar.gz", "linux", "arm64", "tar.gz"),
    ("GetCat-windows-x64.exe", "windows", "x86_64", "exe"),
    ("GetCat-windows-x64.msi", "windows", "x86_64", "msi"),
    ("GetCat-windows-arm64.exe", "windows", "arm64", "exe"),
    ("GetCat-windows-arm64.msi", "windows", "arm64", "msi"),
]


def sha256_of(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def parse_sums(path: Path) -> dict[str, str]:
    """解析 `sha256sum` 输出：每行 "<hex>  <name>"，按空白切分。"""
    sums: dict[str, str] = {}
    for line in path.read_text().splitlines():
        parts = line.split()
        if len(parts) >= 2:
            sums[parts[-1]] = parts[0].lower()
    return sums


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--tag", required=True, help="发布 tag，如 v0.2.0")
    ap.add_argument("--assets-dir", required=True, type=Path)
    ap.add_argument("--base-url", required=True, help="镜像域名下资产所在目录，不带末尾斜杠")
    ap.add_argument("--out", required=True, type=Path)
    args = ap.parse_args()

    base = args.base_url.rstrip("/")
    assets_dir: Path = args.assets_dir
    errors: list[str] = []

    sums_file = assets_dir / "SHA256SUMS"
    if not sums_file.is_file():
        print(f"错误：缺少 {sums_file}", file=sys.stderr)
        return 1
    sums = parse_sums(sums_file)

    entries = []
    for name, os_, arch, fmt in EXPECTED_ASSETS:
        path = assets_dir / name
        if not path.is_file():
            errors.append(f"缺少资产 {name}")
            continue
        if not (assets_dir / f"{name}.minisig").is_file():
            errors.append(f"缺少签名 {name}.minisig")
        expected = sums.get(name)
        if expected is None:
            errors.append(f"SHA256SUMS 里没有 {name} 的记录")
        else:
            actual = sha256_of(path)
            if actual != expected:
                errors.append(f"{name} 的 sha256 与 SHA256SUMS 不符：{actual} != {expected}")
        entries.append(
            {
                "name": name,
                "url": f"{base}/{name}",
                "size": path.stat().st_size,
                "sha256": sums.get(name, ""),
                "signature_url": f"{base}/{name}.minisig",
                "os": os_,
                "arch": arch,
                "format": fmt,
            }
        )

    if errors:
        for e in errors:
            print(f"错误：{e}", file=sys.stderr)
        return 1

    manifest = {
        "schema_version": 1,
        "app_id": "top.catonthe.getcat",
        "version": args.tag,
        "notes_url": f"https://github.com/finch-xu/GetCat/releases/tag/{args.tag}",
        "assets": entries,
    }
    args.out.write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n")
    print(f"已生成 {args.out}（{len(entries)} 个资产）")
    return 0


if __name__ == "__main__":
    sys.exit(main())
