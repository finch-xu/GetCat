<div align="center">

<img src="crates/getcat-app/assets/logo/getcat.png" width="128" alt="GetCat">

# GetCat

**A native, cross-platform HTTP API client built with Rust + [GPUI](https://gpui.rs)**

No Postman, Just GetCat!

GPU-rendered · Light on resources · No account · Your data stays local

[![License](https://img.shields.io/badge/License-Apache%202.0-007EC6?style=flat-square)](LICENSE)
[![Rust](https://img.shields.io/badge/Rust-1.97%2B-CE422B?style=flat-square&logo=rust&logoColor=white)](https://www.rust-lang.org)
[![GPUI](https://img.shields.io/badge/UI-GPUI-8B5CF6?style=flat-square)](https://gpui.rs)
[![macOS](https://img.shields.io/badge/macOS-000000?style=flat-square&logo=apple&logoColor=white)](https://github.com/finch-xu/GetCat/releases)
[![Linux](https://img.shields.io/badge/Linux-FCC624?style=flat-square&logo=linux&logoColor=black)](https://github.com/finch-xu/GetCat/releases)
[![Windows](https://img.shields.io/badge/Windows-0078D6?style=flat-square&logo=windows&logoColor=white)](https://github.com/finch-xu/GetCat/releases)

**English** · [简体中文](README.md)

</div>

## Highlights

- **Native and fast**: a GPU-rendered native window — not Electron, Tauri, or a WebView. One interface across macOS, Linux, and Windows.
- **Large responses stay smooth**: streamed reception, live progress, cancel at any time. Up to 5 MB opens in the highlighted editor, up to 64 MB is line-virtualized, and anything larger spills to disk with a preview and one-click save — a few hundred MB won't lock up the UI.
- **Complete request building**: GET / POST / PUT / PATCH / DELETE / HEAD / OPTIONS; path parameters (`{name}` in the URL), query, and headers; bodies as form-data (text and file fields, files streamed with a known length), x-www-form-urlencoded, raw JSON / Text / XML, or a whole binary file.
- **Your data is yours**: no history, no stored responses, nothing uploaded anywhere. Saved requests, drafts, and settings are pretty-printed JSON files you can hand-edit and track in Git.
- **Saves as you go**: every tab's draft is written to disk and restored on restart; ⌘S saves to the sidebar; tab order, split direction, and theme preference are all remembered.
- **Theme follows the system**, or pin it to light / dark. The title bar is custom-drawn, so all three platforms look the same.
- **Accessible**: every control has an accessible name and works with screen readers.
- **In-app updates**: new versions show up in the status bar and install in one click; packages are verified by both SHA-256 and a signature.

## Install

Download the package for your platform from [Releases](https://github.com/finch-xu/GetCat/releases):

| Platform | File | Notes |
|---|---|---|
| macOS (Apple Silicon / Intel) | `GetCat-macos-arm64.dmg` / `GetCat-macos-x64.dmg` | Signed and notarized — drag it into Applications |
| Linux x64 | `GetCat-linux-x64.tar.gz` | Unpacks to `getcat` — see the system requirements below |
| Windows x64 | `GetCat-windows-x64.exe` | Portable single file — see the system requirements below |

<details>
<summary>Supported Linux distributions</summary>

Runs on mainstream desktop distributions from 2022 onward: **Ubuntu 22.04+**, **Debian 12+**, **Fedora 36+**, **Linux Mint 21+**, **openSUSE Leap 15.6+**, and rolling releases such as Arch and openSUSE Tumbleweed. Graphics drivers on these work out of the box — there is nothing extra to install.

Older releases won't run it: Ubuntu 20.04, Debian 11, and RHEL / Rocky / AlmaLinux 9 all sit below the glibc 2.35 floor.

Unpack it into `~/.local/bin` and add a menu entry:

```bash
tar -xzf GetCat-linux-x64.tar.gz
install -Dm755 getcat ~/.local/bin/getcat
mkdir -p ~/.local/share/applications
cat > ~/.local/share/applications/getcat.desktop <<EOF
[Desktop Entry]
Type=Application
Name=GetCat
Exec=$HOME/.local/bin/getcat
Categories=Development;
EOF
```

If `getcat` is not found afterwards, `~/.local/bin` is not on your PATH yet — log out and back in.

</details>

<details>
<summary>Blank window on Linux, or a Vulkan / no GPU found error</summary>

The interface is GPU-rendered through Vulkan. Desktop distributions normally ship the driver already, so check first:

```bash
vulkaninfo --summary
```

If that prints nothing or reports no devices, install the driver for your GPU:

| Environment | Command |
|---|---|
| Ubuntu / Debian with Intel or AMD graphics | `sudo apt install mesa-vulkan-drivers` |
| Fedora with Intel or AMD graphics | `sudo dnf install mesa-vulkan-drivers` |
| Arch with Intel or AMD graphics | `sudo pacman -S vulkan-intel` or `vulkan-radeon` |
| NVIDIA graphics | Install the proprietary driver (e.g. `nvidia-driver-550`); the open-source nouveau driver has no Vulkan |
| Virtual machine / no discrete GPU | Install `mesa-vulkan-drivers` to fall back to lavapipe software rendering — usable but slow |

</details>

<details>
<summary>Supported Windows versions</summary>

Requires **Windows 10 1803 (April 2018 Update) or later**, or Windows 11. The interface renders through Direct3D 11, so graphics hardware from around 2010 is enough (feature level 10.1 and up) — DirectX 12 is not required.

`GetCat-windows-x64.exe` is a portable single file; put it anywhere and run it. It is not code-signed yet, so SmartScreen will stop it the first time: click **More info** → **Run anyway**.

</details>

## Usage

1. Pick a method, type a URL, and press **⌘ Enter** (Ctrl Enter on Windows / Linux) to send.
2. Fill in parameters under the Params / Headers / Body tabs; any `{name}` in the URL shows up automatically in the path parameter table.
3. The response pane shows status / time / size, toggles between Pretty and Raw, searches with **⌘ F**, and saves to a file.
4. **⌘ S** saves the request to the sidebar — click it later to load it back.

| Action | macOS | Windows / Linux |
|---|---|---|
| Send | ⌘ Enter | Ctrl Enter |
| New tab / close tab | ⌘ T / ⌘ W | Ctrl T / Ctrl W |
| Collapse sidebar | ⌘ B | Ctrl B |
| Save request | ⌘ S | Ctrl S |
| Search in response | ⌘ F | Ctrl F |
| Settings | ⌘ , | Ctrl , |

Settings cover request timeout, redirects, TLS verification, editor font size, and whether to check for updates at startup.

### Data directory

| Platform | Directory |
|---|---|
| macOS | `~/Library/Application Support/GetCat/` |
| Linux | `$XDG_DATA_HOME/getcat/` (defaults to `~/.local/share/getcat/`) |
| Windows | `%APPDATA%\GetCat\data\` |

```
workspace.json          # tab order, sidebar, split direction, theme preference
requests/<ulid>.json    # one file per saved request
drafts/<tab-id>.json    # one draft per tab
settings.json           # application settings
```

Writes are atomic (temp file → rename), so a crash never leaves a half-written file; a file that fails to parse is renamed to `.corrupt-<timestamp>` and skipped. Headers such as `Authorization` are stored in plain text (same as Postman's and Insomnia's local stores), with 0600 file permissions on Unix.

## Development

### Architecture

```
crates/
├─ getcat-core   # UI-free core: request model, sending (reqwest + tokio), large-response tiering and spill-to-disk, JSON file storage
└─ getcat-app    # GPUI interface: Workspace / RequestTab state, settings dialog, in-app updates
```

- The UI is built on Zed's [gpui](https://github.com/zed-industries/zed/tree/main/crates/gpui) plus the [gpui-component](https://github.com/longbridge/gpui-component) library. Both are git dependencies pinned by `Cargo.lock` (see the comments in `Cargo.toml` for how to upgrade).
- Networking runs on the tokio runtime and results come back to the GPUI main thread over a channel; background work (pretty-printing, indexing) is wrapped in `catch_unwind`, so a panic only surfaces as a "background processing error".
- There is no database: `getcat-core/src/store` handles reads and writes, with writes on a dedicated thread coalesced over 500 ms.

### Building and debugging

- Rust ≥ 1.97 (edition 2024). macOS needs no extra toolchain; Linux needs Vulkan plus the Wayland / X11 / fontconfig headers (the full list is in `.github/workflows/ci.yml`); Windows needs the MSVC toolchain, and Direct3D 11 ships with the Windows SDK.
- App logo: `crates/getcat-app/assets/logo/cat.png` is the background-free original, and `scripts/gen-logo.py` composes it into the embedded `getcat.png` and the macOS icon source `resources/macos/getcat-1024.png`. After changing the logo, rerun the script by hand and commit the output (CI does not generate it; requires `pip install pillow numpy`).

```bash
cargo run -p getcat-app                         # run
cargo test --workspace                          # unit + wiremock + gpui TestAppContext tests
RUST_LOG=debug cargo run -p getcat-app          # change the log level
cargo run -p getcat-app --features inspector    # element inspector: ⌘⌥I / Ctrl+Shift+I to see ids and roles
GETCAT_UPDATE_CHECK=1 cargo run -p getcat-app   # make dev builds check for updates at startup too (check only, no install)
```

Before committing: `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`. CI builds and tests on all three platforms and uses cargo-deny to block copyleft dependencies.

## License

[Apache-2.0](LICENSE). The third-party dependency list is in [THIRD-PARTY.md](THIRD-PARTY.md).
