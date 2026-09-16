//! 「变量」抽屉：全局 / 环境 / 分类三层变量的编辑，环境的增删改与激活，Postman 导入导出。
//!
//! 与 [`crate::ui::code_sheet`] 同一条硬约束：正文是独立实体，`Sheet` builder 里只
//! `.child(entity.clone())`。这里连宿主都不需要回调——所有写入直接走
//! [`crate::state::variables::update`]（全局），Workspace 只负责开关抽屉。
//!
//! 抽屉不自己存一份变量：每一页的数据都从全局句柄现读，表格只是它的编辑视图。
//! 于是有两个方向要对齐：
//! - 表格 `Changed` → 读整表写回当前页；
//! - 全局变了（抽屉开着时响应到达后的提取、前置操作）→ 观察者比对后重填表格，
//!   否则用户再改任一格，写回的陈旧整表会抹掉刚提取的值。

use std::path::PathBuf;

use getcat_core::model::{Environment, Ulid, Variable, VariableSets};
use getcat_core::postman_env::{self, PostmanEnv, PostmanEnvError, PostmanScope};
use gpui_kit::component::{
    ActiveTheme, Disableable, IndexPath, Sizable, WindowExt,
    alert::Alert,
    button::{Button, ButtonVariant, ButtonVariants},
    dialog::DialogButtonProps,
    h_flex,
    input::{Input, InputEvent, InputState},
    select::{Select, SelectEvent, SelectState},
    tab::TabBar,
    tag::Tag,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext, Context, Entity, InteractiveElement, IntoElement, ParentElement,
    PathPromptOptions, Render, Role, SharedString, StatefulInteractiveElement, Styled,
    Subscription, Window, div,
};

use crate::i18n::{Locale, tr};
use crate::state::variables::{self, VariablesHandle};
use crate::ui::kv_table::{KvPlaceholder, KvTable, KvTableEvent};
use crate::ui::text::postman_env_error_line;

/// 抽屉的起始宽度：三列变量表加一排环境工具按钮，`Sheet` 默认的 350 px 放不下。
pub const VARIABLES_SHEET_WIDTH: f32 = 640.;

/// 抽屉的三个页，与 [`getcat_core::model::VarScope`] 同序。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SheetScope {
    #[default]
    Global,
    Environment,
    Group,
}

impl SheetScope {
    pub const ALL: [SheetScope; 3] = [
        SheetScope::Global,
        SheetScope::Environment,
        SheetScope::Group,
    ];

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|s| *s == self).unwrap_or(0)
    }

    pub fn from_index(ix: usize) -> Self {
        Self::ALL.get(ix).copied().unwrap_or(SheetScope::Global)
    }

    fn label(self) -> SharedString {
        match self {
            SheetScope::Global => tr!("variables.scope_global"),
            SheetScope::Environment => tr!("variables.scope_environment"),
            SheetScope::Group => tr!("variables.scope_group"),
        }
    }
}

/// 导入失败的原因。
enum ImportError {
    /// 文件读出来了，但不是 Postman environment / globals：种类走翻译（见 [`postman_env_error_line`]）。
    Postman(PostmanEnvError),
    /// 读文件失败：io 原话（技术细节）保留。
    Io(String),
}

/// 一行提示（导入 / 导出结果），下一次打开抽屉时清掉。
/// 存数据而不是译好的文案：切换界面语言后在渲染时重新翻译。
enum Notice {
    Imported(usize),
    ImportFailed(ImportError),
    Exported(PathBuf),
    ExportFailed(String),
}

impl Notice {
    fn is_error(&self) -> bool {
        matches!(self, Notice::ImportFailed(_) | Notice::ExportFailed(_))
    }

    fn text(&self) -> SharedString {
        match self {
            Notice::Imported(count) => tr!("variables.imported", count = count),
            Notice::ImportFailed(ImportError::Postman(error)) => postman_env_error_line(error),
            Notice::ImportFailed(ImportError::Io(error)) => {
                tr!("variables.import_failed", error = error)
            }
            Notice::Exported(path) => tr!("variables.exported", path = path.display()),
            Notice::ExportFailed(error) => tr!("variables.export_failed", error = error),
        }
    }
}

/// 打开环境页时默认选中的环境：激活的那个，没有激活就第一个。
fn default_env_id(sets: &VariableSets) -> Option<Ulid> {
    sets.active_env()
        .map(|e| e.id)
        .or_else(|| sets.environments.first().map(|e| e.id))
}

type LabelSelect = SelectState<Vec<SharedString>>;

pub struct VariablesSheet {
    scope: SheetScope,
    /// 环境页当前选中的环境（不一定是激活的那个）。
    env_id: Option<Ulid>,
    /// 分类页当前选中的分类。
    group: Option<String>,
    /// 分类候选（打开抽屉时由 Workspace 传入快照）。
    groups: Vec<String>,
    env_select: Entity<LabelSelect>,
    group_select: Entity<LabelSelect>,
    /// 上一次灌进 `env_select` 的 `(各环境名, 选中下标)`：没变就不重灌，
    /// 免得每次写回（每次按键）都重置下拉的高亮与滚动。
    env_items: (Vec<SharedString>, Option<usize>),
    env_name: Entity<InputState>,
    table: Entity<KvTable>,
    notice: Option<Notice>,
    _subs: Vec<Subscription>,
}

impl VariablesSheet {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let env_select = cx.new(|cx| SelectState::new(Vec::new(), None, window, cx));
        let group_select = cx.new(|cx| SelectState::new(Vec::new(), None, window, cx));
        let env_name = cx
            .new(|cx| InputState::new(window, cx).placeholder(tr!("variables.rename_placeholder")));
        let table =
            cx.new(|cx| KvTable::new(KvPlaceholder::Variable, window, cx).secret_capable(true));
        let subs = vec![
            cx.subscribe_in(&table, window, |this, _, _: &KvTableEvent, _, cx| {
                this.commit_table(cx)
            }),
            cx.subscribe_in(
                &env_select,
                window,
                |this, sel, _: &SelectEvent<Vec<SharedString>>, window, cx| {
                    let ix = sel.read(cx).selected_index(cx).map(|p| p.row);
                    let id = ix
                        .and_then(|ix| variables::variables(cx).environments.get(ix).map(|e| e.id));
                    this.select_env(id, window, cx);
                },
            ),
            cx.subscribe_in(
                &group_select,
                window,
                |this, sel, _: &SelectEvent<Vec<SharedString>>, window, cx| {
                    let ix = sel.read(cx).selected_index(cx).map(|p| p.row);
                    this.group = ix.and_then(|ix| this.groups.get(ix).cloned());
                    this.fill_table(window, cx);
                },
            ),
            cx.subscribe_in(&env_name, window, Self::on_env_name_event),
            // 变量表会在抽屉开着时被后台改写（响应到达后的提取、前置操作），抽屉必须跟上
            cx.observe_global_in::<VariablesHandle>(window, Self::on_variables_changed),
            // 改名输入框的占位符驻留在 InputState 里，切换界面语言时自己刷新
            cx.observe_global_in::<Locale>(window, |this, window, cx| {
                this.env_name.update(cx, |s, cx| {
                    s.set_placeholder(tr!("variables.rename_placeholder"), window, cx)
                });
            }),
        ];
        Self {
            scope: SheetScope::Global,
            env_id: None,
            group: None,
            groups: Vec::new(),
            env_select,
            group_select,
            env_items: (Vec::new(), None),
            env_name,
            table,
            notice: None,
            _subs: subs,
        }
    }

    #[cfg(test)]
    pub fn table(&self) -> &Entity<KvTable> {
        &self.table
    }

    #[cfg(test)]
    pub fn env_select(&self) -> &Entity<LabelSelect> {
        &self.env_select
    }

    #[cfg(test)]
    pub fn group_select(&self) -> &Entity<LabelSelect> {
        &self.group_select
    }

    #[cfg(test)]
    pub fn env_name(&self) -> &Entity<InputState> {
        &self.env_name
    }

    /// 打开抽屉时由 Workspace 调用：定位到某一页，重读变量表。
    /// `group` 为 None 时分类页取候选里的第一个。
    pub fn load(
        &mut self,
        scope: SheetScope,
        group: Option<String>,
        groups: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.scope = scope;
        self.groups = groups;
        // 指名的分类不在候选里（调用方快照与变量表不同步）也照样能编辑
        if let Some(g) = &group
            && !self.groups.contains(g)
        {
            self.groups.push(g.clone());
        }
        self.group = group.or_else(|| self.groups.first().cloned());
        self.env_id = default_env_id(variables::variables(cx));
        self.notice = None;
        self.refresh_env_items(window, cx);
        self.refresh_group_items(window, cx);
        self.fill_table(window, cx);
    }

    /// 全局变量变了。抽屉自己的写回也会走到这里：那时当前页数据与表格相等，不重填，
    /// 免得打字打到一半光标被重置、焦点丢失。
    fn on_variables_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let sets = variables::variables(cx);
        // 当前选中的环境被删掉了（或之前没有环境、现在有了）：退回默认
        let env_gone = self
            .env_id
            .is_none_or(|id| !sets.environments.iter().any(|e| e.id == id));
        if env_gone {
            self.env_id = default_env_id(sets);
        }
        self.refresh_env_items(window, cx);
        let current = self.current_vars(cx);
        if current != self.table.read(cx).variables(cx) {
            self.table
                .update(cx, |t, cx| t.set_variables(&current, window, cx));
        }
        self.sync_env_name(env_gone, window, cx);
        cx.notify();
    }

    fn on_env_name_event(
        &mut self,
        input: &Entity<InputState>,
        ev: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !matches!(ev, InputEvent::Change) {
            return;
        }
        let name = input.read(cx).value().trim().to_string();
        let Some(id) = self.env_id else { return };
        // 清空名字不写回：环境不能没有名字，停在上一个有效名
        if name.is_empty() {
            return;
        }
        variables::update(cx, |s| {
            if let Some(e) = s.environments.iter_mut().find(|e| e.id == id) {
                e.name = name;
            }
        });
        self.refresh_env_items(window, cx);
    }

    fn set_scope(&mut self, scope: SheetScope, window: &mut Window, cx: &mut Context<Self>) {
        if self.scope == scope {
            return;
        }
        self.scope = scope;
        self.fill_table(window, cx);
    }

    fn select_env(&mut self, id: Option<Ulid>, window: &mut Window, cx: &mut Context<Self>) {
        self.env_id = id;
        self.refresh_env_items(window, cx);
        self.fill_table(window, cx);
    }

    /// 当前页对应的变量（从全局句柄现读）；页不可用（没有环境 / 分类）时为空。
    fn current_vars(&self, cx: &App) -> Vec<Variable> {
        let sets = variables::variables(cx);
        match self.scope {
            SheetScope::Global => sets.globals.clone(),
            SheetScope::Environment => sets
                .environments
                .iter()
                .find(|e| Some(e.id) == self.env_id)
                .map(|e| e.variables.clone())
                .unwrap_or_default(),
            SheetScope::Group => self
                .group
                .as_deref()
                .map(|g| sets.group_vars(Some(g)).to_vec())
                .unwrap_or_default(),
        }
    }

    /// 切页 / 切环境 / 切分类 / 结构性变化之后：程序化重填表格与改名框（都不发事件，不会触发写回）。
    fn fill_table(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let vars = self.current_vars(cx);
        self.table
            .update(cx, |t, cx| t.set_variables(&vars, window, cx));
        self.sync_env_name(true, window, cx);
        cx.notify();
    }

    /// 让改名框显示当前环境的名字。`set_value` 会把光标挪到末尾，所以只在真的不一致时才设：
    /// - `switched`（换了环境）：与存储值逐字比较；
    /// - 否则（同一个环境，多半是用户正在打字触发的写回）：输入框去掉首尾空白后与存储值相等、
    ///   或输入框被清空，都视为一致——否则打 "Dev 2" 时中间态 "Dev " 会被改回 "Dev"。
    fn sync_env_name(&mut self, switched: bool, window: &mut Window, cx: &mut Context<Self>) {
        let stored: SharedString = variables::variables(cx)
            .environments
            .iter()
            .find(|e| Some(e.id) == self.env_id)
            .map(|e| SharedString::from(e.name.clone()))
            .unwrap_or_default();
        let shown = self.env_name.read(cx).value();
        let in_sync = if switched {
            shown == stored
        } else {
            shown.trim().is_empty() || shown.trim() == stored.as_ref()
        };
        if !in_sync {
            self.env_name
                .update(cx, |s, cx| s.set_value(stored, window, cx));
        }
    }

    fn refresh_env_items(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let sets = variables::variables(cx);
        let items: Vec<SharedString> = sets
            .environments
            .iter()
            .map(|e| SharedString::from(e.name.clone()))
            .collect();
        let selected = sets
            .environments
            .iter()
            .position(|e| Some(e.id) == self.env_id);
        if self.env_items.0 == items && self.env_items.1 == selected {
            return;
        }
        self.env_items = (items.clone(), selected);
        self.env_select.update(cx, |s, cx| {
            s.set_items(items, window, cx);
            s.set_selected_index(selected.map(IndexPath::new), window, cx);
        });
    }

    fn refresh_group_items(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let items: Vec<SharedString> = self
            .groups
            .iter()
            .map(|g| SharedString::from(g.clone()))
            .collect();
        let selected = self
            .groups
            .iter()
            .position(|g| Some(g) == self.group.as_ref())
            .map(IndexPath::new);
        self.group_select.update(cx, |s, cx| {
            s.set_items(items, window, cx);
            s.set_selected_index(selected, window, cx);
        });
    }

    /// 表格改动 → 写回当前页。
    fn commit_table(&mut self, cx: &mut Context<Self>) {
        let vars = self.table.read(cx).variables(cx);
        let (scope, env_id, group) = (self.scope, self.env_id, self.group.clone());
        variables::update(cx, |s| match scope {
            SheetScope::Global => s.globals = vars,
            SheetScope::Environment => {
                if let Some(e) = s.environments.iter_mut().find(|e| Some(e.id) == env_id) {
                    e.variables = vars;
                }
            }
            SheetScope::Group => {
                if let Some(g) = group {
                    // 清空的分类不留空条目，variables.json 里只有真正挂着变量的分类
                    if vars.is_empty() {
                        s.groups.remove(&g);
                    } else {
                        s.groups.insert(g, vars);
                    }
                }
            }
        });
    }

    /// 测试用：表格的 `Changed` 走事件订阅，测试里程序化填表不会发事件。
    #[cfg(test)]
    pub fn commit_table_for_test(&mut self, cx: &mut Context<Self>) {
        self.commit_table(cx);
    }

    /// 测试用：「使用此环境」按钮的点击回调走的是 `cx.listener`。
    #[cfg(test)]
    pub fn activate_env_for_test(&mut self, cx: &mut Context<Self>) {
        self.activate_env(cx);
    }

    /// 测试用：导入 / 导出按钮的点击回调走的是 `cx.listener`；文件对话框由测试平台模拟应答。
    #[cfg(test)]
    pub fn import_file_for_test(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.import_file(window, cx);
    }

    #[cfg(test)]
    pub fn export_file_for_test(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.export_file(window, cx);
    }

    /// 测试用：当前提示条的文案（按当前界面语言翻译）。
    #[cfg(test)]
    pub fn notice_text(&self) -> Option<SharedString> {
        self.notice.as_ref().map(Notice::text)
    }

    /// 测试用：「删除」按钮的点击回调走的是 `cx.listener`。
    #[cfg(test)]
    pub fn confirm_delete_env_for_test(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.confirm_delete_env(window, cx);
    }

    fn new_env(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let base = tr!("variables.new_env_default_name");
        let mut id = None;
        variables::update(cx, |s| {
            let env = Environment::new(s.unique_env_name(&base));
            id = Some(env.id);
            s.environments.push(env);
        });
        self.env_id = id;
        self.refresh_env_items(window, cx);
        self.fill_table(window, cx);
        // 新环境的第一件事多半是改名
        self.env_name.update(cx, |s, cx| s.focus(window, cx));
    }

    fn activate_env(&mut self, cx: &mut Context<Self>) {
        variables::set_active_environment(cx, self.env_id);
        cx.notify();
    }

    /// 删除前确认。对话框叠在抽屉之上（`Workspace::render` 里 dialog 层画在 sheet 层之后）；
    /// builder 只捕获本实体的弱引用，确认时才 `update`——那时没有任何渲染借用在身上。
    fn confirm_delete_env(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // 已有对话框时再开一个会叠层且抢焦点：忽略这次请求（同 Workspace::confirm_delete_saved）
        if window.has_active_dialog(cx) {
            return;
        }
        let Some(id) = self.env_id else { return };
        let Some(name) = variables::variables(cx)
            .environments
            .iter()
            .find(|e| e.id == id)
            .map(|e| e.name.clone())
        else {
            return;
        };
        let weak = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let weak = weak.clone();
            alert
                .title(tr!("variables.delete_env_title", name = name))
                .description(tr!("variables.delete_env_body"))
                .button_props(
                    DialogButtonProps::default()
                        .ok_text(tr!("variables.delete_env_ok"))
                        .ok_variant(ButtonVariant::Danger)
                        .cancel_text(tr!("common.cancel"))
                        .show_cancel(true),
                )
                .on_ok(move |_, window, cx| {
                    if let Some(sheet) = weak.upgrade() {
                        sheet.update(cx, |s, cx| s.delete_env(id, window, cx));
                    }
                    true
                })
        });
    }

    fn delete_env(&mut self, id: Ulid, window: &mut Window, cx: &mut Context<Self>) {
        variables::update(cx, |s| {
            s.environments.retain(|e| e.id != id);
            if s.active_environment == Some(id) {
                s.active_environment = None;
            }
        });
        self.env_id = default_env_id(variables::variables(cx));
        self.refresh_env_items(window, cx);
        self.fill_table(window, cx);
    }

    /// 测试用：同步解析一段文本再走与文件对话框相同的落地步骤（生产路径在后台线程解析，见 [`Self::import_file`]）。
    #[cfg(test)]
    pub fn import_from_text(
        &mut self,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<usize, PostmanEnvError> {
        match postman_env::parse(text) {
            Ok(parsed) => Ok(self.apply_import(parsed, window, cx)),
            Err(e) => {
                self.fail_import(ImportError::Postman(e.clone()), cx);
                Err(e)
            }
        }
    }

    /// 导入失败：只记提示，不改任何变量。
    fn fail_import(&mut self, error: ImportError, cx: &mut Context<Self>) {
        self.notice = Some(Notice::ImportFailed(error));
        cx.notify();
    }

    /// 把解析好的 Postman environment / globals 落进变量表，返回导入的变量数：
    /// environment → 新环境（重名加后缀）并激活；globals → 合并进全局，同 key 覆盖。
    fn apply_import(
        &mut self,
        parsed: PostmanEnv,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> usize {
        let count = parsed.variables.len();
        match parsed.scope {
            PostmanScope::Environment => {
                let base = match parsed.name.trim() {
                    "" => tr!("variables.new_env_default_name").to_string(),
                    name => name.to_string(),
                };
                let mut id = None;
                variables::update(cx, |s| {
                    let mut env = Environment::new(s.unique_env_name(&base));
                    env.variables = parsed.variables;
                    id = Some(env.id);
                    s.active_environment = id;
                    s.environments.push(env);
                });
                self.scope = SheetScope::Environment;
                self.env_id = id;
                self.refresh_env_items(window, cx);
            }
            PostmanScope::Globals => {
                variables::update(cx, |s| {
                    for v in parsed.variables {
                        match s.globals.iter_mut().find(|g| g.key == v.key) {
                            Some(slot) => *slot = v,
                            None => s.globals.push(v),
                        }
                    }
                });
                self.scope = SheetScope::Global;
            }
        }
        self.notice = Some(Notice::Imported(count));
        self.fill_table(window, cx);
        count
    }

    /// 当前页能不能导出（与 [`Self::export_text`] 返回 Some 同条件）。渲染期用它决定按钮置灰：
    /// `export_text` 会把整张表序列化一遍，不该每帧跑。
    fn can_export(&self, cx: &App) -> bool {
        match self.scope {
            SheetScope::Global => true,
            SheetScope::Environment => self.env_id.is_some_and(|id| {
                variables::variables(cx)
                    .environments
                    .iter()
                    .any(|e| e.id == id)
            }),
            SheetScope::Group => false,
        }
    }

    /// 当前页可导出的内容：`(建议文件名, JSON)`；分类页（Postman 格式里没有对应物）与没有环境时不导出。
    pub fn export_text(&self, cx: &App) -> Option<(String, String)> {
        let sets = variables::variables(cx);
        match self.scope {
            SheetScope::Global => Some((
                "globals.postman_globals.json".into(),
                postman_env::render("Globals", PostmanScope::Globals, &sets.globals),
            )),
            SheetScope::Environment => {
                let env = sets
                    .environments
                    .iter()
                    .find(|e| Some(e.id) == self.env_id)?;
                let stem: String = env
                    .name
                    .chars()
                    .map(|c| {
                        if c.is_alphanumeric() || c == '-' || c == '_' {
                            c
                        } else {
                            '_'
                        }
                    })
                    .collect();
                // 名字全是空白时别给出一个以点开头的隐藏文件名
                let stem = if stem.trim_matches('_').is_empty() {
                    "environment".to_string()
                } else {
                    stem
                };
                Some((
                    format!("{stem}.postman_environment.json"),
                    postman_env::render(&env.name, PostmanScope::Environment, &env.variables),
                ))
            }
            SheetScope::Group => None,
        }
    }

    fn import_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(tr!("common.choose")),
        });
        cx.spawn_in(window, async move |this, cx| {
            // 对话框取消 / 出错都静默返回
            let Ok(Ok(Some(paths))) = rx.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            // 读文件与解析都在后台线程：导入的文件多大都不卡主线程，回到实体上只做落地
            let parsed = cx
                .background_spawn(async move {
                    let text = std::fs::read_to_string(&path)
                        .map_err(|e| ImportError::Io(e.to_string()))?;
                    postman_env::parse(&text).map_err(ImportError::Postman)
                })
                .await;
            let _ = this.update_in(cx, |this, window, cx| match parsed {
                Ok(parsed) => {
                    this.apply_import(parsed, window, cx);
                }
                Err(error) => this.fail_import(error, cx),
            });
        })
        .detach();
    }

    fn export_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((suggested, text)) = self.export_text(cx) else {
            return;
        };
        let dir = std::env::home_dir().unwrap_or_default();
        let rx = cx.prompt_for_new_path(&dir, Some(suggested.as_str()));
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(dest))) = rx.await else {
                return;
            };
            let result = cx
                .background_spawn({
                    let dest = dest.clone();
                    async move { getcat_core::store::write_atomic_user(&dest, text.as_bytes()) }
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.notice = Some(match result {
                    Ok(()) => Notice::Exported(dest),
                    Err(e) => Notice::ExportFailed(e.to_string()),
                });
                cx.notify();
            });
        })
        .detach();
    }
}

impl Render for VariablesSheet {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let scope = self.scope;
        v_flex()
            .size_full()
            .min_h_0()
            .gap_3()
            .pb_4()
            .child(
                // 包一层 h_flex 让分段控件按内容收缩（同 request_pane 的 body-mode）
                h_flex()
                    .id("variables-scope")
                    .flex_none()
                    .role(Role::Group)
                    .aria_label(tr!("variables.scope_aria"))
                    .child(
                        TabBar::new("variables-scope-tabs")
                            .segmented()
                            .small()
                            .selected_index(scope.index())
                            .on_click(cx.listener(|this, ix: &usize, window, cx| {
                                this.set_scope(SheetScope::from_index(*ix), window, cx)
                            }))
                            .children(SheetScope::ALL.iter().map(|s| s.label())),
                    ),
            )
            .child(match scope {
                SheetScope::Global => self.render_import_export(cx),
                SheetScope::Environment => self.render_env_toolbar(cx),
                SheetScope::Group => self.render_group_toolbar(),
            })
            .when_some(self.notice.as_ref(), |v, notice| {
                let text = notice.text();
                let alert = if notice.is_error() {
                    Alert::error("variables-notice", text)
                } else {
                    Alert::info("variables-notice", text)
                };
                v.child(alert.banner().xsmall())
            })
            .child(self.render_body(cx))
    }
}

impl VariablesSheet {
    fn render_import_export(&self, cx: &mut Context<Self>) -> AnyElement {
        let can_export = self.can_export(cx);
        h_flex()
            .flex_none()
            .gap_2()
            .justify_end()
            .child(
                Button::new("vars-import")
                    .outline()
                    .small()
                    .label(tr!("variables.import"))
                    .tooltip(tr!("variables.import_tooltip"))
                    .on_click(cx.listener(|this, _, window, cx| this.import_file(window, cx))),
            )
            .child(
                Button::new("vars-export")
                    .outline()
                    .small()
                    .label(tr!("variables.export"))
                    .tooltip(tr!("variables.export_tooltip"))
                    .disabled(!can_export)
                    .on_click(cx.listener(|this, _, window, cx| this.export_file(window, cx))),
            )
            .into_any_element()
    }

    fn render_env_toolbar(&self, cx: &mut Context<Self>) -> AnyElement {
        let has_env = self.env_id.is_some();
        let is_active = has_env && variables::variables(cx).active_environment == self.env_id;
        v_flex()
            .flex_none()
            .gap_2()
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        div().w_48().flex_none().child(
                            Select::new(&self.env_select)
                                .small()
                                .disabled(!has_env)
                                .accessibility_label(tr!("variables.env_select_aria")),
                        ),
                    )
                    .child(
                        div().flex_1().min_w_0().child(
                            Input::new(&self.env_name)
                                .small()
                                .disabled(!has_env)
                                .aria_label(tr!("variables.rename_placeholder")),
                        ),
                    )
                    .when(is_active, |h| {
                        h.child(
                            Tag::secondary()
                                .small()
                                .flex_none()
                                .child(tr!("variables.active_tag")),
                        )
                    })
                    .when(has_env && !is_active, |h| {
                        h.child(
                            Button::new("vars-activate")
                                .primary()
                                .small()
                                .flex_none()
                                .label(tr!("variables.activate"))
                                .on_click(cx.listener(|this, _, _, cx| this.activate_env(cx))),
                        )
                    }),
            )
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        Button::new("vars-new-env")
                            .outline()
                            .small()
                            .label(tr!("variables.new_env"))
                            .on_click(cx.listener(|this, _, window, cx| this.new_env(window, cx))),
                    )
                    .child(
                        Button::new("vars-delete-env")
                            .outline()
                            .small()
                            .label(tr!("variables.delete_env"))
                            .disabled(!has_env)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.confirm_delete_env(window, cx)
                            })),
                    )
                    .child(div().flex_1())
                    .child(self.render_import_export(cx)),
            )
            .into_any_element()
    }

    fn render_group_toolbar(&self) -> AnyElement {
        h_flex()
            .flex_none()
            .gap_2()
            .items_center()
            .child(
                div().w_64().flex_none().child(
                    Select::new(&self.group_select)
                        .small()
                        .disabled(self.groups.is_empty())
                        .accessibility_label(tr!("variables.group_select_aria")),
                ),
            )
            .into_any_element()
    }

    fn render_body(&self, cx: &mut Context<Self>) -> AnyElement {
        let empty_hint = match self.scope {
            SheetScope::Environment if self.env_id.is_none() => {
                Some(tr!("variables.no_environment"))
            }
            SheetScope::Group if self.group.is_none() => Some(tr!("variables.no_group")),
            _ => None,
        };
        if let Some(text) = empty_hint {
            return div()
                .flex_1()
                .min_h_0()
                .flex()
                .items_center()
                .justify_center()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(text)
                .into_any_element();
        }
        div()
            .id("variables-table-scroll")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .role(Role::Group)
            .aria_label(tr!("variables.table_aria"))
            .child(self.table.clone())
            .into_any_element()
    }
}
