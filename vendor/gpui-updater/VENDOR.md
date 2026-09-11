# vendor/gpui-updater

来源：<https://github.com/AprilNEA/gpui-updater> tag `v0.0.7`
（commit `1196e5ee7832a0f9eb37f17e0be294d4ac214cff`），许可证 MIT OR Apache-2.0，原文见同目录 LICENSE-*。

为什么 vendor：上游的可选 `gpui` 依赖指向 zed 仓库的 git 源，而 GetCat 自 GPUI Kit 0.6 起
只依赖 crates.io 上的 `gpui-kit`（它锁定 `gpui-pre` 系列包）。两个来源的 gpui 是不同的
crate，`Entity<Updater>` 之类的类型无法互通，所以把 Cargo.toml 里那一行改成 `gpui-pre`。

与上游的差异只有 `Cargo.toml` 中标注了「GetCat vendor 改动」的那一处；`src/` 原样未动。
升级时重新复制上游 tag 并重做这一处改动即可。已删除与构建无关的 `.github`、`devenv.*`、`examples`。
