# gpui-updater

Cross-platform self-update for [GPUI] desktop apps, hosted on GitHub Releases or
static JSON manifests.

GPUI ships no updater of its own, and Zed's `auto_update` crate is GPL-licensed
and wired to Zed's private update server. `gpui-updater` is an independent,
MIT/Apache implementation of the same idea: check a release source, download the
platform artifact, verify it, and swap it into place — on macOS (`.dmg`),
Linux (`.tar.gz`), and Windows (bare `.exe`, or `.msi` for installer-shipped
apps).

## What it does

- **Sources** — `GitHubSource` reads a repo's Releases (latest or pre-releases),
  picks the asset for the running platform, and resolves a `SHA256SUMS`
  checksum and an optional `.minisig` signature. `StaticManifestSource` reads a
  CDN/S3/R2-friendly `latest.json` with a flat asset list. Bring your own by
  implementing `UpdateSource`.
- **Verification** — SHA-256 against the published checksums, plus optional
  [minisign] (Ed25519) signature verification. Transport security alone is not
  trusted.
- **Install** — platform-native swaps:
  - **macOS**: mount the `.dmg`, `ditto` the new (already-notarized) `.app` onto
    the target volume, then atomically replace the bundle. Nothing is re-signed
    at runtime — the new bundle carries its own signature.
  - **Linux**: extract the `.tar.gz` and atomically replace the binary.
  - **Windows**: rename-in-place for a bare `.exe`, or a staged msiexec
    handoff for an `.msi` — the verified package is applied
    (`/passive /norestart`) after the app exits, via the restart path, then
    the app relaunches. (Restart-Manager integration for locked sibling DLLs
    remains future work.)
- **GPUI integration** (`gpui` feature) — an observable `Entity<Updater>` that
  runs the work on the background executor and sets `App::set_restart_path` when
  an update is staged. No background polling: trigger checks explicitly, which
  suits a privacy-conscious "Check for updates" button.

## Installation

Not published on crates.io: the `gpui` feature depends on `gpui` from the zed
git repo, and a crate with a git dependency can't be published to the registry.
Install from git instead:

```toml
[dependencies]
# Core only (blocking engine, no GPUI):
gpui-updater = { git = "https://github.com/AprilNEA/gpui-updater", tag = "v0.0.3" }

# With the GPUI integration (Entity<Updater>):
gpui-updater = { git = "https://github.com/AprilNEA/gpui-updater", tag = "v0.0.3", features = ["gpui"] }
```

When your app already depends on `gpui` from the same zed git source, Cargo
unifies the two onto your pinned commit — `gpui-updater` does not impose a gpui
version.

## Usage

### Pick an update source

Use one of the built-in sources, or implement `UpdateSource` for your own
release service:

| Source | Best for | How it works |
| --- | --- | --- |
| `GitHubSource` | Existing GitHub Releases pipelines | Calls the GitHub Releases API, selects a matching asset, and can read `SHA256SUMS` / `.minisig` assets. |
| `StaticManifestSource` | Cloudflare R2, S3, MinIO, B2, CDNs, static hosting | Fetches a static `latest.json`, selects an asset from `assets[]`, then downloads the asset URL directly. |

### GitHub Releases

```rust
use gpui_updater::{EngineConfig, GitHubSource, UpdateEngine};
use semver::Version;

let source = GitHubSource::new("AprilNEA", "OpenLogi")
    .asset_contains("macos")
    .asset_contains(".dmg")
    .with_checksums("SHA256SUMS")
    .with_minisig();

let engine = UpdateEngine::new(
    source,
    EngineConfig::new(Version::parse(env!("CARGO_PKG_VERSION"))?)
        .minisign_public_key("RWQ…"), // optional
);

if let Some(release) = engine.check()? {
    let artifact = engine.download(&release, |done, total| { /* progress */ })?;
    engine.install(&artifact)?;
}
```

### Static manifests (S3/R2/CDN)

Use `StaticManifestSource` when your release metadata is hosted as a static JSON
file on Cloudflare R2, AWS S3, MinIO, Backblaze B2, GitHub Pages, or any normal
HTTPS file server. The updater does not speak the S3 API; it fetches JSON and
downloads direct artifact URLs. This keeps credentials and provider SDKs out of
the app: upload artifacts however you like, then publish a small JSON pointer.

```rust
use gpui_updater::{EngineConfig, StaticManifestSource, UpdateEngine};

let source = StaticManifestSource::new("https://dl.example.com/channels/stable/latest.json")
    .os("macos")
    .arch("arm64")
    .format("dmg");

let engine = UpdateEngine::new(source, EngineConfig::new(current_version));
```

Manifest v1 describes one latest release and a flat list of downloadable assets.
`schema_version`, `version`, `assets[].name`, and `assets[].url` are the only
required fields; all other fields are optional and unknown fields are ignored.
Put channel pointers at mutable URLs such as `/channels/stable/latest.json`, but
keep artifact URLs versioned and immutable.

Recommended layout:

```text
https://dl.example.com/
├── channels/
│   └── stable/
│       └── latest.json          # mutable pointer
└── releases/
    └── v1.2.3/
        ├── App-1.2.3-macos-arm64.dmg
        ├── App-1.2.3-macos-arm64.dmg.minisig
        └── SHA256SUMS
```

```json
{
  "schema_version": 1,
  "app_id": "org.example.App",
  "version": "1.2.3",
  "tag": "v1.2.3",
  "channel": "stable",
  "published_at": "2026-06-01T12:00:00Z",
  "release_url": "https://example.com/releases/v1.2.3",
  "notes": "Markdown release notes.",
  "assets": [
    {
      "name": "App-1.2.3-macos-arm64.dmg",
      "url": "https://dl.example.com/releases/v1.2.3/App-1.2.3-macos-arm64.dmg",
      "os": "macos",
      "arch": "arm64",
      "format": "dmg",
      "size": 12345678,
      "sha256": "0123456789abcdef...",
      "signature": null,
      "signature_url": null,
      "minimum_os_version": "13.0"
    }
  ]
}
```

`sha256` is checked after download when present. `signature` may contain an
inline minisign signature; `signature_url` may point to a detached signature file.
If both are present and a minisign public key is configured, the inline signature
is used.

By default (`Verification::BestEffort`) these checks are skipped when their input
is absent, so an unsigned release still installs. To **fail closed**, set a
stricter policy:

```rust
use gpui_updater::Verification;

EngineConfig::new(current_version)
    .minisign_public_key("RWQ…")
    .verification(Verification::Strict);
```

| Policy | Behaviour |
| ------ | --------- |
| `BestEffort` (default) | Verify what's available; skip missing checks (fails open). |
| `Off` | Skip all checks (tests/local dev only). |
| `Checksum` | Require a matching SHA-256. |
| `Signature` | Require a public key **and** an advertised minisign signature. |
| `Strict` | Require both a valid signature and a matching SHA-256. |

Under `Signature`/`Strict`, a release that cannot be verified is rejected at
`check()` time — before it is ever surfaced as available — and again before a
download is verified.

Asset selection is explicit when you use `os`, `arch`, and `format`:

```rust
let source = StaticManifestSource::new("https://dl.example.com/channels/stable/latest.json")
    .os("macos")
    .arch("arm64")
    .format("dmg");
```

You can also add filename substring filters with `asset_contains(...)`, or
replace them completely with `asset_patterns(...)`. Matching is
case-insensitive. If no selector is configured, the source falls back to a
platform extension guess: `.dmg` on macOS, `.exe` on Windows, and `.tar.gz` on
Linux/other platforms. Installer-shipped Windows apps should select their MSI
explicitly with `.format("msi")` — the install step stages it and applies via
msiexec on restart.

For R2/S3-style hosting, the recommended release flow is:

1. Build and sign/notarize platform artifacts.
2. Upload artifacts to an immutable, versioned prefix such as
   `/releases/v1.2.3/`.
3. Generate SHA-256 hashes and, optionally, minisign signatures.
4. Upload `latest.json` to a mutable channel prefix such as
   `/channels/stable/latest.json` after all artifacts are in place.

### GPUI entity

Enable the `gpui` feature (see [Installation](#installation)):

```rust
use gpui_updater::{EngineConfig, GitHubSource, UpdateStatus, Updater};

let updater = cx.new(|cx| Updater::new(
    GitHubSource::new("AprilNEA", "OpenLogi")
        .asset_contains("macos").asset_contains(".dmg")
        .with_checksums("SHA256SUMS"),
    EngineConfig::new(current_version),
    cx,
));

// Re-render on status changes:
cx.observe(&updater, |_, _, cx| cx.notify()).detach();

// Drive it from buttons:
updater.update(cx, |u, cx| u.check(cx));
// when status is Available → u.download_and_install(cx)
// when status is Staged → u.restart(cx)
```

`UpdateStatus`: `Idle → Checking → {UpToDate | Available(v)} →
Downloading { downloaded, total } → Installing → Staged(v) | Errored(msg)`.
`Downloading` carries live byte counts (`total` is `None` when the server omits
`Content-Length`).

## Platform notes

- **macOS** replacing an app in `/Applications` needs write permission to it.
  An admin who drag-installed the app can replace it without a prompt; a
  standard user cannot. A privilege-escalation fallback (an `osascript … with
  administrator privileges` prompt, as Velopack does) is not yet implemented —
  a permission error just surfaces as a failed update.
- The `gpui` feature pulls `gpui` from the zed git repo (no registry release),
  so it needs the same native toolchain GPUI itself requires (a real Xcode +
  Metal on macOS). The core (default features) builds with stable Rust alone.

## Development notes

On macOS inside some Nix shells, Cargo may pick Nix's clang wrapper as `cc` and
fail to link build scripts with missing system symbols such as `__Unwind_*` or
`_pthread_*`. Point Cargo at Apple's linker for the host target:

```bash
CARGO_TARGET_AARCH64_APPLE_DARWIN_LINKER=/usr/bin/cc cargo test
```

The same prefix works for `cargo check` and `cargo clippy`.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

[GPUI]: https://www.gpui.rs/
[minisign]: https://jedisct1.github.io/minisign/
