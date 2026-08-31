#!/usr/bin/env bash
# 发版准备：bump 版本号 → 同步 Cargo.lock → 自检 → 跑一遍 CI 的检查，最后把该敲的 git 命令打出来。
#
#   用法：scripts/release.sh <新版本号>        例：scripts/release.sh 0.2.3
#
# 只改 Cargo.toml 与 Cargo.lock 两个文件，不执行任何 git 写操作（不 add / commit / tag / push）。
# 用到 git 的地方都是只读：判断这两个文件是否干净、以及 lock 的改动是不是只有版本号。
#
# 为什么要有这个脚本：0.2.2 那次发版只改了 Cargo.toml、没同步 Cargo.lock，两边版本不一致就等于
# lock 过期，CI 与 release 流水线里所有 --locked 命令一律拒绝执行，四个平台的打包一起挂在
# "cannot update the lock file ... because --locked was passed"。版本号必须同时落到两个文件，
# 而且要在同一个提交里。
#
# 为什么 cargo update 带 --offline：gpui / gpui_platform / gpui_tokio 这几个 git 依赖不写 rev
# （原因见 Cargo.toml 里的注释），版本锁定完全靠 Cargo.lock。联网跑 cargo update 有把它们滚到
# 远端最新 HEAD 的风险，--offline 断掉这条路；随后再逐行确认 lock 的改动只有版本号。
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

log() { printf '\n==> %s\n' "$*"; }
die() { printf '\n错误：%s\n' "$*" >&2; exit 1; }

new_version="${1:-}"
[ -n "$new_version" ] || die "用法：scripts/release.sh <新版本号>，例 scripts/release.sh 0.2.3"

# 允许 0.3.0-rc.1 这类预发布号：release.yml 见到带 "-" 的 tag 会发成 prerelease 且不设为 latest
[[ "$new_version" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]] ||
  die "版本号 \"$new_version\" 不合法，要写成 X.Y.Z 或 X.Y.Z-rc.1（不带 v 前缀）"

current_version="$(awk -F'"' '
  /^\[/ { in_wp = ($0 == "[workspace.package]") }
  in_wp && /^version = "/ { print $2; exit }
' Cargo.toml)"
[ -n "$current_version" ] || die "Cargo.toml 的 [workspace.package] 段里没找到 version"
[ "$current_version" != "$new_version" ] || die "Cargo.toml 已经是 $new_version 了"

# 只要求这两个文件干净：别的文件脏不影响判断，但这两个脏了就没法确认 lock 的改动只有版本号
dirty="$(git status --porcelain -- Cargo.toml Cargo.lock)"
[ -z "$dirty" ] || die "Cargo.toml / Cargo.lock 有未提交的改动，先处理掉：
$dirty"

log "版本 $current_version -> $new_version"

# 精确改 [workspace.package] 段里的 version。不用 sed -i：BSD sed 与 GNU sed 的 -i 参数不兼容，
# 本机（macOS）和 CI（Linux）的行为会不一样。
tmp_toml="$(mktemp)"
trap 'rm -f "$tmp_toml"' EXIT
awk -v v="$new_version" '
  /^\[/ { in_wp = ($0 == "[workspace.package]") }
  !done && in_wp && /^version = "/ { print "version = \"" v "\""; done = 1; next }
  { print }
  END { if (!done) exit 1 }
' Cargo.toml > "$tmp_toml" || die "改写 Cargo.toml 失败：[workspace.package] 段里没有 version"
cat "$tmp_toml" > Cargo.toml

# 从这里开始工作区已被改动，失败时提示怎么退回去
trap 'printf "\n中断了。Cargo.toml / Cargo.lock 已被改动，要退回原样跑：\n  git checkout -- Cargo.toml Cargo.lock\n" >&2' ERR

log "cargo update --workspace --offline"
cargo update --workspace --offline

log "自检 Cargo.lock 的改动"
# 预期恰好 4 行：getcat-core 与 getcat-app 各一增一删。多出任何一行都说明有依赖被动了。
lock_diff="$(git diff -U0 -- Cargo.lock | grep -E '^[+-][^+-]' || true)"
lock_lines="$(printf '%s\n' "$lock_diff" | grep -c . || true)"
[ "$lock_lines" -eq 4 ] || die "Cargo.lock 的改动不是预期的 4 行，而是 $lock_lines 行：
$lock_diff"
non_version="$(printf '%s\n' "$lock_diff" | grep -cv '^[+-]version = "' || true)"
[ "$non_version" -eq 0 ] || die "Cargo.lock 除版本号外还有别的改动，多半是 git 依赖被滚动了：
$lock_diff"

# lock 里两个 crate 的版本必须真的等于新版本 —— 这正是 release.yml 的 verify-version 会卡的地方
for crate in getcat-core getcat-app; do
  have="$(grep -A1 -Fx "name = \"$crate\"" Cargo.lock | sed -n 's/^version = "\(.*\)"/\1/p' || true)"
  [ "$have" = "$new_version" ] ||
    die "Cargo.lock 里 $crate 是 ${have:-<缺失>}，不是 $new_version"
done

log "cargo metadata --locked（确认 lock 不再过期）"
cargo metadata --locked --offline --format-version 1 >/dev/null

# 下面三条与 ci.yml 完全同参，本地先挡一遍，免得推上去才发现
log "cargo fmt --all -- --check"
cargo fmt --all -- --check

log "cargo clippy --workspace --all-targets --locked -- -D warnings"
cargo clippy --workspace --all-targets --locked -- -D warnings

# gpui 的每个 #[gpui::test] 都要起一个完整 App（CoreText 字体、Metal 设备、tokio
# runtime），90 多个测试按核数并行跑，fd 峰值落在 256 与 512 之间。而 macOS 的 launchd
# 默认软限制恰好是 256（launchctl limit maxfiles），从 GUI 应用启动的终端继承的就是它——
# 于是 cargo test 挂在 "Failed to initialize Tokio: Too many open files"，且每次被判死刑
# 的测试都不一样。硬限制是 unlimited，脚本可以自己抬高软限制，只影响本进程与其子进程。
#
# 实测：ulimit -n 256 → 二十来个测试失败；512 → 全过。取 4096：不贴实测下限
# （测试数量还会涨），也远离 kern.maxfilesperproc 的 92160。只在低于阈值时才动，
# 抬不上去（硬限制被管过的机器）就 die——与其让 cargo test 死在随机测试的 EMFILE，
# 不如在这里给出明确指引。
min_fds=4096
soft_fds="$(ulimit -n)"
if [ "$soft_fds" != "unlimited" ] && [ "$soft_fds" -lt "$min_fds" ]; then
  log "文件描述符软限制 $soft_fds 不够 gpui 测试并行跑，抬到 $min_fds"
  ulimit -n "$min_fds" ||
    die "抬不上去（硬限制 $(ulimit -Hn)）。手动跑 ulimit -n $min_fds 或调整 launchctl limit maxfiles 后重试"
fi

log "cargo test --workspace --locked"
cargo test --workspace --locked

trap - ERR

cat <<EOF

==> 准备好了：Cargo.toml 与 Cargo.lock 都是 ${new_version}，本地检查全过。

接下来（两个文件必须在同一个提交里，分开提交就是 0.2.2 那次的事故）：

  git add Cargo.toml Cargo.lock && git commit -m "release $new_version"
  git push origin main

等 CI 三个平台全绿之后再打 tag —— CI 红着打 tag，release 流水线会挂在同样的地方：

  git tag v$new_version && git push origin v$new_version
EOF
