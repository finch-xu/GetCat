//! 键值表：Params / Headers / Form 共用。最后一行永远保留一个空行用于新增。

use std::path::PathBuf;

use getcat_core::http::guess_content_type;
use getcat_core::model::{FormField, FormValue, KeyValue};
use gpui::prelude::FluentBuilder as _;
// 显式导入而非 `use gpui::*`：本文件含 `#[cfg(test)] mod tests`，通配符会引入 gpui 重导出的
// `#[test]` 属性宏并与标准库同名冲突。编译器报"找不到 X"时把 X 加进这里，不要改回通配符。
use gpui::{
    AnyElement, App, AppContext, Context, CursorStyle, DragMoveEvent, Entity, EventEmitter,
    FontWeight, InteractiveElement, IntoElement, ParentElement, PathPromptOptions, Render, Role,
    SharedString, StatefulInteractiveElement, Styled, Subscription, Window, div, px, relative,
};
use gpui_component::{
    ActiveTheme, Disableable, IconName, Sizable, Size,
    button::{Button, ButtonVariants},
    checkbox::Checkbox,
    h_flex,
    input::{Input, InputEvent, InputState},
    tooltip::Tooltip,
    v_flex,
};

use crate::i18n::{Locale, tr};
use crate::ui::format_bytes;

/// 每行控件的可访问名称带行号，屏幕阅读器才分得清"第 3 行的参数名"和"第 4 行的参数名"。
pub fn row_aria_label(ix: usize, what: &str) -> SharedString {
    tr!("kv.row_aria", what = what, row = ix + 1)
}

/// 表格的用途决定 Key / Value 列的占位符；存枚举，切换界面语言时重新翻译。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KvPlaceholder {
    /// Query / Path 参数
    Param,
    /// 请求头
    Header,
    /// 表单字段（urlencoded / form-data）
    Field,
}

impl KvPlaceholder {
    pub fn key_text(self) -> SharedString {
        match self {
            KvPlaceholder::Param => tr!("kv.param_key"),
            KvPlaceholder::Header => tr!("kv.header_key"),
            KvPlaceholder::Field => tr!("kv.field_key"),
        }
    }

    pub fn value_text(self) -> SharedString {
        tr!("kv.value")
    }
}

pub enum KvTableEvent {
    Changed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    Text,
    File,
}

impl RowKind {
    fn label(self) -> &'static str {
        match self {
            RowKind::Text => "Text",
            RowKind::File => "File",
        }
    }
    fn toggled(self) -> RowKind {
        match self {
            RowKind::Text => RowKind::File,
            RowKind::File => RowKind::Text,
        }
    }
}

/// 文件行已选的文件；`size` 只用于显示，后台读取。
struct FileCell {
    path: PathBuf,
    content_type: Option<String>,
    size: Option<u64>,
}

struct KvRow {
    key: Entity<InputState>,
    value: Entity<InputState>,
    description: Entity<InputState>,
    enabled: bool,
    kind: RowKind,
    file: Option<FileCell>,
    _subs: Vec<Subscription>,
}

/// 三个可拖宽的列：Key / Value / Description。
pub const COLUMNS: usize = 3;
/// 默认列宽占比（Value 略宽：值通常比名字长）。
pub const DEFAULT_COLUMN_FRACTIONS: [f32; COLUMNS] = [0.30, 0.42, 0.28];
/// 单列最窄占比：再窄就连占位文字都放不下。
pub const MIN_COLUMN_FRACTION: f32 = 0.12;
/// 表格密度：与 gpui-component `Table` 的 small 档对齐（行高 30 px），
/// 行高与单元格内边距都从这个 `Size` 取，不再各写各的像素值。
/// 公开出去是为了让「默认参数（Header）」那块只读表与本表行高一致。
pub const TABLE_SIZE: Size = Size::Small;

/// 拖动表头分隔线时携带的数据：正在拖第几条分隔线（在列 `ix` 与 `ix + 1` 之间）。
/// gpui 的 `on_drag` 要求拖拽值同时是一个可渲染的"拖拽预览"，这里什么都不画。
#[derive(Clone, Copy)]
struct ColumnDrag(usize);

impl Render for ColumnDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

/// 把第 `boundary` 条分隔线移到表格可变区域宽度的 `x` 处（0..1），只动它两侧的列，
/// 其余列不变；两侧各自不小于 [`MIN_COLUMN_FRACTION`]。
pub(crate) fn move_column_boundary(fractions: &mut [f32; COLUMNS], boundary: usize, x: f32) {
    if boundary + 1 >= COLUMNS {
        return;
    }
    let left_edge: f32 = fractions[..boundary].iter().sum();
    let pair = fractions[boundary] + fractions[boundary + 1];
    let min = MIN_COLUMN_FRACTION;
    if pair < 2. * min {
        return;
    }
    let left = (x - left_edge).clamp(min, pair - min);
    fractions[boundary] = left;
    fractions[boundary + 1] = pair - left;
}

pub struct KvTable {
    placeholder: KvPlaceholder,
    rows: Vec<KvRow>,
    /// Path 参数模式：key 只读、由 URL 驱动，不自动追加空行。
    locked_keys: bool,
    /// form-data 模式：每行可在 Text / File 间切换。
    file_capable: bool,
    /// 三列占可变区域的比例，和为 1；拖表头分隔线改。按比例而不是像素记，
    /// 请求区被拖宽拖窄时各列跟着等比缩放。
    column_fractions: [f32; COLUMNS],
    /// 占位符驻留在每行的 InputState 里，切换界面语言时由这个订阅刷新。
    _locale_sub: Subscription,
}

impl EventEmitter<KvTableEvent> for KvTable {}

impl KvTable {
    pub fn new(placeholder: KvPlaceholder, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let locale_sub = cx.observe_global_in::<Locale>(window, Self::refresh_placeholders);
        let mut this = Self {
            placeholder,
            rows: Vec::new(),
            locked_keys: false,
            file_capable: false,
            column_fractions: DEFAULT_COLUMN_FRACTIONS,
            _locale_sub: locale_sub,
        };
        this.push_row("", "", "", true, window, cx);
        this
    }

    /// 界面语言变了：把每行三个输入框的占位符换成当前语言。
    fn refresh_placeholders(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let key = self.placeholder.key_text();
        let value = self.placeholder.value_text();
        let description = tr!("kv.description");
        for row in &self.rows {
            row.key
                .update(cx, |s, cx| s.set_placeholder(key.clone(), window, cx));
            row.value
                .update(cx, |s, cx| s.set_placeholder(value.clone(), window, cx));
            row.description.update(cx, |s, cx| {
                s.set_placeholder(description.clone(), window, cx)
            });
        }
    }

    pub fn locked_keys(mut self, locked: bool) -> Self {
        self.locked_keys = locked;
        if locked {
            self.rows.clear();
        }
        self
    }

    /// form-data 模式：每行可在 Text / File 间切换，File 行按行选文件。
    pub fn file_capable(mut self, yes: bool) -> Self {
        self.file_capable = yes;
        self
    }

    fn push_row(
        &mut self,
        key: &str,
        value: &str,
        description: &str,
        enabled: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key_ph = self.placeholder.key_text();
        let value_ph = self.placeholder.value_text();
        let key_state = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(key_ph)
                .default_value(key.to_string())
        });
        let value_state = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(value_ph)
                .default_value(value.to_string())
        });
        let description_state = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(tr!("kv.description"))
                .default_value(description.to_string())
        });
        let subs = vec![
            cx.subscribe_in(&key_state, window, Self::on_input_event),
            cx.subscribe_in(&value_state, window, Self::on_input_event),
            cx.subscribe_in(&description_state, window, Self::on_input_event),
        ];
        self.rows.push(KvRow {
            key: key_state,
            value: value_state,
            description: description_state,
            enabled,
            kind: RowKind::Text,
            file: None,
            _subs: subs,
        });
    }

    fn on_input_event(
        &mut self,
        _: &Entity<InputState>,
        ev: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(ev, InputEvent::Change) {
            self.ensure_trailing_empty_row(window, cx);
            cx.emit(KvTableEvent::Changed);
        }
    }

    fn row_is_empty(&self, r: &KvRow, cx: &App) -> bool {
        let value_empty = match r.kind {
            RowKind::Text => r.value.read(cx).value().is_empty(),
            RowKind::File => r.file.is_none(),
        };
        r.key.read(cx).value().is_empty()
            && value_empty
            && r.description.read(cx).value().is_empty()
    }

    fn ensure_trailing_empty_row(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.locked_keys {
            return;
        }
        let last_is_empty = self
            .rows
            .last()
            .map(|r| self.row_is_empty(r, cx))
            .unwrap_or(false);
        if !last_is_empty {
            self.push_row("", "", "", true, window, cx);
            cx.notify();
        }
    }

    pub fn values(&self, cx: &App) -> Vec<KeyValue> {
        self.rows
            .iter()
            .filter(|r| !self.row_is_empty(r, cx))
            .map(|r| KeyValue {
                key: r.key.read(cx).value().to_string(),
                value: r.value.read(cx).value().to_string(),
                enabled: r.enabled,
                description: r.description.read(cx).value().to_string(),
            })
            .collect()
    }

    pub fn count(&self, cx: &App) -> usize {
        self.values(cx)
            .iter()
            .filter(|kv| kv.enabled && !kv.key.is_empty())
            .count()
    }

    /// 用给定键值整体替换所有行（程序化载入，**不发** `Changed` 事件）；非锁定表末尾补一个空行用于新增。
    pub fn set_values(&mut self, values: &[KeyValue], window: &mut Window, cx: &mut Context<Self>) {
        self.rows.clear();
        for kv in values {
            self.push_row(&kv.key, &kv.value, &kv.description, kv.enabled, window, cx);
        }
        if !self.locked_keys {
            self.push_row("", "", "", true, window, cx);
        }
        cx.notify();
    }

    pub fn form_fields(&self, cx: &App) -> Vec<FormField> {
        self.rows
            .iter()
            .filter(|r| !self.row_is_empty(r, cx))
            .map(|r| FormField {
                key: r.key.read(cx).value().to_string(),
                enabled: r.enabled,
                description: r.description.read(cx).value().to_string(),
                value: match r.kind {
                    RowKind::Text => FormValue::Text {
                        value: r.value.read(cx).value().to_string(),
                    },
                    RowKind::File => match &r.file {
                        Some(f) => FormValue::File {
                            path: f.path.clone(),
                            content_type: f.content_type.clone(),
                        },
                        None => FormValue::File {
                            path: PathBuf::new(),
                            content_type: None,
                        },
                    },
                },
            })
            .collect()
    }

    /// 程序化载入（不发 `Changed`）；末尾补空行；文件行后台读大小。
    pub fn set_form_fields(
        &mut self,
        fields: &[FormField],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.rows.clear();
        for f in fields {
            match &f.value {
                FormValue::Text { value } => {
                    self.push_row(&f.key, value, &f.description, f.enabled, window, cx);
                }
                FormValue::File { path, content_type } => {
                    self.push_row(&f.key, "", &f.description, f.enabled, window, cx);
                    let has_path = !path.as_os_str().is_empty();
                    let row = self.rows.last_mut().expect("just pushed");
                    row.kind = RowKind::File;
                    if has_path {
                        row.file = Some(FileCell {
                            path: path.clone(),
                            content_type: content_type.clone(),
                            size: None,
                        });
                        self.refresh_row_file_size(path.clone(), cx);
                    }
                }
            }
        }
        self.push_row("", "", "", true, window, cx);
        cx.notify();
    }

    pub fn set_row_kind(&mut self, ix: usize, kind: RowKind, cx: &mut Context<Self>) {
        let Some(row) = self.rows.get_mut(ix) else {
            return;
        };
        if row.kind == kind {
            return;
        }
        row.kind = kind;
        if kind == RowKind::Text {
            row.file = None;
        }
        cx.emit(KvTableEvent::Changed);
        cx.notify();
    }

    /// 系统打开对话框 → 立刻按行号写路径（对话框期间行被删则丢弃结果）→ 大小按路径后台补齐。
    /// 路径不等后台读大小就写回：中间再 await 一次的话，期间删掉 `ix` 之上的行会把文件写到别人身上。
    /// 写回走 `update_in`：在末尾空行上选文件会让那行不再为空，得顺手补出新的末尾空行。
    pub fn choose_row_file(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(tr!("common.choose")),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = rx.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let _ = this.update_in(cx, |this, window, cx| {
                let Some(row) = this.rows.get_mut(ix) else {
                    return;
                };
                row.kind = RowKind::File;
                row.file = Some(FileCell {
                    path: path.clone(),
                    content_type: None,
                    size: None,
                });
                this.ensure_trailing_empty_row(window, cx);
                cx.emit(KvTableEvent::Changed);
                cx.notify();
                this.refresh_row_file_size(path, cx);
            });
        })
        .detach();
    }

    /// 后台读大小，按路径写回（载入草稿时行号可能还会变，按路径更稳）。
    fn refresh_row_file_size(&self, path: PathBuf, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let size = cx
                .background_spawn({
                    let path = path.clone();
                    async move { std::fs::metadata(&path).map(|m| m.len()).ok() }
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                for row in this.rows.iter_mut() {
                    if let Some(f) = row.file.as_mut()
                        && f.path == path
                    {
                        f.size = size;
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    #[cfg(test)]
    pub fn row_file_size(&self, ix: usize) -> Option<u64> {
        self.rows
            .get(ix)
            .and_then(|r| r.file.as_ref())
            .and_then(|f| f.size)
    }

    #[cfg(test)]
    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    /// 第 `ix` 行 Key 输入框当前的占位符（测试切换语言后的刷新）。
    #[cfg(test)]
    pub fn key_placeholder(&self, ix: usize, cx: &App) -> SharedString {
        self.rows[ix]
            .key
            .read(cx)
            .presentation()
            .placeholder()
            .clone()
    }

    /// 让行的 key 集合等于 `names`（保持顺序），保留同名行已有的 value 与 enabled。
    pub fn sync_keys(&mut self, names: &[String], window: &mut Window, cx: &mut Context<Self>) {
        let existing: Vec<(String, String, String, bool)> = self
            .rows
            .iter()
            .map(|r| {
                (
                    r.key.read(cx).value().to_string(),
                    r.value.read(cx).value().to_string(),
                    r.description.read(cx).value().to_string(),
                    r.enabled,
                )
            })
            .collect();
        if existing
            .iter()
            .map(|(k, _, _, _)| k.as_str())
            .eq(names.iter().map(String::as_str))
        {
            return;
        }
        self.rows.clear();
        for name in names {
            let (value, description, enabled) = existing
                .iter()
                .find(|(k, _, _, _)| k == name)
                .map(|(_, v, d, e)| (v.clone(), d.clone(), *e))
                .unwrap_or_else(|| (String::new(), String::new(), true));
            self.push_row(name, &value, &description, enabled, window, cx);
        }
        cx.emit(KvTableEvent::Changed);
        cx.notify();
    }

    fn render_value_cell(&self, ix: usize, row: &KvRow, cx: &mut Context<Self>) -> AnyElement {
        if row.kind == RowKind::Text {
            return Input::new(&row.value)
                .small()
                .appearance(false)
                .w_full()
                .aria_label(row_aria_label(ix, &self.placeholder.value_text()))
                .into_any_element();
        }
        let muted = cx.theme().muted_foreground;
        h_flex()
            .w_full()
            .min_w_0()
            .px_1()
            .gap_2()
            .items_center()
            .child(
                Button::new(("kv-choose-file", ix))
                    .outline()
                    .xsmall()
                    .label(tr!("common.choose_file"))
                    .tooltip(row_aria_label(ix, &tr!("kv.choose_file_aria")))
                    .on_click(
                        cx.listener(move |this, _, window, cx| {
                            this.choose_row_file(ix, window, cx)
                        }),
                    ),
            )
            .child(match &row.file {
                Some(f) => {
                    let full = f.path.display().to_string();
                    let name = f
                        .path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| full.clone());
                    let mime = f
                        .content_type
                        .clone()
                        .unwrap_or_else(|| guess_content_type(&f.path).to_string());
                    h_flex()
                        .min_w_0()
                        .gap_2()
                        .text_sm()
                        .child(
                            div()
                                .id(("kv-file-name", ix))
                                .min_w_0()
                                .truncate()
                                .font_family(cx.theme().mono_font_family.clone())
                                .tooltip(move |window, cx| {
                                    Tooltip::new(full.clone()).build(window, cx)
                                })
                                .child(name),
                        )
                        .child(div().flex_none().text_xs().text_color(muted).child(mime))
                        .when_some(f.size, |h, size| {
                            h.child(
                                div()
                                    .flex_none()
                                    .text_xs()
                                    .text_color(muted)
                                    .child(format_bytes(size)),
                            )
                        })
                        .into_any_element()
                }
                None => div()
                    .text_sm()
                    .text_color(muted)
                    .child(tr!("kv.no_file"))
                    .into_any_element(),
            })
            .into_any_element()
    }

    fn remove_row(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if ix < self.rows.len() {
            self.rows.remove(ix);
        }
        self.ensure_trailing_empty_row(window, cx);
        cx.emit(KvTableEvent::Changed);
        cx.notify();
    }

    fn toggle_row(&mut self, ix: usize, checked: bool, cx: &mut Context<Self>) {
        if let Some(r) = self.rows.get_mut(ix) {
            r.enabled = checked;
        }
        cx.emit(KvTableEvent::Changed);
        cx.notify();
    }
}

impl KvTable {
    fn on_column_drag(&mut self, event: &DragMoveEvent<ColumnDrag>, cx: &mut Context<Self>) {
        let width = event.bounds.size.width.as_f32();
        if width <= 0. {
            return;
        }
        let x = (event.event.position.x - event.bounds.origin.x).as_f32() / width;
        let boundary = event.drag(cx).0;
        let before = self.column_fractions;
        move_column_boundary(&mut self.column_fractions, boundary, x);
        if before != self.column_fractions {
            cx.notify();
        }
    }

    /// 一个表格单元：按比例占宽；除最后一列外右侧画分隔线。
    fn cell(&self, col: usize, cx: &App) -> gpui::Div {
        div()
            // 顺序有讲究：flex_none() 会把 flex-basis 一并重置成 auto，必须先调它再给 basis
            .flex_none()
            .flex_basis(relative(self.column_fractions[col]))
            .min_w_0()
            .h_full()
            .flex()
            .items_center()
            .when(col + 1 < COLUMNS, |d| {
                d.border_r_1().border_color(cx.theme().table_row_border)
            })
    }

    fn render_header(&self, cx: &mut Context<Self>) -> AnyElement {
        let labels = ["Key", "Value", "Description"];
        let primary = cx.theme().primary;
        h_flex()
            .w_full()
            .h(TABLE_SIZE.table_row_height() - px(2.))
            .flex_none()
            .bg(cx.theme().table_head)
            .border_b_1()
            .border_color(cx.theme().table_row_border)
            .text_xs()
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(cx.theme().table_head_foreground)
            .child(div().w_8().flex_none())
            .child(
                h_flex()
                    .id("kv-header-columns")
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    // 拖拽期间 gpui 在捕获阶段把每次移动都送到这里（不要求悬停），
                    // 并附带本元素的 bounds：正好是三列共享的可变区域。
                    .on_drag_move(cx.listener(|this, e: &DragMoveEvent<ColumnDrag>, _, cx| {
                        this.on_column_drag(e, cx)
                    }))
                    .children((0..COLUMNS).map(|col| {
                        self.cell(col, cx)
                            .relative()
                            .px_2()
                            .child(labels[col])
                            .when(col + 1 < COLUMNS, |d| {
                                d.child(
                                    // 盖在分隔线上的 7 px 把手：悬停变色、拖动改列宽
                                    div()
                                        .id(("kv-col-handle", col))
                                        .absolute()
                                        .top_0()
                                        .bottom_0()
                                        .right(px(-4.))
                                        .w(px(7.))
                                        .cursor(CursorStyle::ResizeLeftRight)
                                        // 拖柄悬停色：主题没有「拖柄」角色，用 primary 减半透明度
                                        // 表达"可拖"而不是"已选"——与 px(7.) 命中区一样是文档化例外
                                        .hover(|d| d.bg(primary.opacity(0.5)))
                                        .on_drag(ColumnDrag(col), |drag, _, _, cx| {
                                            cx.new(|_| *drag)
                                        }),
                                )
                            })
                    })),
            )
            .child(div().w_8().flex_none())
            .into_any_element()
    }

    // 返回 AnyElement 而不是 impl IntoElement：2024 edition 的 impl Trait 会捕获 `cx` 的
    // 生命周期，在 `rows.iter().map(...)` 里逐行调用时借用检查过不去。
    fn render_row(&self, ix: usize, row: &KvRow, cx: &mut Context<Self>) -> AnyElement {
        let locked = self.locked_keys;
        let file_capable = self.file_capable;
        let hover_bg = cx.theme().table_hover;
        h_flex()
            .id(("kv-row", ix))
            .w_full()
            .h(TABLE_SIZE.table_row_height())
            .flex_none()
            .border_b_1()
            .border_color(cx.theme().table_row_border)
            .hover(|d| d.bg(hover_bg))
            .child(
                // 可访问名称：gpui-component Checkbox 只有可见 label 会进入 a11y 树（tooltip 不算），
                // 每行加可见文字又会让表格变宽。折中：外面包一个带 role + aria_label 的组，
                // 屏幕阅读器先读"启用（第 N 行）"再读复选框。上游加 Checkbox::aria_label 后可去掉包装。
                div()
                    .id(("kv-enabled-cell", ix))
                    .role(Role::Group)
                    .aria_label(row_aria_label(ix, &tr!("kv.enabled")))
                    .w_8()
                    .h_full()
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        Checkbox::new(("kv-enabled", ix))
                            .checked(row.enabled)
                            .tooltip(row_aria_label(ix, &tr!("kv.enabled")))
                            .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                                this.toggle_row(ix, *checked, cx)
                            })),
                    ),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .child(
                        self.cell(0, cx).child(
                            Input::new(&row.key)
                                .small()
                                .appearance(false)
                                .w_full()
                                .readonly(locked)
                                .aria_label(row_aria_label(ix, &self.placeholder.key_text())),
                        ),
                    )
                    .child(
                        self.cell(1, cx)
                            .when(file_capable, |d| {
                                let kind = row.kind;
                                d.child(
                                    div().pl_1().flex_none().child(
                                        Button::new(("kv-kind", ix))
                                            .ghost()
                                            .xsmall()
                                            .label(kind.label())
                                            .tooltip(row_aria_label(ix, &tr!("kv.kind_toggle")))
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.set_row_kind(ix, kind.toggled(), cx)
                                            })),
                                    ),
                                )
                            })
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(self.render_value_cell(ix, row, cx)),
                            ),
                    )
                    .child(
                        self.cell(2, cx).child(
                            Input::new(&row.description)
                                .small()
                                .appearance(false)
                                .w_full()
                                .aria_label(row_aria_label(ix, &tr!("kv.description"))),
                        ),
                    ),
            )
            .child(
                div()
                    .w_8()
                    .h_full()
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        Button::new(("kv-remove", ix))
                            .ghost()
                            .xsmall()
                            .icon(IconName::Close)
                            .tooltip(row_aria_label(ix, &tr!("kv.remove")))
                            .disabled(locked)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.remove_row(ix, window, cx)
                            })),
                    ),
            )
            .into_any_element()
    }
}

impl Render for KvTable {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let locked = self.locked_keys;
        v_flex()
            .w_full()
            .rounded(cx.theme().radius)
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().table)
            .overflow_hidden()
            .child(self.render_header(cx))
            .when(locked && self.rows.is_empty(), |v| {
                v.child(
                    div()
                        .px_3()
                        .py_2()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(tr!("kv.no_path_params")),
                )
            })
            .children(
                self.rows
                    .iter()
                    .enumerate()
                    .map(|(ix, row)| self.render_row(ix, row, cx))
                    .collect::<Vec<_>>(),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::{
        COLUMNS, DEFAULT_COLUMN_FRACTIONS, MIN_COLUMN_FRACTION, move_column_boundary,
        row_aria_label,
    };

    #[test]
    fn row_aria_labels_carry_the_row_number() {
        // 测试进程的 locale 是 en（见 i18n::locale_test_lock）
        let _locale = crate::i18n::locale_test_lock();
        assert_eq!(row_aria_label(0, "Name").as_ref(), "Name (row 1)");
        assert_eq!(row_aria_label(2, "Remove").as_ref(), "Remove (row 3)");
    }

    fn total(f: &[f32; COLUMNS]) -> f32 {
        f.iter().sum()
    }

    #[test]
    fn default_fractions_sum_to_one() {
        assert!((total(&DEFAULT_COLUMN_FRACTIONS) - 1.).abs() < 1e-6);
    }

    #[test]
    fn dragging_a_boundary_only_moves_its_two_neighbours() {
        let mut f = DEFAULT_COLUMN_FRACTIONS;
        // 第一条分隔线拖到 40%：Key 变 0.40，Value 吸收差值，Description 不动
        move_column_boundary(&mut f, 0, 0.40);
        assert!((f[0] - 0.40).abs() < 1e-6);
        assert!((f[1] - 0.32).abs() < 1e-6);
        assert!((f[2] - 0.28).abs() < 1e-6);
        assert!((total(&f) - 1.).abs() < 1e-6);

        // 第二条分隔线拖到 60%：Key 不动，Value / Description 重新分
        move_column_boundary(&mut f, 1, 0.60);
        assert!((f[0] - 0.40).abs() < 1e-6);
        assert!((f[1] - 0.20).abs() < 1e-6);
        assert!((f[2] - 0.40).abs() < 1e-6);
    }

    #[test]
    fn columns_never_shrink_below_the_minimum() {
        let mut f = DEFAULT_COLUMN_FRACTIONS;
        move_column_boundary(&mut f, 0, -1.);
        assert!((f[0] - MIN_COLUMN_FRACTION).abs() < 1e-6);
        assert!((total(&f) - 1.).abs() < 1e-6);

        move_column_boundary(&mut f, 1, 2.);
        assert!((f[2] - MIN_COLUMN_FRACTION).abs() < 1e-6);
        assert!((total(&f) - 1.).abs() < 1e-6);
    }

    #[test]
    fn the_last_column_has_no_boundary_to_drag() {
        let mut f = DEFAULT_COLUMN_FRACTIONS;
        move_column_boundary(&mut f, COLUMNS - 1, 0.5);
        assert_eq!(f, DEFAULT_COLUMN_FRACTIONS);
    }
}
