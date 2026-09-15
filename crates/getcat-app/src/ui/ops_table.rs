//! 前后置操作表：与 [`crate::ui::kv_table`] 同款的"末尾常驻空行"表格，两种模式共用一个行结构。
//!
//! 前置模式一行 = 一条「设置变量」；后置模式的类型下拉决定这一行是提取还是断言、
//! 读响应的哪一部分。行内两个输入框的含义随类型变（见 `OpsTable::placeholders`）。

use getcat_core::model::{
    AssertOp, PostOp, PostOpKind, PreOp, PreOpKind, ResponseSource, VarScope,
};
use gpui_kit::prelude::FluentBuilder as _;
// 显式导入而非 `use gpui_kit::*`：本文件含 `#[cfg(test)] mod tests`，通配符会引入 gpui 重导出的
// `#[test]` 属性宏并与标准库同名冲突。编译器报"找不到 X"时把 X 加进这里，不要改回通配符。
use gpui_kit::component::{
    ActiveTheme, IconName, IndexPath, Sizable,
    button::{Button, ButtonVariants},
    checkbox::Checkbox,
    h_flex,
    input::{Input, InputEvent, InputState},
    select::{Select, SelectEvent, SelectState},
    v_flex,
};
use gpui_kit::{
    AnyElement, App, AppContext, Context, Entity, EventEmitter, InteractiveElement, IntoElement,
    ParentElement, Render, Role, SharedString, StatefulInteractiveElement, Styled, Subscription,
    Window, div,
};

use crate::i18n::{Locale, tr};
use crate::ui::kv_table::{TABLE_SIZE, row_aria_label};

pub enum OpsTableEvent {
    Changed,
}

// 任务 3 把两张表接进 RequestTab 后才会在非测试代码里构造，本任务只有测试在用
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpsMode {
    Pre,
    Post,
}

/// 后置行的类型；`ALL` 的顺序就是类型下拉的顺序。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PostRowKind {
    ExtractJson,
    ExtractHeader,
    ExtractStatus,
    AssertStatus,
    AssertHeader,
    AssertJson,
}

impl PostRowKind {
    pub const ALL: [PostRowKind; 6] = [
        PostRowKind::ExtractJson,
        PostRowKind::ExtractHeader,
        PostRowKind::ExtractStatus,
        PostRowKind::AssertStatus,
        PostRowKind::AssertHeader,
        PostRowKind::AssertJson,
    ];

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|k| *k == self).unwrap_or(0)
    }

    pub fn from_index(ix: usize) -> Self {
        Self::ALL
            .get(ix)
            .copied()
            .unwrap_or(PostRowKind::ExtractJson)
    }

    pub fn label(self) -> SharedString {
        match self {
            PostRowKind::ExtractJson => tr!("ops.kind_extract_json"),
            PostRowKind::ExtractHeader => tr!("ops.kind_extract_header"),
            PostRowKind::ExtractStatus => tr!("ops.kind_extract_status"),
            PostRowKind::AssertStatus => tr!("ops.kind_assert_status"),
            PostRowKind::AssertHeader => tr!("ops.kind_assert_header"),
            PostRowKind::AssertJson => tr!("ops.kind_assert_json"),
        }
    }

    pub fn is_assert(self) -> bool {
        matches!(
            self,
            PostRowKind::AssertStatus | PostRowKind::AssertHeader | PostRowKind::AssertJson
        )
    }

    /// 参数 A 是否有意义（Status 类型没有）。
    fn has_source_arg(self) -> bool {
        !matches!(self, PostRowKind::ExtractStatus | PostRowKind::AssertStatus)
    }

    fn source_placeholder(self) -> SharedString {
        match self {
            PostRowKind::ExtractJson | PostRowKind::AssertJson => tr!("ops.path_placeholder"),
            _ => tr!("ops.header_placeholder"),
        }
    }

    pub fn from_op(op: &PostOp) -> Self {
        match &op.kind {
            PostOpKind::Extract { source, .. } => match source {
                ResponseSource::JsonPath { .. } => PostRowKind::ExtractJson,
                ResponseSource::Header { .. } => PostRowKind::ExtractHeader,
                ResponseSource::Status => PostRowKind::ExtractStatus,
            },
            PostOpKind::Assert { subject, .. } => match subject {
                ResponseSource::JsonPath { .. } => PostRowKind::AssertJson,
                ResponseSource::Header { .. } => PostRowKind::AssertHeader,
                ResponseSource::Status => PostRowKind::AssertStatus,
            },
        }
    }
}

pub fn scope_label(scope: VarScope) -> SharedString {
    match scope {
        VarScope::Global => tr!("ops.scope_global"),
        VarScope::Environment => tr!("ops.scope_environment"),
        VarScope::Group => tr!("ops.scope_group"),
    }
}

pub fn assert_op_label(op: AssertOp) -> SharedString {
    match op {
        AssertOp::Equals => tr!("ops.op_equals"),
        AssertOp::NotEquals => tr!("ops.op_not_equals"),
        AssertOp::Contains => tr!("ops.op_contains"),
        AssertOp::Exists => tr!("ops.op_exists"),
    }
}

/// 一条后置操作在行里的两个文本：`(参数 A, 参数 B)`。
pub fn row_texts(op: &PostOp) -> (String, String) {
    let source_text = |s: &ResponseSource| match s {
        ResponseSource::Status => String::new(),
        ResponseSource::Header { name } => name.clone(),
        ResponseSource::JsonPath { path } => path.clone(),
    };
    match &op.kind {
        PostOpKind::Extract { key, source, .. } => (source_text(source), key.clone()),
        PostOpKind::Assert {
            subject, expected, ..
        } => (source_text(subject), expected.clone()),
    }
}

fn source_of(kind: PostRowKind, a: &str) -> ResponseSource {
    match kind {
        PostRowKind::ExtractJson | PostRowKind::AssertJson => ResponseSource::JsonPath {
            path: a.to_string(),
        },
        PostRowKind::ExtractHeader | PostRowKind::AssertHeader => ResponseSource::Header {
            name: a.to_string(),
        },
        PostRowKind::ExtractStatus | PostRowKind::AssertStatus => ResponseSource::Status,
    }
}

pub(crate) fn build_post_op(
    kind: PostRowKind,
    enabled: bool,
    a: &str,
    scope: VarScope,
    op: AssertOp,
    b: &str,
) -> PostOp {
    let kind = if kind.is_assert() {
        PostOpKind::Assert {
            subject: source_of(kind, a),
            op,
            expected: b.to_string(),
        }
    } else {
        PostOpKind::Extract {
            scope,
            key: b.to_string(),
            source: source_of(kind, a),
        }
    };
    PostOp { enabled, kind }
}

/// 一行里哪些输入框生效：`(参数 A, 参数 B)`。前置模式两个都生效；后置模式 Status 类型没有参数 A，
/// Exists 断言没有期望值。失效的输入框禁用但文字保留（切回来还在），判空与生成操作时当作空串。
fn active_inputs(mode: OpsMode, kind: PostRowKind, op: AssertOp) -> (bool, bool) {
    match mode {
        OpsMode::Pre => (true, true),
        OpsMode::Post => (
            kind.has_source_arg(),
            !(kind.is_assert() && op == AssertOp::Exists),
        ),
    }
}

type LabelSelect = SelectState<Vec<SharedString>>;

struct OpsRow {
    enabled: bool,
    kind: PostRowKind,
    scope: VarScope,
    op: AssertOp,
    kind_select: Entity<LabelSelect>,
    scope_select: Entity<LabelSelect>,
    op_select: Entity<LabelSelect>,
    a: Entity<InputState>,
    b: Entity<InputState>,
    _subs: Vec<Subscription>,
}

pub struct OpsTable {
    mode: OpsMode,
    rows: Vec<OpsRow>,
    /// 下拉项与占位符都驻留在实体里，切换界面语言时由这个订阅整表重建。
    _locale_sub: Subscription,
}

impl EventEmitter<OpsTableEvent> for OpsTable {}

fn kind_items() -> Vec<SharedString> {
    PostRowKind::ALL.iter().map(|k| k.label()).collect()
}

fn scope_items() -> Vec<SharedString> {
    VarScope::ALL.iter().map(|s| scope_label(*s)).collect()
}

fn op_items() -> Vec<SharedString> {
    AssertOp::ALL.iter().map(|o| assert_op_label(*o)).collect()
}

fn selected_row(sel: &Entity<LabelSelect>, cx: &App) -> usize {
    sel.read(cx).selected_index(cx).map(|p| p.row).unwrap_or(0)
}

impl OpsTable {
    // 任务 3 的 RequestTab 调用，本任务只有测试在用
    #[allow(dead_code)]
    pub fn new(mode: OpsMode, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let locale_sub = cx.observe_global_in::<Locale>(window, Self::refresh_locale);
        let mut this = Self {
            mode,
            rows: Vec::new(),
            _locale_sub: locale_sub,
        };
        this.push_empty_row(window, cx);
        this
    }

    /// 界面语言变了：下拉项与占位符都是文案，整表重建（行数少，代价可忽略）。
    fn refresh_locale(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.mode {
            OpsMode::Pre => {
                let ops = self.pre_ops(cx);
                self.set_pre_ops(&ops, window, cx);
            }
            OpsMode::Post => {
                let ops = self.post_ops(cx);
                self.set_post_ops(&ops, window, cx);
            }
        }
    }

    // 私有构造器，逐个字段传参比临时建一个只用一次的参数结构体更直观。
    #[allow(clippy::too_many_arguments)]
    fn push_row(
        &mut self,
        enabled: bool,
        kind: PostRowKind,
        scope: VarScope,
        op: AssertOp,
        a: &str,
        b: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let kind_select = cx.new(|cx| {
            SelectState::new(kind_items(), Some(IndexPath::new(kind.index())), window, cx)
        });
        let scope_select = cx.new(|cx| {
            SelectState::new(
                scope_items(),
                Some(IndexPath::new(scope.index())),
                window,
                cx,
            )
        });
        let op_select =
            cx.new(|cx| SelectState::new(op_items(), Some(IndexPath::new(op.index())), window, cx));
        let (a_ph, b_ph) = self.placeholders(kind);
        let a_state = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(a_ph)
                .default_value(a.to_string())
        });
        let b_state = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(b_ph)
                .default_value(b.to_string())
        });
        // 订阅里不捕获行号：删掉前面的行后，后面的行整体上移，创建时记下的行号就指错了行。
        // 一律按下拉实体找回它所在的行，找不到（行已删）就忽略。
        let subs = vec![
            cx.subscribe_in(
                &kind_select,
                window,
                |this, sel, _: &SelectEvent<Vec<SharedString>>, window, cx| {
                    let Some(ix) = this.rows.iter().position(|r| r.kind_select == *sel) else {
                        return;
                    };
                    let kind = PostRowKind::from_index(selected_row(sel, cx));
                    this.set_row_kind(ix, kind, window, cx);
                },
            ),
            cx.subscribe_in(
                &scope_select,
                window,
                |this, sel, _: &SelectEvent<Vec<SharedString>>, window, cx| {
                    let scope = VarScope::from_index(selected_row(sel, cx));
                    let Some(ix) = this.rows.iter().position(|r| r.scope_select == *sel) else {
                        return;
                    };
                    if this.rows[ix].scope == scope {
                        return;
                    }
                    let was_empty = this.row_is_empty_at(ix, cx);
                    this.rows[ix].scope = scope;
                    this.after_row_select_change(ix, was_empty, window, cx);
                },
            ),
            cx.subscribe_in(
                &op_select,
                window,
                |this, sel, _: &SelectEvent<Vec<SharedString>>, window, cx| {
                    let op = AssertOp::from_index(selected_row(sel, cx));
                    let Some(ix) = this.rows.iter().position(|r| r.op_select == *sel) else {
                        return;
                    };
                    if this.rows[ix].op == op {
                        return;
                    }
                    // 算子决定期望值输入框是否生效（Exists），可能改变这一行是否为空
                    let was_empty = this.row_is_empty_at(ix, cx);
                    this.rows[ix].op = op;
                    this.after_row_select_change(ix, was_empty, window, cx);
                },
            ),
            cx.subscribe_in(&a_state, window, Self::on_input_event),
            cx.subscribe_in(&b_state, window, Self::on_input_event),
        ];
        self.rows.push(OpsRow {
            enabled,
            kind,
            scope,
            op,
            kind_select,
            scope_select,
            op_select,
            a: a_state,
            b: b_state,
            _subs: subs,
        });
    }

    fn placeholders(&self, kind: PostRowKind) -> (SharedString, SharedString) {
        match self.mode {
            OpsMode::Pre => (
                tr!("ops.variable_placeholder"),
                tr!("ops.value_placeholder"),
            ),
            OpsMode::Post => (
                kind.source_placeholder(),
                if kind.is_assert() {
                    tr!("ops.expected_placeholder")
                } else {
                    tr!("ops.variable_placeholder")
                },
            ),
        }
    }

    fn push_empty_row(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.push_row(
            true,
            PostRowKind::ExtractJson,
            VarScope::Global,
            AssertOp::Equals,
            "",
            "",
            window,
            cx,
        );
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
            cx.emit(OpsTableEvent::Changed);
        }
    }

    /// 一行参与生成操作的两个文本 `(参数 A, 参数 B)`：失效的输入框当作空串。
    fn active_texts(&self, r: &OpsRow, cx: &App) -> (SharedString, SharedString) {
        let (a_on, b_on) = active_inputs(self.mode, r.kind, r.op);
        let text = |on: bool, input: &Entity<InputState>| {
            if on {
                input.read(cx).value()
            } else {
                SharedString::default()
            }
        };
        (text(a_on, &r.a), text(b_on, &r.b))
    }

    /// 生效的输入框全空即为空行（后置行一个生效输入框都没有时也算空，不产出操作）。
    fn row_is_empty(&self, r: &OpsRow, cx: &App) -> bool {
        let (a, b) = self.active_texts(r, cx);
        a.is_empty() && b.is_empty()
    }

    fn row_is_empty_at(&self, ix: usize, cx: &App) -> bool {
        self.rows.get(ix).is_none_or(|r| self.row_is_empty(r, cx))
    }

    /// 下拉改完之后调用。改动前或改动后这一行不为空，才可能影响 `pre_ops` / `post_ops`，
    /// 这时才发 `Changed`：空行上挑下拉不该把页签标成已修改。
    /// 下拉可能让失效输入框里保留的文字重新生效，末尾行因此变成非空时要补新的空行。
    fn after_row_select_change(
        &mut self,
        ix: usize,
        was_empty: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let now_empty = self.row_is_empty_at(ix, cx);
        self.ensure_trailing_empty_row(window, cx);
        if !(was_empty && now_empty) {
            cx.emit(OpsTableEvent::Changed);
        }
        cx.notify();
    }

    fn ensure_trailing_empty_row(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let last_is_empty = self
            .rows
            .last()
            .map(|r| self.row_is_empty(r, cx))
            .unwrap_or(false);
        if !last_is_empty {
            self.push_empty_row(window, cx);
            cx.notify();
        }
    }

    fn set_row_kind(
        &mut self,
        ix: usize,
        kind: PostRowKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (a_ph, b_ph) = self.placeholders(kind);
        let Some(row) = self.rows.get(ix) else {
            return;
        };
        if row.kind == kind {
            return;
        }
        let was_empty = self.row_is_empty(row, cx);
        let row = &mut self.rows[ix];
        row.kind = kind;
        row.a
            .update(cx, |s, cx| s.set_placeholder(a_ph, window, cx));
        row.b
            .update(cx, |s, cx| s.set_placeholder(b_ph, window, cx));
        self.after_row_select_change(ix, was_empty, window, cx);
    }

    /// 删掉空行不影响操作列表，不发 `Changed`。
    pub(crate) fn remove_row(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let was_empty = self.row_is_empty_at(ix, cx);
        if ix < self.rows.len() {
            self.rows.remove(ix);
        }
        self.ensure_trailing_empty_row(window, cx);
        if !was_empty {
            cx.emit(OpsTableEvent::Changed);
        }
        cx.notify();
    }

    /// 启用状态不影响判空；空行不进操作列表，勾选它不发 `Changed`。
    fn toggle_row(&mut self, ix: usize, checked: bool, cx: &mut Context<Self>) {
        let Some(r) = self.rows.get_mut(ix) else {
            return;
        };
        r.enabled = checked;
        if !self.row_is_empty_at(ix, cx) {
            cx.emit(OpsTableEvent::Changed);
        }
        cx.notify();
    }

    pub fn pre_ops(&self, cx: &App) -> Vec<PreOp> {
        self.rows
            .iter()
            .filter(|r| !self.row_is_empty(r, cx))
            .map(|r| {
                let (key, value) = self.active_texts(r, cx);
                PreOp {
                    enabled: r.enabled,
                    kind: PreOpKind::SetVariable {
                        scope: r.scope,
                        key: key.to_string(),
                        value: value.to_string(),
                    },
                }
            })
            .collect()
    }

    /// 程序化载入（不发 `Changed`）；末尾补空行。
    pub fn set_pre_ops(&mut self, ops: &[PreOp], window: &mut Window, cx: &mut Context<Self>) {
        self.rows.clear();
        for op in ops {
            // 目前只有「设置变量」一种；以后加种类时这里按 kind 分派
            let PreOpKind::SetVariable { scope, key, value } = &op.kind;
            self.push_row(
                op.enabled,
                PostRowKind::ExtractJson,
                *scope,
                AssertOp::Equals,
                key,
                value,
                window,
                cx,
            );
        }
        self.push_empty_row(window, cx);
        cx.notify();
    }

    pub fn post_ops(&self, cx: &App) -> Vec<PostOp> {
        self.rows
            .iter()
            .filter(|r| !self.row_is_empty(r, cx))
            .map(|r| {
                let (a, b) = self.active_texts(r, cx);
                build_post_op(r.kind, r.enabled, &a, r.scope, r.op, &b)
            })
            .collect()
    }

    /// 程序化载入（不发 `Changed`）；末尾补空行。
    pub fn set_post_ops(&mut self, ops: &[PostOp], window: &mut Window, cx: &mut Context<Self>) {
        self.rows.clear();
        for op in ops {
            let kind = PostRowKind::from_op(op);
            let (scope, aop) = match &op.kind {
                PostOpKind::Extract { scope, .. } => (*scope, AssertOp::Equals),
                PostOpKind::Assert { op, .. } => (VarScope::Global, *op),
            };
            let (a, b) = row_texts(op);
            self.push_row(op.enabled, kind, scope, aop, &a, &b, window, cx);
        }
        self.push_empty_row(window, cx);
        cx.notify();
    }

    /// 启用且非空的行数（页签角标）。
    // 任务 3 的请求页签角标调用，本任务还没有调用方
    #[allow(dead_code)]
    pub fn count(&self, cx: &App) -> usize {
        self.rows
            .iter()
            .filter(|r| r.enabled && !self.row_is_empty(r, cx))
            .count()
    }

    #[cfg(test)]
    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    #[cfg(test)]
    pub fn row_kind_select(&self, ix: usize) -> Entity<LabelSelect> {
        self.rows[ix].kind_select.clone()
    }

    #[cfg(test)]
    pub fn row_scope_select(&self, ix: usize) -> Entity<LabelSelect> {
        self.rows[ix].scope_select.clone()
    }

    #[cfg(test)]
    pub fn row_op_select(&self, ix: usize) -> Entity<LabelSelect> {
        self.rows[ix].op_select.clone()
    }

    #[cfg(test)]
    pub fn row_a_input(&self, ix: usize) -> Entity<InputState> {
        self.rows[ix].a.clone()
    }

    #[cfg(test)]
    pub fn row_b_input(&self, ix: usize) -> Entity<InputState> {
        self.rows[ix].b.clone()
    }
}

impl OpsTable {
    // 返回 AnyElement 而不是 impl IntoElement：2024 edition 的 impl Trait 会捕获 `cx` 的
    // 生命周期，在 `rows.iter().map(...)` 里逐行调用时借用检查过不去。
    fn render_row(&self, ix: usize, row: &OpsRow, cx: &mut Context<Self>) -> AnyElement {
        let mode = self.mode;
        let kind = row.kind;
        let hover_bg = cx.theme().table_hover;
        let is_assert_row = mode == OpsMode::Post && kind.is_assert();
        let (a_on, b_on) = active_inputs(mode, kind, row.op);
        let (a_disabled, b_disabled) = (!a_on, !b_on);
        let (a_ph, b_ph) = self.placeholders(kind);
        h_flex()
            .id(("ops-row", ix))
            .w_full()
            .h(TABLE_SIZE.table_row_height())
            .flex_none()
            .gap_1()
            .px_1()
            .border_b_1()
            .border_color(cx.theme().table_row_border)
            .hover(|d| d.bg(hover_bg))
            .child(
                // Checkbox 只有可见 label 会进入 a11y 树（tooltip 不算），外面包一个带名字的组
                div()
                    .id(("ops-enabled-cell", ix))
                    .role(Role::Group)
                    .aria_label(row_aria_label(ix, &tr!("kv.enabled")))
                    .w_8()
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        Checkbox::new(("ops-enabled", ix))
                            .checked(row.enabled)
                            .tooltip(row_aria_label(ix, &tr!("kv.enabled")))
                            .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                                this.toggle_row(ix, *checked, cx)
                            })),
                    ),
            )
            .when(mode == OpsMode::Post, |h| {
                let label = row_aria_label(ix, &tr!("ops.kind_aria"));
                h.child(
                    // Select 外层无条件 size_full()，必须用定宽 flex_none 容器约束
                    div()
                        .id(("ops-kind", ix))
                        .role(Role::Group)
                        .aria_label(label.clone())
                        .w_40()
                        .flex_none()
                        .child(
                            Select::new(&row.kind_select)
                                .small()
                                .accessibility_label(label),
                        ),
                )
            })
            .child(
                div().flex_1().min_w_0().child(
                    Input::new(&row.a)
                        .small()
                        .appearance(false)
                        .w_full()
                        .disabled(a_disabled)
                        .aria_label(row_aria_label(ix, &a_ph)),
                ),
            )
            .child({
                let label = row_aria_label(
                    ix,
                    &if is_assert_row {
                        tr!("ops.assert_op_aria")
                    } else {
                        tr!("ops.scope_aria")
                    },
                );
                div()
                    .id(("ops-scope-or-op", ix))
                    .role(Role::Group)
                    .aria_label(label.clone())
                    .w_32()
                    .flex_none()
                    .child(
                        Select::new(if is_assert_row {
                            &row.op_select
                        } else {
                            &row.scope_select
                        })
                        .small()
                        .accessibility_label(label),
                    )
            })
            .child(
                div().flex_1().min_w_0().child(
                    Input::new(&row.b)
                        .small()
                        .appearance(false)
                        .w_full()
                        .disabled(b_disabled)
                        .aria_label(row_aria_label(ix, &b_ph)),
                ),
            )
            .child(
                div()
                    .w_8()
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child({
                        let label = row_aria_label(ix, &tr!("kv.remove"));
                        Button::new(("ops-remove", ix))
                            .ghost()
                            .xsmall()
                            .icon(IconName::Close)
                            // 纯图标按钮：可访问名称取 accessibility_label.or(label)，
                            // 不会退回 tooltip，必须显式给一份。
                            .accessibility_label(label.clone())
                            .tooltip(label)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.remove_row(ix, window, cx)
                            }))
                    }),
            )
            .into_any_element()
    }
}

impl Render for OpsTable {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let hint = match self.mode {
            OpsMode::Pre => tr!("ops.pre_hint"),
            OpsMode::Post => tr!("ops.post_hint"),
        };
        v_flex()
            .w_full()
            .gap_1()
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(hint),
            )
            .child(
                v_flex()
                    .w_full()
                    .rounded(cx.theme().radius)
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().table)
                    .overflow_hidden()
                    .children(
                        self.rows
                            .iter()
                            .enumerate()
                            .map(|(ix, row)| self.render_row(ix, row, cx))
                            .collect::<Vec<_>>(),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn post_row_kind_round_trips_through_index_and_op() {
        for (ix, kind) in PostRowKind::ALL.iter().enumerate() {
            assert_eq!(kind.index(), ix);
            assert_eq!(PostRowKind::from_index(ix), *kind);
        }
        let op = PostOp {
            enabled: true,
            kind: PostOpKind::Assert {
                subject: ResponseSource::Header { name: "x".into() },
                op: AssertOp::Contains,
                expected: "y".into(),
            },
        };
        assert_eq!(PostRowKind::from_op(&op), PostRowKind::AssertHeader);
        let (a, b) = row_texts(&op);
        assert_eq!((a.as_str(), b.as_str()), ("x", "y"));
        assert_eq!(
            build_post_op(
                PostRowKind::ExtractJson,
                true,
                "$.a",
                VarScope::Group,
                AssertOp::Equals,
                "k"
            ),
            PostOp {
                enabled: true,
                kind: PostOpKind::Extract {
                    scope: VarScope::Group,
                    key: "k".into(),
                    source: ResponseSource::JsonPath { path: "$.a".into() },
                },
            }
        );
        assert_eq!(
            build_post_op(
                PostRowKind::AssertStatus,
                false,
                "ignored",
                VarScope::Global,
                AssertOp::Exists,
                ""
            ),
            PostOp {
                enabled: false,
                kind: PostOpKind::Assert {
                    subject: ResponseSource::Status,
                    op: AssertOp::Exists,
                    expected: String::new(),
                },
            }
        );
    }
}
