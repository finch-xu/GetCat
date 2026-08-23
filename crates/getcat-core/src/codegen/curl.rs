//! curl 命令：POSIX shell 与 Windows cmd 两种方言。

use crate::http::{HttpRequest, OutboundBody, OutboundPart, guess_content_type};

/// 目标 shell。两种方言的参数完全一样，区别只在引用规则与续行符。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Shell {
    Posix,
    WindowsCmd,
}

/// 续行：POSIX 用 `\`，cmd 用 `^`；后接换行与两空格缩进。
fn continuation(shell: Shell) -> &'static str {
    match shell {
        Shell::Posix => " \\\n  ",
        Shell::WindowsCmd => " ^\n  ",
    }
}

pub(super) fn render(req: &HttpRequest, headers: &[(String, String)], shell: Shell) -> String {
    let cont = continuation(shell);
    // 始终写出 -X：只给 -d 而不给 -X 时 curl 会强行发成 POST，PUT / PATCH 会被悄悄改掉。
    let mut parts = vec![format!(
        "curl -X {} {}",
        req.method.as_str(),
        quote(req.url.as_str(), shell)
    )];

    for (key, value) in headers {
        parts.push(format!("-H {}", quote(&format!("{key}: {value}"), shell)));
    }

    match &req.body {
        OutboundBody::Empty => {}
        OutboundBody::Bytes { data, .. } => {
            let text = String::from_utf8_lossy(data);
            parts.push(format!("-d {}", quote(&body_text(&text, shell), shell)));
        }
        OutboundBody::Multipart { parts: fields } => {
            for field in fields {
                parts.push(format!("-F {}", quote(&form_field(field), shell)));
            }
        }
        OutboundBody::File { path, .. } => {
            parts.push(format!(
                "--data-binary {}",
                quote(&format!("@{}", path.display()), shell)
            ));
        }
    }

    parts.join(cont)
}

/// 一个 `-F` 参数：文本是 `k=v`，文件是 `k=@/path;type=image/png`
/// （content_type 为 None 时省略 `;type=`，交给 curl 自己按扩展名猜）。
fn form_field(part: &OutboundPart) -> String {
    match part {
        OutboundPart::Text { name, value } => format!("{name}={value}"),
        OutboundPart::File {
            name,
            path,
            content_type,
        } => {
            let mime = content_type
                .clone()
                .unwrap_or_else(|| guess_content_type(path).to_string());
            format!("{name}=@{};type={mime}", path.display())
        }
    }
}

/// cmd 的双引号字符串不能跨行，Body 里的换行只能折成空格。
/// 对 JSON / urlencoded 无损；raw text 与 XML 会丢掉排版，这是 cmd 的固有限制。
fn body_text(text: &str, shell: Shell) -> String {
    match shell {
        Shell::Posix => text.to_string(),
        Shell::WindowsCmd => text.replace("\r\n", " ").replace(['\n', '\r'], " "),
    }
}

fn quote(value: &str, shell: Shell) -> String {
    match shell {
        Shell::Posix => quote_posix(value),
        Shell::WindowsCmd => quote_windows(value),
    }
}

/// 单引号包裹；单引号本身只能先关引号、转义、再开引号（`'\''`）。
fn quote_posix(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

/// 双引号包裹，按 Windows CRT 的命令行规则转义：
/// `"` 前的连续反斜杠要加倍再写 `\"`，结尾的连续反斜杠也要加倍
/// （否则它们会把收尾的引号转义掉）；孤立的反斜杠是字面量，不动。
/// `%` 加倍以抵消 cmd 的 `%VAR%` 展开。
fn quote_windows(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    let mut backslashes = 0usize;
    for c in value.chars() {
        match c {
            '\\' => {
                backslashes += 1;
                out.push('\\');
            }
            '"' => {
                for _ in 0..backslashes {
                    out.push('\\');
                }
                out.push('\\');
                out.push('"');
                backslashes = 0;
            }
            '%' => {
                backslashes = 0;
                out.push_str("%%");
            }
            _ => {
                backslashes = 0;
                out.push(c);
            }
        }
    }
    for _ in 0..backslashes {
        out.push('\\');
    }
    out.push('"');
    out
}
