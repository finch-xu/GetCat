//! 把一份 [`RequestDraft`] 转成可直接运行的示例代码（curl / Python）。
//!
//! 生成的代码必须**等于 GetCat 真正发出去的请求**，所以这里不重写任何拼装逻辑：
//! URL 合成、路径参数替换、query 拼接、header 校验、Body 编码全部走
//! [`crate::http::prepare`]，本模块只负责两件事——
//!
//! 1. 按 reqwest 的真实合并语义补齐 header（见 [`merge_headers`]）；
//! 2. 把结果渲染成目标语言的字符串。
//!
//! 刻意**不**生成 TLS 校验、重定向、超时相关的参数：那些是 GetCat 的运行时设置，
//! 不属于「这条请求是什么」，带进代码只会让命令行变长而不变准。

mod curl;
mod python;

use crate::http::{
    DEFAULT_HEADERS, HttpRequest, OutboundBody, RequestError, default_header_enabled, prepare,
};
use crate::model::RequestDraft;

/// URL 还没填时代入的占位。用 RFC 2606 保留给文档示例的域名，
/// 一眼能看出是占位、又不会指向任何真实主机。
pub const PLACEHOLDER_URL: &str = "https://api.example.com/path";

/// 生成目标。判别值即界面上分段控件的位置，`ALL` 的顺序必须与变体声明顺序一致（有测试钉住）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CodeTarget {
    #[default]
    Curl,
    CurlWindows,
    PythonRequests,
}

impl CodeTarget {
    pub const ALL: [CodeTarget; 3] = [
        CodeTarget::Curl,
        CodeTarget::CurlWindows,
        CodeTarget::PythonRequests,
    ];

    /// `ALL` 里的下标，就是分段控件上的位置（与 [`crate::model::RawFormat::index`] 对称）。
    pub fn index(self) -> usize {
        Self::ALL.iter().position(|t| *t == self).unwrap_or(0)
    }

    pub fn from_index(ix: usize) -> Self {
        Self::ALL.get(ix).copied().unwrap_or(CodeTarget::Curl)
    }

    /// 技术专名，照 `templates::TemplateGroup::label` 的先例不进 i18n。
    pub fn label(self) -> &'static str {
        match self {
            CodeTarget::Curl => "cURL",
            CodeTarget::CurlWindows => "cURL (Windows)",
            CodeTarget::PythonRequests => "Python (requests)",
        }
    }

    /// 只读编辑器使用的语法高亮 id（与 [`crate::model::RawFormat::editor_language`] 对称）。
    pub fn editor_language(self) -> &'static str {
        match self {
            CodeTarget::Curl | CodeTarget::CurlWindows => "bash",
            CodeTarget::PythonRequests => "python",
        }
    }
}

/// 把一份草稿转成目标语言的代码。
///
/// `disabled_default_headers` 直接透传 [`crate::model::RequestSettings::disabled_default_headers`]；
/// 刻意只收这一个字段而不是整份设置，免得后来者顺手把 TLS / 超时也读进来。
///
/// URL 为空或非法、header 非法、form-data 未选文件都由 `prepare` 以 [`RequestError`] 形式返回。
pub fn generate(
    draft: &RequestDraft,
    disabled_default_headers: &[String],
    target: CodeTarget,
) -> Result<String, RequestError> {
    // URL 还没填是新建 Tab 的正常状态，不该当错误报——代入占位先给出骨架，
    // 用户至少能看到目标语言长什么样。填错了的（非法 scheme、非法 header、
    // form-data 没选文件）仍旧照常报错，那是真的需要他去改。
    let placeholder;
    let draft = if draft.url.trim().is_empty() {
        placeholder = RequestDraft {
            url: PLACEHOLDER_URL.to_string(),
            ..draft.clone()
        };
        &placeholder
    } else {
        draft
    };

    let req = prepare(draft)?;
    let headers = merge_headers(&req, disabled_default_headers);
    Ok(match target {
        CodeTarget::Curl => curl::render(&req, &headers, curl::Shell::Posix),
        CodeTarget::CurlWindows => curl::render(&req, &headers, curl::Shell::WindowsCmd),
        CodeTarget::PythonRequests => python::render_requests(&req, &headers),
    })
}

/// 这次请求实际会发出去的完整 header 列表，顺序与 reqwest 的行为一致
/// （见 `http::execute_with_threshold` 里的同名处理）：
///
/// 1. 用户自己填的头，但丢掉 `Content-Length` / `Transfer-Encoding` / `Host`
///    （由 hyper 按实际 body 与连接计算）；body 是 multipart 时连 `Content-Type` 也丢掉
///    （boundary 只能由库生成）。
/// 2. 用户没填 `Content-Type` 时，补上 Body 自带的那个。
/// 3. 最后补默认头：跳过被用户关掉的，也跳过用户已经写过的同名头
///    （reqwest 以 vacant-entry 语义合并 client 级默认头）。
fn merge_headers(req: &HttpRequest, disabled: &[String]) -> Vec<(String, String)> {
    let is_multipart = matches!(req.body, OutboundBody::Multipart { .. });
    let mut out: Vec<(String, String)> =
        Vec::with_capacity(req.headers.len() + DEFAULT_HEADERS.len());
    let mut has_content_type = false;

    for (key, value) in &req.headers {
        if key.eq_ignore_ascii_case("content-type") {
            if is_multipart {
                continue;
            }
            has_content_type = true;
        }
        if key.eq_ignore_ascii_case("content-length")
            || key.eq_ignore_ascii_case("transfer-encoding")
            || key.eq_ignore_ascii_case("host")
        {
            continue;
        }
        out.push((key.clone(), value.clone()));
    }

    if !has_content_type {
        // `OutboundBody::File` 只在 content_type 是 Some 时才发这个头（binary 且用户没选类型
        // 时 GetCat 压根不发 Content-Type），这里如实复刻。
        let body_content_type = match &req.body {
            OutboundBody::Bytes { content_type, .. } => Some(content_type.clone()),
            OutboundBody::File { content_type, .. } => content_type.clone(),
            OutboundBody::Empty | OutboundBody::Multipart { .. } => None,
        };
        if let Some(ct) = body_content_type {
            out.push(("Content-Type".to_string(), ct));
        }
    }

    for (key, value) in DEFAULT_HEADERS {
        if !default_header_enabled(disabled, key) {
            continue;
        }
        if out.iter().any(|(k, _)| k.eq_ignore_ascii_case(key)) {
            continue;
        }
        out.push((key.to_string(), value.to_string()));
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::USER_AGENT_VALUE;
    use crate::model::{BodyKind, FormField, KeyValue, Method, RawFormat};
    use std::path::PathBuf;

    /// 一份「什么都有」的草稿：path 参数、query、自定义 header、JSON body。
    fn full_draft() -> RequestDraft {
        RequestDraft {
            method: Method::Post,
            url: "https://api.example.com/v1/users/{id}".into(),
            path_params: vec![KeyValue::new("id", "42")],
            params: vec![KeyValue::new("page", "2")],
            headers: vec![KeyValue::new("Authorization", "Bearer sk-test")],
            body: BodyKind::Raw {
                format: RawFormat::Json,
                text: "{\n  \"name\": \"cat\"\n}".into(),
            },
        }
    }

    #[test]
    fn curl_renders_method_url_headers_and_body() {
        let code = generate(&full_draft(), &[], CodeTarget::Curl).unwrap();
        assert_eq!(
            code,
            format!(
                "curl -X POST 'https://api.example.com/v1/users/42?page=2' \\\n  \
                 -H 'Authorization: Bearer sk-test' \\\n  \
                 -H 'Content-Type: application/json' \\\n  \
                 -H 'Accept: */*' \\\n  \
                 -H 'Accept-Encoding: gzip, br, zstd' \\\n  \
                 -H 'User-Agent: {USER_AGENT_VALUE}' \\\n  \
                 -H 'Connection: keep-alive' \\\n  \
                 -d '{{\n  \"name\": \"cat\"\n}}'"
            )
        );
    }

    #[test]
    fn python_requests_keeps_the_query_in_the_url() {
        let code = generate(&full_draft(), &[], CodeTarget::PythonRequests).unwrap();
        assert_eq!(
            code,
            format!(
                "import requests\n\n\
                 url = \"https://api.example.com/v1/users/42?page=2\"\n\n\
                 payload = \"{{\\n  \\\"name\\\": \\\"cat\\\"\\n}}\"\n\
                 headers = {{\n\
                 \x20   \"Authorization\": \"Bearer sk-test\",\n\
                 \x20   \"Content-Type\": \"application/json\",\n\
                 \x20   \"Accept\": \"*/*\",\n\
                 \x20   \"Accept-Encoding\": \"gzip, br, zstd\",\n\
                 \x20   \"User-Agent\": \"{USER_AGENT_VALUE}\",\n\
                 \x20   \"Connection\": \"keep-alive\"\n\
                 }}\n\n\
                 response = requests.request(\"POST\", url, data=payload, headers=headers)\n\n\
                 print(response.text)\n"
            )
        );
    }

    #[test]
    fn disabled_default_headers_are_left_out() {
        let disabled = vec!["user-agent".to_string(), "connection".to_string()];
        let code = generate(&full_draft(), &disabled, CodeTarget::Curl).unwrap();
        assert!(!code.contains("User-Agent"), "{code}");
        assert!(!code.contains("Connection"), "{code}");
        assert!(code.contains("-H 'Accept: */*'"), "{code}");
    }

    /// reqwest 以 vacant-entry 语义合并 client 级默认头，请求自己填的同名头天然优先。
    #[test]
    fn user_headers_win_over_the_defaults() {
        let mut draft = full_draft();
        draft
            .headers
            .push(KeyValue::new("User-Agent", "my-agent/1.0"));
        draft
            .headers
            .push(KeyValue::new("Content-Type", "application/vnd.api+json"));
        let code = generate(&draft, &[], CodeTarget::Curl).unwrap();
        assert!(code.contains("-H 'User-Agent: my-agent/1.0'"), "{code}");
        assert_eq!(code.matches("User-Agent").count(), 1, "{code}");
        assert!(
            code.contains("-H 'Content-Type: application/vnd.api+json'"),
            "{code}"
        );
        assert_eq!(code.matches("Content-Type").count(), 1, "{code}");
    }

    /// Content-Length / Transfer-Encoding / Host 由 hyper 按实际 body 与连接计算，
    /// GetCat 发送时就把用户填的丢掉了，生成的代码同样不能带。
    #[test]
    fn hop_by_hop_headers_are_dropped() {
        let mut draft = full_draft();
        draft.headers.push(KeyValue::new("Content-Length", "999"));
        draft
            .headers
            .push(KeyValue::new("Host", "spoofed.example.com"));
        draft
            .headers
            .push(KeyValue::new("Transfer-Encoding", "chunked"));
        let code = generate(&draft, &[], CodeTarget::Curl).unwrap();
        assert!(!code.contains("Content-Length"), "{code}");
        assert!(!code.contains("Host"), "{code}");
        assert!(!code.contains("Transfer-Encoding"), "{code}");
    }

    #[test]
    fn multipart_drops_the_user_content_type_because_only_the_client_knows_the_boundary() {
        let mut draft = full_draft();
        draft
            .headers
            .push(KeyValue::new("Content-Type", "multipart/form-data"));
        draft.body = BodyKind::FormData {
            fields: vec![FormField::text("note", "hi")],
        };
        let code = generate(&draft, &[], CodeTarget::Curl).unwrap();
        assert!(!code.contains("Content-Type"), "{code}");
        assert!(code.contains("-F 'note=hi'"), "{code}");
    }

    #[test]
    fn posix_curl_escapes_single_quotes() {
        let mut draft = full_draft();
        draft.headers = vec![KeyValue::new("X-Note", "it's fine")];
        let code = generate(&draft, &[], CodeTarget::Curl).unwrap();
        assert!(code.contains(r"-H 'X-Note: it'\''s fine'"), "{code}");
    }

    #[test]
    fn windows_curl_uses_caret_continuations_and_a_single_line_body() {
        let code = generate(&full_draft(), &[], CodeTarget::CurlWindows).unwrap();
        assert!(code.contains(" ^\n"), "应当用 ^ 续行：{code}");
        assert!(!code.contains(" \\\n"), "不该出现 POSIX 的续行：{code}");
        assert!(
            code.contains("-d \"{   \\\"name\\\": \\\"cat\\\" }\""),
            "{code}"
        );
    }

    /// cmd 会展开 `%VAR%`，写进命令行的百分号必须加倍；孤立的反斜杠是字面量，不动。
    #[test]
    fn windows_curl_doubles_percent_signs() {
        let mut draft = full_draft();
        draft.headers = vec![KeyValue::new("X-Path", "%TEMP%\\out")];
        let code = generate(&draft, &[], CodeTarget::CurlWindows).unwrap();
        assert!(code.contains("-H \"X-Path: %%TEMP%%\\out\""), "{code}");
    }

    #[test]
    fn binary_body_becomes_a_file_reference() {
        let mut draft = full_draft();
        draft.body = BodyKind::Binary {
            path: PathBuf::from("/tmp/photo.png"),
            content_type: Some("image/png".into()),
        };
        let code = generate(&draft, &[], CodeTarget::Curl).unwrap();
        assert!(code.contains("--data-binary '@/tmp/photo.png'"), "{code}");
        assert!(code.contains("-H 'Content-Type: image/png'"), "{code}");
    }

    /// `OutboundBody::File` 只在 content_type 是 Some 时才补这个头——没选类型时
    /// GetCat 自己也不发 Content-Type，代码里同样不该凭空冒出来。
    #[test]
    fn binary_body_without_a_type_sends_no_content_type() {
        let mut draft = full_draft();
        draft.body = BodyKind::Binary {
            path: PathBuf::from("/tmp/blob.bin"),
            content_type: None,
        };
        let code = generate(&draft, &[], CodeTarget::Curl).unwrap();
        assert!(!code.contains("Content-Type"), "{code}");
    }

    #[test]
    fn form_data_files_carry_the_guessed_type_in_curl() {
        let mut draft = full_draft();
        draft.body = BodyKind::FormData {
            fields: vec![
                FormField::text("note", "hi"),
                FormField::file("avatar", PathBuf::from("/tmp/a.png")),
            ],
        };
        let code = generate(&draft, &[], CodeTarget::Curl).unwrap();
        assert!(code.contains("-F 'note=hi'"), "{code}");
        assert!(
            code.contains("-F 'avatar=@/tmp/a.png;type=image/png'"),
            "{code}"
        );
    }

    #[test]
    fn requests_uses_files_for_form_data() {
        let mut draft = full_draft();
        draft.body = BodyKind::FormData {
            fields: vec![
                FormField::text("note", "hi"),
                FormField::file("avatar", PathBuf::from("/tmp/a.png")),
            ],
        };
        let code = generate(&draft, &[], CodeTarget::PythonRequests).unwrap();
        assert!(code.contains("payload = {\"note\": \"hi\"}"), "{code}");
        assert!(
            code.contains(
                "files = [(\"avatar\", (\"a.png\", open(\"/tmp/a.png\", \"rb\"), \"image/png\"))]"
            ),
            "{code}"
        );
        assert!(
            code.contains("data=payload, files=files, headers=headers"),
            "{code}"
        );
    }

    /// requests 接受 file-like 对象并流式上传，不必先 read() 进内存。
    #[test]
    fn requests_streams_a_binary_body() {
        let mut draft = full_draft();
        draft.body = BodyKind::Binary {
            path: PathBuf::from("/tmp/blob.bin"),
            content_type: None,
        };
        let code = generate(&draft, &[], CodeTarget::PythonRequests).unwrap();
        assert!(
            code.contains("payload = open(\"/tmp/blob.bin\", \"rb\")\n"),
            "{code}"
        );
        assert!(!code.contains(".read()"), "requests 不必先读进内存：{code}");
    }

    #[test]
    fn targets_round_trip_through_their_index() {
        for (ix, target) in CodeTarget::ALL.iter().enumerate() {
            assert_eq!(target.index(), ix);
            assert_eq!(CodeTarget::from_index(ix), *target);
        }
        assert_eq!(CodeTarget::from_index(99), CodeTarget::Curl);
        assert_eq!(CodeTarget::default(), CodeTarget::Curl);
    }

    #[test]
    fn each_target_names_a_highlighter_language() {
        assert_eq!(CodeTarget::Curl.editor_language(), "bash");
        assert_eq!(CodeTarget::CurlWindows.editor_language(), "bash");
        assert_eq!(CodeTarget::PythonRequests.editor_language(), "python");
    }

    /// URL 还没填是新建 Tab 的正常状态，不是错误：给一段能看懂结构的骨架，
    /// 让用户先知道目标语言长什么样。
    #[test]
    fn an_empty_url_falls_back_to_a_placeholder_skeleton() {
        let code = generate(&RequestDraft::default(), &[], CodeTarget::Curl).unwrap();
        assert!(
            code.starts_with(&format!("curl -X GET '{PLACEHOLDER_URL}'")),
            "{code}"
        );
        assert!(code.contains("-H 'Accept: */*'"), "默认头照常带上：{code}");

        let python = generate(&RequestDraft::default(), &[], CodeTarget::PythonRequests).unwrap();
        assert!(
            python.contains(&format!("url = \"{PLACEHOLDER_URL}\"")),
            "{python}"
        );
    }

    /// 只有「空 URL」享受骨架待遇；真填错了的还是要报出来。
    #[test]
    fn a_malformed_url_still_reports_an_error() {
        let draft = RequestDraft {
            url: "ftp://files.example.com".into(),
            ..Default::default()
        };
        let err = generate(&draft, &[], CodeTarget::Curl).unwrap_err();
        assert!(matches!(err, RequestError::InvalidUrl(_)), "{err:?}");
    }

    /// header 填错了不是「还没填」，照常报错。
    #[test]
    fn an_invalid_header_still_reports_an_error() {
        let draft = RequestDraft {
            url: "https://x.com/p".into(),
            headers: vec![KeyValue::new("Bad Header", "v")],
            ..Default::default()
        };
        let err = generate(&draft, &[], CodeTarget::Curl).unwrap_err();
        assert!(matches!(err, RequestError::InvalidHeader(_)), "{err:?}");
    }

    #[test]
    fn a_get_without_a_body_has_no_data_flag() {
        let draft = RequestDraft {
            url: "https://x.com/ping".into(),
            ..Default::default()
        };
        let code = generate(&draft, &[], CodeTarget::Curl).unwrap();
        assert!(
            code.starts_with("curl -X GET 'https://x.com/ping'"),
            "{code}"
        );
        assert!(!code.contains("-d "), "{code}");
    }

    /// 把 Windows 变体的命令行按真实规则还原成参数列表：cmd 先把 `%%` 展开回 `%`，
    /// 再由 CRT 按「`2n` 个反斜杠 + `"` = n 个反斜杠并切换引号态；`2n+1` 个 = n 个反斜杠加一个
    /// 字面引号」拆分 argv。用它在非 Windows 机器上验证引用是否真的能被解回原值。
    fn parse_windows_command_line(line: &str) -> Vec<String> {
        let expanded = line.replace("%%", "%");
        let mut args = Vec::new();
        let mut current = String::new();
        let mut in_quotes = false;
        let mut started = false;
        let mut backslashes = 0usize;
        let flush_backslashes = |current: &mut String, n: &mut usize| {
            for _ in 0..*n {
                current.push('\\');
            }
            *n = 0;
        };

        for c in expanded.chars() {
            match c {
                '\\' => {
                    backslashes += 1;
                    started = true;
                }
                '"' => {
                    let literal = backslashes % 2 == 1;
                    backslashes /= 2;
                    flush_backslashes(&mut current, &mut backslashes);
                    if literal {
                        current.push('"');
                    } else {
                        in_quotes = !in_quotes;
                    }
                    started = true;
                }
                c if c.is_whitespace() && !in_quotes => {
                    flush_backslashes(&mut current, &mut backslashes);
                    if started {
                        args.push(std::mem::take(&mut current));
                        started = false;
                    }
                }
                c => {
                    flush_backslashes(&mut current, &mut backslashes);
                    current.push(c);
                    started = true;
                }
            }
        }
        flush_backslashes(&mut current, &mut backslashes);
        if started {
            args.push(current);
        }
        args
    }

    /// 引号、反斜杠、百分号、空格全挤在一个 header 里——cmd 展开 + CRT 拆分之后
    /// 必须一字不差地还原。这条在 macOS / Linux 上跑，替代跑不了的真 cmd。
    #[test]
    fn windows_quoting_survives_the_cmd_and_crt_rules() {
        let messy = r#"it's "quoted" 100% C:\tmp\a b"#;
        let draft = RequestDraft {
            url: "https://x.com/p".into(),
            headers: vec![KeyValue::new("X-Note", messy)],
            ..Default::default()
        };
        let line = generate(&draft, &[], CodeTarget::CurlWindows)
            .unwrap()
            .replace(" ^\n  ", " ");

        let args = parse_windows_command_line(&line);
        assert_eq!(args[0], "curl");
        assert_eq!(args[1], "-X");
        assert_eq!(args[2], "GET");
        assert_eq!(args[3], "https://x.com/p");
        assert!(
            args.contains(&format!("X-Note: {messy}")),
            "还原出来的参数对不上：{args:#?}"
        );
    }

    /// 结尾的反斜杠若不加倍，会把收尾的双引号转义掉、把后面的参数一起吞进来。
    #[test]
    fn windows_quoting_handles_a_trailing_backslash() {
        let draft = RequestDraft {
            url: "https://x.com/p".into(),
            headers: vec![KeyValue::new("X-Dir", r"C:\logs\")],
            ..Default::default()
        };
        let line = generate(&draft, &[], CodeTarget::CurlWindows)
            .unwrap()
            .replace(" ^\n  ", " ");
        let args = parse_windows_command_line(&line);
        assert!(args.contains(&r"X-Dir: C:\logs\".to_string()), "{args:#?}");
        // 反斜杠没吞掉引号：后面的默认头仍是独立参数
        assert!(args.contains(&"Accept: */*".to_string()), "{args:#?}");
    }
}
