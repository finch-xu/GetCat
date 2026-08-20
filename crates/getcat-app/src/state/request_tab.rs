//! 一个请求 Tab 的全部状态：输入组件实体、响应状态、视图选择。

use std::time::{Duration, Instant};

use getcat_core::http::{self, HttpResponse, RequestError};
use getcat_core::model::{BodyKind, Method, RawFormat, RequestDraft};
use getcat_core::url::extract_path_params;
// 显式导入而非 `use gpui::*`：本文件内 `#[cfg(test)] mod tests { use super::*; #[test] .. }`
// 若通过通配符引入 `gpui::test`（gpui 重导出的 `#[proc_macro_attribute]`），会与标准库的
// `#[test]` 属性同名冲突，导致该属性宏对自身生成的 `#[test]` 反复展开直至递归上限溢出。
use gpui::{
    App, AppContext, Context, Entity, IntoElement, ParentElement, Render, SharedString, Styled,
    Subscription, Window, div, px,
};
use gpui_component::IndexPath;
use gpui_component::input::{EditorState, InputEvent, InputState};
use gpui_component::resizable::{resizable_panel, v_resizable};
use gpui_component::select::SelectState;
use gpui_component::v_flex;

use tokio::sync::mpsc;

use crate::bridge;
use crate::state::response::{ResponseState, ResponseView};
use crate::ui::kv_table::{KvTable, KvTableEvent};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestSection {
    Params,
    Headers,
    Body,
}

impl RequestSection {
    pub const ALL: [RequestSection; 3] = [
        RequestSection::Params,
        RequestSection::Headers,
        RequestSection::Body,
    ];
    pub fn index(self) -> usize {
        Self::ALL.iter().position(|s| *s == self).unwrap_or(0)
    }
    pub fn from_index(ix: usize) -> Self {
        Self::ALL.get(ix).copied().unwrap_or(RequestSection::Params)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponseSection {
    Body,
    Headers,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyMode {
    None,
    Raw,
    Form,
}

impl BodyMode {
    pub const ALL: [BodyMode; 3] = [BodyMode::None, BodyMode::Raw, BodyMode::Form];
    pub fn index(self) -> usize {
        Self::ALL.iter().position(|m| *m == self).unwrap_or(0)
    }
    pub fn from_index(ix: usize) -> Self {
        Self::ALL.get(ix).copied().unwrap_or(BodyMode::None)
    }
}

/// 响应编辑器按语言各一个（gpui-component 的 EditorState 创建后不能换语言）。
const RESPONSE_LANGUAGES: [&str; 3] = ["json", "html", "text"];

/// 在途时状态行重绘的间隔（实时耗时）。
const TICK_INTERVAL: Duration = Duration::from_millis(100);

pub struct RequestTab {
    /// Tab 的稳定标识；当前只由 Workspace 分配，展示与持久化在后续阶段使用。
    #[allow(dead_code)]
    pub id: u64,
    pub method: Entity<SelectState<Vec<&'static str>>>,
    pub url: Entity<InputState>,
    pub url_error: Option<String>,
    pub path_params: Entity<KvTable>,
    pub params: Entity<KvTable>,
    pub headers: Entity<KvTable>,
    pub form: Entity<KvTable>,
    pub body_mode: BodyMode,
    pub raw_format: RawFormat,
    body_editors: Vec<(RawFormat, Entity<EditorState>)>,
    response_editors: Vec<(&'static str, Entity<EditorState>)>,
    pub request_section: RequestSection,
    pub response_section: ResponseSection,
    pub pretty: bool,
    pub response: ResponseState,
    pub generation: u64,
    _subs: Vec<Subscription>,
}

impl RequestTab {
    pub fn new(id: u64, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let methods: Vec<&'static str> = Method::ALL.iter().map(|m| m.as_str()).collect();
        let method = cx.new(|cx| SelectState::new(methods, Some(IndexPath::default()), window, cx));
        let url = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("输入请求 URL，例如 https://api.example.com/users/{id}")
        });
        let path_params = cx.new(|cx| KvTable::new("参数名", "值", window, cx).locked_keys(true));
        let params = cx.new(|cx| KvTable::new("参数名", "值", window, cx));
        let headers = cx.new(|cx| KvTable::new("Header 名", "值", window, cx));
        let form = cx.new(|cx| KvTable::new("字段名", "值", window, cx));

        let body_editors: Vec<(RawFormat, Entity<EditorState>)> = RawFormat::ALL
            .iter()
            .map(|f| {
                let lang = f.editor_language();
                (
                    *f,
                    cx.new(|cx| {
                        EditorState::new(window, cx)
                            .language(lang)
                            .line_number(true)
                            .soft_wrap(false)
                    }),
                )
            })
            .collect();
        let response_editors = RESPONSE_LANGUAGES
            .iter()
            .map(|lang| {
                (
                    *lang,
                    cx.new(|cx| {
                        EditorState::new(window, cx)
                            .language(*lang)
                            .line_number(true)
                            .soft_wrap(false)
                            .searchable(true)
                    }),
                )
            })
            .collect();

        let subs = vec![
            cx.subscribe_in(&url, window, Self::on_url_event),
            cx.subscribe_in(&params, window, |_, _, _: &KvTableEvent, _, cx| cx.notify()),
            cx.subscribe_in(&headers, window, |_, _, _: &KvTableEvent, _, cx| {
                cx.notify()
            }),
        ];
        // body 编辑器内的 ⌘⏎ 由全局 SendRequest 动作处理（见 main.rs 的 bind_keys），不在此订阅

        Self {
            id,
            method,
            url,
            url_error: None,
            path_params,
            params,
            headers,
            form,
            body_mode: BodyMode::None,
            raw_format: RawFormat::Json,
            body_editors,
            response_editors,
            request_section: RequestSection::Params,
            response_section: ResponseSection::Body,
            pretty: true,
            response: ResponseState::Idle,
            generation: 0,
            _subs: subs,
        }
    }

    fn on_url_event(
        &mut self,
        _: &Entity<InputState>,
        ev: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match ev {
            InputEvent::Change => {
                let names = extract_path_params(&self.url.read(cx).value());
                self.path_params
                    .update(cx, |t, cx| t.sync_keys(&names, window, cx));
                self.url_error = None;
                cx.notify();
            }
            InputEvent::PressEnter { .. } => self.send(window, cx),
            _ => {}
        }
    }

    pub fn focus_url(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.url.update(cx, |s, cx| s.focus(window, cx));
    }

    pub fn current_method(&self, cx: &App) -> Method {
        self.method
            .read(cx)
            .selected_value()
            .and_then(|s| Method::parse(s))
            .unwrap_or(Method::Get)
    }

    pub fn editor_for(&self, format: RawFormat) -> &Entity<EditorState> {
        &self
            .body_editors
            .iter()
            .find(|(f, _)| *f == format)
            .expect("editor per format")
            .1
    }

    pub fn response_editor_for(&self, language: &str) -> &Entity<EditorState> {
        &self
            .response_editors
            .iter()
            .find(|(l, _)| *l == language)
            .unwrap_or(&self.response_editors[2])
            .1
    }

    pub fn has_path_params(&self, cx: &App) -> bool {
        !extract_path_params(&self.url.read(cx).value()).is_empty()
    }

    /// 从各输入组件快照出纯数据的 RequestDraft。
    pub fn draft(&self, cx: &App) -> RequestDraft {
        let body = match self.body_mode {
            BodyMode::None => BodyKind::None,
            BodyMode::Raw => BodyKind::Raw {
                format: self.raw_format,
                text: self
                    .editor_for(self.raw_format)
                    .read(cx)
                    .value()
                    .to_string(),
            },
            BodyMode::Form => BodyKind::FormUrlEncoded {
                fields: self.form.read(cx).values(cx),
            },
        };
        RequestDraft {
            method: self.current_method(cx),
            url: self.url.read(cx).value().to_string(),
            path_params: self.path_params.read(cx).values(cx),
            params: self.params.read(cx).values(cx),
            headers: self.headers.read(cx).values(cx),
            body,
        }
    }

    pub fn title(&self, cx: &App) -> SharedString {
        tab_title(&self.url.read(cx).value())
    }

    pub fn set_pretty(&mut self, pretty: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.pretty == pretty {
            return;
        }
        self.pretty = pretty;
        if let ResponseState::Done(view) = &self.response {
            let text = view.text(pretty);
            let editor = self
                .response_editor_for(view.kind.editor_language())
                .clone();
            editor.update(cx, |e, cx| e.set_value(text, window, cx));
        }
        cx.notify();
    }
}

impl RequestTab {
    pub fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // 在途请求期间按钮显示"取消"，此时 ⌘⏎ / Enter 不应重复发送。
        if self.response.is_in_flight() {
            return;
        }
        let draft = self.draft(cx);
        let req = match http::prepare(&draft) {
            Ok(r) => r,
            Err(e) => {
                self.url_error = Some(e.to_string());
                cx.notify();
                return;
            }
        };
        self.url_error = None;
        self.generation += 1;
        let generation = self.generation;

        let (tx, mut rx) = mpsc::channel::<http::Progress>(64);
        let request_task = bridge::send(cx, req, tx);

        // 进度任务：把 tokio 侧的进度事件写回 Entity（已节流到 ≤ 30 Hz）。
        let progress_task = cx.spawn_in(window, async move |this, cx| {
            while let Some(p) = rx.recv().await {
                let keep_going = this
                    .update(cx, |this, cx| {
                        if this.generation != generation {
                            return false;
                        }
                        if let ResponseState::InFlight {
                            received, total, ..
                        } = &mut this.response
                        {
                            *received = p.received;
                            *total = p.total;
                            cx.notify();
                        }
                        true
                    })
                    .unwrap_or(false);
                if !keep_going {
                    break;
                }
            }
        });

        // 计时任务：每 TICK_INTERVAL 触发一次重绘，让状态行的耗时实时更新；generation 变化即退出。
        let tick_task = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(TICK_INTERVAL).await;
                let keep_going = this
                    .update(cx, |this, cx| {
                        if this.generation != generation {
                            return false;
                        }
                        cx.notify();
                        true
                    })
                    .unwrap_or(false);
                if !keep_going {
                    break;
                }
            }
        });

        // 完成任务：等待请求 → 后台准备视图 → 回主线程写入（generation 不匹配则丢弃）。
        let completion_task = cx.spawn_in(window, async move |this, cx| {
            let outcome: Result<HttpResponse, RequestError> = match request_task.await {
                Ok(inner) => inner,
                Err(e) => Err(RequestError::Other(e.to_string())),
            };
            let prepared = match outcome {
                Ok(resp) => Ok(cx
                    .background_spawn(async move { ResponseView::prepare(resp) })
                    .await),
                Err(e) => Err(e),
            };
            let _ = this.update_in(cx, |this, window, cx| {
                this.apply_outcome(generation, prepared, window, cx)
            });
        });

        self.response = ResponseState::InFlight {
            started: Instant::now(),
            received: 0,
            total: None,
            _tasks: vec![progress_task, tick_task, completion_task],
        };
        cx.notify();
    }

    /// 完成任务的最后一步，也是唯一写入 Done / Failed 的入口：
    /// generation 不匹配（已取消或已重发）则直接丢弃，过期响应不得覆盖新状态。
    pub(crate) fn apply_outcome(
        &mut self,
        generation: u64,
        outcome: Result<ResponseView, RequestError>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.generation != generation {
            return;
        }
        match outcome {
            Ok(view) => {
                let text = view.text(self.pretty);
                let editor = self
                    .response_editor_for(view.kind.editor_language())
                    .clone();
                editor.update(cx, |e, cx| e.set_value(text, window, cx));
                self.response = ResponseState::Done(view);
                self.response_section = ResponseSection::Body;
            }
            Err(error) => self.response = ResponseState::Failed { error },
        }
        cx.notify();
    }

    /// 取消进行中的请求：递增 generation 让在途任务的回调失效，drop 掉任务本身，状态显示"已取消"。
    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        if !self.response.is_in_flight() {
            return;
        }
        self.generation += 1;
        self.response = ResponseState::Failed {
            error: RequestError::Cancelled,
        };
        cx.notify();
    }
}

impl Render for RequestTab {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex().size_full().child(self.render_url_bar(cx)).child(
            div().flex_1().min_h_0().child(
                v_resizable("request-response")
                    .child(
                        resizable_panel()
                            .size(px(300.))
                            .size_range(px(140.)..px(900.))
                            .child(self.render_request_pane(cx)),
                    )
                    .child(resizable_panel().child(self.render_response_pane(cx))),
            ),
        )
    }
}

/// Tab 标题：有路径取路径，否则取主机名；空 URL 显示"新请求"。
pub fn tab_title(url: &str) -> SharedString {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return "新请求".into();
    }
    let without_scheme = trimmed
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(trimmed);
    let without_query = without_scheme.split('?').next().unwrap_or(without_scheme);
    if without_query.is_empty() {
        return "新请求".into();
    }
    match without_query.find('/') {
        Some(ix) if ix + 1 < without_query.len() => without_query[ix..].to_string().into(),
        Some(ix) => without_query[..ix].to_string().into(),
        None => without_query.to_string().into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_from_url() {
        assert_eq!(tab_title("").as_ref(), "新请求");
        assert_eq!(
            tab_title("https://api.example.com/users/42?x=1").as_ref(),
            "/users/42"
        );
        assert_eq!(
            tab_title("https://api.example.com").as_ref(),
            "api.example.com"
        );
        assert_eq!(tab_title("api.example.com/").as_ref(), "api.example.com");
        assert_eq!(tab_title("not a url").as_ref(), "not a url");
        assert_eq!(tab_title("https://").as_ref(), "新请求");
        assert_eq!(tab_title("http://").as_ref(), "新请求");
    }
}
