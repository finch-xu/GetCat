# GetCat

用 Rust + [GPUI](https://gpui.rs) 构建的跨平台 HTTP 接口调试工具：原生渲染、启动即用、纯文件存储，不需要账号。

## 亮点

- **原生且轻快**：GPU 渲染的原生窗口，不是 Electron / WebView；macOS、Linux、Windows 三平台同一套界面。
- **大响应不卡**：流式接收、实时进度、随时取消；≤ 5 MB 用高亮编辑器，≤ 64 MB 按行虚拟化，更大的落盘预览 + 一键保存，百 MB 响应也不会拖住界面。
- **完整的请求构造**：GET / POST / PUT / PATCH / DELETE / HEAD / OPTIONS；Path 参数（URL 中 `{name}`）、Query、Headers；Body 支持 form-data（文本 / 文件字段，文件定长流式上传）、x-www-form-urlencoded、raw JSON / Text / XML、binary 整文件上传。
- **数据属于你**：不存历史、不存响应、不上传任何东西。已保存请求、草稿、设置都是美化过的 JSON 文件，可手工编辑、可用 Git 管理。
- **随手即存**：每个 Tab 的草稿自动落盘、重启恢复；⌘S 保存到侧栏；Tab 顺序、分栏方向、主题偏好都记住。
- **主题跟随系统**，也可固定浅色 / 深色；自绘标题栏，三平台外观一致。
- **无障碍**：所有控件都有可访问名称，屏幕阅读器可用。
- **应用内更新**：有新版本时状态栏提示，一键下载安装；安装包经 SHA-256 + 签名双重校验。

## 安装

到 [Releases](https://github.com/finch-xu/GetCat/releases) 下载对应平台的包：

| 平台 | 文件 | 说明 |
|---|---|---|
| macOS（Apple Silicon / Intel） | `GetCat-macos-arm64.dmg` / `GetCat-macos-x64.dmg` | 已签名公证，拖进「应用程序」即可 |
| Linux x64 | `GetCat-linux-x64.tar.gz` | 解压得到 `getcat`；需要 Vulkan 驱动，glibc ≥ 2.35 |
| Windows x64 | `GetCat-windows-x64.exe` | 单文件；需要 DirectX 12；首次运行可能有 SmartScreen 提示 |

## 使用

1. 选方法、输入 URL，按 **⌘ Enter**（Windows / Linux 为 Ctrl Enter）发送。
2. 在 Params / Headers / Body 标签页填参数；URL 里的 `{name}` 会自动出现在 Path 参数表里。
3. 响应区看状态 / 耗时 / 大小，Pretty / Raw 切换，**⌘ F** 在响应内搜索，或保存到文件。
4. **⌘ S** 保存请求到侧栏，之后点开即用。

| 操作 | macOS | Windows / Linux |
|---|---|---|
| 发送 | ⌘ Enter | Ctrl Enter |
| 新 Tab / 关闭 Tab | ⌘ T / ⌘ W | Ctrl T / Ctrl W |
| 折叠侧栏 | ⌘ B | Ctrl B |
| 保存请求 | ⌘ S | Ctrl S |
| 响应内搜索 | ⌘ F | Ctrl F |
| 设置 | ⌘ , | Ctrl , |

设置里可以调请求超时、跳转、TLS 校验、编辑器字号，以及是否在启动时检查更新。

### 数据目录

| 平台 | 目录 |
|---|---|
| macOS | `~/Library/Application Support/GetCat/` |
| Linux | `$XDG_DATA_HOME/getcat/`（默认 `~/.local/share/getcat/`） |
| Windows | `%APPDATA%\GetCat\data\` |

```
workspace.json          # Tab 顺序、侧栏、分栏方向、主题偏好
requests/<ulid>.json    # 一个已保存请求一个文件
drafts/<tab-id>.json    # 一个 Tab 一个草稿
settings.json           # 应用设置
```

写入是原子的（临时文件 → 替换），崩溃不会留下半个文件；解析失败的文件会被改名为 `.corrupt-<时间>` 并跳过。Header 里的 `Authorization` 等以明文保存（与 Postman / Insomnia 本地库一致），Unix 上文件权限 0600。

## 二次开发

### 架构

```
crates/
├─ getcat-core   # 无 UI 的核心：请求模型、发送（reqwest + tokio）、大响应分档与落盘、JSON 文件存储
└─ getcat-app    # GPUI 界面：Workspace / RequestTab 状态、设置对话框、应用内更新
```

- UI 框架是 Zed 的 [gpui](https://github.com/zed-industries/zed/tree/main/crates/gpui) + [gpui-component](https://github.com/longbridge/gpui-component) 组件库，两者都走 git 依赖、由 `Cargo.lock` 锁定（升级方式见 `Cargo.toml` 注释）。
- 网络在 tokio 运行时里跑，结果通过 channel 回到 GPUI 主线程；后台处理（美化 / 建索引）被 `catch_unwind` 包裹，panic 只会显示为"后台处理异常"。
- 持久化没有数据库：`getcat-core/src/store` 负责读写，写入走独立线程并做 500 ms 合并。

### 构建与调试

- Rust ≥ 1.97（edition 2024）。macOS 不需要额外工具链；Linux 需要 Vulkan 与 Wayland / X11 / fontconfig 头文件（清单见 `.github/workflows/ci.yml`）；Windows 需要 DirectX 12。

```bash
cargo run -p getcat-app                         # 运行
cargo test --workspace                          # 单元 + wiremock + gpui TestAppContext 测试
RUST_LOG=debug cargo run -p getcat-app          # 调整日志级别
cargo run -p getcat-app --features inspector    # 元素检查器：⌘⌥I / Ctrl+Shift+I 查看 id / role
GETCAT_UPDATE_CHECK=1 cargo run -p getcat-app   # 开发构建也在启动时检查更新（只检查不安装）
```

提交前：`cargo fmt --all`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace`。CI 会在三平台跑构建与测试，并用 cargo-deny 拦截 copyleft 依赖。

## 许可证

[Apache-2.0](LICENSE)。第三方依赖清单见 [THIRD-PARTY.md](THIRD-PARTY.md)。
