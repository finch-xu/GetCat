//! 解析 curl 命令行 → [`RequestDraft`]。
//!
//! 目标是把「从别处复制来的一条 curl」原样搬进来：Chrome / Firefox / Safari 的
//! “Copy as cURL”、文档里的示例、同事贴过来的命令，以及 GetCat 自己
//! [`crate::codegen`] 生成的两种方言。
//!
//! 分两层：
//!
//! 1. [`tokenize`] 把命令行切成 argv，负责 shell 的引用与续行规则；
//! 2. [`parse`] 把 argv 归并成一份草稿，负责 curl 的选项语义。
//!
//! # 宽进
//!
//! 认不出的选项**不会让整条命令失败**，只记一条 [`CurlWarning`]。从浏览器复制的
//! 命令常带一大串 `-H` 之外的杂项，为了一个 `--http2` 就整条拒绝毫无道理——
//! 能还原多少还原多少，剩下的如实告诉用户。

use std::path::PathBuf;

use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};

use crate::model::{BodyKind, FormField, FormValue, KeyValue, Method, RawFormat, RequestDraft};
use crate::url::extract_path_params;

/// 解析结果：一份草稿，外加解析过程中没能原样搬过来的东西。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurlImport {
    pub draft: RequestDraft,
    pub warnings: Vec<CurlWarning>,
}

/// 没能原样搬过来的东西。都不影响草稿可用，只是需要让用户知道。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CurlWarning {
    /// 认识，但它描述的是「怎么发」而不是「这条请求是什么」——归 GetCat 的全局设置管。
    RuntimeOption(String),
    /// 不认识的选项，已跳过。
    Unknown(String),
    /// 认识，但 GetCat 的模型表达不了。
    Unsupported(String),
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CurlParseError {
    #[error("命令为空")]
    Empty,
    #[error("不是 curl 命令")]
    NotCurl,
    #[error("引号没有闭合")]
    UnterminatedQuote,
    #[error("没有找到 URL")]
    MissingUrl,
    #[error("选项 {0} 缺少参数")]
    MissingValue(String),
}

/// 解析一整条 curl 命令。
pub fn parse(input: &str) -> Result<CurlImport, CurlParseError> {
    let argv = tokenize(input)?;
    let mut argv = argv.into_iter();
    match argv.next() {
        // 允许 `$ curl ...` 与带路径的 `/usr/bin/curl ...`
        Some(first) if is_curl(&first) => {}
        Some(first) if first == "$" => match argv.next() {
            Some(second) if is_curl(&second) => {}
            _ => return Err(CurlParseError::NotCurl),
        },
        Some(_) => return Err(CurlParseError::NotCurl),
        None => return Err(CurlParseError::Empty),
    }
    from_argv(argv.collect())
}

fn is_curl(word: &str) -> bool {
    let name = word.rsplit(['/', '\\']).next().unwrap_or(word);
    name.eq_ignore_ascii_case("curl") || name.eq_ignore_ascii_case("curl.exe")
}

/// 解析过程中攒起来的东西，最后一次性归并成 [`BodyKind`]。
#[derive(Default)]
struct Collected {
    method: Option<Method>,
    url: Option<String>,
    headers: Vec<KeyValue>,
    /// `-d` / `--data-*` 的片段，按 curl 的语义最后用 `&` 连起来。
    data: Vec<String>,
    /// `--data-binary @file` 指向的文件（整个请求体就是它）。
    data_file: Option<PathBuf>,
    form: Vec<FormField>,
    /// `-G`：把 data 挪进 query 而不是请求体。
    get_style: bool,
    /// `-I`：只要响应头。
    head_only: bool,
    warnings: Vec<CurlWarning>,
}

/// 从 argv（不含 `curl` 本身）解析。
fn from_argv(argv: Vec<String>) -> Result<CurlImport, CurlParseError> {
    let mut c = Collected::default();
    let mut it = argv.into_iter().peekable();

    while let Some(arg) = it.next() {
        // `--data=xxx` 与 `--data xxx` 都要吃下
        let (flag, inline) = match arg.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f.to_string(), Some(v.to_string())),
            _ => (arg.clone(), None),
        };
        let mut take = |flag: &str| -> Result<String, CurlParseError> {
            match inline.clone().or_else(|| it.next()) {
                Some(v) => Ok(v),
                None => Err(CurlParseError::MissingValue(flag.to_string())),
            }
        };

        match flag.as_str() {
            "-X" | "--request" => {
                let raw = take(&flag)?;
                match Method::parse(&raw) {
                    Some(m) => c.method = Some(m),
                    // GetCat 只支持这 7 个方法；别的（PROPFIND 之类）如实说一声
                    None => c
                        .warnings
                        .push(CurlWarning::Unsupported(format!("-X {raw}"))),
                }
            }
            "-H" | "--header" => {
                let raw = take(&flag)?;
                match parse_header(&raw) {
                    Some(kv) => c.headers.push(kv),
                    None => c
                        .warnings
                        .push(CurlWarning::Unsupported(format!("-H {raw}"))),
                }
            }
            "-A" | "--user-agent" => c.headers.push(KeyValue::new("User-Agent", take(&flag)?)),
            "-e" | "--referer" => c.headers.push(KeyValue::new("Referer", take(&flag)?)),
            "-b" | "--cookie" => {
                let raw = take(&flag)?;
                // `-b file` 是读 cookie jar，不是字面 cookie
                if raw.contains('=') {
                    c.headers.push(KeyValue::new("Cookie", raw));
                } else {
                    c.warnings
                        .push(CurlWarning::Unsupported(format!("-b {raw}")));
                }
            }
            "-u" | "--user" => {
                let raw = take(&flag)?;
                c.headers.push(KeyValue::new(
                    "Authorization",
                    format!("Basic {}", base64(raw.as_bytes())),
                ));
            }
            "--url" => c.url = Some(take(&flag)?),
            "-d" | "--data" | "--data-raw" | "--data-ascii" | "--data-binary" => {
                let raw = take(&flag)?;
                // `@file` 读文件作为请求体；`--data-raw` 不认这个语法，@ 是字面量
                match raw.strip_prefix('@') {
                    Some(path) if flag != "--data-raw" => c.data_file = Some(PathBuf::from(path)),
                    _ => c.data.push(raw),
                }
            }
            "--data-urlencode" => {
                let raw = take(&flag)?;
                c.data.push(urlencode_data(&raw));
            }
            "--json" => {
                // curl 7.82+：等价于 -d 加上这两个头
                c.data.push(take(&flag)?);
                c.headers
                    .push(KeyValue::new("Content-Type", "application/json"));
                c.headers.push(KeyValue::new("Accept", "application/json"));
            }
            "-F" | "--form" | "--form-string" => {
                let raw = take(&flag)?;
                c.form.push(parse_form_field(&raw, flag == "--form-string"));
            }
            "-G" | "--get" => c.get_style = true,
            "-I" | "--head" => c.head_only = true,
            // 认识，但描述的是「怎么发」——归 GetCat 的全局设置管，不进草稿
            _ if is_runtime_flag(&flag) => {
                if takes_value(&flag) && inline.is_none() {
                    it.next();
                }
                c.warnings.push(CurlWarning::RuntimeOption(flag));
            }
            // 不是选项的裸词就是 URL。第二个裸词 curl 会当成另一个请求，
            // GetCat 一个 Tab 只表达一条请求，多出来的如实说一声。
            _ if !flag.starts_with('-') => {
                if c.url.is_none() {
                    c.url = Some(arg);
                } else {
                    c.warnings.push(CurlWarning::Unsupported(arg));
                }
            }
            _ => {
                if takes_value(&flag) && inline.is_none() {
                    it.next();
                }
                c.warnings.push(CurlWarning::Unknown(flag));
            }
        }
    }

    c.finish()
}

impl Collected {
    fn finish(mut self) -> Result<CurlImport, CurlParseError> {
        let raw_url = self.url.take().ok_or(CurlParseError::MissingUrl)?;
        let mut data = std::mem::take(&mut self.data);

        // -G：data 变成 query，请求体清空
        let mut extra_query = Vec::new();
        if self.get_style {
            extra_query = std::mem::take(&mut data);
        }
        let (url, mut params) = split_query(&raw_url, &extra_query);

        let body = self.body(data);
        // 方法优先级：显式 -X > -I > 有请求体则 POST > GET
        let method = self.method.unwrap_or({
            if self.head_only {
                Method::Head
            } else if body != BodyKind::None {
                Method::Post
            } else {
                Method::Get
            }
        });

        // URL 里若带 `{name}` 占位，与手输 URL 一样自动出现在 Path 参数表里
        let path_params = extract_path_params(&url)
            .into_iter()
            .map(|name| KeyValue::new(name, ""))
            .collect();
        params.retain(|kv: &KeyValue| !kv.key.is_empty());

        Ok(CurlImport {
            draft: RequestDraft {
                method,
                url,
                path_params,
                params,
                headers: self.headers,
                body,
            },
            warnings: self.warnings,
        })
    }

    /// 把攒下的 data / form / 文件归并成一种请求体。
    fn body(&mut self, data: Vec<String>) -> BodyKind {
        if !self.form.is_empty() {
            return BodyKind::FormData {
                fields: std::mem::take(&mut self.form),
            };
        }
        if let Some(path) = self.data_file.take() {
            return BodyKind::Binary {
                path,
                content_type: None,
            };
        }
        if data.is_empty() {
            return BodyKind::None;
        }
        // curl 的语义：多个 -d 用 & 连起来
        let text = data.join("&");
        let content_type = self.content_type();

        // 还原成 urlencoded 表格比塞进 raw 编辑器好编辑得多。两种情况这么做：
        // 显式声明了 urlencoded，或者压根没写 Content-Type——后者是 curl 自己
        // 对 `-d` 的默认值，不认这条的话 `-d a=1 -d b=2` 会变成一串裸文本。
        let urlencoded_by_default = match content_type.as_deref() {
            Some(ct) => ct.starts_with("application/x-www-form-urlencoded"),
            // 看着像 JSON / XML 的就别硬拆：`-d '{"q":"a=b"}'` 里的 = 不是分隔符
            None => !matches!(text.trim_start().chars().next(), Some('{' | '[' | '<')),
        };
        if urlencoded_by_default && let Some(fields) = parse_urlencoded(&text) {
            return BodyKind::FormUrlEncoded { fields };
        }
        BodyKind::Raw {
            format: raw_format_for(content_type.as_deref(), &text),
            text,
        }
    }

    fn content_type(&self) -> Option<String> {
        self.headers
            .iter()
            .find(|h| h.key.eq_ignore_ascii_case("content-type"))
            .map(|h| h.value.to_ascii_lowercase())
    }
}

/// 请求体的 raw 格式：先信 Content-Type，没有就看内容长什么样。
fn raw_format_for(content_type: Option<&str>, text: &str) -> RawFormat {
    if let Some(ct) = content_type {
        if ct.contains("json") {
            return RawFormat::Json;
        }
        if ct.contains("xml") || ct.contains("html") {
            return RawFormat::Xml;
        }
        if ct.starts_with("text/") {
            return RawFormat::Text;
        }
    }
    let head = text.trim_start();
    if head.starts_with('{') || head.starts_with('[') {
        RawFormat::Json
    } else if head.starts_with('<') {
        RawFormat::Xml
    } else {
        RawFormat::Text
    }
}

/// `Name: value`，以及 curl 用来发空值头的 `Name;`。
///
/// `Name:`（冒号后什么都没有）在 curl 里是**删除**这个头的意思，不是发空值——
/// GetCat 没有「默认头」以外的东西可删，忽略掉。
fn parse_header(raw: &str) -> Option<KeyValue> {
    if let Some((name, value)) = raw.split_once(':') {
        let value = value.trim();
        if value.is_empty() {
            return None;
        }
        return Some(KeyValue::new(name.trim(), value));
    }
    raw.strip_suffix(';')
        .map(|name| KeyValue::new(name.trim(), ""))
}

/// `-F` 的一个字段：`k=v` / `k=@path` / `k=@path;type=image/png`。
///
/// `--form-string` 不解释 `@`，值一律是字面量。
fn parse_form_field(raw: &str, literal: bool) -> FormField {
    let (key, value) = raw.split_once('=').unwrap_or((raw, ""));
    if literal {
        return FormField::text(key, value);
    }
    let Some(rest) = value.strip_prefix('@').or_else(|| value.strip_prefix('<')) else {
        return FormField::text(key, value);
    };
    // `;type=` / `;filename=` 这类参数跟在路径后面
    let mut parts = rest.split(';');
    let path = parts.next().unwrap_or_default();
    let content_type =
        parts.find_map(|p| p.trim().strip_prefix("type=").map(|t| t.trim().to_string()));
    FormField {
        key: key.to_string(),
        enabled: true,
        description: String::new(),
        value: FormValue::File {
            path: PathBuf::from(path),
            content_type,
        },
    }
}

/// `--data-urlencode` 的几种写法，返回已编码的 `k=v` 片段。
/// 只编码值那一半，`name=` 前缀原样保留（curl 也是这么干的）。
fn urlencode_data(raw: &str) -> String {
    let encode = |s: &str| utf8_percent_encode(s, NON_ALPHANUMERIC).to_string();
    match raw.split_once('=') {
        // `=content`：整串都是值，没有名字
        Some(("", value)) => encode(value),
        Some((name, value)) => format!("{name}={}", encode(value)),
        // 没有 `=`：整串是要编码的内容
        None => encode(raw),
    }
}

/// 把 `a=1&b=2` 拆成表格；任一段没有 `=` 就整体放弃（留在 raw 里更忠实）。
fn parse_urlencoded(text: &str) -> Option<Vec<KeyValue>> {
    text.split('&')
        .map(|pair| {
            pair.split_once('=')
                .map(|(k, v)| KeyValue::new(decode_plus(k), decode_plus(v)))
        })
        .collect()
}

/// urlencoded 里的 `+` 是空格。百分号转义交给发送时的编码器，这里不动——
/// 解开了再编回去反而可能变形。
fn decode_plus(s: &str) -> String {
    s.replace('+', " ")
}

/// 把 URL 自带的 query 拆进参数表（与手输 URL 的行为一致），
/// 再接上 `-G` 挪过来的那些。
fn split_query(raw: &str, extra: &[String]) -> (String, Vec<KeyValue>) {
    let (base, query) = match raw.split_once('?') {
        Some((b, q)) => (b.to_string(), q.to_string()),
        None => (raw.to_string(), String::new()),
    };
    let mut params = Vec::new();
    for chunk in std::iter::once(query.as_str()).chain(extra.iter().map(String::as_str)) {
        for pair in chunk.split('&').filter(|p| !p.is_empty()) {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            params.push(KeyValue::new(decode_plus(k), decode_plus(v)));
        }
    }
    (base, params)
}

/// 描述「怎么发」而不是「这条请求是什么」的选项：TLS 校验、重定向、超时、
/// 输出控制……它们对应 GetCat 的全局设置，读进草稿只会和用户的选择打架。
/// 与 [`crate::codegen`] 刻意不生成这些参数是同一条判据。
fn is_runtime_flag(flag: &str) -> bool {
    const FLAGS: &[&str] = &[
        "-k",
        "--insecure",
        "-L",
        "--location",
        "--compressed",
        "-s",
        "--silent",
        "-S",
        "--show-error",
        "-v",
        "--verbose",
        "-i",
        "--include",
        "-f",
        "--fail",
        "-o",
        "--output",
        "-O",
        "--remote-name",
        "-w",
        "--write-out",
        "--connect-timeout",
        "-m",
        "--max-time",
        "--retry",
        "--max-redirs",
        "--http1.1",
        "--http2",
        "--http3",
        "--cacert",
        "--cert",
        "--key",
        "-x",
        "--proxy",
        "--resolve",
        "--interface",
        "-4",
        "-6",
        "--no-buffer",
        "--globoff",
        "-g",
        "--tlsv1.2",
        "--tlsv1.3",
        "--path-as-is",
    ];
    FLAGS.contains(&flag)
}

/// 这个选项后面还跟一个参数吗——跳过它时要连参数一起跳，
/// 否则那个参数会被当成 URL。
fn takes_value(flag: &str) -> bool {
    const WITH_VALUE: &[&str] = &[
        "-o",
        "--output",
        "-w",
        "--write-out",
        "--connect-timeout",
        "-m",
        "--max-time",
        "--retry",
        "--max-redirs",
        "--cacert",
        "--cert",
        "--key",
        "-x",
        "--proxy",
        "--resolve",
        "--interface",
        "--limit-rate",
        "-C",
        "--continue-at",
        "-E",
    ];
    WITH_VALUE.contains(&flag)
}

/// 标准 base64（`-u user:pass` → `Authorization: Basic ...`）。
///
/// 自己写而不是引一个 crate：只用到编码方向的二十来行，为此多一个依赖、
/// 多一条许可证审查不划算。
fn base64(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = u32::from(b[0]) << 16 | u32::from(b[1]) << 8 | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(TABLE[(n >> (18 - i * 6)) as usize & 0x3f] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// 命令行用的是哪种 shell 的引用规则。
///
/// 必须分开，因为**同一段 `"…"` 在两边规则相反**：POSIX 双引号里 `\\` 解成一个
/// 反斜杠，而 Windows CRT 规则下孤立的 `\\` 是两个字面反斜杠（只有紧跟 `"` 的那些
/// 才参与转义）。拿 POSIX 规则去读 cmd 命令，JSON 里的 `C:\\tmp` 会少一个反斜杠。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Dialect {
    Posix,
    WindowsCmd,
}

/// 靠续行符认方言：`^` 换行是 cmd，`\` 换行是 POSIX。
/// 单行命令认不出来，按更常见的 POSIX 处理。
fn detect_dialect(input: &str) -> Dialect {
    let bytes = input.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        if !matches!(b, b'^' | b'\\') {
            continue;
        }
        if matches!(bytes.get(i + 1), Some(b'\n' | b'\r')) {
            return if b == b'^' {
                Dialect::WindowsCmd
            } else {
                Dialect::Posix
            };
        }
    }
    Dialect::Posix
}

/// 把命令行切成 argv。
///
/// 要同时吃下 POSIX shell（`'…'`、`"…"`、`$'…'`、行尾 `\` 续行）与 Windows cmd
/// （行尾 `^` 续行、`""` 转义、CRT 的反斜杠规则）的写法——从浏览器复制出来的
/// 就是这两种。方言由 [`detect_dialect`] 判定。
pub fn tokenize(input: &str) -> Result<Vec<String>, CurlParseError> {
    let dialect = detect_dialect(input);
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut has_token = false;
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            // 续行：反斜杠 / 脱字符后面紧跟换行
            '\\' | '^' if matches!(chars.peek(), Some('\n' | '\r')) => {
                while matches!(chars.peek(), Some('\n' | '\r')) {
                    chars.next();
                }
            }
            c if c.is_whitespace() => {
                if has_token {
                    out.push(std::mem::take(&mut cur));
                    has_token = false;
                }
            }
            '\'' => {
                has_token = true;
                // 单引号里一切都是字面量，直到下一个单引号
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some(c) => cur.push(c),
                        None => return Err(CurlParseError::UnterminatedQuote),
                    }
                }
            }
            '"' if dialect == Dialect::WindowsCmd => {
                has_token = true;
                // CRT 规则（`codegen::curl::quote_windows` 的逆）：只有紧跟 `"` 的
                // 反斜杠才参与转义，2n 个 + `"` = n 个反斜杠且引号收尾，
                // 2n+1 个 + `"` = n 个反斜杠加一个字面引号；不跟引号的一律字面量。
                loop {
                    match chars.peek() {
                        None => return Err(CurlParseError::UnterminatedQuote),
                        Some('\\') => {
                            let mut slashes = 0usize;
                            while chars.peek() == Some(&'\\') {
                                chars.next();
                                slashes += 1;
                            }
                            if chars.peek() == Some(&'"') {
                                for _ in 0..slashes / 2 {
                                    cur.push('\\');
                                }
                                if slashes % 2 == 1 {
                                    chars.next();
                                    cur.push('"');
                                }
                                // 偶数个：引号留给下一轮当收尾符
                            } else {
                                for _ in 0..slashes {
                                    cur.push('\\');
                                }
                            }
                        }
                        Some('"') => {
                            chars.next();
                            // cmd 用 `""` 表示一个字面双引号
                            if chars.peek() == Some(&'"') {
                                chars.next();
                                cur.push('"');
                                continue;
                            }
                            break;
                        }
                        // `%` 在 cmd 里会触发 `%VAR%` 展开，生成端为此把它加倍了
                        Some('%') => {
                            chars.next();
                            if chars.peek() == Some(&'%') {
                                chars.next();
                            }
                            cur.push('%');
                        }
                        Some(_) => cur.push(chars.next().unwrap_or_default()),
                    }
                }
            }
            '"' => {
                has_token = true;
                loop {
                    match chars.next() {
                        Some('"') => break,
                        // 双引号内只有这几个字符会被反斜杠转义，其余反斜杠是字面量
                        Some('\\') => match chars.next() {
                            Some(c @ ('"' | '\\' | '$' | '`')) => cur.push(c),
                            Some('\n') | Some('\r') => {}
                            Some(c) => {
                                cur.push('\\');
                                cur.push(c);
                            }
                            None => return Err(CurlParseError::UnterminatedQuote),
                        },
                        Some(c) => cur.push(c),
                        None => return Err(CurlParseError::UnterminatedQuote),
                    }
                }
            }
            // `$'…'`：ANSI-C 引用，Chrome 在值里有控制字符时会用
            '$' if chars.peek() == Some(&'\'') => {
                chars.next();
                has_token = true;
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some('\\') => match chars.next() {
                            Some('n') => cur.push('\n'),
                            Some('r') => cur.push('\r'),
                            Some('t') => cur.push('\t'),
                            Some('\\') => cur.push('\\'),
                            Some('\'') => cur.push('\''),
                            Some('"') => cur.push('"'),
                            Some(c) => cur.push(c),
                            None => return Err(CurlParseError::UnterminatedQuote),
                        },
                        Some(c) => cur.push(c),
                        None => return Err(CurlParseError::UnterminatedQuote),
                    }
                }
            }
            '\\' => {
                has_token = true;
                // 引号外的反斜杠：只有转义空白与引号时才当转义符。其余原样保留，
                // 这样 `C:\tools\curl.exe` 这种裸路径不会被啃掉一半——
                // 命令行里出现 Windows 路径远比转义普通字符常见。
                match chars.peek() {
                    Some(c) if c.is_whitespace() || matches!(c, '\'' | '"' | '\\') => {
                        cur.push(chars.next().unwrap_or_default());
                    }
                    _ => cur.push('\\'),
                }
            }
            c => {
                has_token = true;
                cur.push(c);
            }
        }
    }
    if has_token {
        out.push(cur);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::{self, CodeTarget};

    fn draft_of(cmd: &str) -> RequestDraft {
        parse(cmd).expect("解析成功").draft
    }

    #[test]
    fn tokenizes_posix_quoting() {
        assert_eq!(
            tokenize(r#"curl -H 'a: b' -H "c: d" plain"#).unwrap(),
            ["curl", "-H", "a: b", "-H", "c: d", "plain"]
        );
        // 单引号里一切都是字面量
        assert_eq!(tokenize(r#"'a\nb'"#).unwrap(), [r"a\nb"]);
        // 双引号里只有这几个会被转义，别的反斜杠原样保留（Windows 路径要靠这条）
        assert_eq!(
            tokenize(r#""a\"b" "c\\d" "e\qf""#).unwrap(),
            [r#"a"b"#, r"c\d", r"e\qf"]
        );
        // $'…' 解转义
        assert_eq!(tokenize(r#"$'a\nb\tc'"#).unwrap(), ["a\nb\tc"]);
        // 空串是有效 token，不该被吞掉
        assert_eq!(
            tokenize(r#"curl -d '' x"#).unwrap(),
            ["curl", "-d", "", "x"]
        );
    }

    #[test]
    fn tokenizes_line_continuations() {
        // POSIX 的 \ 与 cmd 的 ^ 续行都要吃下
        let posix = "curl -X GET \\\n  'https://x.com' \\\n  -H 'a: b'";
        let cmd = "curl -X GET ^\n  \"https://x.com\" ^\n  -H \"a: b\"";
        for input in [posix, cmd] {
            assert_eq!(
                tokenize(input).unwrap(),
                ["curl", "-X", "GET", "https://x.com", "-H", "a: b"]
            );
        }
    }

    /// 同一段 `"…"` 在两种 shell 里规则相反，方言认错就会啃掉反斜杠。
    #[test]
    fn dialect_is_detected_from_the_line_continuation() {
        // cmd：CRT 规则下孤立的 `\\` 是两个字面反斜杠
        let cmd = "curl ^\n  \"C:\\\\tmp\"";
        assert_eq!(detect_dialect(cmd), Dialect::WindowsCmd);
        assert_eq!(tokenize(cmd).unwrap(), ["curl", r"C:\\tmp"]);
        // POSIX：同样的字面量里 `\\` 解成一个
        let posix = "curl \\\n  \"C:\\\\tmp\"";
        assert_eq!(detect_dialect(posix), Dialect::Posix);
        assert_eq!(tokenize(posix).unwrap(), ["curl", r"C:\tmp"]);
        // 认不出来时按更常见的 POSIX 处理
        assert_eq!(detect_dialect("curl https://x.com"), Dialect::Posix);
    }

    /// cmd 的 `\"` 与 `%%` 是 `codegen::curl::quote_windows` 的产物，要能读回来。
    #[test]
    fn windows_crt_escapes_round_trip() {
        let cmd = "curl ^\n  -d \"say \\\"hi\\\" to 100%%\"";
        assert_eq!(
            tokenize(cmd).unwrap(),
            ["curl", "-d", r#"say "hi" to 100%"#]
        );
    }

    /// 引号外的反斜杠：Windows 裸路径比转义普通字符常见得多，不能一律吃掉。
    #[test]
    fn bare_backslashes_survive_outside_quotes() {
        assert_eq!(
            tokenize(r"C:\tools\curl.exe").unwrap(),
            [r"C:\tools\curl.exe"]
        );
        // 但转义空白仍然要认，否则带空格的裸 URL 会被切开
        assert_eq!(tokenize(r"a\ b").unwrap(), ["a b"]);
    }

    #[test]
    fn unterminated_quote_is_an_error() {
        assert_eq!(
            tokenize("curl 'abc"),
            Err(CurlParseError::UnterminatedQuote)
        );
        assert_eq!(
            tokenize(r#"curl "abc"#),
            Err(CurlParseError::UnterminatedQuote)
        );
    }

    #[test]
    fn rejects_input_that_is_not_curl() {
        assert_eq!(parse(""), Err(CurlParseError::Empty));
        assert_eq!(parse("wget https://x.com"), Err(CurlParseError::NotCurl));
        assert_eq!(parse("curl"), Err(CurlParseError::MissingUrl));
        // 提示符与带路径的写法都得认
        assert!(parse("$ curl https://x.com").is_ok());
        assert!(parse("/usr/bin/curl https://x.com").is_ok());
        assert!(parse(r"C:\tools\curl.exe https://x.com").is_ok());
    }

    #[test]
    fn method_is_inferred_when_not_given() {
        assert_eq!(draft_of("curl https://x.com").method, Method::Get);
        // 有请求体但没写 -X：curl 自己也会发成 POST
        assert_eq!(draft_of("curl https://x.com -d 'a=1'").method, Method::Post);
        assert_eq!(draft_of("curl -I https://x.com").method, Method::Head);
        // 显式 -X 压过一切
        assert_eq!(
            draft_of("curl -X PUT https://x.com -d 'a=1'").method,
            Method::Put
        );
    }

    #[test]
    fn headers_and_auth() {
        let d = draft_of(
            r#"curl https://x.com -H 'X-Token: abc' -A 'GetCat/1' -e 'https://ref' -b 'k=v' -u 'user:pw'"#,
        );
        let get = |name: &str| {
            d.headers
                .iter()
                .find(|h| h.key.eq_ignore_ascii_case(name))
                .map(|h| h.value.as_str())
        };
        assert_eq!(get("X-Token"), Some("abc"));
        assert_eq!(get("User-Agent"), Some("GetCat/1"));
        assert_eq!(get("Referer"), Some("https://ref"));
        assert_eq!(get("Cookie"), Some("k=v"));
        // base64("user:pw")
        assert_eq!(get("Authorization"), Some("Basic dXNlcjpwdw=="));
    }

    #[test]
    fn empty_valued_header_uses_the_semicolon_form() {
        // curl 的 `-H 'X-Empty;'` 是「发一个空值头」
        let d = draft_of("curl https://x.com -H 'X-Empty;'");
        assert_eq!(d.headers, vec![KeyValue::new("X-Empty", "")]);
        // 而 `-H 'X-Drop:'` 是「删掉这个头」，GetCat 没有对应概念，忽略
        let d = draft_of("curl https://x.com -H 'X-Drop:'");
        assert!(d.headers.is_empty());
    }

    #[test]
    fn base64_pads_correctly() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
        assert_eq!(base64(b"user:pw"), "dXNlcjpwdw==");
    }

    #[test]
    fn query_in_the_url_becomes_params() {
        let d = draft_of("curl 'https://x.com/a?q=hello+world&n=1'");
        assert_eq!(d.url, "https://x.com/a");
        assert_eq!(
            d.params,
            vec![KeyValue::new("q", "hello world"), KeyValue::new("n", "1")]
        );
    }

    #[test]
    fn dash_g_moves_data_into_the_query() {
        let d = draft_of("curl -G https://x.com -d 'a=1' -d 'b=2'");
        assert_eq!(d.method, Method::Get, "-G 之后没有请求体，方法回到 GET");
        assert_eq!(d.body, BodyKind::None);
        assert_eq!(
            d.params,
            vec![KeyValue::new("a", "1"), KeyValue::new("b", "2")]
        );
    }

    #[test]
    fn multiple_data_flags_are_joined_with_ampersand() {
        // curl 的语义就是用 & 连起来
        let d = draft_of("curl https://x.com -d 'a=1' -d 'b=2'");
        assert_eq!(
            d.body,
            BodyKind::FormUrlEncoded {
                fields: vec![KeyValue::new("a", "1"), KeyValue::new("b", "2")]
            },
            "没有 Content-Type 时 curl 默认就是 urlencoded"
        );
    }

    #[test]
    fn json_body_keeps_its_format() {
        let d = draft_of(r#"curl https://x.com -H 'Content-Type: application/json' -d '{"a":1}'"#);
        assert_eq!(
            d.body,
            BodyKind::Raw {
                format: RawFormat::Json,
                text: r#"{"a":1}"#.into()
            }
        );
        // 没有 Content-Type 时看内容长相
        let d = draft_of(r#"curl https://x.com --data-raw '{"a":1}'"#);
        assert!(matches!(
            d.body,
            BodyKind::Raw {
                format: RawFormat::Json,
                ..
            }
        ));
    }

    #[test]
    fn json_flag_adds_both_headers() {
        let d = draft_of(r#"curl https://x.com --json '{"a":1}'"#);
        assert_eq!(d.method, Method::Post);
        assert!(
            d.headers
                .iter()
                .any(|h| h.key == "Content-Type" && h.value == "application/json")
        );
        assert!(d.headers.iter().any(|h| h.key == "Accept"));
    }

    #[test]
    fn form_fields_text_and_file() {
        let d = draft_of(
            "curl https://x.com -F 'note=hi' -F 'avatar=@/tmp/a.png;type=image/png' -F 'doc=@/tmp/d.pdf'",
        );
        let BodyKind::FormData { fields } = d.body else {
            panic!("应当是 form-data，实际 {:?}", d.body);
        };
        assert_eq!(fields[0], FormField::text("note", "hi"));
        assert_eq!(
            fields[1].value,
            FormValue::File {
                path: PathBuf::from("/tmp/a.png"),
                content_type: Some("image/png".into())
            }
        );
        assert_eq!(
            fields[2].value,
            FormValue::File {
                path: PathBuf::from("/tmp/d.pdf"),
                // 没写 ;type= 就交给发送时按扩展名猜
                content_type: None
            },
        );
        // --form-string 不解释 @
        let d = draft_of("curl https://x.com --form-string 'raw=@notafile'");
        let BodyKind::FormData { fields } = d.body else {
            panic!("应当是 form-data");
        };
        assert_eq!(fields[0], FormField::text("raw", "@notafile"));
    }

    #[test]
    fn data_binary_at_file_becomes_a_binary_body() {
        let d = draft_of("curl -X POST https://x.com --data-binary '@/tmp/payload.bin'");
        assert_eq!(
            d.body,
            BodyKind::Binary {
                path: PathBuf::from("/tmp/payload.bin"),
                content_type: None
            }
        );
        // --data-raw 里的 @ 是字面量，不读文件
        let d = draft_of("curl -X POST https://x.com --data-raw '@literal'");
        assert!(matches!(d.body, BodyKind::Raw { .. }));
    }

    #[test]
    fn data_urlencode_encodes_only_the_value() {
        let d = draft_of("curl -G https://x.com --data-urlencode 'q=a b&c'");
        assert_eq!(d.params, vec![KeyValue::new("q", "a%20b%26c")]);
    }

    #[test]
    fn runtime_options_are_reported_not_applied() {
        let r = parse("curl -k -L --compressed --max-time 30 https://x.com").unwrap();
        assert_eq!(
            r.draft.url, "https://x.com",
            "带值的运行时选项不能被当成 URL"
        );
        assert!(r.draft.headers.is_empty());
        assert!(
            r.warnings
                .iter()
                .all(|w| matches!(w, CurlWarning::RuntimeOption(_))),
            "{:?}",
            r.warnings
        );
        assert_eq!(r.warnings.len(), 4);
    }

    #[test]
    fn unknown_options_do_not_fail_the_whole_command() {
        // 认不出的选项只记一条警告：为了一个 --frobnicate 拒绝整条命令毫无道理
        let r = parse("curl --frobnicate https://x.com -H 'a: b'").unwrap();
        assert_eq!(r.draft.url, "https://x.com");
        assert_eq!(r.draft.headers, vec![KeyValue::new("a", "b")]);
        assert_eq!(
            r.warnings,
            vec![CurlWarning::Unknown("--frobnicate".into())]
        );
    }

    #[test]
    fn long_options_accept_the_equals_form() {
        let d = draft_of("curl --request=PUT --url=https://x.com --header='a: b'");
        assert_eq!(d.method, Method::Put);
        assert_eq!(d.url, "https://x.com");
        assert_eq!(d.headers, vec![KeyValue::new("a", "b")]);
    }

    #[test]
    fn url_placeholders_become_path_params() {
        let d = draft_of("curl 'https://x.com/users/{id}/posts/{postId}'");
        assert_eq!(
            d.path_params,
            vec![KeyValue::new("id", ""), KeyValue::new("postId", "")]
        );
    }

    /// 真实场景：Chrome DevTools 的「Copy as cURL」。
    #[test]
    fn parses_a_chrome_devtools_command() {
        let cmd = r#"curl 'https://api.example.com/v1/search?q=rust' \
  -H 'accept: application/json' \
  -H 'accept-language: en-US,en;q=0.9' \
  -H 'content-type: application/json' \
  -H $'cookie: session=abc\u003ddef' \
  --data-raw '{"page":1,"filters":["a","b"]}' \
  --compressed"#;
        let r = parse(cmd).unwrap();
        assert_eq!(r.draft.method, Method::Post);
        assert_eq!(r.draft.url, "https://api.example.com/v1/search");
        assert_eq!(r.draft.params, vec![KeyValue::new("q", "rust")]);
        assert_eq!(r.draft.headers.len(), 4);
        assert!(matches!(
            r.draft.body,
            BodyKind::Raw {
                format: RawFormat::Json,
                ..
            }
        ));
        assert_eq!(
            r.warnings,
            vec![CurlWarning::RuntimeOption("--compressed".into())]
        );
    }

    /// **这个模块最要紧的一条**：GetCat 自己生成的 curl 必须能原样读回来。
    /// 两种方言各跑一遍，覆盖各种请求体形态与需要转义的值。
    #[test]
    fn round_trips_through_codegen() {
        let cases = vec![
            RequestDraft {
                method: Method::Get,
                url: "https://api.example.com/users".into(),
                params: vec![KeyValue::new("q", "hello world"), KeyValue::new("n", "1")],
                headers: vec![KeyValue::new("X-Token", "abc")],
                ..Default::default()
            },
            RequestDraft {
                method: Method::Post,
                url: "https://api.example.com/users".into(),
                headers: vec![KeyValue::new("Content-Type", "application/json")],
                body: BodyKind::Raw {
                    format: RawFormat::Json,
                    text: r#"{"name":"it's \"quoted\"","path":"C:\\tmp"}"#.into(),
                },
                ..Default::default()
            },
            RequestDraft {
                method: Method::Put,
                url: "https://api.example.com/form".into(),
                headers: vec![KeyValue::new(
                    "Content-Type",
                    "application/x-www-form-urlencoded",
                )],
                body: BodyKind::FormUrlEncoded {
                    fields: vec![KeyValue::new("a", "1"), KeyValue::new("b", "2")],
                },
                ..Default::default()
            },
        ];

        for draft in cases {
            for target in [CodeTarget::Curl, CodeTarget::CurlWindows] {
                let cmd = codegen::generate(&draft, &[], target).expect("生成成功");
                let back = parse(&cmd)
                    .unwrap_or_else(|e| panic!("{target:?} 生成的命令读不回来：{e}\n{cmd}"))
                    .draft;

                assert_eq!(back.method, draft.method, "{target:?}\n{cmd}");
                // codegen 把 params 拼进了 URL，解析时又拆回参数表
                assert_eq!(back.params, draft.params, "{target:?}\n{cmd}");
                assert_eq!(back.url, draft.url, "{target:?}\n{cmd}");
                assert_eq!(back.body, draft.body, "{target:?}\n{cmd}");
                // 生成的命令带上了默认头，只断言我们自己写的那些还在
                for want in &draft.headers {
                    assert!(
                        back.headers.iter().any(|h| h.key.eq_ignore_ascii_case(&want.key)
                            && h.value == want.value),
                        "{target:?} 丢了 {}: {}\n{cmd}",
                        want.key,
                        want.value
                    );
                }
            }
        }
    }
}
