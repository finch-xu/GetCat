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

## 当前功能（Plan 1–4）

- GET / POST / PUT / PATCH / DELETE / HEAD / OPTIONS
- Path 参数（URL 中 `{name}`）、Query 参数、Headers、Body（raw JSON/Text/XML、form-urlencoded、文件流式上传）
- 流式接收、实时进度与耗时、取消
- 响应：状态/耗时/大小、Pretty/Raw、Headers 列表（虚拟化）、保存到文件
- 大响应三档展示：≤ 5 MB 且 ≤ 20 万行用高亮编辑器；≤ 64 MB 用按行虚拟化的纯文本视图；> 64 MB 落盘到 `<临时目录>/getcat-<pid>/`，显示摘要 + 前 1 MB 预览 + 保存 / 用系统程序打开（临时文件随响应释放或应用退出删除）
- 文本 Body 超过 10 MB 时提示改用文件 Body
- 纯文件持久化（无数据库、不存历史、不存响应）：每个 Tab 的草稿随输入自动落盘并在重启后恢复；⌘S 保存请求到侧栏列表，可点开 / 删除；Tab 顺序、激活 Tab、侧栏宽度与折叠、主题偏好写入 `workspace.json`
- 主题跟随系统，可在侧栏按钮固定为浅色 / 深色
- 自绘标题栏（gpui-component `TitleBar`）：三平台统一外观；macOS 红绿灯 + 拖动 / 双击，Linux / Windows 由组件绘制最小化 / 最大化 / 关闭
- 请求 / 响应分栏可在上下 ↔ 左右之间切换（Tab 栏右侧按钮），方向随 `workspace.json` 持久化
- ⌘F / Ctrl F 或工具栏放大镜：在响应编辑器内搜索（≤ 5 MB 且 ≤ 20 万行的高保真视图）；纯文本虚拟化视图只提示，不搜索
- "保存到文件"为原子写（同目录临时文件 → fsync → 替换），中断不会留下半个文件；对话框记住本次会话上次保存的目录
- 后台处理（美化 / 建索引）被 `catch_unwind` 包裹：任何 panic 都显示为"后台处理异常"，不会让 Tab 停在"发送中"
- 启动时后台清扫其它进程遗留超过 24 h 的 `<临时目录>/getcat-<pid>/`

## 数据目录与持久化

| 平台 | 目录 |
|---|---|
| macOS | `~/Library/Application Support/GetCat/` |
| Linux | `$XDG_DATA_HOME/getcat/`（默认 `~/.local/share/getcat/`） |
| Windows | `%APPDATA%\GetCat\data\` |

```
workspace.json          # Tab 顺序、激活 Tab、侧栏宽度 / 折叠、分栏方向、主题偏好
requests/<ulid>.json    # 一个已保存请求一个文件
drafts/<tab-id>.json    # 一个 Tab 一个草稿（含未保存修改）
```

- 每个文件顶层带 `"version": 1`，美化 JSON，可手工编辑。
- 写入走独立线程：同一文件 500 ms 内的多次改动只落盘最后一份；写入先写同目录临时文件再原子替换，崩溃不会留下半个文件。
- 启动时解析失败的文件会被改名为 `<原名>.corrupt-<unix毫秒>` 并跳过（日志 `warn`）；数据目录不可写时窗口顶部显示横幅，其余功能照常（只读）。
- 不存历史记录、不存响应。Header 中的 `Authorization` 等以明文落盘（与 Postman / Insomnia 本地库一致）（Unix 上数据目录内部文件权限为 0600，仅当前用户可读；用户另存的响应文件按系统 umask 创建，通常是 0644）。
- `workspace.json` 的 `split` 字段（`vertical` / `horizontal`）记录分栏方向；旧文件没有该字段时按上下处理。

## 无障碍

- 所有输入框 / 编辑器都有可访问名称；键值表每行的控件名称带行号（"参数名（第 3 行）"）；仅图标的按钮用 tooltip 作为名称。
- gpui-component 的 `Checkbox` 只能用可见 label 命名、`Select` 未透传 `accessibility_label`：这两处用带名称的组（`role=Group`）包裹，屏幕阅读器先读组名再读控件。上游补齐 API 后可去掉包装。
- 巡检方式：macOS VoiceOver（⌘F5）或 Xcode Accessibility Inspector；开发期 `cargo run -p getcat-app --features inspector` 后 ⌘⌥I 查看元素 id / role。

## 持续集成

`.github/workflows/ci.yml` 在 macOS / Ubuntu / Windows 上 `cargo build --locked` + `cargo test --locked`（fmt 与 clippy 在 macOS 跑一次）。Ubuntu 需要 `libwayland-dev libxkbcommon-x11-dev libx11-xcb-dev libfontconfig-dev libvulkan1` 等头文件与 Vulkan 加载器（见 workflow）。性能回归测试带 `#[ignore]`，不在 CI 跑。

## 测试

```bash
cargo test --workspace                                   # 单元 + wiremock + gpui TestAppContext
cargo test -p getcat-core --release --test perf_large_body -- --ignored --nocapture   # 100 MB 性能回归（手动）
cargo run -p getcat-app --features inspector             # 开发期元素检查器：⌘⌥I / Ctrl+Shift+I
RUST_LOG=debug cargo run -p getcat-app                   # 调整日志级别
```

## 快捷键

| 操作 | macOS | Windows / Linux |
|---|---|---|
| 发送 | ⌘ Enter | Ctrl Enter |
| 新 Tab | ⌘ T | Ctrl T |
| 关闭 Tab | ⌘ W | Ctrl W |
| 折叠侧栏 | ⌘ B | Ctrl B |
| 保存请求 | ⌘ S | Ctrl S |
| 响应内搜索 | ⌘ F | Ctrl F |

响应内搜索只对 A 档（编辑器视图）的响应生效：它把焦点交给只读响应编辑器并打开编辑器自带的搜索面板。大响应的纯文本 / 预览视图与二进制内容没有编辑器，只会提示。

已知限制：当焦点停在任意文本输入框（URL 栏、键值表、请求 Body 编辑器）里时，⌘F 会被该输入框自身的绑定吞掉而没有反应；此时改用响应工具栏上的放大镜按钮，或先点一下响应区再按 ⌘F。
