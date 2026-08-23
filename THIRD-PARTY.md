# 第三方依赖清单

GetCat 以 [Apache License 2.0](LICENSE) 分发。本文件按 Apache-2.0 第 4 条的要求，
列出构建 GetCat 所使用的第三方依赖及其许可证。

清单由 `scripts/gen-third-party.py` 从 `cargo license` 自动生成，涵盖 macOS、
Linux、Windows 三个平台的依赖并集，已排除仅测试期使用的 dev-dependencies。
各依赖的完整许可证文本请见其 `repository` 字段所指向的上游仓库。

> 依赖变化后请重新生成本文件：`python3 scripts/gen-third-party.py`


共 **915** 个第三方依赖，分属 **36** 种许可证声明。


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


## 依赖清单

### Apache-2.0 OR MIT

_551 个依赖_

| 依赖 | 版本 | 来源 | 备注 |
| --- | --- | --- | --- |
| `accesskit` | 0.24.1 | [github.com/AccessKit/accesskit](https://github.com/AccessKit/accesskit) |  |
| `accesskit_atspi_common` | 0.18.1 | [github.com/AccessKit/accesskit](https://github.com/AccessKit/accesskit) |  |
| `accesskit_consumer` | 0.36.0 | [github.com/AccessKit/accesskit](https://github.com/AccessKit/accesskit) |  |
| `accesskit_consumer` | 0.37.0 | [github.com/AccessKit/accesskit](https://github.com/AccessKit/accesskit) |  |
| `accesskit_consumer` | 0.38.0 | [github.com/AccessKit/accesskit](https://github.com/AccessKit/accesskit) |  |
| `accesskit_macos` | 0.26.3 | [github.com/AccessKit/accesskit](https://github.com/AccessKit/accesskit) |  |
| `accesskit_unix` | 0.21.1 | [github.com/AccessKit/accesskit](https://github.com/AccessKit/accesskit) |  |
| `accesskit_windows` | 0.33.1 | [github.com/AccessKit/accesskit](https://github.com/AccessKit/accesskit) |  |
| `addr2line` | 0.25.1 | [github.com/gimli-rs/addr2line](https://github.com/gimli-rs/addr2line) |  |
| `aes` | 0.8.4 | [github.com/RustCrypto/block-ciphers](https://github.com/RustCrypto/block-ciphers) |  |
| `ahash` | 0.8.12 | [github.com/tkaitchuck/ahash](https://github.com/tkaitchuck/ahash) |  |
| `aligned` | 0.4.3 | [github.com/rust-embedded-community/aligned](https://github.com/rust-embedded-community/aligned) |  |
| `allocator-api2` | 0.2.21 | [github.com/zakarumych/allocator-api2](https://github.com/zakarumych/allocator-api2) |  |
| `android_system_properties` | 0.1.6 | [github.com/nical/android_system_properties](https://github.com/nical/android_system_properties) |  |
| `annotate-snippets` | 0.12.16 | [github.com/rust-lang/annotate-snippets-rs](https://github.com/rust-lang/annotate-snippets-rs) |  |
| `anstyle` | 1.0.14 | [github.com/rust-cli/anstyle.git](https://github.com/rust-cli/anstyle.git) |  |
| `anyhow` | 1.0.104 | [github.com/dtolnay/anyhow](https://github.com/dtolnay/anyhow) |  |
| `arbitrary` | 1.4.2 | [github.com/rust-fuzz/arbitrary/](https://github.com/rust-fuzz/arbitrary/) |  |
| `arc-swap` | 1.9.2 | [github.com/vorner/arc-swap](https://github.com/vorner/arc-swap) |  |
| `arraydeque` | 0.5.1 | [github.com/andylokandy/arraydeque](https://github.com/andylokandy/arraydeque) |  |
| `arrayvec` | 0.7.8 | [github.com/bluss/arrayvec](https://github.com/bluss/arrayvec) |  |
| `as-raw-xcb-connection` | 1.0.1 | [github.com/psychon/as-raw-xcb-connection](https://github.com/psychon/as-raw-xcb-connection) |  |
| `as-slice` | 0.2.1 | [github.com/japaric/as-slice](https://github.com/japaric/as-slice) |  |
| `ash` | 0.38.0+1.3.281 | [github.com/ash-rs/ash](https://github.com/ash-rs/ash) |  |
| `asn1-rs` | 0.7.2 | [github.com/rusticata/asn1-rs.git](https://github.com/rusticata/asn1-rs.git) |  |
| `asn1-rs-derive` | 0.6.0 | [github.com/rusticata/asn1-rs.git](https://github.com/rusticata/asn1-rs.git) |  |
| `asn1-rs-impl` | 0.2.0 | [github.com/rusticata/asn1-rs.git](https://github.com/rusticata/asn1-rs.git) |  |
| `async-broadcast` | 0.7.2 | [github.com/smol-rs/async-broadcast](https://github.com/smol-rs/async-broadcast) |  |
| `async-channel` | 2.5.0 | [github.com/smol-rs/async-channel](https://github.com/smol-rs/async-channel) |  |
| `async-compression` | 0.4.43 | [github.com/Nullus157/async-compression](https://github.com/Nullus157/async-compression) |  |
| `async-executor` | 1.14.0 | [github.com/smol-rs/async-executor](https://github.com/smol-rs/async-executor) |  |
| `async-fs` | 2.2.0 | [github.com/smol-rs/async-fs](https://github.com/smol-rs/async-fs) |  |
| `async-io` | 2.6.0 | [github.com/smol-rs/async-io](https://github.com/smol-rs/async-io) |  |
| `async-lock` | 3.4.2 | [github.com/smol-rs/async-lock](https://github.com/smol-rs/async-lock) |  |
| `async-net` | 2.0.0 | [github.com/smol-rs/async-net](https://github.com/smol-rs/async-net) |  |
| `async-process` | 2.5.0 | [github.com/smol-rs/async-process](https://github.com/smol-rs/async-process) |  |
| `async-recursion` | 1.1.1 | [github.com/dcchut/async-recursion](https://github.com/dcchut/async-recursion) |  |
| `async-signal` | 0.2.14 | [github.com/smol-rs/async-signal](https://github.com/smol-rs/async-signal) |  |
| `async-task` | 4.7.1 | [github.com/smol-rs/async-task](https://github.com/smol-rs/async-task) |  |
| `async-trait` | 0.1.92 | [github.com/dtolnay/async-trait](https://github.com/dtolnay/async-trait) |  |
| `atomic` | 0.5.3 | [github.com/Amanieu/atomic-rs](https://github.com/Amanieu/atomic-rs) |  |
| `atomic-waker` | 1.1.2 | [github.com/smol-rs/atomic-waker](https://github.com/smol-rs/atomic-waker) |  |
| `atspi` | 0.29.0 | [github.com/odilia-app/atspi](https://github.com/odilia-app/atspi) |  |
| `atspi-common` | 0.13.0 | [github.com/odilia-app/atspi](https://github.com/odilia-app/atspi) |  |
| `atspi-proxies` | 0.13.0 | [github.com/odilia-app/atspi](https://github.com/odilia-app/atspi) |  |
| `autocfg` | 1.5.1 | [github.com/cuviper/autocfg](https://github.com/cuviper/autocfg) |  |
| `backtrace` | 0.3.76 | [github.com/rust-lang/backtrace-rs](https://github.com/rust-lang/backtrace-rs) |  |
| `base64` | 0.22.1 | [github.com/marshallpierce/rust-base64](https://github.com/marshallpierce/rust-base64) |  |
| `base64` | 0.23.1 | [github.com/marshallpierce/rust-base64](https://github.com/marshallpierce/rust-base64) |  |
| `bit-set` | 0.8.0 | [github.com/contain-rs/bit-set](https://github.com/contain-rs/bit-set) |  |
| `bit-set` | 0.9.1 | [github.com/contain-rs/bit-set](https://github.com/contain-rs/bit-set) |  |
| `bit-vec` | 0.8.0 | [github.com/contain-rs/bit-vec](https://github.com/contain-rs/bit-vec) |  |
| `bit-vec` | 0.9.1 | [github.com/contain-rs/bit-vec](https://github.com/contain-rs/bit-vec) |  |
| `bit_field` | 0.10.3 | [github.com/phil-opp/rust-bit-field](https://github.com/phil-opp/rust-bit-field) |  |
| `bitflags` | 1.3.2 | [github.com/bitflags/bitflags](https://github.com/bitflags/bitflags) |  |
| `bitflags` | 2.13.1 | [github.com/bitflags/bitflags](https://github.com/bitflags/bitflags) |  |
| `bitstream-io` | 4.10.0 | [github.com/tuffy/bitstream-io](https://github.com/tuffy/bitstream-io) |  |
| `block-buffer` | 0.10.4 | [github.com/RustCrypto/utils](https://github.com/RustCrypto/utils) |  |
| `block-buffer` | 0.12.1 | [github.com/RustCrypto/utils](https://github.com/RustCrypto/utils) |  |
| `block-padding` | 0.3.3 | [github.com/RustCrypto/utils](https://github.com/RustCrypto/utils) |  |
| `blocking` | 1.6.2 | [github.com/smol-rs/blocking](https://github.com/smol-rs/blocking) |  |
| `borsh` | 1.8.0 | [github.com/near/borsh-rs](https://github.com/near/borsh-rs) |  |
| `bstr` | 1.13.1 | [github.com/BurntSushi/bstr](https://github.com/BurntSushi/bstr) |  |
| `bumpalo` | 3.20.3 | [github.com/fitzgen/bumpalo](https://github.com/fitzgen/bumpalo) |  |
| `bzip2` | 0.6.1 | [github.com/trifectatechfoundation/bzip2-rs](https://github.com/trifectatechfoundation/bzip2-rs) |  |
| `cbc` | 0.1.2 | [github.com/RustCrypto/block-modes](https://github.com/RustCrypto/block-modes) |  |
| `cc` | 1.4.3 | [github.com/rust-lang/cc-rs](https://github.com/rust-lang/cc-rs) |  |
| `cexpr` | 0.6.0 | [github.com/jethrogb/rust-cexpr](https://github.com/jethrogb/rust-cexpr) |  |
| `cfg-if` | 1.0.4 | [github.com/rust-lang/cfg-if](https://github.com/rust-lang/cfg-if) |  |
| `cgl` | 0.3.2 | [github.com/servo/cgl-rs](https://github.com/servo/cgl-rs) |  |
| `chacha20` | 0.10.1 | [github.com/RustCrypto/stream-ciphers](https://github.com/RustCrypto/stream-ciphers) |  |
| `chrono` | 0.4.45 | [github.com/chronotope/chrono](https://github.com/chronotope/chrono) |  |
| `cipher` | 0.4.4 | [github.com/RustCrypto/traits](https://github.com/RustCrypto/traits) |  |
| `cmake` | 0.1.58 | [github.com/rust-lang/cmake-rs](https://github.com/rust-lang/cmake-rs) |  |
| `cocoa` | 0.25.0 | [github.com/servo/core-foundation-rs](https://github.com/servo/core-foundation-rs) |  |
| `cocoa` | 0.26.0 | [github.com/servo/core-foundation-rs](https://github.com/servo/core-foundation-rs) |  |
| `cocoa-foundation` | 0.1.2 | [github.com/servo/core-foundation-rs](https://github.com/servo/core-foundation-rs) |  |
| `cocoa-foundation` | 0.2.0 | [github.com/servo/core-foundation-rs](https://github.com/servo/core-foundation-rs) |  |
| `compression-codecs` | 0.4.38 | [github.com/Nullus157/async-compression](https://github.com/Nullus157/async-compression) |  |
| `compression-core` | 0.4.32 | [github.com/Nullus157/async-compression](https://github.com/Nullus157/async-compression) |  |
| `concurrent-queue` | 2.5.0 | [github.com/smol-rs/concurrent-queue](https://github.com/smol-rs/concurrent-queue) |  |
| `console_error_panic_hook` | 0.1.7 | [github.com/rustwasm/console_error_panic_hook](https://github.com/rustwasm/console_error_panic_hook) |  |
| `const-oid` | 0.10.2 | [github.com/RustCrypto/formats](https://github.com/RustCrypto/formats) |  |
| `const-random` | 0.1.18 | [github.com/tkaitchuck/constrandom](https://github.com/tkaitchuck/constrandom) |  |
| `const-random-macro` | 0.1.16 | [github.com/tkaitchuck/constrandom](https://github.com/tkaitchuck/constrandom) |  |
| `cookie` | 0.18.2 | [github.com/SergioBenitez/cookie-rs](https://github.com/SergioBenitez/cookie-rs) |  |
| `cookie_store` | 0.22.1 | [github.com/pfernie/cookie_store](https://github.com/pfernie/cookie_store) |  |
| `core-foundation` | 0.9.4 | [github.com/servo/core-foundation-rs](https://github.com/servo/core-foundation-rs) |  |
| `core-foundation` | 0.10.1 | [github.com/servo/core-foundation-rs](https://github.com/servo/core-foundation-rs) |  |
| `core-foundation-sys` | 0.8.7 | [github.com/servo/core-foundation-rs](https://github.com/servo/core-foundation-rs) |  |
| `core-graphics` | 0.23.2 | [github.com/servo/core-foundation-rs](https://github.com/servo/core-foundation-rs) |  |
| `core-graphics` | 0.24.0 | [github.com/servo/core-foundation-rs](https://github.com/servo/core-foundation-rs) |  |
| `core-graphics-helmer-fork` | 0.24.0 | [github.com/servo/core-foundation-rs](https://github.com/servo/core-foundation-rs) |  |
| `core-graphics-types` | 0.1.3 | [github.com/servo/core-foundation-rs](https://github.com/servo/core-foundation-rs) |  |
| `core-graphics-types` | 0.2.0 | [github.com/servo/core-foundation-rs](https://github.com/servo/core-foundation-rs) |  |
| `core-graphics2` | 0.5.2 | [github.com/rust-media/apple-media-rs](https://github.com/rust-media/apple-media-rs) |  |
| `core-text` | 21.0.0 | [github.com/servo/core-foundation-rs](https://github.com/servo/core-foundation-rs) |  |
| `core-video` | 0.5.2 | [github.com/rust-media/apple-media-rs](https://github.com/rust-media/apple-media-rs) |  |
| `cosmic-text` | 0.19.0 | [github.com/pop-os/cosmic-text](https://github.com/pop-os/cosmic-text) |  |
| `cpufeatures` | 0.2.17 | [github.com/RustCrypto/utils](https://github.com/RustCrypto/utils) |  |
| `cpufeatures` | 0.3.0 | [github.com/RustCrypto/utils](https://github.com/RustCrypto/utils) |  |
| `crc32fast` | 1.5.0 | [github.com/srijs/rust-crc32fast](https://github.com/srijs/rust-crc32fast) |  |
| `crossbeam-channel` | 0.5.16 | [github.com/crossbeam-rs/crossbeam](https://github.com/crossbeam-rs/crossbeam) |  |
| `crossbeam-deque` | 0.8.7 | [github.com/crossbeam-rs/crossbeam](https://github.com/crossbeam-rs/crossbeam) |  |
| `crossbeam-epoch` | 0.9.20 | [github.com/crossbeam-rs/crossbeam](https://github.com/crossbeam-rs/crossbeam) |  |
| `crossbeam-queue` | 0.3.13 | [github.com/crossbeam-rs/crossbeam](https://github.com/crossbeam-rs/crossbeam) |  |
| `crossbeam-utils` | 0.8.22 | [github.com/crossbeam-rs/crossbeam](https://github.com/crossbeam-rs/crossbeam) |  |
| `crypto-common` | 0.1.7 | [github.com/RustCrypto/traits](https://github.com/RustCrypto/traits) |  |
| `crypto-common` | 0.2.2 | [github.com/RustCrypto/traits](https://github.com/RustCrypto/traits) |  |
| `ctor` | 1.0.13 | [github.com/mmastrac/linktime](https://github.com/mmastrac/linktime) |  |
| `data-url` | 0.3.2 | [github.com/servo/rust-url](https://github.com/servo/rust-url) |  |
| `der-parser` | 10.0.0 | [github.com/rusticata/der-parser.git](https://github.com/rusticata/der-parser.git) |  |
| `deranged` | 0.5.8 | [github.com/jhpratt/deranged](https://github.com/jhpratt/deranged) |  |
| `digest` | 0.10.7 | [github.com/RustCrypto/traits](https://github.com/RustCrypto/traits) |  |
| `digest` | 0.11.3 | [github.com/RustCrypto/traits](https://github.com/RustCrypto/traits) |  |
| `directories` | 6.0.0 | [github.com/soc/directories-rs](https://github.com/soc/directories-rs) |  |
| `dirs` | 6.0.0 | [github.com/soc/dirs-rs](https://github.com/soc/dirs-rs) |  |
| `dirs-sys` | 0.5.0 | [github.com/dirs-dev/dirs-sys-rs](https://github.com/dirs-dev/dirs-sys-rs) |  |
| `displaydoc` | 0.2.7 | [github.com/yaahc/displaydoc](https://github.com/yaahc/displaydoc) |  |
| `document-features` | 0.2.12 | [github.com/slint-ui/document-features](https://github.com/slint-ui/document-features) |  |
| `downcast-rs` | 1.2.1 | [github.com/marcianx/downcast-rs](https://github.com/marcianx/downcast-rs) |  |
| `dyn-clone` | 1.0.20 | [github.com/dtolnay/dyn-clone](https://github.com/dtolnay/dyn-clone) |  |
| `either` | 1.17.0 | [github.com/rayon-rs/either](https://github.com/rayon-rs/either) |  |
| `encoding_rs_io` | 0.1.8 | [github.com/BurntSushi/encoding_rs_io](https://github.com/BurntSushi/encoding_rs_io) |  |
| `enumflags2` | 0.7.12 | [github.com/meithecatte/enumflags2](https://github.com/meithecatte/enumflags2) |  |
| `enumflags2_derive` | 0.7.12 | [github.com/meithecatte/enumflags2](https://github.com/meithecatte/enumflags2) |  |
| `enumn` | 0.1.14 | [github.com/dtolnay/enumn](https://github.com/dtolnay/enumn) |  |
| `equivalent` | 1.0.2 | [github.com/indexmap-rs/equivalent](https://github.com/indexmap-rs/equivalent) |  |
| `erased-serde` | 0.4.10 | [github.com/dtolnay/erased-serde](https://github.com/dtolnay/erased-serde) |  |
| `errno` | 0.3.14 | [github.com/lambda-fairy/rust-errno](https://github.com/lambda-fairy/rust-errno) |  |
| `etagere` | 0.2.15 | [github.com/nical/etagere](https://github.com/nical/etagere) |  |
| `euclid` | 0.22.14 | [github.com/servo/euclid](https://github.com/servo/euclid) |  |
| `event-listener` | 5.4.2 | [github.com/smol-rs/event-listener](https://github.com/smol-rs/event-listener) |  |
| `event-listener-strategy` | 0.5.4 | [github.com/smol-rs/event-listener-strategy](https://github.com/smol-rs/event-listener-strategy) |  |
| `fastrand` | 2.5.0 | [github.com/smol-rs/fastrand](https://github.com/smol-rs/fastrand) |  |
| `fdeflate` | 0.3.7 | [github.com/image-rs/fdeflate](https://github.com/image-rs/fdeflate) |  |
| `filetime` | 0.2.29 | [github.com/alexcrichton/filetime](https://github.com/alexcrichton/filetime) |  |
| `find-msvc-tools` | 0.1.11 | [github.com/rust-lang/cc-rs](https://github.com/rust-lang/cc-rs) |  |
| `fixedbitset` | 0.5.7 | [github.com/petgraph/fixedbitset](https://github.com/petgraph/fixedbitset) |  |
| `flate2` | 1.1.9 | [github.com/rust-lang/flate2-rs](https://github.com/rust-lang/flate2-rs) |  |
| `float-ord` | 0.3.2 | [github.com/notriddle/rust-float-ord](https://github.com/notriddle/rust-float-ord) |  |
| `flume` | 0.12.0 | [github.com/zesterer/flume](https://github.com/zesterer/flume) |  |
| `fnv` | 1.0.7 | [github.com/servo/rust-fnv](https://github.com/servo/rust-fnv) |  |
| `font-types` | 0.11.3 | [github.com/googlefonts/fontations](https://github.com/googlefonts/fontations) |  |
| `font-types` | 0.12.3 | [github.com/googlefonts/fontations](https://github.com/googlefonts/fontations) |  |
| `foreign-types` | 0.5.0 | [github.com/sfackler/foreign-types](https://github.com/sfackler/foreign-types) |  |
| `foreign-types-macros` | 0.2.4 | [github.com/sfackler/foreign-types](https://github.com/sfackler/foreign-types) |  |
| `foreign-types-shared` | 0.3.1 | [github.com/sfackler/foreign-types](https://github.com/sfackler/foreign-types) |  |
| `form_urlencoded` | 1.2.2 | [github.com/servo/rust-url](https://github.com/servo/rust-url) |  |
| `futf` | 0.1.5 | [github.com/servo/futf](https://github.com/servo/futf) |  |
| `futures` | 0.3.34 | [github.com/rust-lang/futures-rs](https://github.com/rust-lang/futures-rs) |  |
| `futures-channel` | 0.3.34 | [github.com/rust-lang/futures-rs](https://github.com/rust-lang/futures-rs) |  |
| `futures-concurrency` | 7.7.1 | [github.com/yoshuawuyts/futures-concurrency](https://github.com/yoshuawuyts/futures-concurrency) |  |
| `futures-core` | 0.3.34 | [github.com/rust-lang/futures-rs](https://github.com/rust-lang/futures-rs) |  |
| `futures-executor` | 0.3.34 | [github.com/rust-lang/futures-rs](https://github.com/rust-lang/futures-rs) |  |
| `futures-io` | 0.3.34 | [github.com/rust-lang/futures-rs](https://github.com/rust-lang/futures-rs) |  |
| `futures-lite` | 2.6.1 | [github.com/smol-rs/futures-lite](https://github.com/smol-rs/futures-lite) |  |
| `futures-macro` | 0.3.34 | [github.com/rust-lang/futures-rs](https://github.com/rust-lang/futures-rs) |  |
| `futures-sink` | 0.3.34 | [github.com/rust-lang/futures-rs](https://github.com/rust-lang/futures-rs) |  |
| `futures-task` | 0.3.34 | [github.com/rust-lang/futures-rs](https://github.com/rust-lang/futures-rs) |  |
| `futures-util` | 0.3.34 | [github.com/rust-lang/futures-rs](https://github.com/rust-lang/futures-rs) |  |
| `getrandom` | 0.2.17 | [github.com/rust-random/getrandom](https://github.com/rust-random/getrandom) |  |
| `getrandom` | 0.3.4 | [github.com/rust-random/getrandom](https://github.com/rust-random/getrandom) |  |
| `getrandom` | 0.4.3 | [github.com/rust-random/getrandom](https://github.com/rust-random/getrandom) |  |
| `gif` | 0.13.3 | [github.com/image-rs/image-gif](https://github.com/image-rs/image-gif) |  |
| `gif` | 0.14.2 | [github.com/image-rs/image-gif](https://github.com/image-rs/image-gif) |  |
| `gimli` | 0.32.3 | [github.com/gimli-rs/gimli](https://github.com/gimli-rs/gimli) |  |
| `glob` | 0.3.4 | [github.com/rust-lang/glob](https://github.com/rust-lang/glob) |  |
| `gpu-allocator` | 0.28.0 | [github.com/Traverse-Research/gpu-allocator](https://github.com/Traverse-Research/gpu-allocator) |  |
| `gpu-descriptor` | 0.3.2 | [github.com/zakarumych/gpu-descriptor](https://github.com/zakarumych/gpu-descriptor) |  |
| `gpu-descriptor-types` | 0.2.0 | [github.com/zakarumych/gpu-descriptor](https://github.com/zakarumych/gpu-descriptor) |  |
| `gpui-updater` | 0.0.7 | [github.com/AprilNEA/gpui-updater](https://github.com/AprilNEA/gpui-updater) |  |
| `granit-parser` | 0.0.7 | [github.com/bourumir-wyngs/granit-parser](https://github.com/bourumir-wyngs/granit-parser) |  |
| `half` | 2.7.1 | [github.com/VoidStarKat/half-rs](https://github.com/VoidStarKat/half-rs) |  |
| `hash32` | 0.3.1 | [github.com/japaric/hash32](https://github.com/japaric/hash32) |  |
| `hashbrown` | 0.14.5 | [github.com/rust-lang/hashbrown](https://github.com/rust-lang/hashbrown) |  |
| `hashbrown` | 0.15.5 | [github.com/rust-lang/hashbrown](https://github.com/rust-lang/hashbrown) |  |
| `hashbrown` | 0.16.1 | [github.com/rust-lang/hashbrown](https://github.com/rust-lang/hashbrown) |  |
| `hashbrown` | 0.17.1 | [github.com/rust-lang/hashbrown](https://github.com/rust-lang/hashbrown) |  |
| `hdrhistogram` | 7.6.0 | [github.com/HdrHistogram/HdrHistogram_rust.git](https://github.com/HdrHistogram/HdrHistogram_rust.git) |  |
| `heapless` | 0.9.3 | [github.com/rust-embedded/heapless](https://github.com/rust-embedded/heapless) |  |
| `heck` | 0.4.1 | [github.com/withoutboats/heck](https://github.com/withoutboats/heck) |  |
| `heck` | 0.5.0 | [github.com/withoutboats/heck](https://github.com/withoutboats/heck) |  |
| `hermit-abi` | 0.5.2 | [github.com/hermit-os/hermit-rs](https://github.com/hermit-os/hermit-rs) |  |
| `hex` | 0.4.3 | [github.com/KokaKiwi/rust-hex](https://github.com/KokaKiwi/rust-hex) |  |
| `hkdf` | 0.12.4 | [github.com/RustCrypto/KDFs/](https://github.com/RustCrypto/KDFs/) |  |
| `hmac` | 0.12.1 | [github.com/RustCrypto/MACs](https://github.com/RustCrypto/MACs) |  |
| `home` | 0.5.12 | [github.com/rust-lang/cargo](https://github.com/rust-lang/cargo) |  |
| `html5ever` | 0.27.0 | [github.com/servo/html5ever](https://github.com/servo/html5ever) |  |
| `http` | 1.5.0 | [github.com/hyperium/http](https://github.com/hyperium/http) |  |
| `httparse` | 1.10.1 | [github.com/seanmonstar/httparse](https://github.com/seanmonstar/httparse) |  |
| `httpdate` | 1.0.3 | [github.com/pyfisch/httpdate](https://github.com/pyfisch/httpdate) |  |
| `hybrid-array` | 0.4.14 | [github.com/RustCrypto/hybrid-array](https://github.com/RustCrypto/hybrid-array) |  |
| `iana-time-zone` | 0.1.65 | [github.com/strawlab/iana-time-zone](https://github.com/strawlab/iana-time-zone) |  |
| `iana-time-zone-haiku` | 0.1.2 | [github.com/strawlab/iana-time-zone](https://github.com/strawlab/iana-time-zone) |  |
| `idna` | 1.1.0 | [github.com/servo/rust-url/](https://github.com/servo/rust-url/) |  |
| `idna_adapter` | 1.2.2 | [github.com/hsivonen/idna_adapter](https://github.com/hsivonen/idna_adapter) |  |
| `image` | 0.25.10 | [github.com/image-rs/image](https://github.com/image-rs/image) |  |
| `image-webp` | 0.2.4 | [github.com/image-rs/image-webp](https://github.com/image-rs/image-webp) |  |
| `indexmap` | 2.14.0 | [github.com/indexmap-rs/indexmap](https://github.com/indexmap-rs/indexmap) |  |
| `inout` | 0.1.4 | [github.com/RustCrypto/utils](https://github.com/RustCrypto/utils) |  |
| `inventory` | 0.3.24 | [github.com/dtolnay/inventory](https://github.com/dtolnay/inventory) |  |
| `io-surface` | 0.16.1 | [github.com/servo/core-foundation-rs](https://github.com/servo/core-foundation-rs) |  |
| `ipnet` | 2.12.1 | [github.com/krisprice/ipnet](https://github.com/krisprice/ipnet) |  |
| `itertools` | 0.11.0 | [github.com/rust-itertools/itertools](https://github.com/rust-itertools/itertools) |  |
| `itertools` | 0.13.0 | [github.com/rust-itertools/itertools](https://github.com/rust-itertools/itertools) |  |
| `itertools` | 0.14.0 | [github.com/rust-itertools/itertools](https://github.com/rust-itertools/itertools) |  |
| `itoa` | 1.0.18 | [github.com/dtolnay/itoa](https://github.com/dtolnay/itoa) |  |
| `jni` | 0.22.4 | [github.com/jni-rs/jni-rs](https://github.com/jni-rs/jni-rs) |  |
| `jni-macros` | 0.22.4 | [github.com/jni-rs/jni-rs](https://github.com/jni-rs/jni-rs) |  |
| `jni-sys` | 0.3.1 | [github.com/jni-rs/jni-sys](https://github.com/jni-rs/jni-sys) |  |
| `jni-sys` | 0.4.1 | [github.com/jni-rs/jni-sys](https://github.com/jni-rs/jni-sys) |  |
| `jni-sys-macros` | 0.4.1 | [github.com/jni-rs/jni-sys](https://github.com/jni-rs/jni-sys) |  |
| `jobserver` | 0.1.35 | [github.com/rust-lang/jobserver-rs](https://github.com/rust-lang/jobserver-rs) |  |
| `js-sys` | 0.3.104 | [github.com/wasm-bindgen/wasm-bindgen/tree/master/crates/js-sys](https://github.com/wasm-bindgen/wasm-bindgen/tree/master/crates/js-sys) |  |
| `khronos-egl` | 6.0.0 | [github.com/timothee-haudebourg/khronos-egl](https://github.com/timothee-haudebourg/khronos-egl) |  |
| `kurbo` | 0.11.3 | [github.com/linebender/kurbo](https://github.com/linebender/kurbo) |  |
| `kurbo` | 0.13.1 | [github.com/linebender/kurbo](https://github.com/linebender/kurbo) |  |
| `lazy_static` | 1.5.0 | [github.com/rust-lang-nursery/lazy-static.rs](https://github.com/rust-lang-nursery/lazy-static.rs) |  |
| `leak` | 0.1.2 | [github.com/jmesmon/leak.git](https://github.com/jmesmon/leak.git) |  |
| `leaky-cow` | 0.1.1 | [github.com/notriddle/rust-leaky-cow](https://github.com/notriddle/rust-leaky-cow) |  |
| `libc` | 0.2.189 | [github.com/rust-lang/libc](https://github.com/rust-lang/libc) |  |
| `linebender_resource_handle` | 0.1.1 | [github.com/linebender/raw_resource_handle](https://github.com/linebender/raw_resource_handle) |  |
| `link-section` | 0.19.3 | [github.com/mmastrac/linktime](https://github.com/mmastrac/linktime) |  |
| `linktime-proc-macro` | 0.2.3 | [github.com/mmastrac/linktime](https://github.com/mmastrac/linktime) |  |
| `litrs` | 1.0.0 | [github.com/LukasKalbertodt/litrs](https://github.com/LukasKalbertodt/litrs) |  |
| `lock_api` | 0.4.14 | [github.com/Amanieu/parking_lot](https://github.com/Amanieu/parking_lot) |  |
| `log` | 0.4.33 | [github.com/rust-lang/log](https://github.com/rust-lang/log) |  |
| `lyon` | 1.0.19 | [github.com/nical/lyon](https://github.com/nical/lyon) |  |
| `lyon_algorithms` | 1.0.20 | [github.com/nical/lyon](https://github.com/nical/lyon) |  |
| `lyon_geom` | 1.0.19 | [github.com/nical/lyon](https://github.com/nical/lyon) |  |
| `lyon_path` | 1.0.19 | [github.com/nical/lyon](https://github.com/nical/lyon) |  |
| `lyon_tessellation` | 1.0.20 | [github.com/nical/lyon](https://github.com/nical/lyon) |  |
| `mac` | 0.1.1 | [github.com/reem/rust-mac.git](https://github.com/reem/rust-mac.git) |  |
| `mac-notification-sys` | 0.6.15 | [github.com/h4llow3En/mac-notification-sys](https://github.com/h4llow3En/mac-notification-sys) |  |
| `markup5ever` | 0.12.1 | [github.com/servo/html5ever](https://github.com/servo/html5ever) |  |
| `markup5ever_rcdom` | 0.3.0 | [github.com/servo/html5ever](https://github.com/servo/html5ever) |  |
| `md-5` | 0.10.6 | [github.com/RustCrypto/hashes](https://github.com/RustCrypto/hashes) |  |
| `memmap2` | 0.9.11 | [github.com/RazrFalcon/memmap2-rs](https://github.com/RazrFalcon/memmap2-rs) |  |
| `metal` | 0.33.0 | [github.com/gfx-rs/metal-rs](https://github.com/gfx-rs/metal-rs) |  |
| `mime` | 0.3.17 | [github.com/hyperium/mime](https://github.com/hyperium/mime) |  |
| `minimal-lexical` | 0.2.1 | [github.com/Alexhuszagh/minimal-lexical](https://github.com/Alexhuszagh/minimal-lexical) |  |
| `naga` | 29.0.4 | [github.com/gfx-rs/wgpu](https://github.com/gfx-rs/wgpu) |  |
| `ndk-sys` | 0.6.0+11769913 | [github.com/rust-mobile/ndk](https://github.com/rust-mobile/ndk) |  |
| `no_std_io2` | 0.9.4 | [github.com/wcampbell0x2a/no-std-io2](https://github.com/wcampbell0x2a/no-std-io2) |  |
| `nohash-hasher` | 0.2.0 | [github.com/paritytech/nohash-hasher](https://github.com/paritytech/nohash-hasher) |  |
| `normpath` | 1.5.1 | [github.com/dylni/normpath](https://github.com/dylni/normpath) |  |
| `notify-rust` | 4.18.0 | [github.com/hoodie/notify-rust](https://github.com/hoodie/notify-rust) |  |
| `notify-types` | 1.0.1 | [github.com/notify-rs/notify.git](https://github.com/notify-rs/notify.git) |  |
| `ntapi` | 0.4.3 | [github.com/MSxDOS/ntapi](https://github.com/MSxDOS/ntapi) |  |
| `num` | 0.4.3 | [github.com/rust-num/num](https://github.com/rust-num/num) |  |
| `num-bigint` | 0.4.8 | [github.com/rust-num/num-bigint](https://github.com/rust-num/num-bigint) |  |
| `num-bigint-dig` | 0.9.1 | [github.com/dignifiedquire/num-bigint](https://github.com/dignifiedquire/num-bigint) |  |
| `num-complex` | 0.4.6 | [github.com/rust-num/num-complex](https://github.com/rust-num/num-complex) |  |
| `num-conv` | 0.2.2 | [github.com/jhpratt/num-conv](https://github.com/jhpratt/num-conv) |  |
| `num-derive` | 0.4.2 | [github.com/rust-num/num-derive](https://github.com/rust-num/num-derive) |  |
| `num-integer` | 0.1.47 | [github.com/rust-num/num-integer](https://github.com/rust-num/num-integer) |  |
| `num-iter` | 0.1.46 | [github.com/rust-num/num-iter](https://github.com/rust-num/num-iter) |  |
| `num-rational` | 0.4.2 | [github.com/rust-num/num-rational](https://github.com/rust-num/num-rational) |  |
| `num-traits` | 0.2.19 | [github.com/rust-num/num-traits](https://github.com/rust-num/num-traits) |  |
| `num_cpus` | 1.17.0 | [github.com/seanmonstar/num_cpus](https://github.com/seanmonstar/num_cpus) |  |
| `object` | 0.37.3 | [github.com/gimli-rs/object](https://github.com/gimli-rs/object) |  |
| `object` | 0.39.1 | [github.com/gimli-rs/object](https://github.com/gimli-rs/object) |  |
| `oid-registry` | 0.8.1 | [github.com/rusticata/oid-registry.git](https://github.com/rusticata/oid-registry.git) |  |
| `once_cell` | 1.21.4 | [github.com/matklad/once_cell](https://github.com/matklad/once_cell) |  |
| `openssl-probe` | 0.2.1 | [github.com/rustls/openssl-probe](https://github.com/rustls/openssl-probe) |  |
| `ordered-stream` | 0.2.0 | [github.com/danieldg/ordered-stream](https://github.com/danieldg/ordered-stream) |  |
| `parking` | 2.2.1 | [github.com/smol-rs/parking](https://github.com/smol-rs/parking) |  |
| `parking_lot` | 0.12.5 | [github.com/Amanieu/parking_lot](https://github.com/Amanieu/parking_lot) |  |
| `parking_lot_core` | 0.9.12 | [github.com/Amanieu/parking_lot](https://github.com/Amanieu/parking_lot) |  |
| `paste` | 1.0.15 | [github.com/dtolnay/paste](https://github.com/dtolnay/paste) |  |
| `pastey` | 0.1.1 | [github.com/as1100k/pastey](https://github.com/as1100k/pastey) |  |
| `pathfinder_geometry` | 0.5.1 | [github.com/servo/pathfinder](https://github.com/servo/pathfinder) |  |
| `pathfinder_simd` | 0.5.6 | [github.com/servo/pathfinder](https://github.com/servo/pathfinder) |  |
| `pbkdf2` | 0.12.2 | [github.com/RustCrypto/password-hashes/tree/master/pbkdf2](https://github.com/RustCrypto/password-hashes/tree/master/pbkdf2) |  |
| `percent-encoding` | 2.3.2 | [github.com/servo/rust-url/](https://github.com/servo/rust-url/) |  |
| `pin-project` | 1.1.13 | [github.com/taiki-e/pin-project](https://github.com/taiki-e/pin-project) |  |
| `pin-project-internal` | 1.1.13 | [github.com/taiki-e/pin-project](https://github.com/taiki-e/pin-project) |  |
| `pin-project-lite` | 0.2.17 | [github.com/taiki-e/pin-project-lite](https://github.com/taiki-e/pin-project-lite) |  |
| `piper` | 0.2.5 | [github.com/smol-rs/piper](https://github.com/smol-rs/piper) |  |
| `pkg-config` | 0.3.34 | [github.com/rust-lang/pkg-config-rs](https://github.com/rust-lang/pkg-config-rs) |  |
| `png` | 0.17.16 | [github.com/image-rs/image-png](https://github.com/image-rs/image-png) |  |
| `png` | 0.18.1 | [github.com/image-rs/image-png](https://github.com/image-rs/image-png) |  |
| `polling` | 3.11.0 | [github.com/smol-rs/polling](https://github.com/smol-rs/polling) |  |
| `pollster` | 0.2.5 | [github.com/zesterer/pollster](https://github.com/zesterer/pollster) |  |
| `pollster` | 0.4.0 | [github.com/zesterer/pollster](https://github.com/zesterer/pollster) |  |
| `polycool` | 0.4.0 | [github.com/linebender/kurbo](https://github.com/linebender/kurbo) |  |
| `portable-atomic` | 1.15.0 | [github.com/taiki-e/portable-atomic](https://github.com/taiki-e/portable-atomic) |  |
| `portable-atomic-util` | 0.2.7 | [github.com/taiki-e/portable-atomic-util](https://github.com/taiki-e/portable-atomic-util) |  |
| `powerfmt` | 0.2.0 | [github.com/jhpratt/powerfmt](https://github.com/jhpratt/powerfmt) |  |
| `ppv-lite86` | 0.2.21 | [github.com/cryptocorrosion/cryptocorrosion](https://github.com/cryptocorrosion/cryptocorrosion) |  |
| `presser` | 0.3.1 | [github.com/EmbarkStudios/presser](https://github.com/EmbarkStudios/presser) |  |
| `prettyplease` | 0.2.37 | [github.com/dtolnay/prettyplease](https://github.com/dtolnay/prettyplease) |  |
| `proc-macro-crate` | 3.5.0 | [github.com/bkchr/proc-macro-crate](https://github.com/bkchr/proc-macro-crate) |  |
| `proc-macro2` | 1.0.107 | [github.com/dtolnay/proc-macro2](https://github.com/dtolnay/proc-macro2) |  |
| `profiling` | 1.0.18 | [github.com/aclysma/profiling](https://github.com/aclysma/profiling) |  |
| `profiling-procmacros` | 1.0.18 | [github.com/aclysma/profiling](https://github.com/aclysma/profiling) |  |
| `proptest` | 1.10.0 | [github.com/proptest-rs/proptest](https://github.com/proptest-rs/proptest) |  |
| `proptest-macro` | 0.5.0 | [github.com/proptest-rs/proptest](https://github.com/proptest-rs/proptest) |  |
| `psm` | 0.1.32 | [github.com/rust-lang/stacker/](https://github.com/rust-lang/stacker/) |  |
| `qoi` | 0.4.1 | [github.com/aldanor/qoi-rust](https://github.com/aldanor/qoi-rust) |  |
| `quick-error` | 1.2.3 | [http://github.com/tailhook/quick-error](http://github.com/tailhook/quick-error) |  |
| `quick-error` | 2.0.1 | [http://github.com/tailhook/quick-error](http://github.com/tailhook/quick-error) |  |
| `quinn` | 0.11.11 | [github.com/quinn-rs/quinn](https://github.com/quinn-rs/quinn) |  |
| `quinn-proto` | 0.11.17 | [github.com/quinn-rs/quinn](https://github.com/quinn-rs/quinn) |  |
| `quinn-udp` | 0.5.15 | [github.com/quinn-rs/quinn](https://github.com/quinn-rs/quinn) |  |
| `quote` | 1.0.47 | [github.com/dtolnay/quote](https://github.com/dtolnay/quote) |  |
| `rand` | 0.8.7 | [github.com/rust-random/rand](https://github.com/rust-random/rand) |  |
| `rand` | 0.9.5 | [github.com/rust-random/rand](https://github.com/rust-random/rand) |  |
| `rand` | 0.10.2 | [github.com/rust-random/rand](https://github.com/rust-random/rand) |  |
| `rand_chacha` | 0.3.1 | [github.com/rust-random/rand](https://github.com/rust-random/rand) |  |
| `rand_chacha` | 0.9.0 | [github.com/rust-random/rand](https://github.com/rust-random/rand) |  |
| `rand_core` | 0.6.4 | [github.com/rust-random/rand](https://github.com/rust-random/rand) |  |
| `rand_core` | 0.9.5 | [github.com/rust-random/rand](https://github.com/rust-random/rand) |  |
| `rand_core` | 0.10.1 | [github.com/rust-random/rand_core](https://github.com/rust-random/rand_core) |  |
| `rand_pcg` | 0.10.2 | [github.com/rust-random/rngs](https://github.com/rust-random/rngs) |  |
| `rand_xorshift` | 0.4.0 | [github.com/rust-random/rngs](https://github.com/rust-random/rngs) |  |
| `range-alloc` | 0.1.5 | [github.com/gfx-rs/range-alloc](https://github.com/gfx-rs/range-alloc) |  |
| `rangemap` | 1.8.0 | [github.com/jeffparsons/rangemap](https://github.com/jeffparsons/rangemap) |  |
| `raw-window-metal` | 1.1.0 | [github.com/rust-windowing/raw-window-metal](https://github.com/rust-windowing/raw-window-metal) |  |
| `rayon` | 1.12.0 | [github.com/rayon-rs/rayon](https://github.com/rayon-rs/rayon) |  |
| `rayon-core` | 1.13.0 | [github.com/rayon-rs/rayon](https://github.com/rayon-rs/rayon) |  |
| `read-fonts` | 0.37.0 | [github.com/googlefonts/fontations](https://github.com/googlefonts/fontations) |  |
| `read-fonts` | 0.41.0 | [github.com/googlefonts/fontations](https://github.com/googlefonts/fontations) |  |
| `ref-cast` | 1.0.27 | [github.com/dtolnay/ref-cast](https://github.com/dtolnay/ref-cast) |  |
| `ref-cast-impl` | 1.0.27 | [github.com/dtolnay/ref-cast](https://github.com/dtolnay/ref-cast) |  |
| `regex` | 1.13.1 | [github.com/rust-lang/regex](https://github.com/rust-lang/regex) |  |
| `regex-automata` | 0.4.18 | [github.com/rust-lang/regex](https://github.com/rust-lang/regex) |  |
| `regex-syntax` | 0.8.11 | [github.com/rust-lang/regex](https://github.com/rust-lang/regex) |  |
| `renderdoc-sys` | 1.1.0 | [github.com/ebkalderon/renderdoc-rs](https://github.com/ebkalderon/renderdoc-rs) |  |
| `reqwest` | 0.13.4 | [github.com/seanmonstar/reqwest](https://github.com/seanmonstar/reqwest) |  |
| `resvg` | 0.45.1 | [github.com/linebender/resvg](https://github.com/linebender/resvg) |  |
| `resvg` | 0.46.0 | [github.com/linebender/resvg](https://github.com/linebender/resvg) |  |
| `ropey` | 2.0.0-beta.1 | [github.com/cessen/ropey](https://github.com/cessen/ropey) |  |
| `roxmltree` | 0.20.0 | [github.com/RazrFalcon/roxmltree](https://github.com/RazrFalcon/roxmltree) |  |
| `roxmltree` | 0.21.1 | [github.com/RazrFalcon/roxmltree](https://github.com/RazrFalcon/roxmltree) |  |
| `rustc-demangle` | 0.1.28 | [github.com/rust-lang/rustc-demangle](https://github.com/rust-lang/rustc-demangle) |  |
| `rustc-hash` | 1.1.0 | [github.com/rust-lang-nursery/rustc-hash](https://github.com/rust-lang-nursery/rustc-hash) |  |
| `rustc-hash` | 2.1.3 | [github.com/rust-lang/rustc-hash](https://github.com/rust-lang/rustc-hash) |  |
| `rustc_version` | 0.4.1 | [github.com/djc/rustc-version-rs](https://github.com/djc/rustc-version-rs) |  |
| `rusticata-macros` | 4.1.0 | [github.com/rusticata/rusticata-macros.git](https://github.com/rusticata/rusticata-macros.git) |  |
| `rustls-pki-types` | 1.15.1 | [github.com/rustls/pki-types](https://github.com/rustls/pki-types) |  |
| `rustls-platform-verifier` | 0.7.0 | [github.com/rustls/rustls-platform-verifier](https://github.com/rustls/rustls-platform-verifier) |  |
| `rustls-platform-verifier-android` | 0.1.1 | [github.com/rustls/rustls-platform-verifier](https://github.com/rustls/rustls-platform-verifier) |  |
| `rustversion` | 1.0.23 | [github.com/dtolnay/rustversion](https://github.com/dtolnay/rustversion) |  |
| `rusty-fork` | 0.3.1 | [github.com/altsysrq/rusty-fork](https://github.com/altsysrq/rusty-fork) |  |
| `scoped-tls` | 1.0.1 | [github.com/alexcrichton/scoped-tls](https://github.com/alexcrichton/scoped-tls) |  |
| `scopeguard` | 1.2.0 | [github.com/bluss/scopeguard](https://github.com/bluss/scopeguard) |  |
| `screencapturekit` | 0.2.8 | [github.com/svtlabs/screencapturekit-rs/tree/main/screencapturekit](https://github.com/svtlabs/screencapturekit-rs/tree/main/screencapturekit) |  |
| `screencapturekit-sys` | 0.2.8 | [github.com/svtlabs/screencapturekit-rs/tree/main/screencapturekit-sys](https://github.com/svtlabs/screencapturekit-rs/tree/main/screencapturekit-sys) |  |
| `security-framework` | 3.7.0 | [github.com/kornelski/rust-security-framework](https://github.com/kornelski/rust-security-framework) |  |
| `security-framework-sys` | 2.17.0 | [github.com/kornelski/rust-security-framework](https://github.com/kornelski/rust-security-framework) |  |
| `semver` | 1.0.28 | [github.com/dtolnay/semver](https://github.com/dtolnay/semver) |  |
| `serde` | 1.0.229 | [github.com/serde-rs/serde](https://github.com/serde-rs/serde) |  |
| `serde-saphyr` | 0.0.29 | [github.com/bourumir-wyngs/serde-saphyr](https://github.com/bourumir-wyngs/serde-saphyr) |  |
| `serde_bytes` | 0.11.19 | [github.com/serde-rs/bytes](https://github.com/serde-rs/bytes) |  |
| `serde_core` | 1.0.229 | [github.com/serde-rs/serde](https://github.com/serde-rs/serde) |  |
| `serde_derive` | 1.0.229 | [github.com/serde-rs/serde](https://github.com/serde-rs/serde) |  |
| `serde_derive_internals` | 0.30.0 | [github.com/serde-rs/serde](https://github.com/serde-rs/serde) |  |
| `serde_fmt` | 1.1.0 | [github.com/KodrAus/serde_fmt.git](https://github.com/KodrAus/serde_fmt.git) |  |
| `serde_json` | 1.0.151 | [github.com/serde-rs/json](https://github.com/serde-rs/json) |  |
| `serde_repr` | 0.1.21 | [github.com/dtolnay/serde-repr](https://github.com/dtolnay/serde-repr) |  |
| `serde_spanned` | 0.6.9 | [github.com/toml-rs/toml](https://github.com/toml-rs/toml) |  |
| `serde_spanned` | 1.1.1 | [github.com/toml-rs/toml](https://github.com/toml-rs/toml) |  |
| `serde_urlencoded` | 0.7.1 | [github.com/nox/serde_urlencoded](https://github.com/nox/serde_urlencoded) |  |
| `sha2` | 0.10.9 | [github.com/RustCrypto/hashes](https://github.com/RustCrypto/hashes) |  |
| `sha2` | 0.11.0 | [github.com/RustCrypto/hashes](https://github.com/RustCrypto/hashes) |  |
| `shellexpand` | 3.1.2 | [gitlab.com/ijackson/rust-shellexpand](https://gitlab.com/ijackson/rust-shellexpand) |  |
| `shlex` | 1.3.0 | [github.com/comex/rust-shlex](https://github.com/comex/rust-shlex) |  |
| `shlex` | 2.0.1 | [github.com/comex/rust-shlex](https://github.com/comex/rust-shlex) |  |
| `signal-hook-registry` | 1.4.8 | [github.com/vorner/signal-hook](https://github.com/vorner/signal-hook) |  |
| `simd_cesu8` | 1.2.0 | [github.com/seancroach/simd_cesu8](https://github.com/seancroach/simd_cesu8) |  |
| `simdutf8` | 0.1.5 | [github.com/rusticstuff/simdutf8](https://github.com/rusticstuff/simdutf8) |  |
| `simplecss` | 0.2.2 | [github.com/linebender/simplecss](https://github.com/linebender/simplecss) |  |
| `siphasher` | 1.0.3 | [github.com/jedisct1/rust-siphash](https://github.com/jedisct1/rust-siphash) |  |
| `skrifa` | 0.40.0 | [github.com/googlefonts/fontations](https://github.com/googlefonts/fontations) |  |
| `skrifa` | 0.44.0 | [github.com/googlefonts/fontations](https://github.com/googlefonts/fontations) |  |
| `smallvec` | 1.15.2 | [github.com/servo/rust-smallvec](https://github.com/servo/rust-smallvec) |  |
| `smol` | 2.0.2 | [github.com/smol-rs/smol](https://github.com/smol-rs/smol) |  |
| `smol_str` | 0.3.6 | [github.com/rust-lang/rust-analyzer/tree/master/lib/smol_str](https://github.com/rust-lang/rust-analyzer/tree/master/lib/smol_str) |  |
| `socket2` | 0.6.5 | [github.com/rust-lang/socket2](https://github.com/rust-lang/socket2) |  |
| `stable_deref_trait` | 1.2.1 | [github.com/storyyeller/stable_deref_trait](https://github.com/storyyeller/stable_deref_trait) |  |
| `stacker` | 0.1.25 | [github.com/rust-lang/stacker](https://github.com/rust-lang/stacker) |  |
| `static_assertions` | 1.1.0 | [github.com/nvzqz/static-assertions-rs](https://github.com/nvzqz/static-assertions-rs) |  |
| `str_indices` | 0.4.4 | [github.com/cessen/str_indices](https://github.com/cessen/str_indices) |  |
| `streaming-iterator` | 0.1.9 | [github.com/sfackler/streaming-iterator](https://github.com/sfackler/streaming-iterator) |  |
| `string_cache` | 0.8.9 | [github.com/servo/string-cache](https://github.com/servo/string-cache) |  |
| `string_cache_codegen` | 0.5.4 | [github.com/servo/string-cache](https://github.com/servo/string-cache) |  |
| `sval` | 2.21.1 | [github.com/sval-rs/sval](https://github.com/sval-rs/sval) |  |
| `sval_buffer` | 2.21.1 | [github.com/sval-rs/sval](https://github.com/sval-rs/sval) |  |
| `sval_dynamic` | 2.21.1 | [github.com/sval-rs/sval](https://github.com/sval-rs/sval) |  |
| `sval_fmt` | 2.21.1 | [github.com/sval-rs/sval](https://github.com/sval-rs/sval) |  |
| `sval_json` | 2.21.1 | [github.com/sval-rs/sval](https://github.com/sval-rs/sval) |  |
| `sval_nested` | 2.21.1 | [github.com/sval-rs/sval](https://github.com/sval-rs/sval) |  |
| `sval_ref` | 2.21.1 | [github.com/sval-rs/sval](https://github.com/sval-rs/sval) |  |
| `sval_serde` | 2.21.1 | [github.com/sval-rs/sval](https://github.com/sval-rs/sval) |  |
| `svg_fmt` | 0.4.5 | [github.com/nical/rust_debug](https://github.com/nical/rust_debug) |  |
| `svgtypes` | 0.15.3 | [github.com/linebender/svgtypes](https://github.com/linebender/svgtypes) |  |
| `svgtypes` | 0.16.1 | [github.com/linebender/svgtypes](https://github.com/linebender/svgtypes) |  |
| `swash` | 0.2.10 | [github.com/dfrg/swash](https://github.com/dfrg/swash) |  |
| `syn` | 2.0.119 | [github.com/dtolnay/syn](https://github.com/dtolnay/syn) |  |
| `syn` | 3.0.3 | [github.com/dtolnay/syn](https://github.com/dtolnay/syn) |  |
| `sys-locale` | 0.3.2 | [github.com/1Password/sys-locale](https://github.com/1Password/sys-locale) |  |
| `system-configuration` | 0.6.1 | [github.com/mullvad/system-configuration-rs](https://github.com/mullvad/system-configuration-rs) |  |
| `system-configuration` | 0.7.0 | [github.com/mullvad/system-configuration-rs](https://github.com/mullvad/system-configuration-rs) |  |
| `system-configuration-sys` | 0.6.0 | [github.com/mullvad/system-configuration-rs](https://github.com/mullvad/system-configuration-rs) |  |
| `tauri-winrt-notification` | 0.7.3 | [github.com/tauri-apps/winrt-notification](https://github.com/tauri-apps/winrt-notification) |  |
| `tempfile` | 3.27.0 | [github.com/Stebalien/tempfile](https://github.com/Stebalien/tempfile) |  |
| `tendril` | 0.4.3 | [github.com/servo/tendril](https://github.com/servo/tendril) |  |
| `thiserror` | 1.0.69 | [github.com/dtolnay/thiserror](https://github.com/dtolnay/thiserror) |  |
| `thiserror` | 2.0.20 | [github.com/dtolnay/thiserror](https://github.com/dtolnay/thiserror) |  |
| `thiserror-impl` | 1.0.69 | [github.com/dtolnay/thiserror](https://github.com/dtolnay/thiserror) |  |
| `thiserror-impl` | 2.0.20 | [github.com/dtolnay/thiserror](https://github.com/dtolnay/thiserror) |  |
| `thread_local` | 1.1.10 | [github.com/Amanieu/thread_local-rs](https://github.com/Amanieu/thread_local-rs) |  |
| `time` | 0.3.55 | [github.com/time-rs/time](https://github.com/time-rs/time) |  |
| `time-core` | 0.1.9 | [github.com/time-rs/time](https://github.com/time-rs/time) |  |
| `time-macros` | 0.2.32 | [github.com/time-rs/time](https://github.com/time-rs/time) |  |
| `tokio-rustls` | 0.26.4 | [github.com/rustls/tokio-rustls](https://github.com/rustls/tokio-rustls) |  |
| `toml` | 0.8.23 | [github.com/toml-rs/toml](https://github.com/toml-rs/toml) |  |
| `toml` | 1.1.4+spec-1.1.0 | [github.com/toml-rs/toml](https://github.com/toml-rs/toml) |  |
| `toml_datetime` | 0.6.11 | [github.com/toml-rs/toml](https://github.com/toml-rs/toml) |  |
| `toml_datetime` | 1.1.1+spec-1.1.0 | [github.com/toml-rs/toml](https://github.com/toml-rs/toml) |  |
| `toml_edit` | 0.22.27 | [github.com/toml-rs/toml](https://github.com/toml-rs/toml) |  |
| `toml_edit` | 0.25.13+spec-1.1.0 | [github.com/toml-rs/toml](https://github.com/toml-rs/toml) |  |
| `toml_parser` | 1.1.3+spec-1.1.0 | [github.com/toml-rs/toml](https://github.com/toml-rs/toml) |  |
| `toml_write` | 0.1.2 | [github.com/toml-rs/toml](https://github.com/toml-rs/toml) |  |
| `toml_writer` | 1.1.2+spec-1.1.0 | [github.com/toml-rs/toml](https://github.com/toml-rs/toml) |  |
| `triomphe` | 0.1.16 | [github.com/Manishearth/triomphe](https://github.com/Manishearth/triomphe) |  |
| `ttf-parser` | 0.25.1 | [github.com/harfbuzz/ttf-parser](https://github.com/harfbuzz/ttf-parser) |  |
| `typeid` | 1.0.3 | [github.com/dtolnay/typeid](https://github.com/dtolnay/typeid) |  |
| `typenum` | 1.20.1 | [github.com/paholg/typenum](https://github.com/paholg/typenum) |  |
| `unarray` | 0.1.4 | [github.com/cameron1024/unarray](https://github.com/cameron1024/unarray) |  |
| `unicase` | 2.9.0 | [github.com/seanmonstar/unicase](https://github.com/seanmonstar/unicase) |  |
| `unicode-bidi` | 0.3.18 | [github.com/servo/unicode-bidi](https://github.com/servo/unicode-bidi) |  |
| `unicode-bidi-mirroring` | 0.4.0 | [github.com/RazrFalcon/unicode-bidi-mirroring](https://github.com/RazrFalcon/unicode-bidi-mirroring) |  |
| `unicode-ccc` | 0.4.0 | [github.com/RazrFalcon/unicode-ccc](https://github.com/RazrFalcon/unicode-ccc) |  |
| `unicode-id` | 0.3.6 | [github.com/Boshen/unicode-id](https://github.com/Boshen/unicode-id) |  |
| `unicode-properties` | 0.1.4 | [github.com/unicode-rs/unicode-properties](https://github.com/unicode-rs/unicode-properties) |  |
| `unicode-script` | 0.5.8 | [github.com/unicode-rs/unicode-script](https://github.com/unicode-rs/unicode-script) |  |
| `unicode-segmentation` | 1.13.3 | [github.com/unicode-rs/unicode-segmentation](https://github.com/unicode-rs/unicode-segmentation) |  |
| `unicode-vo` | 0.1.0 | [github.com/RazrFalcon/unicode-vo](https://github.com/RazrFalcon/unicode-vo) |  |
| `unicode-width` | 0.2.2 | [github.com/unicode-rs/unicode-width](https://github.com/unicode-rs/unicode-width) |  |
| `unicode-xid` | 0.2.6 | [github.com/unicode-rs/unicode-xid](https://github.com/unicode-rs/unicode-xid) |  |
| `ureq` | 3.4.0 | [github.com/algesten/ureq](https://github.com/algesten/ureq) |  |
| `ureq-proto` | 0.6.1 | [github.com/algesten/ureq-proto](https://github.com/algesten/ureq-proto) |  |
| `url` | 2.5.8 | [github.com/servo/rust-url](https://github.com/servo/rust-url) |  |
| `usvg` | 0.45.1 | [github.com/linebender/resvg](https://github.com/linebender/resvg) |  |
| `usvg` | 0.46.0 | [github.com/linebender/resvg](https://github.com/linebender/resvg) |  |
| `utf-8` | 0.7.6 | [github.com/SimonSapin/rust-utf8](https://github.com/SimonSapin/rust-utf8) |  |
| `utf8-zero` | 0.8.1 | [github.com/algesten/utf8-zero](https://github.com/algesten/utf8-zero) |  |
| `utf8_iter` | 1.0.4 | [github.com/hsivonen/utf8_iter](https://github.com/hsivonen/utf8_iter) |  |
| `uuid` | 1.24.1 | [github.com/uuid-rs/uuid](https://github.com/uuid-rs/uuid) |  |
| `value-bag` | 1.13.2 | [github.com/sval-rs/value-bag](https://github.com/sval-rs/value-bag) |  |
| `value-bag-serde1` | 1.13.2 | — |  |
| `value-bag-sval2` | 1.13.2 | — |  |
| `version_check` | 0.9.5 | [github.com/SergioBenitez/version_check](https://github.com/SergioBenitez/version_check) |  |
| `wait-timeout` | 0.2.1 | [github.com/alexcrichton/wait-timeout](https://github.com/alexcrichton/wait-timeout) |  |
| `waker-fn` | 1.2.0 | [github.com/smol-rs/waker-fn](https://github.com/smol-rs/waker-fn) |  |
| `wasm-bindgen` | 0.2.127 | [github.com/wasm-bindgen/wasm-bindgen](https://github.com/wasm-bindgen/wasm-bindgen) |  |
| `wasm-bindgen-futures` | 0.4.77 | [github.com/wasm-bindgen/wasm-bindgen/tree/master/crates/futures](https://github.com/wasm-bindgen/wasm-bindgen/tree/master/crates/futures) |  |
| `wasm-bindgen-macro` | 0.2.127 | [github.com/wasm-bindgen/wasm-bindgen/tree/master/crates/macro](https://github.com/wasm-bindgen/wasm-bindgen/tree/master/crates/macro) |  |
| `wasm-bindgen-macro-support` | 0.2.127 | [github.com/wasm-bindgen/wasm-bindgen/tree/master/crates/macro-support](https://github.com/wasm-bindgen/wasm-bindgen/tree/master/crates/macro-support) |  |
| `wasm-bindgen-shared` | 0.2.127 | [github.com/wasm-bindgen/wasm-bindgen/tree/master/crates/shared](https://github.com/wasm-bindgen/wasm-bindgen/tree/master/crates/shared) |  |
| `wasm-streams` | 0.4.2 | [github.com/MattiasBuelens/wasm-streams/](https://github.com/MattiasBuelens/wasm-streams/) |  |
| `wasm-streams` | 0.5.0 | [github.com/MattiasBuelens/wasm-streams/](https://github.com/MattiasBuelens/wasm-streams/) |  |
| `wasm_thread` | 0.3.3 | [github.com/chemicstry/wasm_thread](https://github.com/chemicstry/wasm_thread) |  |
| `web-sys` | 0.3.104 | [github.com/wasm-bindgen/wasm-bindgen/tree/master/crates/web-sys](https://github.com/wasm-bindgen/wasm-bindgen/tree/master/crates/web-sys) |  |
| `web-time` | 1.1.0 | [github.com/daxpedda/web-time](https://github.com/daxpedda/web-time) |  |
| `weezl` | 0.1.12 | [github.com/image-rs/weezl](https://github.com/image-rs/weezl) |  |
| `wgpu` | 29.0.4 | [github.com/gfx-rs/wgpu](https://github.com/gfx-rs/wgpu) |  |
| `wgpu-core` | 29.0.4 | [github.com/gfx-rs/wgpu](https://github.com/gfx-rs/wgpu) |  |
| `wgpu-core-deps-apple` | 29.0.4 | [github.com/gfx-rs/wgpu](https://github.com/gfx-rs/wgpu) |  |
| `wgpu-core-deps-emscripten` | 29.0.4 | [github.com/gfx-rs/wgpu](https://github.com/gfx-rs/wgpu) |  |
| `wgpu-core-deps-wasm` | 29.0.4 | [github.com/gfx-rs/wgpu](https://github.com/gfx-rs/wgpu) |  |
| `wgpu-core-deps-windows-linux-android` | 29.0.4 | [github.com/gfx-rs/wgpu](https://github.com/gfx-rs/wgpu) |  |
| `wgpu-hal` | 29.0.4 | [github.com/gfx-rs/wgpu](https://github.com/gfx-rs/wgpu) |  |
| `wgpu-naga-bridge` | 29.0.4 | [github.com/gfx-rs/wgpu](https://github.com/gfx-rs/wgpu) |  |
| `wgpu-types` | 29.0.4 | [github.com/gfx-rs/wgpu](https://github.com/gfx-rs/wgpu) |  |
| `winapi` | 0.3.9 | [github.com/retep998/winapi-rs](https://github.com/retep998/winapi-rs) |  |
| `winapi-i686-pc-windows-gnu` | 0.4.0 | [github.com/retep998/winapi-rs](https://github.com/retep998/winapi-rs) |  |
| `winapi-x86_64-pc-windows-gnu` | 0.4.0 | [github.com/retep998/winapi-rs](https://github.com/retep998/winapi-rs) |  |
| `windows` | 0.57.0 | [github.com/microsoft/windows-rs](https://github.com/microsoft/windows-rs) |  |
| `windows` | 0.58.0 | [github.com/microsoft/windows-rs](https://github.com/microsoft/windows-rs) |  |
| `windows` | 0.61.3 | [github.com/microsoft/windows-rs](https://github.com/microsoft/windows-rs) |  |
| `windows` | 0.62.2 | [github.com/microsoft/windows-rs](https://github.com/microsoft/windows-rs) |  |
| `windows-collections` | 0.2.0 | [github.com/microsoft/windows-rs](https://github.com/microsoft/windows-rs) |  |
| `windows-collections` | 0.3.2 | [github.com/microsoft/windows-rs](https://github.com/microsoft/windows-rs) |  |
| `windows-core` | 0.57.0 | [github.com/microsoft/windows-rs](https://github.com/microsoft/windows-rs) |  |
| `windows-core` | 0.58.0 | [github.com/microsoft/windows-rs](https://github.com/microsoft/windows-rs) |  |
| `windows-core` | 0.61.2 | [github.com/microsoft/windows-rs](https://github.com/microsoft/windows-rs) |  |
| `windows-core` | 0.62.2 | [github.com/microsoft/windows-rs](https://github.com/microsoft/windows-rs) |  |
| `windows-future` | 0.2.1 | [github.com/microsoft/windows-rs](https://github.com/microsoft/windows-rs) |  |
| `windows-future` | 0.3.2 | [github.com/microsoft/windows-rs](https://github.com/microsoft/windows-rs) |  |
| `windows-implement` | 0.57.0 | [github.com/microsoft/windows-rs](https://github.com/microsoft/windows-rs) |  |
| `windows-implement` | 0.58.0 | [github.com/microsoft/windows-rs](https://github.com/microsoft/windows-rs) |  |
| `windows-implement` | 0.60.2 | [github.com/microsoft/windows-rs](https://github.com/microsoft/windows-rs) |  |
| `windows-interface` | 0.57.0 | [github.com/microsoft/windows-rs](https://github.com/microsoft/windows-rs) |  |
| `windows-interface` | 0.58.0 | [github.com/microsoft/windows-rs](https://github.com/microsoft/windows-rs) |  |
| `windows-interface` | 0.59.3 | [github.com/microsoft/windows-rs](https://github.com/microsoft/windows-rs) |  |
| `windows-link` | 0.1.3 | [github.com/microsoft/windows-rs](https://github.com/microsoft/windows-rs) |  |
| `windows-link` | 0.2.1 | [github.com/microsoft/windows-rs](https://github.com/microsoft/windows-rs) |  |
| `windows-numerics` | 0.2.0 | [github.com/microsoft/windows-rs](https://github.com/microsoft/windows-rs) |  |
| `windows-numerics` | 0.3.1 | [github.com/microsoft/windows-rs](https://github.com/microsoft/windows-rs) |  |
| `windows-registry` | 0.4.0 | [github.com/microsoft/windows-rs](https://github.com/microsoft/windows-rs) |  |
| `windows-registry` | 0.5.3 | [github.com/microsoft/windows-rs](https://github.com/microsoft/windows-rs) |  |
| `windows-registry` | 0.6.1 | [github.com/microsoft/windows-rs](https://github.com/microsoft/windows-rs) |  |
| `windows-result` | 0.1.2 | [github.com/microsoft/windows-rs](https://github.com/microsoft/windows-rs) |  |
| `windows-result` | 0.2.0 | [github.com/microsoft/windows-rs](https://github.com/microsoft/windows-rs) |  |
| `windows-result` | 0.3.4 | [github.com/microsoft/windows-rs](https://github.com/microsoft/windows-rs) |  |
| `windows-result` | 0.4.1 | [github.com/microsoft/windows-rs](https://github.com/microsoft/windows-rs) |  |
| `windows-strings` | 0.1.0 | [github.com/microsoft/windows-rs](https://github.com/microsoft/windows-rs) |  |
| `windows-strings` | 0.3.1 | [github.com/microsoft/windows-rs](https://github.com/microsoft/windows-rs) |  |
| `windows-strings` | 0.4.2 | [github.com/microsoft/windows-rs](https://github.com/microsoft/windows-rs) |  |
| `windows-strings` | 0.5.1 | [github.com/microsoft/windows-rs](https://github.com/microsoft/windows-rs) |  |
| `windows-sys` | 0.52.0 | [github.com/microsoft/windows-rs](https://github.com/microsoft/windows-rs) |  |
| `windows-sys` | 0.59.0 | [github.com/microsoft/windows-rs](https://github.com/microsoft/windows-rs) |  |
| `windows-sys` | 0.61.2 | [github.com/microsoft/windows-rs](https://github.com/microsoft/windows-rs) |  |
| `windows-targets` | 0.52.6 | [github.com/microsoft/windows-rs](https://github.com/microsoft/windows-rs) |  |
| `windows-targets` | 0.53.5 | [github.com/microsoft/windows-rs](https://github.com/microsoft/windows-rs) |  |
| `windows-threading` | 0.1.0 | [github.com/microsoft/windows-rs](https://github.com/microsoft/windows-rs) |  |
| `windows-threading` | 0.2.1 | [github.com/microsoft/windows-rs](https://github.com/microsoft/windows-rs) |  |
| `windows-version` | 0.1.7 | [github.com/microsoft/windows-rs](https://github.com/microsoft/windows-rs) |  |
| `windows_aarch64_gnullvm` | 0.52.6 | [github.com/microsoft/windows-rs](https://github.com/microsoft/windows-rs) |  |
| `windows_aarch64_gnullvm` | 0.53.1 | [github.com/microsoft/windows-rs](https://github.com/microsoft/windows-rs) |  |
| `windows_aarch64_msvc` | 0.52.6 | [github.com/microsoft/windows-rs](https://github.com/microsoft/windows-rs) |  |
| `windows_aarch64_msvc` | 0.53.1 | [github.com/microsoft/windows-rs](https://github.com/microsoft/windows-rs) |  |
| `windows_i686_gnu` | 0.52.6 | [github.com/microsoft/windows-rs](https://github.com/microsoft/windows-rs) |  |
| `windows_i686_gnu` | 0.53.1 | [github.com/microsoft/windows-rs](https://github.com/microsoft/windows-rs) |  |
| `windows_i686_gnullvm` | 0.52.6 | [github.com/microsoft/windows-rs](https://github.com/microsoft/windows-rs) |  |
| `windows_i686_gnullvm` | 0.53.1 | [github.com/microsoft/windows-rs](https://github.com/microsoft/windows-rs) |  |
| `windows_i686_msvc` | 0.52.6 | [github.com/microsoft/windows-rs](https://github.com/microsoft/windows-rs) |  |
| `windows_i686_msvc` | 0.53.1 | [github.com/microsoft/windows-rs](https://github.com/microsoft/windows-rs) |  |
| `windows_x86_64_gnu` | 0.52.6 | [github.com/microsoft/windows-rs](https://github.com/microsoft/windows-rs) |  |
| `windows_x86_64_gnu` | 0.53.1 | [github.com/microsoft/windows-rs](https://github.com/microsoft/windows-rs) |  |
| `windows_x86_64_gnullvm` | 0.52.6 | [github.com/microsoft/windows-rs](https://github.com/microsoft/windows-rs) |  |
| `windows_x86_64_gnullvm` | 0.53.1 | [github.com/microsoft/windows-rs](https://github.com/microsoft/windows-rs) |  |
| `windows_x86_64_msvc` | 0.52.6 | [github.com/microsoft/windows-rs](https://github.com/microsoft/windows-rs) |  |
| `windows_x86_64_msvc` | 0.53.1 | [github.com/microsoft/windows-rs](https://github.com/microsoft/windows-rs) |  |
| `wio` | 0.2.2 | [github.com/retep998/wio-rs](https://github.com/retep998/wio-rs) |  |
| `x11rb` | 0.13.2 | [github.com/psychon/x11rb](https://github.com/psychon/x11rb) |  |
| `x11rb-protocol` | 0.13.2 | [github.com/psychon/x11rb](https://github.com/psychon/x11rb) |  |
| `x509-parser` | 0.18.1 | [github.com/rusticata/x509-parser.git](https://github.com/rusticata/x509-parser.git) |  |
| `xml5ever` | 0.18.1 | [github.com/servo/html5ever](https://github.com/servo/html5ever) |  |
| `yazi` | 0.2.1 | [github.com/dfrg/yazi](https://github.com/dfrg/yazi) |  |
| `zed-font-kit` | 0.14.1-zed | [github.com/servo/font-kit](https://github.com/servo/font-kit) |  |
| `zed-reqwest` | 0.12.15-zed | [github.com/seanmonstar/reqwest](https://github.com/seanmonstar/reqwest) |  |
| `zeno` | 0.3.3 | [github.com/dfrg/zeno](https://github.com/dfrg/zeno) |  |
| `zeroize` | 1.9.0 | [github.com/RustCrypto/utils](https://github.com/RustCrypto/utils) |  |
| `zeroize_derive` | 1.5.0 | [github.com/RustCrypto/utils](https://github.com/RustCrypto/utils) |  |
| `zstd-safe` | 7.2.4 | [github.com/gyscos/zstd-rs](https://github.com/gyscos/zstd-rs) |  |
| `zstd-sys` | 2.0.16+zstd.1.5.7 | [github.com/gyscos/zstd-rs](https://github.com/gyscos/zstd-rs) |  |

### MIT

_200 个依赖_

| 依赖 | 版本 | 来源 | 备注 |
| --- | --- | --- | --- |
| `aligned-vec` | 0.6.4 | [github.com/sarah-ek/aligned-vec/](https://github.com/sarah-ek/aligned-vec/) |  |
| `arg_enum_proc_macro` | 0.3.4 | [github.com/lu-zero/arg_enum_proc_macro](https://github.com/lu-zero/arg_enum_proc_macro) |  |
| `ashpd` | 0.13.13 | [github.com/bilelmoussaoui/ashpd](https://github.com/bilelmoussaoui/ashpd) |  |
| `av-scenechange` | 0.14.1 | [github.com/rust-av/av-scenechange](https://github.com/rust-av/av-scenechange) |  |
| `base62` | 2.2.4 | [github.com/fbernier/base62](https://github.com/fbernier/base62) |  |
| `bincode` | 1.3.3 | [github.com/servo/bincode](https://github.com/servo/bincode) |  |
| `block` | 0.1.6 | [http://github.com/SSheldon/rust-block](http://github.com/SSheldon/rust-block) |  |
| `block2` | 0.5.1 | [github.com/madsmtm/objc2](https://github.com/madsmtm/objc2) |  |
| `block2` | 0.6.2 | [github.com/madsmtm/objc2](https://github.com/madsmtm/objc2) |  |
| `built` | 0.8.1 | [github.com/lukaslueg/built](https://github.com/lukaslueg/built) |  |
| `bytes` | 1.12.1 | [github.com/tokio-rs/bytes](https://github.com/tokio-rs/bytes) |  |
| `calloop` | 0.14.4 | [github.com/Smithay/calloop](https://github.com/Smithay/calloop) |  |
| `calloop-wayland-source` | 0.4.1 | [github.com/smithay/calloop-wayland-source](https://github.com/smithay/calloop-wayland-source) |  |
| `cfg_aliases` | 0.2.2 | [github.com/katharostech/cfg_aliases](https://github.com/katharostech/cfg_aliases) |  |
| `color_quant` | 1.1.0 | [github.com/image-rs/color_quant.git](https://github.com/image-rs/color_quant.git) |  |
| `combine` | 4.6.7 | [github.com/Marwes/combine](https://github.com/Marwes/combine) |  |
| `convert_case` | 0.10.0 | [github.com/rutrum/convert-case](https://github.com/rutrum/convert-case) |  |
| `convert_case` | 0.11.0 | [github.com/rutrum/convert-case](https://github.com/rutrum/convert-case) |  |
| `core_maths` | 0.1.1 | [github.com/robertbastian/core_maths](https://github.com/robertbastian/core_maths) |  |
| `crunchy` | 0.2.4 | [github.com/eira-fransham/crunchy](https://github.com/eira-fransham/crunchy) |  |
| `data-encoding` | 2.11.1 | [github.com/ia0/data-encoding](https://github.com/ia0/data-encoding) |  |
| `derive_more` | 2.1.1 | [github.com/JelteF/derive_more](https://github.com/JelteF/derive_more) |  |
| `derive_more-impl` | 2.1.1 | [github.com/JelteF/derive_more](https://github.com/JelteF/derive_more) |  |
| `dispatch` | 0.2.0 | [http://github.com/SSheldon/rust-dispatch](http://github.com/SSheldon/rust-dispatch) |  |
| `dlib` | 0.5.3 | [github.com/elinorbgr/dlib](https://github.com/elinorbgr/dlib) |  |
| `embed-resource` | 3.0.11 | [github.com/nabijaczleweli/rust-embed-resource](https://github.com/nabijaczleweli/rust-embed-resource) |  |
| `endi` | 1.1.1 | [github.com/zeenix/endi](https://github.com/zeenix/endi) |  |
| `equator` | 0.4.2 | [github.com/sarah-ek/equator/](https://github.com/sarah-ek/equator/) |  |
| `equator-macro` | 0.4.2 | [github.com/sarah-ek/equator/](https://github.com/sarah-ek/equator/) |  |
| `fancy-regex` | 0.16.2 | [github.com/fancy-regex/fancy-regex](https://github.com/fancy-regex/fancy-regex) |  |
| `fax` | 0.2.7 | [github.com/pdf-rs/fax](https://github.com/pdf-rs/fax) |  |
| `filedescriptor` | 0.8.3 | [github.com/wezterm/wezterm](https://github.com/wezterm/wezterm) |  |
| `float-cmp` | 0.9.0 | [github.com/mikedilger/float-cmp](https://github.com/mikedilger/float-cmp) |  |
| `float_next_after` | 1.0.0 | [gitlab.com/bronsonbdevost/next_afterf](https://gitlab.com/bronsonbdevost/next_afterf) |  |
| `fluent-uri` | 0.1.4 | [github.com/yescallop/fluent-uri-rs](https://github.com/yescallop/fluent-uri-rs) |  |
| `fontconfig-parser` | 0.5.8 | [github.com/Riey/fontconfig-parser](https://github.com/Riey/fontconfig-parser) |  |
| `fontdb` | 0.23.0 | [github.com/RazrFalcon/fontdb](https://github.com/RazrFalcon/fontdb) |  |
| `freetype-sys` | 0.20.1 | [github.com/PistonDevelopers/freetype-sys.git](https://github.com/PistonDevelopers/freetype-sys.git) |  |
| `fs_extra` | 1.3.0 | [github.com/webdesus/fs_extra](https://github.com/webdesus/fs_extra) |  |
| `fsevent-sys` | 4.1.0 | [github.com/octplane/fsevent-rust/tree/master/fsevent-sys](https://github.com/octplane/fsevent-rust/tree/master/fsevent-sys) |  |
| `generic-array` | 0.14.7 | [github.com/fizyk20/generic-array.git](https://github.com/fizyk20/generic-array.git) |  |
| `globwalk` | 0.8.1 | [github.com/gilnaa/globwalk](https://github.com/gilnaa/globwalk) |  |
| `h2` | 0.4.17 | [github.com/hyperium/h2](https://github.com/hyperium/h2) |  |
| `harfrust` | 0.5.2 | [github.com/harfbuzz/harfrust](https://github.com/harfbuzz/harfrust) |  |
| `http-body` | 1.1.0 | [github.com/hyperium/http-body](https://github.com/hyperium/http-body) |  |
| `http-body-util` | 0.1.5 | [github.com/hyperium/http-body](https://github.com/hyperium/http-body) |  |
| `hyper` | 1.11.0 | [github.com/hyperium/hyper](https://github.com/hyperium/hyper) |  |
| `hyper-util` | 0.1.20 | [github.com/hyperium/hyper-util](https://github.com/hyperium/hyper-util) |  |
| `imagesize` | 0.13.0 | [github.com/Roughsketch/imagesize](https://github.com/Roughsketch/imagesize) |  |
| `imagesize` | 0.14.0 | [github.com/Roughsketch/imagesize](https://github.com/Roughsketch/imagesize) |  |
| `interpolate_name` | 0.2.4 | [github.com/lu-zero/interpolate_name](https://github.com/lu-zero/interpolate_name) |  |
| `is-docker` | 0.2.0 | [github.com/TheLarkInn/is-docker](https://github.com/TheLarkInn/is-docker) |  |
| `is-wsl` | 0.4.0 | [github.com/TheLarkInn/is-wsl](https://github.com/TheLarkInn/is-wsl) |  |
| `kqueue` | 1.2.1 | [gitlab.com/rust-kqueue/rust-kqueue](https://gitlab.com/rust-kqueue/rust-kqueue) |  |
| `kqueue-sys` | 1.1.2 | [gitlab.com/rust-kqueue/rust-kqueue-sys](https://gitlab.com/rust-kqueue/rust-kqueue-sys) |  |
| `libm` | 0.2.16 | [github.com/rust-lang/compiler-builtins](https://github.com/rust-lang/compiler-builtins) |  |
| `libredox` | 0.1.20 | [gitlab.redox-os.org/redox-os/libredox.git](https://gitlab.redox-os.org/redox-os/libredox.git) |  |
| `loop9` | 0.1.5 | [gitlab.com/kornelski/loop9.git](https://gitlab.com/kornelski/loop9.git) |  |
| `lsp-types` | 0.97.0 | [github.com/gluon-lang/lsp-types](https://github.com/gluon-lang/lsp-types) |  |
| `malloc_buf` | 0.0.6 | [github.com/SSheldon/malloc_buf](https://github.com/SSheldon/malloc_buf) |  |
| `markdown` | 1.0.0 | [github.com/wooorm/markdown-rs](https://github.com/wooorm/markdown-rs) |  |
| `matchers` | 0.2.0 | [github.com/hawkw/matchers](https://github.com/hawkw/matchers) |  |
| `maybe-rayon` | 0.1.1 | [github.com/shssoichiro/maybe-rayon](https://github.com/shssoichiro/maybe-rayon) |  |
| `memoffset` | 0.9.1 | [github.com/Gilnaa/memoffset](https://github.com/Gilnaa/memoffset) |  |
| `mime_guess` | 2.0.5 | [github.com/abonander/mime_guess](https://github.com/abonander/mime_guess) |  |
| `minisign-verify` | 0.2.5 | [github.com/jedisct1/rust-minisign-verify](https://github.com/jedisct1/rust-minisign-verify) |  |
| `mio` | 1.2.2 | [github.com/tokio-rs/mio](https://github.com/tokio-rs/mio) |  |
| `new_debug_unreachable` | 1.0.6 | [github.com/mbrubeck/rust-debug-unreachable](https://github.com/mbrubeck/rust-debug-unreachable) |  |
| `nom` | 7.1.3 | [github.com/Geal/nom](https://github.com/Geal/nom) |  |
| `nom` | 8.0.0 | [github.com/rust-bakery/nom](https://github.com/rust-bakery/nom) |  |
| `noop_proc_macro` | 0.3.0 | [github.com/lu-zero/noop_proc_macro](https://github.com/lu-zero/noop_proc_macro) |  |
| `nu-ansi-term` | 0.50.3 | [github.com/nushell/nu-ansi-term](https://github.com/nushell/nu-ansi-term) |  |
| `objc` | 0.2.7 | [http://github.com/SSheldon/rust-objc](http://github.com/SSheldon/rust-objc) |  |
| `objc-foundation` | 0.1.1 | [http://github.com/SSheldon/rust-objc-foundation](http://github.com/SSheldon/rust-objc-foundation) |  |
| `objc-sys` | 0.3.5 | [github.com/madsmtm/objc2](https://github.com/madsmtm/objc2) |  |
| `objc2` | 0.5.2 | [github.com/madsmtm/objc2](https://github.com/madsmtm/objc2) |  |
| `objc2` | 0.6.4 | [github.com/madsmtm/objc2](https://github.com/madsmtm/objc2) |  |
| `objc2-app-kit` | 0.2.2 | [github.com/madsmtm/objc2](https://github.com/madsmtm/objc2) |  |
| `objc2-core-data` | 0.2.2 | [github.com/madsmtm/objc2](https://github.com/madsmtm/objc2) |  |
| `objc2-core-image` | 0.2.2 | [github.com/madsmtm/objc2](https://github.com/madsmtm/objc2) |  |
| `objc2-encode` | 4.1.0 | [github.com/madsmtm/objc2](https://github.com/madsmtm/objc2) |  |
| `objc2-foundation` | 0.2.2 | [github.com/madsmtm/objc2](https://github.com/madsmtm/objc2) |  |
| `objc2-foundation` | 0.3.2 | [github.com/madsmtm/objc2](https://github.com/madsmtm/objc2) |  |
| `objc2-metal` | 0.2.2 | [github.com/madsmtm/objc2](https://github.com/madsmtm/objc2) |  |
| `objc2-quartz-core` | 0.2.2 | [github.com/madsmtm/objc2](https://github.com/madsmtm/objc2) |  |
| `objc_exception` | 0.1.2 | [http://github.com/SSheldon/rust-objc-exception](http://github.com/SSheldon/rust-objc-exception) |  |
| `objc_id` | 0.1.1 | [http://github.com/SSheldon/rust-objc-id](http://github.com/SSheldon/rust-objc-id) |  |
| `oo7` | 0.6.0 | [github.com/linux-credentials/oo7](https://github.com/linux-credentials/oo7) |  |
| `open` | 5.4.1 | [github.com/Byron/open-rs](https://github.com/Byron/open-rs) |  |
| `ordered-float` | 5.3.0 | [github.com/reem/rust-ordered-float](https://github.com/reem/rust-ordered-float) |  |
| `phf` | 0.11.3 | [github.com/rust-phf/rust-phf](https://github.com/rust-phf/rust-phf) |  |
| `phf` | 0.13.1 | [github.com/rust-phf/rust-phf](https://github.com/rust-phf/rust-phf) |  |
| `phf_codegen` | 0.11.3 | [github.com/rust-phf/rust-phf](https://github.com/rust-phf/rust-phf) |  |
| `phf_generator` | 0.11.3 | [github.com/rust-phf/rust-phf](https://github.com/rust-phf/rust-phf) |  |
| `phf_generator` | 0.13.1 | [github.com/rust-phf/rust-phf](https://github.com/rust-phf/rust-phf) |  |
| `phf_macros` | 0.13.1 | [github.com/rust-phf/rust-phf](https://github.com/rust-phf/rust-phf) |  |
| `phf_shared` | 0.11.3 | [github.com/rust-phf/rust-phf](https://github.com/rust-phf/rust-phf) |  |
| `phf_shared` | 0.13.1 | [github.com/rust-phf/rust-phf](https://github.com/rust-phf/rust-phf) |  |
| `pico-args` | 0.5.0 | [github.com/RazrFalcon/pico-args](https://github.com/RazrFalcon/pico-args) |  |
| `postage` | 0.5.0 | [github.com/austinjones/postage-rs](https://github.com/austinjones/postage-rs) |  |
| `precomputed-hash` | 0.1.1 | [github.com/emilio/precomputed-hash](https://github.com/emilio/precomputed-hash) |  |
| `pulp` | 0.22.3 | [github.com/sarah-quinones/pulp/](https://github.com/sarah-quinones/pulp/) |  |
| `pulp-wasm-simd-flag` | 0.1.1 | [github.com/sarah-quinones/pulp/](https://github.com/sarah-quinones/pulp/) |  |
| `quick-xml` | 0.41.0 | [github.com/tafia/quick-xml](https://github.com/tafia/quick-xml) |  |
| `raw-cpuid` | 11.6.0 | [github.com/gz/rust-cpuid](https://github.com/gz/rust-cpuid) |  |
| `reborrow` | 0.5.5 | [github.com/sarah-ek/reborrow/](https://github.com/sarah-ek/reborrow/) |  |
| `redox_syscall` | 0.5.18 | [gitlab.redox-os.org/redox-os/syscall](https://gitlab.redox-os.org/redox-os/syscall) |  |
| `redox_users` | 0.5.2 | [gitlab.redox-os.org/redox-os/users](https://gitlab.redox-os.org/redox-os/users) |  |
| `rgb` | 0.8.53 | [github.com/kornelski/rust-rgb](https://github.com/kornelski/rust-rgb) |  |
| `rust-embed` | 8.12.0 | [pyrossh.dev/repos/rust-embed](https://pyrossh.dev/repos/rust-embed) |  |
| `rust-embed-impl` | 8.12.0 | [pyrossh.dev/repos/rust-embed](https://pyrossh.dev/repos/rust-embed) |  |
| `rust-embed-utils` | 8.12.0 | [pyrossh.dev/repos/rust-embed](https://pyrossh.dev/repos/rust-embed) |  |
| `rust-i18n` | 4.2.1 | [github.com/longbridge/rust-i18n](https://github.com/longbridge/rust-i18n) |  |
| `rust-i18n-macro` | 4.2.1 | [github.com/longbridge/rust-i18n](https://github.com/longbridge/rust-i18n) |  |
| `rust-i18n-support` | 4.2.1 | [github.com/longbridge/rust-i18n](https://github.com/longbridge/rust-i18n) |  |
| `rustybuzz` | 0.20.1 | [github.com/harfbuzz/rustybuzz](https://github.com/harfbuzz/rustybuzz) |  |
| `schannel` | 0.1.29 | [github.com/steffengy/schannel-rs](https://github.com/steffengy/schannel-rs) |  |
| `schemars` | 1.2.2 | [github.com/GREsau/schemars](https://github.com/GREsau/schemars) |  |
| `schemars_derive` | 1.2.2 | [github.com/GREsau/schemars](https://github.com/GREsau/schemars) |  |
| `seahash` | 4.1.0 | [gitlab.redox-os.org/redox-os/seahash](https://gitlab.redox-os.org/redox-os/seahash) |  |
| `sharded-slab` | 0.1.7 | [github.com/hawkw/sharded-slab](https://github.com/hawkw/sharded-slab) |  |
| `simd-adler32` | 0.3.10 | [github.com/mcountryman/simd-adler32](https://github.com/mcountryman/simd-adler32) |  |
| `simd_helpers` | 0.1.0 | [github.com/lu-zero/simd_helpers](https://github.com/lu-zero/simd_helpers) |  |
| `slab` | 0.4.12 | [github.com/tokio-rs/slab](https://github.com/tokio-rs/slab) |  |
| `spin` | 0.9.9 | [github.com/mvdnes/spin-rs.git](https://github.com/mvdnes/spin-rs.git) |  |
| `spin` | 0.10.1 | [github.com/mvdnes/spin-rs.git](https://github.com/mvdnes/spin-rs.git) |  |
| `strict-num` | 0.1.1 | [github.com/RazrFalcon/strict-num](https://github.com/RazrFalcon/strict-num) |  |
| `strum` | 0.27.2 | [github.com/Peternator7/strum](https://github.com/Peternator7/strum) |  |
| `strum_macros` | 0.27.2 | [github.com/Peternator7/strum](https://github.com/Peternator7/strum) |  |
| `synstructure` | 0.13.2 | [github.com/mystor/synstructure](https://github.com/mystor/synstructure) |  |
| `syntect` | 5.3.0 | [github.com/trishume/syntect](https://github.com/trishume/syntect) |  |
| `sysinfo` | 0.31.4 | [github.com/GuillaumeGomez/sysinfo](https://github.com/GuillaumeGomez/sysinfo) |  |
| `taffy` | 0.13.0 | [github.com/DioxusLabs/taffy](https://github.com/DioxusLabs/taffy) |  |
| `tao-core-video-sys` | 0.2.0 | — |  |
| `tiff` | 0.11.3 | [github.com/image-rs/image-tiff](https://github.com/image-rs/image-tiff) |  |
| `tokio` | 1.53.1 | [github.com/tokio-rs/tokio](https://github.com/tokio-rs/tokio) |  |
| `tokio-macros` | 2.7.2 | [github.com/tokio-rs/tokio](https://github.com/tokio-rs/tokio) |  |
| `tokio-socks` | 0.5.3 | [github.com/sticnarf/tokio-socks](https://github.com/sticnarf/tokio-socks) |  |
| `tokio-util` | 0.7.19 | [github.com/tokio-rs/tokio](https://github.com/tokio-rs/tokio) |  |
| `tower` | 0.5.3 | [github.com/tower-rs/tower](https://github.com/tower-rs/tower) |  |
| `tower-http` | 0.6.11 | [github.com/tower-rs/tower-http](https://github.com/tower-rs/tower-http) |  |
| `tower-layer` | 0.3.3 | [github.com/tower-rs/tower](https://github.com/tower-rs/tower) |  |
| `tower-service` | 0.3.3 | [github.com/tower-rs/tower](https://github.com/tower-rs/tower) |  |
| `tracing` | 0.1.44 | [github.com/tokio-rs/tracing](https://github.com/tokio-rs/tracing) |  |
| `tracing-attributes` | 0.1.31 | [github.com/tokio-rs/tracing](https://github.com/tokio-rs/tracing) |  |
| `tracing-core` | 0.1.36 | [github.com/tokio-rs/tracing](https://github.com/tokio-rs/tracing) |  |
| `tracing-log` | 0.2.0 | [github.com/tokio-rs/tracing](https://github.com/tokio-rs/tracing) |  |
| `tracing-subscriber` | 0.3.23 | [github.com/tokio-rs/tracing](https://github.com/tokio-rs/tracing) |  |
| `tree-sitter` | 0.26.12 | [github.com/tree-sitter/tree-sitter](https://github.com/tree-sitter/tree-sitter) |  |
| `tree-sitter-bash` | 0.23.3 | [github.com/tree-sitter/tree-sitter-bash](https://github.com/tree-sitter/tree-sitter-bash) |  |
| `tree-sitter-html` | 0.23.2 | [github.com/tree-sitter/tree-sitter-html](https://github.com/tree-sitter/tree-sitter-html) |  |
| `tree-sitter-json` | 0.24.8 | [github.com/tree-sitter/tree-sitter-json](https://github.com/tree-sitter/tree-sitter-json) |  |
| `tree-sitter-language` | 0.1.7 | [github.com/tree-sitter/tree-sitter](https://github.com/tree-sitter/tree-sitter) |  |
| `tree-sitter-python` | 0.23.6 | [github.com/tree-sitter/tree-sitter-python](https://github.com/tree-sitter/tree-sitter-python) |  |
| `try-lock` | 0.2.5 | [github.com/seanmonstar/try-lock](https://github.com/seanmonstar/try-lock) |  |
| `uds_windows` | 1.2.1 | [github.com/haraldh/rust_uds_windows](https://github.com/haraldh/rust_uds_windows) |  |
| `ulid` | 3.0.0 | [github.com/dylanhart/ulid-rs](https://github.com/dylanhart/ulid-rs) |  |
| `valuable` | 0.1.1 | [github.com/tokio-rs/valuable](https://github.com/tokio-rs/valuable) |  |
| `vswhom` | 0.1.0 | [github.com/nabijaczleweli/vswhom.rs](https://github.com/nabijaczleweli/vswhom.rs) |  |
| `vswhom-sys` | 0.1.3 | [github.com/nabijaczleweli/vswhom-sys.rs](https://github.com/nabijaczleweli/vswhom-sys.rs) |  |
| `want` | 0.3.1 | [github.com/seanmonstar/want](https://github.com/seanmonstar/want) |  |
| `wayland-backend` | 0.3.17 | [github.com/smithay/wayland-rs](https://github.com/smithay/wayland-rs) |  |
| `wayland-client` | 0.31.15 | [github.com/smithay/wayland-rs](https://github.com/smithay/wayland-rs) |  |
| `wayland-cursor` | 0.31.14 | [github.com/smithay/wayland-rs](https://github.com/smithay/wayland-rs) |  |
| `wayland-protocols` | 0.32.13 | [github.com/smithay/wayland-rs](https://github.com/smithay/wayland-rs) |  |
| `wayland-protocols-plasma` | 0.3.12 | [github.com/smithay/wayland-rs](https://github.com/smithay/wayland-rs) |  |
| `wayland-protocols-wlr` | 0.3.12 | [github.com/smithay/wayland-rs](https://github.com/smithay/wayland-rs) |  |
| `wayland-scanner` | 0.31.11 | [github.com/smithay/wayland-rs](https://github.com/smithay/wayland-rs) |  |
| `wayland-sys` | 0.31.11 | [github.com/smithay/wayland-rs](https://github.com/smithay/wayland-rs) |  |
| `which` | 6.0.3 | [github.com/harryfei/which-rs.git](https://github.com/harryfei/which-rs.git) |  |
| `windows-capture` | 1.5.0 | [github.com/NiiightmareXD/windows-capture](https://github.com/NiiightmareXD/windows-capture) |  |
| `winnow` | 0.7.15 | [github.com/winnow-rs/winnow](https://github.com/winnow-rs/winnow) |  |
| `winnow` | 1.0.4 | [github.com/winnow-rs/winnow](https://github.com/winnow-rs/winnow) |  |
| `winreg` | 0.55.0 | [github.com/gentoo90/winreg-rs](https://github.com/gentoo90/winreg-rs) |  |
| `winsafe` | 0.0.19 | [github.com/rodrigocfd/winsafe](https://github.com/rodrigocfd/winsafe) |  |
| `x11` | 2.21.0 | [github.com/AltF02/x11-rs.git](https://github.com/AltF02/x11-rs.git) |  |
| `x11-clipboard` | 0.9.3 | [github.com/quininer/x11-clipboard](https://github.com/quininer/x11-clipboard) |  |
| `xcb` | 1.7.1 | [github.com/rust-x-bindings/rust-xcb](https://github.com/rust-x-bindings/rust-xcb) |  |
| `xcursor` | 0.3.11 | [github.com/esposm03/xcursor-rs](https://github.com/esposm03/xcursor-rs) |  |
| `xim-ctext` | 0.3.0 | [github.com/Riey/xim-rs](https://github.com/Riey/xim-rs) |  |
| `xim-parser` | 0.2.1 | [github.com/Riey/xim-rs](https://github.com/Riey/xim-rs) |  |
| `xkbcommon` | 0.8.0 | [github.com/rust-x-bindings/xkbcommon-rs](https://github.com/rust-x-bindings/xkbcommon-rs) |  |
| `xml-rs` | 0.8.29 | [github.com/kornelski/xml-rs](https://github.com/kornelski/xml-rs) |  |
| `xmlwriter` | 0.1.0 | [github.com/RazrFalcon/xmlwriter](https://github.com/RazrFalcon/xmlwriter) |  |
| `y4m` | 0.8.0 | [github.com/image-rs/y4m.git](https://github.com/image-rs/y4m.git) |  |
| `yeslogic-fontconfig-sys` | 6.0.1 | [github.com/yeslogic/fontconfig-rs](https://github.com/yeslogic/fontconfig-rs) |  |
| `zbus` | 5.19.0 | [github.com/z-galaxy/zbus/](https://github.com/z-galaxy/zbus/) |  |
| `zbus-lockstep` | 0.5.2 | [github.com/luukvanderduim/zbus-lockstep](https://github.com/luukvanderduim/zbus-lockstep) |  |
| `zbus-lockstep-macros` | 0.5.2 | [github.com/luukvanderduim/zbus-lockstep](https://github.com/luukvanderduim/zbus-lockstep) |  |
| `zbus_macros` | 5.19.0 | [github.com/z-galaxy/zbus/](https://github.com/z-galaxy/zbus/) |  |
| `zbus_names` | 4.3.4 | [github.com/z-galaxy/zbus/](https://github.com/z-galaxy/zbus/) |  |
| `zbus_xml` | 5.2.1 | [github.com/z-galaxy/zbus/](https://github.com/z-galaxy/zbus/) |  |
| `zcheapstr` | 1.1.0 | [github.com/z-galaxy/zcheapstr/](https://github.com/z-galaxy/zcheapstr/) |  |
| `zed-scap` | 0.0.8-zed | [github.com/helmerapp/scap](https://github.com/helmerapp/scap) |  |
| `zed-xim` | 0.4.0-zed | [github.com/Riey/xim-rs](https://github.com/Riey/xim-rs) |  |
| `zmij` | 1.0.23 | [github.com/dtolnay/zmij](https://github.com/dtolnay/zmij) |  |
| `zstd` | 0.13.3 | [github.com/gyscos/zstd-rs](https://github.com/gyscos/zstd-rs) |  |
| `zvariant` | 5.15.0 | [github.com/z-galaxy/zbus/](https://github.com/z-galaxy/zbus/) |  |
| `zvariant_derive` | 5.15.0 | [github.com/z-galaxy/zbus/](https://github.com/z-galaxy/zbus/) |  |
| `zvariant_utils` | 4.2.0 | [github.com/z-galaxy/zbus/](https://github.com/z-galaxy/zbus/) |  |

### Apache-2.0

_35 个依赖_

| 依赖 | 版本 | 来源 | 备注 |
| --- | --- | --- | --- |
| `clang-sys` | 1.9.1 | [github.com/KyleMayes/clang-sys](https://github.com/KyleMayes/clang-sys) |  |
| `codespan-reporting` | 0.13.1 | [github.com/brendanzab/codespan](https://github.com/brendanzab/codespan) |  |
| `collections` | 0.1.0 | [github.com/zed-industries/zed](https://github.com/zed-industries/zed) |  |
| `derive_refineable` | 0.1.0 | [github.com/zed-industries/zed](https://github.com/zed-industries/zed) |  |
| `gethostname` | 1.1.0 | [codeberg.org/swsnr/gethostname.rs.git](https://codeberg.org/swsnr/gethostname.rs.git) |  |
| `gl_generator` | 0.14.0 | [github.com/brendanzab/gl-rs/](https://github.com/brendanzab/gl-rs/) |  |
| `glutin_wgl_sys` | 0.6.1 | [github.com/rust-windowing/glutin](https://github.com/rust-windowing/glutin) |  |
| `gpui` | 0.2.2 | [github.com/zed-industries/zed](https://github.com/zed-industries/zed) |  |
| `gpui-base` | 0.5.2 | [github.com/longbridge/gpui-component](https://github.com/longbridge/gpui-component) |  |
| `gpui-component` | 0.5.2 | [github.com/longbridge/gpui-component](https://github.com/longbridge/gpui-component) |  |
| `gpui-component-assets` | 0.5.1 | [github.com/longbridge/gpui-component](https://github.com/longbridge/gpui-component) |  |
| `gpui-component-macros` | 0.5.1 | [github.com/longbridge/gpui-component](https://github.com/longbridge/gpui-component) |  |
| `gpui_apple` | 0.1.0 | [github.com/zed-industries/zed](https://github.com/zed-industries/zed) |  |
| `gpui_linux` | 0.1.0 | [github.com/zed-industries/zed](https://github.com/zed-industries/zed) |  |
| `gpui_macos` | 0.1.0 | [github.com/zed-industries/zed](https://github.com/zed-industries/zed) |  |
| `gpui_macros` | 0.1.0 | [github.com/zed-industries/zed](https://github.com/zed-industries/zed) |  |
| `gpui_platform` | 0.1.0 | [github.com/zed-industries/zed](https://github.com/zed-industries/zed) |  |
| `gpui_tokio` | 0.1.0 | [github.com/zed-industries/zed](https://github.com/zed-industries/zed) |  |
| `gpui_web` | 0.1.0 | [github.com/zed-industries/zed](https://github.com/zed-industries/zed) |  |
| `gpui_wgpu` | 0.1.0 | [github.com/zed-industries/zed](https://github.com/zed-industries/zed) |  |
| `gpui_windows` | 0.1.0 | [github.com/zed-industries/zed](https://github.com/zed-industries/zed) |  |
| `http_client` | 0.1.0 | [github.com/zed-industries/zed](https://github.com/zed-industries/zed) |  |
| `khronos_api` | 3.1.0 | [github.com/brendanzab/gl-rs/](https://github.com/brendanzab/gl-rs/) |  |
| `media` | 0.1.0 | [github.com/zed-industries/zed](https://github.com/zed-industries/zed) |  |
| `perf` | 0.1.0 | [github.com/zed-industries/zed](https://github.com/zed-industries/zed) |  |
| `refineable` | 0.1.0 | [github.com/zed-industries/zed](https://github.com/zed-industries/zed) |  |
| `scheduler` | 0.1.0 | [github.com/zed-industries/zed](https://github.com/zed-industries/zed) |  |
| `spirv` | 0.4.0+sdk-1.4.341.0 | [github.com/gfx-rs/rspirv](https://github.com/gfx-rs/rspirv) |  |
| `stacksafe` | 1.0.3 | [github.com/fast/stacksafe](https://github.com/fast/stacksafe) |  |
| `stacksafe-macro` | 1.0.3 | [github.com/fast/stacksafe](https://github.com/fast/stacksafe) |  |
| `sum_tree` | 0.1.0 | [github.com/zed-industries/zed](https://github.com/zed-industries/zed) |  |
| `sync_wrapper` | 1.0.2 | [github.com/Actyx/sync_wrapper](https://github.com/Actyx/sync_wrapper) |  |
| `unicode-linebreak` | 0.1.5 | [github.com/axelf4/unicode-linebreak](https://github.com/axelf4/unicode-linebreak) |  |
| `util_macros` | 0.1.0 | [github.com/zed-industries/zed](https://github.com/zed-industries/zed) |  |
| `zed-sum-tree` | 0.2.0 | — |  |

### Apache-2.0 OR MIT OR Zlib

_28 个依赖_

| 依赖 | 版本 | 来源 | 备注 |
| --- | --- | --- | --- |
| `bytemuck` | 1.25.2 | [github.com/Lokathor/bytemuck](https://github.com/Lokathor/bytemuck) |  |
| `bytemuck_derive` | 1.12.0 | [github.com/Lokathor/bytemuck](https://github.com/Lokathor/bytemuck) |  |
| `dispatch2` | 0.3.1 | [github.com/madsmtm/objc2](https://github.com/madsmtm/objc2) |  |
| `glow` | 0.17.0 | [github.com/grovesNL/glow](https://github.com/grovesNL/glow) |  |
| `lru-slab` | 0.1.2 | [github.com/Ralith/lru-slab](https://github.com/Ralith/lru-slab) |  |
| `miniz_oxide` | 0.8.9 | [github.com/Frommi/miniz_oxide/tree/master/miniz_oxide](https://github.com/Frommi/miniz_oxide/tree/master/miniz_oxide) |  |
| `objc2-app-kit` | 0.3.2 | [github.com/madsmtm/objc2](https://github.com/madsmtm/objc2) |  |
| `objc2-cloud-kit` | 0.3.2 | [github.com/madsmtm/objc2](https://github.com/madsmtm/objc2) |  |
| `objc2-core-data` | 0.3.2 | [github.com/madsmtm/objc2](https://github.com/madsmtm/objc2) |  |
| `objc2-core-foundation` | 0.3.2 | [github.com/madsmtm/objc2](https://github.com/madsmtm/objc2) |  |
| `objc2-core-graphics` | 0.3.2 | [github.com/madsmtm/objc2](https://github.com/madsmtm/objc2) |  |
| `objc2-core-image` | 0.3.2 | [github.com/madsmtm/objc2](https://github.com/madsmtm/objc2) |  |
| `objc2-core-location` | 0.3.2 | [github.com/madsmtm/objc2](https://github.com/madsmtm/objc2) |  |
| `objc2-core-text` | 0.3.2 | [github.com/madsmtm/objc2](https://github.com/madsmtm/objc2) |  |
| `objc2-core-video` | 0.3.2 | [github.com/madsmtm/objc2](https://github.com/madsmtm/objc2) |  |
| `objc2-io-surface` | 0.3.2 | [github.com/madsmtm/objc2](https://github.com/madsmtm/objc2) |  |
| `objc2-metal` | 0.3.2 | [github.com/madsmtm/objc2](https://github.com/madsmtm/objc2) |  |
| `objc2-quartz-core` | 0.3.2 | [github.com/madsmtm/objc2](https://github.com/madsmtm/objc2) |  |
| `objc2-user-notifications` | 0.3.2 | [github.com/madsmtm/objc2](https://github.com/madsmtm/objc2) |  |
| `raw-window-handle` | 0.6.2 | [github.com/rust-windowing/raw-window-handle](https://github.com/rust-windowing/raw-window-handle) |  |
| `tinyvec` | 1.12.0 | [github.com/Lokathor/tinyvec](https://github.com/Lokathor/tinyvec) |  |
| `tinyvec_macros` | 0.1.1 | [github.com/Soveu/tinyvec_macros](https://github.com/Soveu/tinyvec_macros) |  |
| `xkeysym` | 0.2.1 | [github.com/notgull/xkeysym](https://github.com/notgull/xkeysym) |  |
| `zune-core` | 0.4.12 | — |  |
| `zune-core` | 0.5.3 | [github.com/etemesi254/zune-image](https://github.com/etemesi254/zune-image) |  |
| `zune-inflate` | 0.2.54 | — |  |
| `zune-jpeg` | 0.4.21 | [github.com/etemesi254/zune-image/tree/dev/crates/zune-jpeg](https://github.com/etemesi254/zune-image/tree/dev/crates/zune-jpeg) |  |
| `zune-jpeg` | 0.5.15 | [github.com/etemesi254/zune-image/tree/dev/crates/zune-jpeg](https://github.com/etemesi254/zune-image/tree/dev/crates/zune-jpeg) |  |

### Unicode-3.0

_18 个依赖_

| 依赖 | 版本 | 来源 | 备注 |
| --- | --- | --- | --- |
| `icu_collections` | 2.3.0 | [github.com/unicode-org/icu4x](https://github.com/unicode-org/icu4x) |  |
| `icu_locale_core` | 2.3.0 | [github.com/unicode-org/icu4x](https://github.com/unicode-org/icu4x) |  |
| `icu_normalizer` | 2.3.0 | [github.com/unicode-org/icu4x](https://github.com/unicode-org/icu4x) |  |
| `icu_normalizer_data` | 2.3.0 | [github.com/unicode-org/icu4x](https://github.com/unicode-org/icu4x) |  |
| `icu_properties` | 2.3.0 | [github.com/unicode-org/icu4x](https://github.com/unicode-org/icu4x) |  |
| `icu_properties_data` | 2.3.0 | [github.com/unicode-org/icu4x](https://github.com/unicode-org/icu4x) |  |
| `icu_provider` | 2.3.0 | [github.com/unicode-org/icu4x](https://github.com/unicode-org/icu4x) |  |
| `litemap` | 0.8.3 | [github.com/unicode-org/icu4x](https://github.com/unicode-org/icu4x) |  |
| `potential_utf` | 0.1.6 | [github.com/unicode-org/icu4x](https://github.com/unicode-org/icu4x) |  |
| `tinystr` | 0.8.4 | [github.com/unicode-org/icu4x](https://github.com/unicode-org/icu4x) |  |
| `writeable` | 0.6.4 | [github.com/unicode-org/icu4x](https://github.com/unicode-org/icu4x) |  |
| `yoke` | 0.8.3 | [github.com/unicode-org/icu4x](https://github.com/unicode-org/icu4x) |  |
| `yoke-derive` | 0.8.2 | [github.com/unicode-org/icu4x](https://github.com/unicode-org/icu4x) |  |
| `zerofrom` | 0.1.8 | [github.com/unicode-org/icu4x](https://github.com/unicode-org/icu4x) |  |
| `zerofrom-derive` | 0.1.7 | [github.com/unicode-org/icu4x](https://github.com/unicode-org/icu4x) |  |
| `zerotrie` | 0.2.5 | [github.com/unicode-org/icu4x](https://github.com/unicode-org/icu4x) |  |
| `zerovec` | 0.11.8 | [github.com/unicode-org/icu4x](https://github.com/unicode-org/icu4x) |  |
| `zerovec-derive` | 0.11.5 | [github.com/unicode-org/icu4x](https://github.com/unicode-org/icu4x) |  |

### BSD-3-Clause

_12 个依赖_

| 依赖 | 版本 | 来源 | 备注 |
| --- | --- | --- | --- |
| `alloc-no-stdlib` | 2.0.4 | [github.com/dropbox/rust-alloc-no-stdlib](https://github.com/dropbox/rust-alloc-no-stdlib) |  |
| `alloc-stdlib` | 0.2.4 | [github.com/dropbox/rust-alloc-no-stdlib](https://github.com/dropbox/rust-alloc-no-stdlib) |  |
| `avif-serialize` | 0.8.9 | [github.com/kornelski/avif-serialize](https://github.com/kornelski/avif-serialize) |  |
| `bindgen` | 0.71.1 | [github.com/rust-lang/rust-bindgen](https://github.com/rust-lang/rust-bindgen) |  |
| `exr` | 1.74.2 | [github.com/johannesvollmer/exrs](https://github.com/johannesvollmer/exrs) |  |
| `instant` | 0.1.13 | [github.com/sebcrozet/instant](https://github.com/sebcrozet/instant) |  |
| `lebe` | 0.5.3 | [github.com/johannesvollmer/lebe](https://github.com/johannesvollmer/lebe) |  |
| `ravif` | 0.13.0 | [github.com/kornelski/cavif-rs](https://github.com/kornelski/cavif-rs) |  |
| `sha1_smol` | 1.0.1 | [github.com/mitsuhiko/sha1-smol](https://github.com/mitsuhiko/sha1-smol) |  |
| `subtle` | 2.6.1 | [github.com/dalek-cryptography/subtle](https://github.com/dalek-cryptography/subtle) |  |
| `tiny-skia` | 0.11.4 | [github.com/RazrFalcon/tiny-skia](https://github.com/RazrFalcon/tiny-skia) |  |
| `tiny-skia-path` | 0.11.4 | [github.com/RazrFalcon/tiny-skia/tree/master/path](https://github.com/RazrFalcon/tiny-skia/tree/master/path) |  |

### MIT OR Unlicense

_10 个依赖_

| 依赖 | 版本 | 来源 | 备注 |
| --- | --- | --- | --- |
| `aho-corasick` | 1.1.5 | [github.com/BurntSushi/aho-corasick](https://github.com/BurntSushi/aho-corasick) |  |
| `byteorder` | 1.5.0 | [github.com/BurntSushi/byteorder](https://github.com/BurntSushi/byteorder) |  |
| `byteorder-lite` | 0.1.0 | [github.com/image-rs/byteorder-lite](https://github.com/image-rs/byteorder-lite) |  |
| `globset` | 0.4.20 | [github.com/BurntSushi/ripgrep/tree/master/crates/globset](https://github.com/BurntSushi/ripgrep/tree/master/crates/globset) |  |
| `ignore` | 0.4.33 | [github.com/BurntSushi/ripgrep/tree/master/crates/ignore](https://github.com/BurntSushi/ripgrep/tree/master/crates/ignore) |  |
| `memchr` | 2.8.3 | [github.com/BurntSushi/memchr](https://github.com/BurntSushi/memchr) |  |
| `same-file` | 1.0.6 | [github.com/BurntSushi/same-file](https://github.com/BurntSushi/same-file) |  |
| `termcolor` | 1.4.1 | [github.com/BurntSushi/termcolor](https://github.com/BurntSushi/termcolor) |  |
| `walkdir` | 2.5.0 | [github.com/BurntSushi/walkdir](https://github.com/BurntSushi/walkdir) |  |
| `winapi-util` | 0.1.11 | [github.com/BurntSushi/winapi-util](https://github.com/BurntSushi/winapi-util) |  |

### Apache-2.0 OR Apache-2.0 WITH LLVM-exception OR MIT

_7 个依赖_

| 依赖 | 版本 | 来源 | 备注 |
| --- | --- | --- | --- |
| `linux-raw-sys` | 0.4.15 | [github.com/sunfishcode/linux-raw-sys](https://github.com/sunfishcode/linux-raw-sys) |  |
| `linux-raw-sys` | 0.12.1 | [github.com/sunfishcode/linux-raw-sys](https://github.com/sunfishcode/linux-raw-sys) |  |
| `rustix` | 0.38.44 | [github.com/bytecodealliance/rustix](https://github.com/bytecodealliance/rustix) |  |
| `rustix` | 1.1.4 | [github.com/bytecodealliance/rustix](https://github.com/bytecodealliance/rustix) |  |
| `wasi` | 0.11.1+wasi-snapshot-preview1 | [github.com/bytecodealliance/wasi](https://github.com/bytecodealliance/wasi) |  |
| `wasip2` | 1.0.4+wasi-0.2.12 | [github.com/bytecodealliance/wasi-rs](https://github.com/bytecodealliance/wasi-rs) |  |
| `wit-bindgen` | 0.57.1 | [github.com/bytecodealliance/wit-bindgen](https://github.com/bytecodealliance/wit-bindgen) |  |

### ISC

_5 个依赖_

| 依赖 | 版本 | 来源 | 备注 |
| --- | --- | --- | --- |
| `inotify` | 0.10.2 | [github.com/hannobraun/inotify](https://github.com/hannobraun/inotify) |  |
| `inotify-sys` | 0.1.8 | [github.com/hannobraun/inotify-sys](https://github.com/hannobraun/inotify-sys) |  |
| `libloading` | 0.8.9 | [github.com/nagisa/rust_libloading/](https://github.com/nagisa/rust_libloading/) |  |
| `rustls-webpki` | 0.103.14 | [github.com/rustls/webpki](https://github.com/rustls/webpki) |  |
| `untrusted` | 0.9.0 | [github.com/briansmith/untrusted](https://github.com/briansmith/untrusted) |  |

### Apache-2.0 OR ISC OR MIT

_4 个依赖_

| 依赖 | 版本 | 来源 | 备注 |
| --- | --- | --- | --- |
| `hyper-rustls` | 0.27.9 | [github.com/rustls/hyper-rustls](https://github.com/rustls/hyper-rustls) |  |
| `rustls` | 0.23.43 | [github.com/rustls/rustls](https://github.com/rustls/rustls) |  |
| `rustls-native-certs` | 0.8.4 | [github.com/rustls/rustls-native-certs](https://github.com/rustls/rustls-native-certs) |  |
| `rustls-pemfile` | 2.2.0 | [github.com/rustls/pemfile](https://github.com/rustls/pemfile) |  |

### BSD-2-Clause

_4 个依赖_

| 依赖 | 版本 | 来源 | 备注 |
| --- | --- | --- | --- |
| `arrayref` | 0.3.9 | [github.com/droundy/arrayref](https://github.com/droundy/arrayref) |  |
| `av1-grain` | 0.2.5 | [github.com/rust-av/av1-grain](https://github.com/rust-av/av1-grain) |  |
| `rav1e` | 0.8.1 | [github.com/xiph/rav1e/](https://github.com/xiph/rav1e/) |  |
| `v_frame` | 0.3.9 | [github.com/rust-av/v_frame](https://github.com/rust-av/v_frame) |  |

### CC0-1.0

_4 个依赖_

| 依赖 | 版本 | 来源 | 备注 |
| --- | --- | --- | --- |
| `hexf-parse` | 0.2.1 | [github.com/lifthrasiir/hexf](https://github.com/lifthrasiir/hexf) |  |
| `notify` | 7.0.0 | [github.com/notify-rs/notify.git](https://github.com/notify-rs/notify.git) |  |
| `tiny-keccak` | 2.0.2 | — |  |
| `workspace-hack` | 0.1.0 | [github.com/facebookincubator/cargo-guppy](https://github.com/facebookincubator/cargo-guppy) |  |

### Apache-2.0 OR BSD-2-Clause OR MIT

_3 个依赖_

| 依赖 | 版本 | 来源 | 备注 |
| --- | --- | --- | --- |
| `mach2` | 0.5.0 | [github.com/JohnTitor/mach2](https://github.com/JohnTitor/mach2) |  |
| `zerocopy` | 0.8.56 | [github.com/google/zerocopy](https://github.com/google/zerocopy) |  |
| `zerocopy-derive` | 0.8.56 | [github.com/google/zerocopy](https://github.com/google/zerocopy) |  |

### GPL-3.0-or-later

_3 个依赖_

| 依赖 | 版本 | 来源 | 备注 |
| --- | --- | --- | --- |
| `zlog` | 0.1.0 | [github.com/zed-industries/zed](https://github.com/zed-industries/zed) | ⚠️ 强 copyleft，见文首「已知许可证问题」 |
| `ztracing` | 0.1.0 | [github.com/zed-industries/zed](https://github.com/zed-industries/zed) | ⚠️ 强 copyleft，见文首「已知许可证问题」 |
| `ztracing_macro` | 0.1.0 | [github.com/zed-industries/zed](https://github.com/zed-industries/zed) | ⚠️ 强 copyleft，见文首「已知许可证问题」 |

### MPL-2.0

_3 个依赖_

| 依赖 | 版本 | 来源 | 备注 |
| --- | --- | --- | --- |
| `cbindgen` | 0.28.0 | [github.com/mozilla/cbindgen](https://github.com/mozilla/cbindgen) | 弱 copyleft（文件级），仅构建期使用，不进入产物 |
| `dwrote` | 0.11.5 | [github.com/servo/dwrote-rs](https://github.com/servo/dwrote-rs) | 弱 copyleft（文件级），仅 Windows |
| `option-ext` | 0.2.0 | [github.com/soc/option-ext.git](https://github.com/soc/option-ext.git) | 弱 copyleft（文件级），未修改其源码 |

### Zlib

_3 个依赖_

| 依赖 | 版本 | 来源 | 备注 |
| --- | --- | --- | --- |
| `foldhash` | 0.1.5 | [github.com/orlp/foldhash](https://github.com/orlp/foldhash) |  |
| `foldhash` | 0.2.0 | [github.com/orlp/foldhash](https://github.com/orlp/foldhash) |  |
| `slotmap` | 1.1.1 | [github.com/orlp/slotmap](https://github.com/orlp/slotmap) |  |

### 0BSD

_2 个依赖_

| 依赖 | 版本 | 来源 | 备注 |
| --- | --- | --- | --- |
| `enum-iterator` | 2.3.0 | [github.com/stephaneyfx/enum-iterator.git](https://github.com/stephaneyfx/enum-iterator.git) |  |
| `enum-iterator-derive` | 1.5.0 | [github.com/stephaneyfx/enum-iterator.git](https://github.com/stephaneyfx/enum-iterator.git) |  |

### Apache-2.0 OR BSD-3-Clause

_2 个依赖_

| 依赖 | 版本 | 来源 | 备注 |
| --- | --- | --- | --- |
| `moxcms` | 0.8.1 | [github.com/awxkee/moxcms.git](https://github.com/awxkee/moxcms.git) |  |
| `pxfm` | 0.1.30 | [github.com/awxkee/pxfm](https://github.com/awxkee/pxfm) |  |

### Apache-2.0 OR LGPL-2.1-or-later OR MIT

_2 个依赖_

| 依赖 | 版本 | 来源 | 备注 |
| --- | --- | --- | --- |
| `r-efi` | 5.3.0 | [github.com/r-efi/r-efi](https://github.com/r-efi/r-efi) |  |
| `r-efi` | 6.0.0 | [github.com/r-efi/r-efi](https://github.com/r-efi/r-efi) |  |

### CDLA-Permissive-2.0

_2 个依赖_

| 依赖 | 版本 | 来源 | 备注 |
| --- | --- | --- | --- |
| `webpki-root-certs` | 1.0.9 | [github.com/rustls/webpki-roots](https://github.com/rustls/webpki-roots) |  |
| `webpki-roots` | 1.0.9 | [github.com/rustls/webpki-roots](https://github.com/rustls/webpki-roots) |  |

### （未声明）

_2 个依赖_

| 依赖 | 版本 | 来源 | 备注 |
| --- | --- | --- | --- |
| `gpui_shared_string` | 0.1.0 | [github.com/zed-industries/zed](https://github.com/zed-industries/zed) |  |
| `gpui_util` | 0.1.0 | [github.com/zed-industries/zed](https://github.com/zed-industries/zed) |  |

### (Apache-2.0 OR ISC OR MIT) AND (Apache-2.0 OR ISC OR MIT-0) AND (Apache-2.0 OR ISC) AND Apache-2.0 AND BSD-3-Clause AND ISC AND MIT

_1 个依赖_

| 依赖 | 版本 | 来源 | 备注 |
| --- | --- | --- | --- |
| `aws-lc-sys` | 0.44.0 | [github.com/aws/aws-lc-rs](https://github.com/aws/aws-lc-rs) |  |

### (Apache-2.0 OR ISC) AND ISC

_1 个依赖_

| 依赖 | 版本 | 来源 | 备注 |
| --- | --- | --- | --- |
| `aws-lc-rs` | 1.18.0 | [github.com/aws/aws-lc-rs](https://github.com/aws/aws-lc-rs) |  |

### (Apache-2.0 OR MIT) AND BSD-3-Clause

_1 个依赖_

| 依赖 | 版本 | 来源 | 备注 |
| --- | --- | --- | --- |
| `encoding_rs` | 0.8.35 | [github.com/hsivonen/encoding_rs](https://github.com/hsivonen/encoding_rs) |  |

### (Apache-2.0 OR MIT) AND NCSA

_1 个依赖_

| 依赖 | 版本 | 来源 | 备注 |
| --- | --- | --- | --- |
| `libfuzzer-sys` | 0.4.13 | [github.com/rust-fuzz/libfuzzer](https://github.com/rust-fuzz/libfuzzer) |  |

### (Apache-2.0 OR MIT) AND Unicode-3.0

_1 个依赖_

| 依赖 | 版本 | 来源 | 备注 |
| --- | --- | --- | --- |
| `unicode-ident` | 1.0.24 | [github.com/dtolnay/unicode-ident](https://github.com/dtolnay/unicode-ident) |  |

### 0BSD OR Apache-2.0 OR MIT

_1 个依赖_

| 依赖 | 版本 | 来源 | 备注 |
| --- | --- | --- | --- |
| `adler2` | 2.0.1 | [github.com/oyvindln/adler2](https://github.com/oyvindln/adler2) |  |

### Apache-2.0 AND ISC

_1 个依赖_

| 依赖 | 版本 | 来源 | 备注 |
| --- | --- | --- | --- |
| `ring` | 0.17.14 | [github.com/briansmith/ring](https://github.com/briansmith/ring) |  |

### Apache-2.0 OR BSL-1.0

_1 个依赖_

| 依赖 | 版本 | 来源 | 备注 |
| --- | --- | --- | --- |
| `ryu` | 1.0.23 | [github.com/dtolnay/ryu](https://github.com/dtolnay/ryu) |  |

### Apache-2.0 OR CC0-1.0

_1 个依赖_

| 依赖 | 版本 | 来源 | 备注 |
| --- | --- | --- | --- |
| `imgref` | 1.12.2 | [github.com/kornelski/imgref](https://github.com/kornelski/imgref) |  |

### Apache-2.0 OR CC0-1.0 OR MIT-0

_1 个依赖_

| 依赖 | 版本 | 来源 | 备注 |
| --- | --- | --- | --- |
| `dunce` | 1.0.5 | [gitlab.com/kornelski/dunce](https://gitlab.com/kornelski/dunce) |  |

### Apache-2.0 OR GPL-2.0

_1 个依赖_

| 依赖 | 版本 | 来源 | 备注 |
| --- | --- | --- | --- |
| `self_cell` | 1.3.0 | [github.com/Voultapher/self_cell](https://github.com/Voultapher/self_cell) |  |

### Apache-2.0 WITH LLVM-exception

_1 个依赖_

| 依赖 | 版本 | 来源 | 备注 |
| --- | --- | --- | --- |
| `ar_archive_writer` | 0.5.3 | [github.com/rust-lang/ar_archive_writer](https://github.com/rust-lang/ar_archive_writer) |  |

### BSD-3-Clause AND MIT

_1 个依赖_

| 依赖 | 版本 | 来源 | 备注 |
| --- | --- | --- | --- |
| `brotli` | 8.0.4 | [github.com/dropbox/rust-brotli](https://github.com/dropbox/rust-brotli) |  |

### BSD-3-Clause OR MIT

_1 个依赖_

| 依赖 | 版本 | 来源 | 备注 |
| --- | --- | --- | --- |
| `brotli-decompressor` | 5.0.3 | [github.com/dropbox/rust-brotli-decompressor](https://github.com/dropbox/rust-brotli-decompressor) |  |

### bzip2-1.0.6

_1 个依赖_

| 依赖 | 版本 | 来源 | 备注 |
| --- | --- | --- | --- |
| `libbz2-rs-sys` | 0.2.5 | [github.com/trifectatechfoundation/libbzip2-rs](https://github.com/trifectatechfoundation/libbzip2-rs) |  |
