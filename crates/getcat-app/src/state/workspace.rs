//! 顶层工作区：Tab 列表、侧栏、主题、全局动作；负责从磁盘恢复，并把布局变化写回。

// 显式导入而非 `use gpui::*`：本文件含 `#[cfg(test)] mod tests`，通配符会引入
// gpui 重导出的 `#[proc_macro_attribute] test`，与标准库 `#[test]` 同名冲突，
// 导致该属性宏对自身生成的 `#[test]` 反复展开直至递归上限溢出。
use std::rc::Rc;

use getcat_core::model::{
    SavedRequest, SplitDirection, TabDraft, TabId, ThemePref, Ulid, WorkspaceState, now_ms,
};
use getcat_core::store::Loaded;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    App, AppContext, Context, Entity, FocusHandle, FontWeight, InteractiveElement, IntoElement,
    ParentElement, Render, Role, SharedString, StatefulInteractiveElement, Styled, Subscription,
    UniformListScrollHandle, Window, div, px,
};
use gpui_component::{
    ActiveTheme, IconName, Root, Selectable, Sizable, Theme, ThemeMode, ThemeStyled, TitleBar,
    WindowExt,
    alert::Alert,
    button::{Button, ButtonVariant, ButtonVariants},
    dialog::{DialogAction, DialogButtonProps, DialogClose, DialogFooter},
    h_flex,
    input::{Input, InputState},
    resizable::{ResizableState, h_resizable, resizable_panel},
    status_bar::StatusBar,
    tab::{Tab, TabBar},
    v_flex,
};

use gpui_updater::UpdateStatus;

use crate::brand::APP_NAME;
use crate::i18n::tr;
use crate::state::request_tab::RequestTab;
use crate::state::store::{banner, store};
use crate::state::update;
use crate::ui::settings_dialog::{SettingsPage, open_settings, open_settings_page};
use crate::{
    CloseTab, FindInResponse, NewTab, OpenSettings, SaveRequest, SendRequest, ToggleSidebar,
};

/// 侧栏默认宽度（spec §7.1）。
pub const SIDEBAR_DEFAULT_WIDTH: f32 = 240.;

/// 图标栏上的功能；展开的面板显示其中一个的内容。目前只有已保存请求，
/// 枚举留着是为了之后加历史 / 环境等面板时图标栏与面板的切换逻辑不用重写。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SidebarSection {
    #[default]
    Saved,
}

impl SidebarSection {
    pub const ALL: [SidebarSection; 1] = [SidebarSection::Saved];
}

pub struct Workspace {
    tabs: Vec<Entity<RequestTab>>,
    active: usize,
    sidebar_collapsed: bool,
    /// 面板当前显示的功能（图标栏高亮的那一个）。
    sidebar_section: SidebarSection,
    /// 用户拖出来的侧栏宽度；None 用默认值。
    sidebar_width: Option<f32>,
    sidebar_state: Entity<ResizableState>,
    theme: ThemePref,
    /// 请求 / 响应分栏方向（工作区级，写入 workspace.json）。
    split: SplitDirection,
    /// 已保存请求，按 updated_at 降序；Rc 让侧栏列表的渲染闭包每帧只 clone 指针。
    saved: Rc<Vec<SavedRequest>>,
    /// 侧栏列表的滚动句柄。
    saved_scroll: UniformListScrollHandle,
    focus_handle: FocusHandle,
    /// 全局更新器状态的副本（观察到变化时同步），状态栏提示与「关于」页据此渲染。
    update_status: UpdateStatus,
    _subs: Vec<Subscription>,
}

impl Workspace {
    /// 空工作区：一个新 Tab。生产路径一律走 `restore(loaded)`（无数据时 `Loaded` 本身就是空的），
    /// 因此只有测试用得到它；不加 `#[cfg(test)]` 会在 bin crate 里触发 dead_code。
    #[cfg(test)]
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::restore(Loaded::default(), window, cx)
    }

    /// 从启动读取的结果重建：按 workspace.json 的顺序恢复 Tab，没有草稿则新建一个空 Tab。
    pub fn restore(loaded: Loaded, window: &mut Window, cx: &mut Context<Self>) -> Self {
        // `errors` 已经在 `disk::load_file` / `load_dir` 里 `warn!` 过一次；这里不重复记录，
        // 只是保留字段供 UI 将来展示（目前只用于测试断言 `loaded.errors.is_empty()`）。
        let Loaded {
            workspace: state,
            // 设置在开窗前已由 main 取走安装；这里不再用
            settings: _,
            drafts,
            requests,
            errors: _,
        } = loaded;
        let mut saved = requests;
        sort_saved(&mut saved);
        let state = state.unwrap_or_default();
        let mut ws = Self {
            tabs: Vec::new(),
            active: 0,
            sidebar_collapsed: state.sidebar_collapsed,
            sidebar_section: SidebarSection::default(),
            sidebar_width: state.sidebar_width,
            sidebar_state: cx.new(|_| ResizableState::default()),
            theme: state.theme,
            split: state.split,
            saved: Rc::new(saved),
            saved_scroll: UniformListScrollHandle::new(),
            focus_handle: cx.focus_handle(),
            update_status: update::status(cx),
            _subs: Vec::new(),
        };

        // 更新器状态变化 → 刷新状态栏提示；设置对话框层在本实体的 render 里，同一次 notify 也刷新「关于」页
        if let Some(updater) = update::updater(cx) {
            ws._subs.push(cx.observe(&updater, |this, updater, cx| {
                this.update_status = updater.read(cx).status().clone();
                cx.notify();
            }));
        }

        let (drafts, active) = order_drafts(&state, drafts);
        let split = ws.split;
        for d in drafts {
            let saved_name: Option<SharedString> = d
                .saved_id
                .and_then(|id| ws.saved.iter().find(|r| r.id == id))
                .map(|r| SharedString::from(r.name.clone()));
            let still_saved = saved_name.is_some();
            let tab = cx.new(|cx| {
                let mut tab = RequestTab::new(d.id, window, cx);
                tab.load_draft(&d.draft, window, cx);
                tab.split = split;
                // 对应的已保存请求文件已不存在（被手工删除）：退化为有改动的未保存 Tab
                tab.saved_id = d.saved_id.filter(|_| still_saved);
                tab.saved_name = saved_name;
                tab.dirty = d.dirty || (d.saved_id.is_some() && !still_saved);
                tab
            });
            ws.tabs.push(tab);
        }
        if ws.tabs.is_empty() {
            ws.new_tab(window, cx);
        } else {
            ws.active = active
                .and_then(|id| ws.tabs.iter().position(|t| t.read(cx).id == id))
                .unwrap_or(0);
            ws.active_tab().update(cx, |t, cx| t.focus_url(window, cx));
        }

        apply_theme(ws.theme, Some(window), cx);
        // 跟随系统：系统外观变化时重新同步（固定明 / 暗时忽略）
        let weak = cx.entity().downgrade();
        ws._subs
            .push(window.observe_window_appearance(move |window, cx| {
                if let Some(ws) = weak.upgrade()
                    && ws.read(cx).theme == ThemePref::System
                {
                    Theme::sync_system_appearance(Some(window), cx);
                }
            }));
        ws
    }

    /// 当前 Tab。`tabs` 永不为空（关掉最后一个会立刻新建）；`active` 若越界则夹到末尾而不是 panic。
    pub fn active_tab(&self) -> Entity<RequestTab> {
        let ix = self.active.min(self.tabs.len().saturating_sub(1));
        self.tabs[ix].clone()
    }

    #[cfg(test)]
    pub fn tab_count(&self) -> usize {
        self.tabs.len()
    }

    #[cfg(test)]
    pub fn active_index(&self) -> usize {
        self.active
    }

    #[cfg(test)]
    pub fn tab_at(&self, ix: usize) -> Entity<RequestTab> {
        self.tabs[ix].clone()
    }

    pub fn sidebar_collapsed(&self) -> bool {
        self.sidebar_collapsed
    }

    pub fn sidebar_section(&self) -> SidebarSection {
        self.sidebar_section
    }

    /// 图标栏点击：点当前展开的功能 → 收起；点别的（或收起状态下点任一个）→ 展开并切到它。
    pub fn open_sidebar_section(&mut self, section: SidebarSection, cx: &mut Context<Self>) {
        if !self.sidebar_collapsed && self.sidebar_section == section {
            self.sidebar_collapsed = true;
        } else {
            self.sidebar_section = section;
            self.sidebar_collapsed = false;
        }
        self.persist_workspace(cx);
        cx.notify();
    }

    pub fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        open_settings(cx.entity(), window, cx);
    }

    /// 打开设置并停在指定页（状态栏的更新提示 → 「关于」）。
    pub fn open_settings_page(
        &mut self,
        page: SettingsPage,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        open_settings_page(cx.entity(), page, window, cx);
    }

    #[cfg(test)]
    pub fn sidebar_width(&self) -> Option<f32> {
        self.sidebar_width
    }

    #[cfg(test)]
    pub fn update_status(&self) -> &UpdateStatus {
        &self.update_status
    }

    pub fn theme(&self) -> ThemePref {
        self.theme
    }

    pub fn new_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let split = self.split;
        let tab = cx.new(|cx| {
            let mut tab = RequestTab::new(Ulid::generate(), window, cx);
            tab.split = split;
            tab
        });
        tab.update(cx, |t, cx| {
            t.focus_url(window, cx);
            // 立即写一份空草稿：重启时 workspace.json 的 tab_order 才找得到它
            t.save_draft_now(cx);
        });
        self.tabs.push(tab);
        self.active = self.tabs.len() - 1;
        self.persist_workspace(cx);
        cx.notify();
    }

    pub fn close_tab(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if ix >= self.tabs.len() {
            return;
        }
        // 关闭 Tab = 删草稿文件（spec §9.3）；关最后一个时先删再新建
        let closing = self.tabs[ix].read(cx).id;
        if let Some(store) = store(cx) {
            store.delete_draft(closing);
        }
        if self.tabs.len() == 1 {
            self.tabs.clear();
            self.new_tab(window, cx);
            return;
        }
        let len = self.tabs.len();
        self.tabs.remove(ix);
        self.active = active_after_close(self.active, ix, len);
        self.persist_workspace(cx);
        cx.notify();
    }

    pub fn activate(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix < self.tabs.len() && ix != self.active {
            self.active = ix;
            self.persist_workspace(cx);
            cx.notify();
        }
    }

    /// 退出 / 关窗前：每个 Tab 立即投递草稿快照（跳过去抖）。
    pub fn flush_drafts(&self, cx: &mut Context<Self>) {
        for tab in &self.tabs {
            tab.update(cx, |t, cx| t.save_draft_now(cx));
        }
    }

    /// 当前布局的快照（spec §9.1 工作区状态）。
    pub(crate) fn workspace_state(&self, cx: &App) -> WorkspaceState {
        WorkspaceState {
            tab_order: self.tabs.iter().map(|t| t.read(cx).id).collect(),
            active: self.tabs.get(self.active).map(|t| t.read(cx).id),
            sidebar_width: self.sidebar_width,
            sidebar_collapsed: self.sidebar_collapsed,
            theme: self.theme,
            split: self.split,
        }
    }

    /// 布局 / 顺序改动 → 写 workspace.json（写入线程合并 500 ms 内的多次改动）。
    pub(crate) fn persist_workspace(&self, cx: &App) {
        if let Some(store) = store(cx) {
            store.write_workspace(self.workspace_state(cx));
        }
    }

    pub fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.sidebar_collapsed = !self.sidebar_collapsed;
        self.persist_workspace(cx);
        cx.notify();
    }

    pub fn set_theme(&mut self, pref: ThemePref, window: &mut Window, cx: &mut Context<Self>) {
        self.set_theme_with(pref, Some(window), cx);
    }

    /// 没有 Window 可用的调用点（设置对话框的字段回调只拿到 `&mut App`）。
    pub fn set_theme_global(&mut self, pref: ThemePref, cx: &mut Context<Self>) {
        self.set_theme_with(pref, None, cx);
    }

    fn set_theme_with(
        &mut self,
        pref: ThemePref,
        window: Option<&mut Window>,
        cx: &mut Context<Self>,
    ) {
        if self.theme == pref {
            return;
        }
        self.theme = pref;
        apply_theme(pref, window, cx);
        self.persist_workspace(cx);
        cx.notify();
    }

    /// 侧栏按钮：系统 → 浅色 → 深色 → 系统。
    pub fn cycle_theme(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.set_theme(self.theme.next(), window, cx);
    }

    /// 指定响应区的位置（右侧 / 下方）：作用于所有 Tab，并写入 workspace.json。
    /// Tab 栏那组按钮是两段式的，点当前那一段应当没有任何动静，所以这里直接
    /// 指定方向而不是翻转，并在方向没变时提前返回，避免多余的落盘与重绘。
    pub fn set_split(&mut self, split: SplitDirection, cx: &mut Context<Self>) {
        if self.split == split {
            return;
        }
        self.split = split;
        for tab in &self.tabs {
            tab.update(cx, |t, cx| {
                t.split = split;
                cx.notify();
            });
        }
        self.persist_workspace(cx);
        cx.notify();
    }

    #[cfg(test)]
    pub fn split(&self) -> SplitDirection {
        self.split
    }

    /// 拖拽分隔条松手：记录侧栏宽度并写回。
    fn on_sidebar_resized(&mut self, state: &Entity<ResizableState>, cx: &mut Context<Self>) {
        let Some(width) = state.read(cx).sizes().first().copied().map(f32::from) else {
            return;
        };
        if self.sidebar_width != Some(width) {
            self.sidebar_width = Some(width);
            self.persist_workspace(cx);
        }
    }

    /// 侧栏列表读取用。
    pub(crate) fn saved(&self) -> &[SavedRequest] {
        &self.saved
    }

    /// 渲染闭包用：每帧只 clone 一个 Rc。
    pub(crate) fn saved_rc(&self) -> Rc<Vec<SavedRequest>> {
        self.saved.clone()
    }

    pub(crate) fn saved_scroll(&self) -> &UniformListScrollHandle {
        &self.saved_scroll
    }

    /// 侧栏点击：已有 Tab 打开着这条 → 激活它；否则新建 Tab 载入。
    pub fn open_saved(&mut self, id: Ulid, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ix) = self
            .tabs
            .iter()
            .position(|t| t.read(cx).saved_id == Some(id))
        {
            self.activate(ix, cx);
            return;
        }
        let Some(request) = self.saved.iter().find(|r| r.id == id).cloned() else {
            return;
        };
        self.new_tab(window, cx);
        let tab = self.active_tab();
        tab.update(cx, |t, cx| {
            t.load_draft(&request.draft, window, cx);
            t.saved_id = Some(id);
            t.saved_name = Some(request.name.clone().into());
            t.dirty = false;
            t.save_draft_now(cx);
            cx.notify();
        });
        cx.notify();
    }

    /// 删除已保存请求：移出列表、删文件；正打开着它的 Tab 退化为未保存、有改动。
    pub fn delete_saved(&mut self, id: Ulid, cx: &mut Context<Self>) {
        let list = Rc::make_mut(&mut self.saved);
        let before = list.len();
        list.retain(|r| r.id != id);
        if list.len() == before {
            return;
        }
        if let Some(store) = store(cx) {
            store.delete_request(id);
        }
        for tab in &self.tabs {
            if tab.read(cx).saved_id == Some(id) {
                tab.update(cx, |t, cx| {
                    t.saved_id = None;
                    t.saved_name = None;
                    t.dirty = true;
                    t.save_draft_now(cx);
                    cx.notify();
                });
            }
        }
        cx.notify();
    }

    /// 删除前确认（需要窗口根视图是 gpui_component::Root；测试不走这里）。
    pub(crate) fn confirm_delete_saved(
        &mut self,
        id: Ulid,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 已有对话框时再开一个会叠层且抢焦点（Plan 3 决策 P3-3）：忽略这次请求。
        if window.has_active_dialog(cx) {
            return;
        }
        let Some(name) = self
            .saved
            .iter()
            .find(|r| r.id == id)
            .map(|r| r.name.clone())
        else {
            return;
        };
        let weak = cx.entity().downgrade();
        // 破坏性确认走 AlertDialog（设计指南）：标题点名对象，正文只写后果，
        // 按钮由 button_props 自动生成，确认键用结果动词。
        window.open_alert_dialog(cx, move |alert, _, _| {
            let weak = weak.clone();
            alert
                .title(tr!("dialog.delete_saved.title", name = name))
                .description(tr!("dialog.delete_saved.body"))
                .button_props(
                    DialogButtonProps::default()
                        .ok_text(tr!("common.delete"))
                        .ok_variant(ButtonVariant::Danger)
                        .cancel_text(tr!("common.cancel"))
                        .show_cancel(true),
                )
                .on_ok(move |_, _, cx| {
                    if let Some(ws) = weak.upgrade() {
                        ws.update(cx, |ws, cx| ws.delete_saved(id, cx));
                    }
                    true
                })
        });
    }

    /// ⌘S / "保存"按钮：保存过的 Tab 直接覆盖；否则弹名字对话框。
    pub fn save_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let tab = self.active_tab();
        match tab.read(cx).saved_id {
            Some(id) => self.overwrite_saved(&tab, id, cx),
            None => self.prompt_save_name(tab, window, cx),
        }
    }

    fn overwrite_saved(&mut self, tab: &Entity<RequestTab>, id: Ulid, cx: &mut Context<Self>) {
        let Some(existing) = self.saved.iter().find(|r| r.id == id).cloned() else {
            // 列表里已没有这条（文件被外部删除）：当作新保存
            let name = tab.read(cx).title(cx).to_string();
            self.finish_save(tab.clone(), name, cx);
            return;
        };
        let request = SavedRequest {
            draft: tab.read(cx).draft(cx),
            updated_at: now_ms(),
            ..existing
        };
        let name: SharedString = request.name.clone().into();
        self.upsert_saved(request, cx);
        tab.update(cx, |t, cx| {
            t.saved_name = Some(name);
            t.mark_clean(cx);
            t.save_draft_now(cx);
        });
        cx.notify();
    }

    /// 以给定名字保存为一条新请求（空名字回退为 Tab 标题）；返回新 id。
    /// 对话框确认时该 Tab 可能已被关闭（草稿已删除）：此时不写任何文件，返回 `None`。
    pub(crate) fn finish_save(
        &mut self,
        tab: Entity<RequestTab>,
        name: String,
        cx: &mut Context<Self>,
    ) -> Option<Ulid> {
        if !self.tabs.contains(&tab) {
            return None;
        }
        let trimmed = name.trim();
        let name: String = if trimmed.is_empty() {
            tab.read(cx).title(cx).to_string()
        } else {
            trimmed.to_string()
        };
        let request = SavedRequest::new(name.clone(), tab.read(cx).draft(cx));
        let id = request.id;
        self.upsert_saved(request, cx);
        tab.update(cx, |t, cx| {
            t.saved_id = Some(id);
            t.saved_name = Some(name.into());
            t.mark_clean(cx);
            t.save_draft_now(cx);
        });
        cx.notify();
        Some(id)
    }

    /// 插入或替换列表项、保持排序，并写文件。
    fn upsert_saved(&mut self, request: SavedRequest, cx: &App) {
        let list = Rc::make_mut(&mut self.saved);
        match list.iter_mut().find(|r| r.id == request.id) {
            Some(slot) => *slot = request.clone(),
            None => list.push(request.clone()),
        }
        sort_saved(list);
        if let Some(store) = store(cx) {
            store.write_request(request);
        }
    }

    /// 名字对话框：默认名取 Tab 标题；确定后 `finish_save`。
    /// 需要窗口根视图是 gpui_component::Root（测试窗口没有，测试不走这里）。
    fn prompt_save_name(
        &mut self,
        tab: Entity<RequestTab>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 已有对话框时不叠加第二个（Plan 3 决策 P3-3）。
        if window.has_active_dialog(cx) {
            return;
        }
        let default_name = tab.read(cx).title(cx);
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(tr!("dialog.save_request.name"))
                .default_value(default_name)
        });
        let weak = cx.entity().downgrade();
        let input_for_focus = input.clone();
        window.open_dialog(cx, move |dialog, _, _| {
            let input_for_content = input.clone();
            let input_for_ok = input.clone();
            let tab = tab.clone();
            let weak = weak.clone();
            dialog
                .title(tr!("dialog.save_request.title"))
                .content(move |content, _, _| {
                    content.child(
                        Input::new(&input_for_content).aria_label(tr!("dialog.save_request.name")),
                    )
                })
                // `button_props` 的 ok_text/cancel_text 只被 AlertDialog 的自动 footer 消费；
                // 普通 Dialog 必须自己拼 footer，否则 window.open_dialog 只更新状态、画面上不出现按钮。
                .footer(
                    DialogFooter::new()
                        .child(
                            DialogClose::new().child(
                                Button::new("cancel-save")
                                    .outline()
                                    .label(tr!("common.cancel")),
                            ),
                        )
                        .child(
                            DialogAction::new()
                                .child(Button::new("ok-save").primary().label(tr!("common.save"))),
                        ),
                )
                .on_ok(move |_, _, cx| {
                    let name = input_for_ok.read(cx).value().to_string();
                    if let Some(ws) = weak.upgrade() {
                        ws.update(cx, |ws, cx| {
                            ws.finish_save(tab.clone(), name, cx);
                        });
                    }
                    true
                })
        });
        // Dialog 打开时会聚焦自身；把焦点交给名称输入框
        input_for_focus.update(cx, |s, cx| s.focus(window, cx));
    }

    /// 标题栏副标题：当前 Tab 的标题（已保存请求名或 URL 末段）。
    pub(crate) fn title_bar_subtitle(&self, cx: &App) -> SharedString {
        self.active_tab().read(cx).title(cx)
    }

    /// 自绘标题栏（spec §7.2）：居中显示 `GetCat · 当前 Tab 标题`；
    /// 拖动 / 双击 / 平台控制按钮由 TitleBar 处理。
    ///
    /// 品牌名用更重的字重与主前景色，Tab 标题降到 muted——窗口变窄时只有 Tab 标题被截断，
    /// `GetCat ·` 始终完整（品牌段 `flex_none`，标题段才 `min_w_0` + `truncate`）。
    fn render_title_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        TitleBar::new().child(
            h_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_1p5()
                // macOS 的 TitleBar 左侧给红绿灯留了 80 px，右侧补同样的量才是窗口正中
                .when(cfg!(target_os = "macos"), |h| h.pr(px(80.)))
                .text_sm()
                .min_w_0()
                .child(
                    div()
                        .flex_none()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(cx.theme().foreground)
                        .child(APP_NAME),
                )
                .child(
                    div()
                        .flex_none()
                        .text_color(cx.theme().muted_foreground)
                        .child("·"),
                )
                .child(
                    // flex 子项默认 min-width:auto，不会收缩到内容宽度以下，truncate 因此失效：
                    // 必须显式 min_w_0()
                    div()
                        .min_w_0()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(cx.theme().muted_foreground)
                        .truncate()
                        .child(self.title_bar_subtitle(cx)),
                ),
        )
    }

    fn render_tab_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let titles: Vec<(SharedString, bool)> = self
            .tabs
            .iter()
            .map(|t| {
                let t = t.read(cx);
                (t.title(cx), t.dirty)
            })
            .collect();
        TabBar::new("request-tabs")
            .w_full()
            .pl_1()
            .prefix(
                div().pr_1().child(
                    Button::new("new-tab")
                        .ghost()
                        .small()
                        .icon(IconName::Plus)
                        .tooltip_with_action(tr!("tab.new"), &NewTab, None)
                        .on_click(cx.listener(|this, _, window, cx| this.new_tab(window, cx))),
                ),
            )
            .selected_index(self.active)
            .on_click(cx.listener(|this, ix: &usize, _, cx| this.activate(*ix, cx)))
            .children(titles.into_iter().enumerate().map(|(ix, (title, dirty))| {
                let aria = if dirty {
                    tr!("tab.aria_dirty", title = title)
                } else {
                    tr!("tab.aria", title = title)
                };
                let mut tab = Tab::new().label(title).aria_label(aria);
                if dirty {
                    // 未保存改动：标题前的圆点（spec §7.1）
                    tab = tab.prefix(
                        div()
                            .size_1p5()
                            .rounded_full_style(cx)
                            .bg(cx.theme().primary),
                    );
                }
                tab.suffix(
                    Button::new(("close-tab", ix))
                        .ghost()
                        .xsmall()
                        .icon(IconName::Close)
                        .tooltip(tr!("tab.close"))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            // 阻止冒泡到 Tab 的 on_click，否则关闭后会再触发 activate(ix)
                            cx.stop_propagation();
                            this.close_tab(ix, window, cx)
                        })),
                )
            }))
    }
}

/// 关闭 `closing` 后新的 active 下标；`len` 为关闭前的 Tab 数（≥ 2）。
pub(crate) fn active_after_close(active: usize, closing: usize, len: usize) -> usize {
    let new_len = len - 1;
    if active >= new_len {
        // 关掉的是末尾且它就是 active：夹到新的末尾。
        new_len.saturating_sub(1)
    } else if closing < active {
        active - 1
    } else {
        active
    }
}

/// 按 workspace.json 的 tab_order 排列草稿：顺序里没有文件的 id 跳过；没被提到的草稿按 id（即创建时间）追加。
/// 返回排好序的草稿与（仍然存在的）激活 Tab。
pub(crate) fn order_drafts(
    state: &WorkspaceState,
    mut drafts: Vec<TabDraft>,
) -> (Vec<TabDraft>, Option<TabId>) {
    drafts.sort_by_key(|d| d.id);
    let mut ordered = Vec::with_capacity(drafts.len());
    for id in &state.tab_order {
        if let Some(pos) = drafts.iter().position(|d| d.id == *id) {
            ordered.push(drafts.remove(pos));
        }
    }
    ordered.extend(drafts);
    let active = state
        .active
        .filter(|id| ordered.iter().any(|d| d.id == *id));
    (ordered, active)
}

/// 侧栏列表顺序：最近更新在前；同一毫秒内按 id 降序（id 含创建时间）。
pub(crate) fn sort_saved(list: &mut [SavedRequest]) {
    list.sort_by(|a, b| {
        b.updated_at
            .cmp(&a.updated_at)
            .then_with(|| b.id.cmp(&a.id))
    });
}

/// 把主题偏好应用到 gpui-component 的主题系统：System 跟随窗口外观，其余固定。
/// `window` 为 None 时只改全局主题（gpui-component 会刷新所有窗口）。
pub(crate) fn apply_theme(pref: ThemePref, window: Option<&mut Window>, cx: &mut App) {
    match pref {
        ThemePref::System => Theme::sync_system_appearance(window, cx),
        ThemePref::Light => Theme::change(ThemeMode::Light, window, cx),
        ThemePref::Dark => Theme::change(ThemeMode::Dark, window, cx),
    }
}

/// 底部状态栏：右侧是请求 / 响应的分栏方向（两段式），左侧留给以后的功能按钮。
impl Workspace {
    fn render_status_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        // 只在"有新版本"与"已装好待重启"时提示；下载 / 安装进度只在「检查更新」页显示，
        // 离线启动得到的错误也不在这里冒出来
        let update_hint = update::hint_version(&self.update_status)
            .map(|(version, staged)| (version.clone(), staged));
        StatusBar::new().py_0p5().right(
            h_flex()
                .items_center()
                .gap_0p5()
                .when_some(update_hint, |bar, (version, staged)| {
                    bar.child(if staged {
                        Button::new("update-restart")
                            .ghost()
                            .xsmall()
                            .icon(IconName::ArrowUp)
                            .label(tr!("status.update_restart", version = version))
                            .tooltip(tr!("status.update_restart_tooltip"))
                            .on_click(|_, _, cx| update::restart(cx))
                    } else {
                        Button::new("update-available")
                            .ghost()
                            .xsmall()
                            .icon(IconName::ArrowUp)
                            .label(tr!("status.update_available", version = version))
                            .tooltip(tr!("status.update_available_tooltip"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_settings_page(SettingsPage::Updates, window, cx)
                            }))
                    })
                })
                // 两段式而不是单个图标轮换：当前那一段高亮，两段各自带说明，
                // 不用先读懂图标才知道按下去会变成什么。
                .child(
                    Button::new("split-right")
                        .ghost()
                        .xsmall()
                        .selected(self.split == SplitDirection::Horizontal)
                        .icon(IconName::PanelRight)
                        .tooltip(tr!("status.split_right"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.set_split(SplitDirection::Horizontal, cx)
                        })),
                )
                .child(
                    Button::new("split-bottom")
                        .ghost()
                        .xsmall()
                        .selected(self.split == SplitDirection::Vertical)
                        .icon(IconName::PanelBottom)
                        .tooltip(tr!("status.split_bottom"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.set_split(SplitDirection::Vertical, cx)
                        })),
                ),
        )
    }
}

/// 持久化不可用 / 写入失败的顶部横幅（spec §9.4 / §11）：一行文字，不阻塞任何操作。
/// 用官方 `Alert` 的 banner 形态：图标、底色、边框都来自主题，不再手调透明度。
fn render_banner(text: String) -> impl IntoElement {
    Alert::error("store-banner", text).banner().xsmall()
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let active = self.active_tab();
        let sidebar_width = px(self.sidebar_width.unwrap_or(SIDEBAR_DEFAULT_WIDTH));
        // gpui-component 的 Root::render() 不会自动画出 Dialog/Sheet/Notification 层，
        // 需要消费方自己在某处渲染（story crate 的 StoryRoot::render 就是这么接的）；
        // 否则 window.open_dialog 只会更新状态，画面上什么都不会出现。
        let dialog_layer = Root::render_dialog_layer(window, cx);
        // 根元素持有焦点（track_focus）：给它 id + role，否则 gpui 会在日志里提示聚焦元素缺少 role
        div()
            .id("workspace")
            .role(Role::Group)
            .aria_label(tr!("app.workspace_aria"))
            .key_context("Workspace")
            .track_focus(&self.focus_handle)
            .size_full()
            .flex()
            .flex_col()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .on_action(cx.listener(|this, _: &NewTab, window, cx| this.new_tab(window, cx)))
            .on_action(cx.listener(|this, _: &CloseTab, window, cx| {
                let ix = this.active;
                this.close_tab(ix, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ToggleSidebar, _, cx| this.toggle_sidebar(cx)))
            .on_action(cx.listener(|this, _: &SendRequest, window, cx| {
                this.active_tab().update(cx, |tab, cx| tab.send(window, cx))
            }))
            .on_action(
                cx.listener(|this, _: &SaveRequest, window, cx| this.save_active(window, cx)),
            )
            .on_action(cx.listener(|this, _: &FindInResponse, window, cx| {
                this.active_tab()
                    .update(cx, |tab, cx| tab.find_in_response(window, cx))
            }))
            .on_action(
                cx.listener(|this, _: &OpenSettings, window, cx| this.open_settings(window, cx)),
            )
            .child(self.render_title_bar(cx))
            .when_some(banner(cx), |d, text| d.child(render_banner(text)))
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .items_stretch()
                    // 图标栏固定在最左、从不移动；功能面板是可拖宽的分栏面板，
                    // 收起时 visible(false) 整块让出（宽度照旧记在 sidebar_width）。
                    .child(self.render_sidebar_rail(cx))
                    .child(
                        div().flex_1().min_w_0().h_full().child(
                            h_resizable("workspace")
                                .with_state(&self.sidebar_state)
                                .on_resize(cx.listener(
                                    |this, state: &Entity<ResizableState>, _, cx| {
                                        this.on_sidebar_resized(state, cx)
                                    },
                                ))
                                .child(
                                    resizable_panel()
                                        .size(sidebar_width)
                                        .size_range(px(180.)..px(420.))
                                        .visible(!self.sidebar_collapsed)
                                        .child(self.render_sidebar(window, cx)),
                                )
                                .child(
                                    resizable_panel().child(
                                        v_flex()
                                            .size_full()
                                            .min_w_0()
                                            .child(self.render_tab_bar(cx))
                                            .child(div().flex_1().min_h_0().child(active)),
                                    ),
                                ),
                        ),
                    ),
            )
            .child(self.render_status_bar(cx))
            .children(dialog_layer)
    }
}

#[cfg(test)]
mod tests {
    use getcat_core::model::{RequestDraft, TabDraft, Ulid, WorkspaceState};

    use super::{active_after_close, order_drafts};

    fn draft(id: Ulid) -> TabDraft {
        TabDraft {
            id,
            draft: RequestDraft::default(),
            saved_id: None,
            dirty: false,
        }
    }

    #[test]
    fn order_follows_tab_order_and_appends_unlisted_by_id() {
        let a = Ulid::from_parts(1, 0);
        let b = Ulid::from_parts(2, 0);
        let c = Ulid::from_parts(3, 0);
        let missing = Ulid::from_parts(9, 0);
        let state = WorkspaceState {
            tab_order: vec![c, missing, a],
            active: Some(a),
            ..Default::default()
        };
        let (ordered, active) = order_drafts(&state, vec![draft(b), draft(a), draft(c)]);
        let ids: Vec<Ulid> = ordered.iter().map(|d| d.id).collect();
        assert_eq!(ids, vec![c, a, b]);
        assert_eq!(active, Some(a));
    }

    #[test]
    fn missing_active_falls_back_to_none() {
        let a = Ulid::from_parts(1, 0);
        let state = WorkspaceState {
            tab_order: vec![a],
            active: Some(Ulid::from_parts(5, 0)),
            ..Default::default()
        };
        let (ordered, active) = order_drafts(&state, vec![draft(a)]);
        assert_eq!(ordered.len(), 1);
        assert_eq!(active, None);
    }

    #[test]
    fn empty_state_keeps_drafts_sorted_by_id() {
        let a = Ulid::from_parts(1, 0);
        let b = Ulid::from_parts(2, 0);
        let (ordered, active) = order_drafts(&WorkspaceState::default(), vec![draft(b), draft(a)]);
        let ids: Vec<Ulid> = ordered.iter().map(|d| d.id).collect();
        assert_eq!(ids, vec![a, b]);
        assert_eq!(active, None);
    }

    #[test]
    fn closing_left_of_active_shifts_active_left() {
        assert_eq!(active_after_close(2, 0, 4), 1);
        assert_eq!(active_after_close(1, 0, 3), 0);
    }

    #[test]
    fn closing_active_keeps_index_pointing_at_right_neighbour() {
        assert_eq!(active_after_close(1, 1, 3), 1);
        assert_eq!(active_after_close(0, 0, 2), 0);
    }

    #[test]
    fn closing_last_active_clamps_to_new_last() {
        assert_eq!(active_after_close(2, 2, 3), 1);
        assert_eq!(active_after_close(1, 1, 2), 0);
    }

    #[test]
    fn closing_right_of_active_leaves_active_alone() {
        assert_eq!(active_after_close(0, 1, 3), 0);
        assert_eq!(active_after_close(1, 2, 4), 1);
    }

    #[test]
    fn closing_the_only_tab_is_total() {
        // close_tab 在 len == 1 时走"先删再新建"，不会调到这里；但函数本身不得 panic
        assert_eq!(active_after_close(0, 0, 1), 0);
    }
}
