# GetCat

用 Rust + [GPUI](https://gpui.rs) 构建的跨平台 HTTP 接口调试工具。

## 构建

- Rust ≥ 1.97（edition 2024）
- macOS：无需 Xcode Metal Toolchain（已启用 `runtime_shaders`）
- Linux：需要 Vulkan 驱动；Windows：需要 DirectX 12

```bash
cargo run -p getcat-app
cargo test --workspace
```

## 依赖说明

- `gpui` / `gpui_platform` / `gpui_tokio` 来自 Zed 仓库 git，**声明中不写 rev**（必须与 gpui-component 一致），版本由 `Cargo.lock` 锁定。升级：
  `cargo update -p "git+https://github.com/zed-industries/zed#gpui@0.2.2" --precise <zed-rev>`，rev 以 gpui-component 当前 Cargo.lock 中的 zed rev 为准，并同步升级 gpui-component 的 rev。
- `vendor/arrayref`：crates.io 上 arrayref 0.3.5–0.3.9 已被 yank，此处 vendor 0.3.9（BSD-2-Clause）并通过 `[patch.crates-io]` 覆盖。
