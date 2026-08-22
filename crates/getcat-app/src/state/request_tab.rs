//! 一个请求 Tab 的全部状态：输入组件实体、响应状态、视图选择。

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use getcat_core::body::tier::{ViewTier, mib_label};
use getcat_core::detect::ContentKind;
use getcat_core::http::{self, BodyStore, HttpResponse, RequestError, guess_content_type};
use getcat_core::model::{
    BodyKind, Method, RawFormat, RequestDraft, SplitDirection, TabDraft, TabId, Ulid,
};
use getcat_core::url::extract_path_params;
// 显式导入而非 `use gpui::*`：本文件内 `#[cfg(test)] mod tests { use super::*; #[test] .. }`
// 若通过通配符引入 `gpui::test`（gpui 重导出的 `#[proc_macro_attribute]`），会与标准库的
// `#[test]` 属性同名冲突，导致该属性宏对自身生成的 `#[test]` 反复展开直至递归上限溢出。
use gpui::{
    App, AppContext, Context, Entity, Global, IntoElement, ParentElement, PathPromptOptions,
    Render, ScrollStrategy, SharedString, Styled, Subscription, Task, UniformListScrollHandle,
    Window, div, px,
};
use gpui_component::IndexPath;
use gpui_component::input::{EditorState, InputEvent, InputState, Search};
use gpui_component::resizable::{h_resizable, resizable_panel, v_resizable};
use gpui_component::select::{SelectEvent, SelectState};
use gpui_component::v_flex;

use tokio::sync::mpsc;

use crate::bridge;
use crate::state::response::{CancelFlag, ResponseState, ResponseView, prepare_guarded};
use crate::state::store::store;
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

/// Body 模式；`ALL` 的顺序就是模式条顺序（Postman 顺序）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyMode {
    None,
    FormData,
    FormUrlEncoded,
    Raw,
    Binary,
}

impl BodyMode {
    pub const ALL: [BodyMode; 5] = [
        BodyMode::None,
        BodyMode::FormData,
        BodyMode::FormUrlEncoded,
        BodyMode::Raw,
        BodyMode::Binary,
    ];
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

/// 文本 Body 超过此大小时提示改用文件 Body（spec §6.5）。
pub const BODY_HINT_BYTES: usize = 10 * 1024 * 1024;

pub fn body_hint_for(len: usize) -> Option<SharedString> {
    (len > BODY_HINT_BYTES).then(|| {
        format!(
            "文本 Body 超过 {}，建议改用文件 Body 流式上传",
            mib_label(BODY_HINT_BYTES as u64)
        )
        .into()
    })
}

/// form-data 模式下用户自设 Content-Type 的提示（发送时该头会被剔除，见 core::http）。
pub const FORM_DATA_CONTENT_TYPE_HINT: &str =
    "form-data 的 Content-Type（含 boundary）由 GetCat 生成，Headers 中的 Content-Type 将被忽略";

/// 编辑后到投递草稿的去抖：主线程只在窗口结束时做一次 `draft()` 快照（rope → String 拷贝），
/// 序列化与落盘都在写入线程；写入线程再按 500 ms 合并同一 Tab 的重复写入。
pub(crate) const DRAFT_DEBOUNCE: Duration = Duration::from_millis(300);

/// ⌘F 在 B / C 档（没有编辑器）时的提示。
pub const VIRTUAL_SEARCH_NOTICE: &str = "纯文本视图暂不支持搜索：可「保存到文件」后用编辑器查找";
pub const NO_RESPONSE_NOTICE: &str = "没有可搜索的响应";
pub const BINARY_SEARCH_NOTICE: &str = "二进制内容不提供搜索";
/// A 档但正文为空：`render_body_view` 这时画的是「响应体为空」占位而不是编辑器，聚焦一个没渲染的元素没有任何反馈。
pub const EMPTY_BODY_SEARCH_NOTICE: &str = "响应体为空，无可搜索内容";

/// 上次"保存到文件"选择的目录（进程内记忆，不落盘）；下一次保存对话框从这里打开。
pub(crate) struct LastSaveDir(pub Option<PathBuf>);

impl Global for LastSaveDir {}

/// 保存对话框的起始目录：上次保存的目录，否则 home。
fn save_dialog_dir(cx: &App) -> PathBuf {
    cx.try_global::<LastSaveDir>()
        .and_then(|d| d.0.clone())
        .unwrap_or_else(|| std::env::home_dir().unwrap_or_default())
}

pub struct RequestTab {
    /// Tab 的稳定标识：也是草稿文件名 `drafts/<id>.json`。
    pub id: TabId,
    /// 来自哪条已保存请求（保存 / 从侧栏打开时设置；该请求被删除时清空）。
    pub saved_id: Option<Ulid>,
    /// 已保存请求的名字：有则作为 Tab 标题。
    pub saved_name: Option<SharedString>,
    /// 自上次保存以来是否有改动；Tab 标题前显示圆点。
    pub dirty: bool,
    pub method: Entity<SelectState<Vec<&'static str>>>,
    pub url: Entity<InputState>,
    /// 发送前校验失败的内联提示（URL 非法 / Header 非法 / 未选文件），显示在 URL 栏下方（spec §11）。
    pub prepare_error: Option<String>,
    pub path_params: Entity<KvTable>,
    pub params: Entity<KvTable>,
    pub headers: Entity<KvTable>,
    pub form: Entity<KvTable>,
    pub form_data: Entity<KvTable>,
    pub body_mode: BodyMode,
    pub raw_format: RawFormat,
    /// binary Body：所选文件路径与大小（大小只用于显示）。
    pub file_path: Option<PathBuf>,
    pub file_size: Option<u64>,
    /// 非阻塞的 Body 提示：raw 模式下是文本过大提示，form-data 模式下是 Content-Type 冲突提示。
    pub body_hint: Option<SharedString>,
    body_editors: Vec<(RawFormat, Entity<EditorState>)>,
    response_editors: Vec<(&'static str, Entity<EditorState>)>,
    pub request_section: RequestSection,
    pub response_section: ResponseSection,
    pub pretty: bool,
    /// 请求 / 响应分栏方向；由 Workspace 统一设置（工作区级设置，随 workspace.json 持久化）。
    pub split: SplitDirection,
    /// B/C 档行视图与 Headers 列表的滚动位置；新响应到达时回到顶部。
    pub body_scroll: UniformListScrollHandle,
    pub headers_scroll: UniformListScrollHandle,
    pub response: ResponseState,
    /// 工具栏右侧的一行提示（保存结果、搜索不可用等）；重新发送时清空。
    pub notice: Option<SharedString>,
    pub generation: u64,
    /// 去抖中的草稿写入任务；每次改动替换（drop 即取消旧计时器）。
    draft_save: Option<Task<()>>,
    _subs: Vec<Subscription>,
}

impl RequestTab {
    pub fn new(id: TabId, window: &mut Window, cx: &mut Context<Self>) -> Self {
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
        let form_data = cx.new(|cx| KvTable::new("字段名", "值", window, cx).file_capable(true));

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

        // 任何会改变 draft() 的用户操作都经 mark_dirty（置脏 + 去抖写草稿）
        let mut subs = vec![
            cx.subscribe_in(&url, window, Self::on_url_event),
            cx.subscribe_in(
                &method,
                window,
                |this, _, _: &SelectEvent<Vec<&'static str>>, _, cx| this.mark_dirty(cx),
            ),
            cx.subscribe_in(&path_params, window, |this, _, _: &KvTableEvent, _, cx| {
                this.mark_dirty(cx)
            }),
            cx.subscribe_in(&params, window, |this, _, _: &KvTableEvent, _, cx| {
                this.mark_dirty(cx)
            }),
            // Headers 改动可能新增 / 删除 Content-Type，form-data 的冲突提示要跟着重算
            cx.subscribe_in(&headers, window, |this, _, _: &KvTableEvent, _, cx| {
                this.refresh_body_hint(cx);
                this.mark_dirty(cx)
            }),
            cx.subscribe_in(&form, window, |this, _, _: &KvTableEvent, _, cx| {
                this.mark_dirty(cx)
            }),
            cx.subscribe_in(&form_data, window, |this, _, _: &KvTableEvent, _, cx| {
                this.mark_dirty(cx)
            }),
        ];
        for (_, editor) in &body_editors {
            subs.push(cx.subscribe_in(editor, window, Self::on_body_editor_event));
        }
        // body 编辑器内的 ⌘⏎ 由全局 SendRequest 动作处理（见 main.rs 的 bind_keys），不在此订阅

        Self {
            id,
            saved_id: None,
            saved_name: None,
            dirty: false,
            method,
            url,
            prepare_error: None,
            path_params,
            params,
            headers,
            form,
            form_data,
            body_mode: BodyMode::None,
            raw_format: RawFormat::Json,
            file_path: None,
            file_size: None,
            body_hint: None,
            body_editors,
            response_editors,
            request_section: RequestSection::Params,
            response_section: ResponseSection::Body,
            pretty: true,
            split: SplitDirection::Vertical,
            body_scroll: UniformListScrollHandle::new(),
            headers_scroll: UniformListScrollHandle::new(),
            response: ResponseState::Idle,
            notice: None,
            generation: 0,
            draft_save: None,
            _subs: subs,
        }
    }

    pub(crate) fn on_url_event(
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
                self.prepare_error = None;
                self.mark_dirty(cx);
            }
            InputEvent::PressEnter { .. } => self.send(window, cx),
            _ => {}
        }
    }

    /// 文本 Body 编辑器内容变化：按 rope 的字节数（O(1)）判断是否提示改用文件 Body。
    pub(crate) fn on_body_editor_event(
        &mut self,
        editor: &Entity<EditorState>,
        ev: &InputEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !matches!(ev, InputEvent::Change) {
            return;
        }
        // 三个格式各一个编辑器，只看当前格式的那个
        if editor != self.editor_for(self.raw_format) {
            return;
        }
        self.refresh_body_hint(cx);
        self.mark_dirty(cx);
    }

    /// 按当前 body_mode 重新计算 Body 区的非阻塞提示（raw 的超大文本提示、form-data 的
    /// Content-Type 冲突提示共用 `body_hint`）：不在这两个模式时清空。
    /// 切换 raw_format / body_mode、以及 Headers 改动后都要调用，否则会残留上一个模式的提示，
    /// 或者漏掉一个通过 `set_value`（不发 Change 事件）灌入内容的编辑器。
    pub(crate) fn refresh_body_hint(&mut self, cx: &mut Context<Self>) {
        let hint = match self.body_mode {
            BodyMode::Raw => body_hint_for(self.editor_for(self.raw_format).read(cx).text().len()),
            BodyMode::FormData if self.has_user_content_type(cx) => {
                Some(SharedString::from(FORM_DATA_CONTENT_TYPE_HINT))
            }
            _ => None,
        };
        if hint != self.body_hint {
            self.body_hint = hint;
            cx.notify();
        }
    }

    /// Headers 里是否有启用的 Content-Type（form-data 发送时会被剔除）。
    fn has_user_content_type(&self, cx: &App) -> bool {
        self.headers
            .read(cx)
            .values(cx)
            .iter()
            .any(|h| h.enabled && h.key.trim().eq_ignore_ascii_case("content-type"))
    }

    /// "选择文件"：系统打开对话框 → 后台读 metadata → 切到 file 模式。
    pub fn choose_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("选择".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = rx.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let size = cx
                .background_spawn({
                    let path = path.clone();
                    async move { std::fs::metadata(&path).map(|m| m.len()).ok() }
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.file_path = Some(path);
                this.file_size = size;
                this.body_mode = BodyMode::Binary;
                this.mark_dirty(cx);
            });
        })
        .detach();
    }

    pub fn clear_file(&mut self, cx: &mut Context<Self>) {
        // body_mode 保持 Binary 不变：清除只是"未选文件"，draft() 据此报告"未选择文件"，
        // 而不是悄悄退回 none/raw 让用户以为 Body 被清空了。
        self.file_path = None;
        self.file_size = None;
        self.mark_dirty(cx);
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
                text: self.editor_for(self.raw_format).read(cx).text().to_string(),
            },
            BodyMode::FormData => BodyKind::FormData {
                fields: self.form_data.read(cx).form_fields(cx),
            },
            BodyMode::FormUrlEncoded => BodyKind::FormUrlEncoded {
                fields: self.form.read(cx).values(cx),
            },
            BodyMode::Binary => BodyKind::Binary {
                path: self.file_path.clone().unwrap_or_default(),
                content_type: self
                    .file_path
                    .as_deref()
                    .map(|p| guess_content_type(p).to_string()),
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

    /// Tab 标题：已保存请求名优先，否则取 URL 末段（spec §7.1）。
    pub fn title(&self, cx: &App) -> SharedString {
        self.saved_name
            .clone()
            .unwrap_or_else(|| tab_title(&self.url.read(cx).value()))
    }

    pub fn set_pretty(&mut self, pretty: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.pretty == pretty {
            return;
        }
        self.pretty = pretty;
        if let ResponseState::Done { view, .. } = &self.response
            && let Some(doc) = view.doc(pretty)
            && doc.tier == ViewTier::Editor
        {
            let text = doc.shared_text();
            let editor = self
                .response_editor_for(view.kind.editor_language())
                .clone();
            editor.update(cx, |e, cx| e.set_value(text, window, cx));
        }
        self.body_scroll.scroll_to_item(0, ScrollStrategy::Top);
        cx.notify();
    }

    /// 用户改动的唯一入口：置脏、重绘、去抖写草稿。
    pub(crate) fn mark_dirty(&mut self, cx: &mut Context<Self>) {
        self.dirty = true;
        self.schedule_draft_save(cx);
        cx.notify();
    }

    /// 保存成功后调用。
    pub(crate) fn mark_clean(&mut self, cx: &mut Context<Self>) {
        self.dirty = false;
        cx.notify();
    }

    fn schedule_draft_save(&mut self, cx: &mut Context<Self>) {
        if store(cx).is_none() {
            return;
        }
        // 替换旧任务即取消旧计时器
        self.draft_save = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(DRAFT_DEBOUNCE).await;
            let _ = this.update(cx, |this, cx| this.save_draft_now(cx));
        }));
    }

    /// 立即投递一份草稿快照（跳过去抖）；序列化在写入线程完成。
    pub(crate) fn save_draft_now(&mut self, cx: &mut Context<Self>) {
        self.draft_save = None;
        if let Some(store) = store(cx) {
            store.write_draft(self.tab_draft(cx));
        }
    }

    /// 草稿文件的内容：draft 快照 + 来源 + 是否有改动。
    pub fn tab_draft(&self, cx: &App) -> TabDraft {
        TabDraft {
            id: self.id,
            draft: self.draft(cx),
            saved_id: self.saved_id,
            dirty: self.dirty,
        }
    }

    /// 用一份 RequestDraft 重建所有输入组件（恢复草稿 / 打开已保存请求）。
    /// 只走不发事件的程序化写入，因此不会置脏；`saved_id` / `saved_name` / `dirty` 由调用方设置。
    pub fn load_draft(
        &mut self,
        draft: &RequestDraft,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.method.update(cx, |s, cx| {
            s.set_selected_value(&draft.method.as_str(), window, cx)
        });
        self.url
            .update(cx, |u, cx| u.set_value(draft.url.clone(), window, cx));
        self.path_params
            .update(cx, |t, cx| t.set_values(&draft.path_params, window, cx));
        self.params
            .update(cx, |t, cx| t.set_values(&draft.params, window, cx));
        self.headers
            .update(cx, |t, cx| t.set_values(&draft.headers, window, cx));
        self.file_path = None;
        self.file_size = None;
        match &draft.body {
            BodyKind::None => self.body_mode = BodyMode::None,
            BodyKind::Raw { format, text } => {
                self.body_mode = BodyMode::Raw;
                self.raw_format = *format;
                let editor = self.editor_for(*format).clone();
                editor.update(cx, |e, cx| e.set_value(text.clone(), window, cx));
            }
            BodyKind::FormData { fields } => {
                self.body_mode = BodyMode::FormData;
                self.form_data
                    .update(cx, |t, cx| t.set_form_fields(fields, window, cx));
            }
            BodyKind::FormUrlEncoded { fields } => {
                self.body_mode = BodyMode::FormUrlEncoded;
                self.form
                    .update(cx, |t, cx| t.set_values(fields, window, cx));
            }
            BodyKind::Binary { path, .. } => {
                self.body_mode = BodyMode::Binary;
                if !path.as_os_str().is_empty() {
                    self.file_path = Some(path.clone());
                    self.refresh_file_size(cx);
                }
            }
        }
        self.prepare_error = None;
        self.refresh_body_hint(cx);
        cx.notify();
    }

    /// 后台读取文件 Body 的大小（只用于显示）。
    fn refresh_file_size(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.file_path.clone() else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let size = cx
                .background_spawn(async move { std::fs::metadata(&path).map(|m| m.len()).ok() })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.file_size = size;
                cx.notify();
            });
        })
        .detach();
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
                self.prepare_error = Some(e.to_string());
                cx.notify();
                return;
            }
        };
        self.prepare_error = None;
        self.notice = None;
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

        // 后台准备阶段的取消旗标：随 InFlight 一起 drop 即置位
        let cancel = CancelFlag::new();
        let cancelled = cancel.handle();

        // 完成任务：等待请求 → 后台准备视图（可取消）→ 回主线程写入（generation 不匹配则丢弃）。
        let completion_task = cx.spawn_in(window, async move |this, cx| {
            let outcome: Result<HttpResponse, RequestError> = match request_task.await {
                Ok(inner) => inner,
                Err(e) => Err(RequestError::Other(e.to_string())),
            };
            let prepared = match outcome {
                Ok(HttpResponse { meta, body }) => {
                    // prepare_guarded 把后台 panic 转成 Err（spec §11），取消仍是 None
                    let guarded = cx
                        .background_spawn(async move {
                            prepare_guarded(meta, body, || cancelled.load(Ordering::Relaxed))
                        })
                        .await;
                    match guarded {
                        Some(result) => result,
                        // 已取消：不回写任何东西
                        None => return,
                    }
                }
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
            _cancel: cancel,
        };
        cx.notify();
    }

    /// 请求结果的唯一写入口（取消由 `cancel()` 直接写 Failed）：
    /// generation 不匹配（已取消或已重发）则直接丢弃，过期响应不得覆盖新状态。
    pub(crate) fn apply_outcome(
        &mut self,
        generation: u64,
        outcome: Result<(BodyStore, ResponseView), RequestError>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.generation != generation {
            return;
        }
        match outcome {
            Ok((body, view)) => {
                // A 档：整段文本写入对应语言的只读编辑器；B/C 档由虚拟列表直接切片，不经过编辑器
                if let Some(doc) = view.doc(self.pretty)
                    && doc.tier == ViewTier::Editor
                {
                    let editor = self
                        .response_editor_for(view.kind.editor_language())
                        .clone();
                    editor.update(cx, |e, cx| e.set_value(doc.shared_text(), window, cx));
                }
                self.body_scroll.scroll_to_item(0, ScrollStrategy::Top);
                self.headers_scroll.scroll_to_item(0, ScrollStrategy::Top);
                self.response = ResponseState::Done { body, view };
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

    /// ⌘F / 工具栏搜索按钮：A 档把焦点交给只读编辑器并打开它的搜索面板；B / C 档没有编辑器，只提示。
    pub fn find_in_response(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ResponseState::Done { view, .. } = &self.response else {
            self.notice = Some(NO_RESPONSE_NOTICE.into());
            cx.notify();
            return;
        };
        let Some(doc) = view.doc(self.pretty) else {
            self.notice = Some(BINARY_SEARCH_NOTICE.into());
            cx.notify();
            return;
        };
        if doc.tier != ViewTier::Editor {
            self.notice = Some(VIRTUAL_SEARCH_NOTICE.into());
            cx.notify();
            return;
        }
        if doc.doc.line_count() == 0 {
            self.notice = Some(EMPTY_BODY_SEARCH_NOTICE.into());
            cx.notify();
            return;
        }
        let editor = self
            .response_editor_for(view.kind.editor_language())
            .clone();
        let switched = self.response_section != ResponseSection::Body;
        self.response_section = ResponseSection::Body;
        self.notice = None;
        // 立即聚焦：调用返回时焦点就该在编辑器上（"已在 Body"路径紧接着就要 `dispatch_action`，
        // 切换路径也靠这一次让状态同步可见）。切换路径下面还会再聚焦一次，两次各有各的必要：
        // 这一帧编辑器还没进分发树，`window.draw` 会把不在树里的焦点丢掉。
        editor.update(cx, |e, cx| e.focus(window, cx));
        if switched {
            // `dispatch_action` 按「上一帧渲染出的分发树」找焦点节点，而刚从 Headers 切回 Body 时编辑器这一帧才出现。
            // gpui 的帧处理把 `pending_next_frame_callbacks` 跑在 `window.draw` 之前
            // （zed e0931d5 `crates/gpui/src/window.rs:1592-1621`），所以第一层 `on_next_frame` 里 `rendered_frame`
            // 仍是 Headers 那一帧，派发会退回根节点被丢弃；要再嵌一层，等 Body 真的画出来之后再聚焦并派发。
            window.on_next_frame(move |window, _| {
                window.on_next_frame(move |window, cx| {
                    editor.update(cx, |e, cx| e.focus(window, cx));
                    window.dispatch_action(Box::new(Search), cx);
                });
            });
        } else {
            window.dispatch_action(Box::new(Search), cx);
        }
        cx.notify();
    }

    /// "保存到文件"：弹系统保存对话框（从上次保存的目录打开），选中后在 tokio 上原子写入 / 拷贝，
    /// 完成后在工具栏显示结果并记住目录。
    pub fn save_body(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ResponseState::Done { body, view } = &self.response else {
            return;
        };
        let body = body.clone();
        let suggested = format!("response.{}", file_extension(view.kind));
        let generation = self.generation;
        let dir = save_dialog_dir(cx);
        let rx = cx.prompt_for_new_path(&dir, Some(suggested.as_str()));
        cx.spawn_in(window, async move |this, cx| {
            // 对话框取消 / 出错都静默返回
            let Ok(Ok(Some(dest))) = rx.await else {
                return;
            };
            let result = match cx.update(|_, cx| bridge::save_body(cx, body, dest.clone())) {
                Ok(task) => task.await,
                Err(e) => Err(e),
            };
            let _ = this.update(cx, |this, cx| {
                if result.is_ok() {
                    cx.set_global(LastSaveDir(dest.parent().map(Path::to_path_buf)));
                }
                // 已重发：不再展示旧响应的保存结果
                if this.generation != generation {
                    return;
                }
                this.notice = Some(match result {
                    Ok(()) => format!("已保存到 {}", dest.display()).into(),
                    Err(e) => format!("保存失败：{e}").into(),
                });
                cx.notify();
            });
        })
        .detach();
    }

    /// "用系统程序打开"：只有落盘响应才有文件可开。
    pub fn open_body_with_system(&self, cx: &mut Context<Self>) {
        if let ResponseState::Done { body, .. } = &self.response
            && let Some(path) = body.path()
        {
            cx.open_with_system(path);
        }
    }
}

impl Render for RequestTab {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 两个方向各用一个 id：ResizablePanelGroup 按 id 记住拖出来的尺寸，上下与左右互不串扰
        let group = match self.split {
            SplitDirection::Vertical => v_resizable("request-response-v"),
            SplitDirection::Horizontal => h_resizable("request-response-h"),
        };
        // 请求 / 响应默认各占一半：两块面板都不给初始像素尺寸，而是把 flex-basis 压到 0，
        // 剩余空间由同为 flex-grow 1 的两者均分（basis 留 auto 时会按内容宽度分，不均）。
        // 首帧量到的尺寸会被 ResizableState 记下，之后拖动与等比缩放照旧。
        let half = || {
            resizable_panel()
                .flex_basis(px(0.))
                .size_range(px(140.)..px(4000.))
        };
        v_flex().size_full().child(self.render_url_bar(cx)).child(
            div().flex_1().min_h_0().min_w_0().child(
                group
                    .child(half().child(self.render_request_pane(cx)))
                    .child(half().child(self.render_response_pane(window, cx))),
            ),
        )
    }
}

/// 保存对话框的建议扩展名。
fn file_extension(kind: ContentKind) -> &'static str {
    match kind {
        ContentKind::Json => "json",
        ContentKind::Xml => "xml",
        ContentKind::Html => "html",
        ContentKind::Text => "txt",
        ContentKind::Binary => "bin",
    }
}

/// Tab 标题：有路径取路径，否则取主机名；空 URL 显示"新请求"。
pub fn tab_title(url: &str) -> SharedString {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return "新请求".into();
    }
    // 先去 query 再去 scheme：`localhost:8080/cb?to=https://x` 的 `://` 在 query 里
    let without_query = trimmed.split('?').next().unwrap_or(trimmed);
    let without_scheme = without_query
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(without_query);
    if without_scheme.is_empty() {
        return "新请求".into();
    }
    match without_scheme.find('/') {
        Some(ix) if ix + 1 < without_scheme.len() => without_scheme[ix..].to_string().into(),
        Some(ix) => without_scheme[..ix].to_string().into(),
        None => without_scheme.to_string().into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_hint_threshold() {
        assert!(body_hint_for(0).is_none());
        assert!(body_hint_for(BODY_HINT_BYTES).is_none());
        assert!(
            body_hint_for(BODY_HINT_BYTES + 1)
                .unwrap()
                .contains("10 MB")
        );
    }

    #[test]
    fn body_mode_round_trips_through_index() {
        for (ix, mode) in BodyMode::ALL.iter().enumerate() {
            assert_eq!(mode.index(), ix);
            assert_eq!(BodyMode::from_index(ix), *mode);
        }
        assert_eq!(BodyMode::from_index(99), BodyMode::None);
    }

    #[test]
    fn body_modes_follow_postman_order() {
        assert_eq!(
            BodyMode::ALL,
            [
                BodyMode::None,
                BodyMode::FormData,
                BodyMode::FormUrlEncoded,
                BodyMode::Raw,
                BodyMode::Binary
            ]
        );
    }

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
        // query 里的 "://" 不是 scheme
        assert_eq!(tab_title("localhost:8080/cb?to=https://x").as_ref(), "/cb");
        assert_eq!(tab_title("x.com?a=1").as_ref(), "x.com");
    }
}
