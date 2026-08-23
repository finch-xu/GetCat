//! Python 示例：`requests` 写法。
//!
//! 刻意只提供 `requests` 一种：标准库的 `http.client` 没有任何 multipart 支持，
//! form-data 请求得在生成的代码里手拼 boundary，长且脆弱；而 `requests` 的
//! `files=` 参数由库自己生成 boundary，同一份草稿的四种 Body 形态都能干净表达。

use crate::http::{HttpRequest, OutboundBody, OutboundPart, guess_content_type};

/// Python 字符串字面量：双引号包裹，反斜杠 / 引号 / 控制字符转义。
///
/// 刻意不用三引号：Body 里出现 `"""` 时三引号会提前收尾，而转义形态永远安全。
fn py_str(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '\\' => out.push_str(r"\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str(r"\n"),
            '\r' => out.push_str(r"\r"),
            '\t' => out.push_str(r"\t"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

/// header 字典，键值都用双引号。
fn header_dict(headers: &[(String, String)]) -> String {
    if headers.is_empty() {
        return "{}".to_string();
    }
    let rows: Vec<String> = headers
        .iter()
        .map(|(key, value)| format!("    {}: {}", py_str(key), py_str(value)))
        .collect();
    format!("{{\n{}\n}}", rows.join(",\n"))
}

/// 非 multipart 的 Body 表达式。返回 None 表示这次请求没有 Body。
///
/// 文件 Body 直接给出 file-like 对象：requests 会流式上传，不必先读进内存。
/// multipart 不在这里处理——它要同时产出 `data=` 与 `files=` 两个参数，在调用方展开。
fn payload_expr(body: &OutboundBody) -> Option<String> {
    match body {
        OutboundBody::Empty | OutboundBody::Multipart { .. } => None,
        OutboundBody::Bytes { data, .. } => Some(py_str(&String::from_utf8_lossy(data))),
        OutboundBody::File { path, .. } => Some(format!(
            "open({}, \"rb\")",
            py_str(&path.display().to_string())
        )),
    }
}

/// multipart 的一个文件字段在 requests 里的元组形态：
/// `("avatar", ("a.png", open("/tmp/a.png", "rb"), "image/png"))`。
fn requests_file_tuple(part: &OutboundPart) -> Option<String> {
    let OutboundPart::File {
        name,
        path,
        content_type,
    } = part
    else {
        return None;
    };
    // 与 `http::execute_with_threshold` 的 multipart 分支同款兜底
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".to_string());
    let mime = content_type
        .clone()
        .unwrap_or_else(|| guess_content_type(path).to_string());
    Some(format!(
        "({}, ({}, open({}, \"rb\"), {}))",
        py_str(name),
        py_str(&file_name),
        py_str(&path.display().to_string()),
        py_str(&mime),
    ))
}

pub(super) fn render_requests(req: &HttpRequest, headers: &[(String, String)]) -> String {
    // query 就留在 url 里（那是 `prepare` 的产物），不额外拆成 params=
    let (payload, files) = match &req.body {
        OutboundBody::Multipart { parts } => {
            // 文本字段进 data=，文件字段进 files=；requests 据此自己生成 boundary
            let texts: Vec<String> = parts
                .iter()
                .filter_map(|part| match part {
                    OutboundPart::Text { name, value } => {
                        Some(format!("{}: {}", py_str(name), py_str(value)))
                    }
                    OutboundPart::File { .. } => None,
                })
                .collect();
            let tuples: Vec<String> = parts.iter().filter_map(requests_file_tuple).collect();
            (
                format!("{{{}}}", texts.join(", ")),
                Some(format!("[{}]", tuples.join(", "))),
            )
        }
        other => (
            payload_expr(other).unwrap_or_else(|| "\"\"".to_string()),
            None,
        ),
    };

    let files_line = files
        .as_ref()
        .map(|f| format!("files = {f}\n"))
        .unwrap_or_default();
    let files_arg = if files.is_some() { ", files=files" } else { "" };

    format!(
        "import requests\n\n\
         url = {}\n\n\
         payload = {payload}\n\
         {files_line}\
         headers = {}\n\n\
         response = requests.request({}, url, data=payload{files_arg}, headers=headers)\n\n\
         print(response.text)\n",
        py_str(req.url.as_str()),
        header_dict(headers),
        py_str(req.method.as_str()),
    )
}
