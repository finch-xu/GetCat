//! SSE（text/event-stream）增量解析与大模型流的 delta 拼装。
//!
//! 解析器面向"逐 chunk 到达"的输入：chunk 的切口可能落在行中间甚至
//! 多字节字符中间，所以内部按字节缓冲，凑齐整行才解码。拼装器识别
//! 三种主流大模型流格式（OpenAI Chat Completions / OpenAI Responses /
//! Anthropic Messages），从事件里抽出增量文本与 token 用量。

use serde_json::Value;

/// Content-Type 是否为 SSE。参数是完整的头值（可带 `; charset=utf-8` 参数）。
pub fn is_sse(content_type: Option<&str>) -> bool {
    content_type
        .and_then(|ct| ct.split(';').next())
        .map(str::trim)
        .is_some_and(|mime| mime.eq_ignore_ascii_case("text/event-stream"))
}

/// 一个已派发的 SSE 事件。`id` / `retry` 字段对展示无用，解析时忽略。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    /// `event:` 字段；未指定时为 None（规范默认 "message"，展示层自行决定怎么标）。
    pub event: Option<String>,
    /// `data:` 各行以 `\n` 连接后的结果。
    pub data: String,
}

/// 增量 SSE 解析器：`push` 喂入任意切分的字节，返回本次凑齐的完整事件。
///
/// 行界按 `\n` 切（`\r\n` 会剥掉尾部 `\r`）；纯 `\r` 行尾在真实服务里
/// 几乎不存在，不支持。非 UTF-8 字节按 lossy 处理，不会让整个流失败。
#[derive(Debug, Default)]
pub struct SseParser {
    /// 尚未凑齐一行的字节余量。
    buf: Vec<u8>,
    event: Option<String>,
    data: Vec<String>,
}

impl SseParser {
    pub fn new() -> SseParser {
        SseParser::default()
    }

    pub fn push(&mut self, bytes: &[u8]) -> Vec<SseEvent> {
        self.buf.extend_from_slice(bytes);
        let mut out = Vec::new();
        let mut start = 0;
        while let Some(pos) = memchr::memchr(b'\n', &self.buf[start..]) {
            let mut line_end = start + pos;
            if line_end > start && self.buf[line_end - 1] == b'\r' {
                line_end -= 1;
            }
            let line = String::from_utf8_lossy(&self.buf[start..line_end]).into_owned();
            start += pos + 1;
            if let Some(ev) = self.take_line(&line) {
                out.push(ev);
            }
        }
        self.buf.drain(..start);
        out
    }

    /// 流结束：按规范未以空行收尾的事件应当丢弃，但真实服务偶尔漏掉最后
    /// 一个空行，宽容地把残留 data 也派发出来。
    pub fn finish(&mut self) -> Option<SseEvent> {
        if !self.buf.is_empty() {
            let line = String::from_utf8_lossy(&self.buf).into_owned();
            self.buf.clear();
            if let Some(ev) = self.take_line(&line) {
                return Some(ev);
            }
        }
        self.dispatch()
    }

    fn take_line(&mut self, line: &str) -> Option<SseEvent> {
        if line.is_empty() {
            return self.dispatch();
        }
        // `:` 开头是注释（常被用作 keep-alive）
        let (field, value) = match line.split_once(':') {
            Some((f, v)) => (f, v.strip_prefix(' ').unwrap_or(v)),
            None => (line, ""),
        };
        match field {
            "data" => self.data.push(value.to_string()),
            "event" => self.event = Some(value.to_string()),
            // id / retry / 注释（field 为空）/ 未知字段：忽略
            _ => {}
        }
        None
    }

    fn dispatch(&mut self) -> Option<SseEvent> {
        let event = self.event.take();
        if self.data.is_empty() {
            return None;
        }
        Some(SseEvent {
            event,
            data: std::mem::take(&mut self.data).join("\n"),
        })
    }
}

/// 一次性解析整份响应体（Done 视图的重放路径）。
pub fn parse_all(body: &[u8]) -> Vec<SseEvent> {
    let mut parser = SseParser::new();
    let mut events = parser.push(body);
    events.extend(parser.finish());
    events
}

/// 大模型流的 token 用量。两个字段独立可缺：Anthropic 的 input 在
/// message_start、output 在 message_delta，OpenAI 全在末块。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Usage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
}

impl Usage {
    pub fn is_empty(&self) -> bool {
        self.input_tokens.is_none() && self.output_tokens.is_none()
    }
}

/// delta 拼装器：把大模型流的增量文本事件拼成完整回复，顺带收集 usage。
///
/// 识别不了的事件（非 JSON、别家格式）静默跳过——拼不出文本时
/// `text` 为空，展示层退回事件视图。
#[derive(Debug, Default)]
pub struct LlmStream {
    pub text: String,
    pub usage: Usage,
}

impl LlmStream {
    pub fn new() -> LlmStream {
        LlmStream::default()
    }

    /// 喂入一个事件；返回它是否携带了非空的增量文本（TTFT 以此为准）。
    pub fn push(&mut self, event: &SseEvent) -> bool {
        if event.data == "[DONE]" {
            return false;
        }
        let Ok(v) = serde_json::from_str::<Value>(&event.data) else {
            return false;
        };
        self.merge_usage(&v);
        let delta = extract_delta(&v);
        match delta {
            Some(d) if !d.is_empty() => {
                self.text.push_str(d);
                true
            }
            _ => false,
        }
    }

    pub fn has_text(&self) -> bool {
        !self.text.is_empty()
    }

    fn merge_usage(&mut self, v: &Value) {
        // OpenAI Chat Completions：末块 `usage`（需 stream_options.include_usage）
        let openai = &v["usage"];
        // OpenAI Responses：response.completed 事件的 `response.usage`
        let responses = &v["response"]["usage"];
        // Anthropic：message_start 的 `message.usage`（input），message_delta 的 `usage`（output）
        let anthropic_start = &v["message"]["usage"];
        for u in [openai, responses, anthropic_start] {
            if let Some(n) = u["prompt_tokens"].as_u64().or(u["input_tokens"].as_u64()) {
                self.usage.input_tokens = Some(n);
            }
            if let Some(n) = u["completion_tokens"]
                .as_u64()
                .or(u["output_tokens"].as_u64())
            {
                self.usage.output_tokens = Some(n);
            }
        }
    }
}

/// 从一个流事件的 JSON 里抽出增量文本；认不出格式时 None。
fn extract_delta(v: &Value) -> Option<&str> {
    // OpenAI Chat Completions：choices[0].delta.content
    if let Some(s) = v["choices"][0]["delta"]["content"].as_str() {
        return Some(s);
    }
    match v["type"].as_str() {
        // OpenAI Responses：response.output_text.delta 的顶层 delta 字符串
        Some("response.output_text.delta") => v["delta"].as_str(),
        // Anthropic：content_block_delta 的 delta.text（thinking 流是 delta.thinking，不拼）
        Some("content_block_delta") => v["delta"]["text"].as_str(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(event: Option<&str>, data: &str) -> SseEvent {
        SseEvent {
            event: event.map(str::to_string),
            data: data.to_string(),
        }
    }

    #[test]
    fn detects_sse_content_type() {
        assert!(is_sse(Some("text/event-stream")));
        assert!(is_sse(Some("text/event-stream; charset=utf-8")));
        assert!(is_sse(Some("Text/Event-Stream")));
        assert!(!is_sse(Some("application/json")));
        assert!(!is_sse(None));
    }

    #[test]
    fn parses_basic_events() {
        let mut p = SseParser::new();
        let events = p.push(b"data: hello\n\ndata: world\n\n");
        assert_eq!(events, vec![ev(None, "hello"), ev(None, "world")]);
    }

    #[test]
    fn joins_multi_line_data_and_reads_event_field() {
        let mut p = SseParser::new();
        let events = p.push(b"event: message_start\ndata: {\"a\":\ndata: 1}\n\n");
        assert_eq!(events, vec![ev(Some("message_start"), "{\"a\":\n1}")]);
        // event 字段不跨事件残留
        assert_eq!(p.push(b"data: x\n\n"), vec![ev(None, "x")]);
    }

    #[test]
    fn handles_crlf_comments_and_unknown_fields() {
        let mut p = SseParser::new();
        let events = p.push(b": keep-alive\r\nid: 7\r\nretry: 100\r\ndata: hi\r\n\r\n");
        assert_eq!(events, vec![ev(None, "hi")]);
    }

    #[test]
    fn survives_arbitrary_chunk_boundaries() {
        // 同一份流按每 3 字节一切喂入，结果必须与整体喂入一致；
        // 切口会落在字段名、UTF-8 多字节字符与空行中间
        let stream = "event: delta\ndata: 你好\n\ndata: 世界\ndata: !\n\n".as_bytes();
        let whole = parse_all(stream);
        let mut p = SseParser::new();
        let mut chunked = Vec::new();
        for c in stream.chunks(3) {
            chunked.extend(p.push(c));
        }
        chunked.extend(p.finish());
        assert_eq!(whole, chunked);
        assert_eq!(whole[0], ev(Some("delta"), "你好"));
        assert_eq!(whole[1], ev(None, "世界\n!"));
    }

    #[test]
    fn data_without_space_and_bare_field_names() {
        let mut p = SseParser::new();
        // 冒号后无空格；"data" 独行等价于空 data 行
        let events = p.push(b"data:tight\ndata\n\n");
        assert_eq!(events, vec![ev(None, "tight\n")]);
    }

    #[test]
    fn finish_flushes_unterminated_event() {
        let mut p = SseParser::new();
        assert!(p.push(b"data: tail").is_empty());
        assert_eq!(p.finish(), Some(ev(None, "tail")));
        // 再次 finish 不重复派发
        assert_eq!(p.finish(), None);
    }

    #[test]
    fn empty_event_dispatches_nothing() {
        let mut p = SseParser::new();
        assert!(p.push(b"event: ping\n\n: comment\n\n").is_empty());
    }

    #[test]
    fn assembles_openai_chat_deltas() {
        let mut s = LlmStream::new();
        assert!(s.push(&ev(None, r#"{"choices":[{"delta":{"content":"Hel"}}]}"#)));
        assert!(s.push(&ev(None, r#"{"choices":[{"delta":{"content":"lo"}}]}"#)));
        // 末块：delta 为空对象、usage 到达
        assert!(!s.push(&ev(
            None,
            r#"{"choices":[],"usage":{"prompt_tokens":12,"completion_tokens":34}}"#
        )));
        assert!(!s.push(&ev(None, "[DONE]")));
        assert_eq!(s.text, "Hello");
        assert_eq!(s.usage.input_tokens, Some(12));
        assert_eq!(s.usage.output_tokens, Some(34));
    }

    #[test]
    fn assembles_openai_responses_deltas() {
        let mut s = LlmStream::new();
        assert!(s.push(&ev(
            Some("response.output_text.delta"),
            r#"{"type":"response.output_text.delta","delta":"Hi"}"#
        )));
        assert!(!s.push(&ev(
            Some("response.completed"),
            r#"{"type":"response.completed","response":{"usage":{"input_tokens":5,"output_tokens":7}}}"#
        )));
        assert_eq!(s.text, "Hi");
        assert_eq!(
            s.usage,
            Usage {
                input_tokens: Some(5),
                output_tokens: Some(7)
            }
        );
    }

    #[test]
    fn assembles_anthropic_deltas_and_split_usage() {
        let mut s = LlmStream::new();
        assert!(!s.push(&ev(
            Some("message_start"),
            r#"{"type":"message_start","message":{"usage":{"input_tokens":9,"output_tokens":1}}}"#
        )));
        assert!(s.push(&ev(
            Some("content_block_delta"),
            r#"{"type":"content_block_delta","delta":{"type":"text_delta","text":"喵"}}"#
        )));
        assert!(!s.push(&ev(
            Some("message_delta"),
            r#"{"type":"message_delta","usage":{"output_tokens":42}}"#
        )));
        assert_eq!(s.text, "喵");
        assert_eq!(s.usage.input_tokens, Some(9));
        assert_eq!(s.usage.output_tokens, Some(42));
    }

    #[test]
    fn unknown_payloads_are_skipped() {
        let mut s = LlmStream::new();
        assert!(!s.push(&ev(None, "not json")));
        assert!(!s.push(&ev(None, r#"{"jsonrpc":"2.0","result":{}}"#)));
        assert!(!s.push(&ev(None, r#"{"choices":[{"delta":{"content":""}}]}"#)));
        assert!(!s.has_text());
        assert!(s.usage.is_empty());
    }
}
