#!/usr/bin/env python3
"""重新生成 THIRD-PARTY.md。

依赖变化后运行：

    python3 scripts/gen-third-party.py

数据来自 `cargo license --json`，范围为「会进入分发产物的依赖」
（即排除 dev-dependencies，保留 build-dependencies 以求透明）。
"""

import json
import re
import subprocess
import sys
from collections import defaultdict

# 需要在清单中额外加注说明的依赖
NOTES = {
    "zlog": "⚠️ 强 copyleft，见文首「已知许可证问题」",
    "ztracing": "⚠️ 强 copyleft，见文首「已知许可证问题」",
    "ztracing_macro": "⚠️ 强 copyleft，见文首「已知许可证问题」",
    "option-ext": "弱 copyleft（文件级），未修改其源码",
    "dwrote": "弱 copyleft（文件级），仅 Windows",
    "cbindgen": "弱 copyleft（文件级），仅构建期使用，不进入产物",
}

HEADER = """# 第三方依赖清单

GetCat 以 [Apache License 2.0](LICENSE) 分发。本文件按 Apache-2.0 第 4 条的要求，
列出构建 GetCat 所使用的第三方依赖及其许可证。

清单由 `scripts/gen-third-party.py` 从 `cargo license` 自动生成，涵盖 macOS、
Linux、Windows 三个平台的依赖并集，已排除仅测试期使用的 dev-dependencies。
各依赖的完整许可证文本请见其 `repository` 字段所指向的上游仓库。

> 依赖变化后请重新生成本文件：`python3 scripts/gen-third-party.py`

"""

GPL_SECTION = """
## 已知许可证问题

`zlog`、`ztracing`、`ztracing_macro` 由 Zed 上游声明为 **GPL-3.0-or-later**。
三者均为**传递依赖**，本项目 `Cargo.toml` 未直接引用，而是经 `gpui` 与
`sum_tree` 传入：

```
getcat-app -> gpui (Apache-2.0)             -> ztracing -> zlog
getcat-app -> gpui -> sum_tree (Apache-2.0) -> ztracing
```

来源是 Zed 在这两个 crate 中加的 13 处性能剖析埋点。

**GPL 代码不会进入分发产物。** `ztracing/build.rs` 仅在环境变量 `ZTRACING`
存在时才启用埋点，本项目从未设置该变量；默认构建下 `#[instrument]` 退化为
恒等宏，span 宏退化为空结构体，`zlog` 的调用点位于 `#[cfg(ztracing)]` 与
`#[cfg(test)]` 内，均不参与编译。实测 release 二进制中 `gpui` 符号 15077 个，
`zlog` 与 `ztracing` 符号各 0 个。

因此这是**元数据层面**而非产物层面的问题：依赖图会被自动化合规扫描标记为
含 GPL，且这份「干净」依赖于上游 cfg 默认关闭这一实现细节，并不稳固。
处置方案与进展记录在 `deny.toml` 中。

"""


def git_sources() -> dict:
    """从 cargo metadata 取 name -> git 仓库 URL，用于补全 cargo-license 缺失的
    repository 字段（git 依赖的 Cargo.toml 通常不写 repository）。"""
    try:
        meta = json.loads(
            subprocess.run(
                ["cargo", "metadata", "--format-version", "1"],
                capture_output=True, text=True, check=True,
            ).stdout
        )
    except (subprocess.CalledProcessError, FileNotFoundError):
        return {}
    found = {}
    for pkg in meta.get("packages", []):
        src = pkg.get("source") or ""
        m = re.match(r"git\+(?P<url>[^?#]+)", src)
        if m:
            found[pkg["name"]] = m.group("url")
    return found


def main() -> int:
    fallback_repo = git_sources()
    try:
        out = subprocess.run(
            ["cargo", "license", "--json", "--avoid-dev-deps"],
            capture_output=True,
            text=True,
            check=True,
        ).stdout
    except FileNotFoundError:
        print("需要先安装 cargo-license：cargo install cargo-license", file=sys.stderr)
        return 1

    by_license = defaultdict(list)
    for crate in json.loads(out):
        # 跳过本仓库自己的 crate
        if crate["name"].startswith("getcat-"):
            continue
        by_license[crate.get("license") or "（未声明）"].append(crate)

    lines = [HEADER]
    total = sum(len(v) for v in by_license.values())
    lines.append(f"共 **{total}** 个第三方依赖，分属 **{len(by_license)}** 种许可证声明。\n")
    lines.append(GPL_SECTION)
    lines.append("## 依赖清单\n")

    for license_name in sorted(by_license, key=lambda k: (-len(by_license[k]), k)):
        crates = sorted(by_license[license_name], key=lambda c: c["name"].lower())
        lines.append(f"### {license_name}\n")
        lines.append(f"_{len(crates)} 个依赖_\n")
        lines.append("| 依赖 | 版本 | 来源 | 备注 |")
        lines.append("| --- | --- | --- | --- |")
        for c in crates:
            repo = c.get("repository") or fallback_repo.get(c["name"], "")
            repo_cell = f"[{repo.removeprefix('https://')}]({repo})" if repo else "—"
            lines.append(
                f"| `{c['name']}` | {c['version']} | {repo_cell} | {NOTES.get(c['name'], '')} |"
            )
        lines.append("")

    with open("THIRD-PARTY.md", "w") as fh:
        fh.write("\n".join(lines).rstrip() + "\n")
    print(f"✅ 已生成 THIRD-PARTY.md（{total} 个依赖，{len(by_license)} 种许可证）")
    return 0


if __name__ == "__main__":
    sys.exit(main())
