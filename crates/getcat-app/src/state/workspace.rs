//! 顶层工作区：Tab 列表、侧栏、主题、全局动作；负责从磁盘恢复，并把布局变化写回。

// 显式导入而非 `use gpui::*`：本文件含 `#[cfg(test)] mod tests`，通配符会引入
// gpui 重导出的 `#[proc_macro_attribute] test`，与标准库 `#[test]` 同名冲突，
// 导致该属性宏对自身生成的 `#[test]` 反复展开直至递归上限溢出。
use std::rc::Rc;

use getcat_core::model::{
    Method, SavedRequest, SplitDirection, TabDraft, TabId, ThemePref, Ulid, WorkspaceState, now_ms,
};
use getcat_core::store::Loaded;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    App, AppContext, Context, Entity, FocusHandle, FontWeight, InteractiveElement, IntoElement,
    ParentElement, Render, Role, ScrollHandle, SharedString, StatefulInteractiveElement, Styled,
    Subscription, UniformListScrollHandle, Window, div, point, px,
};
use gpui_component::{
    ActiveTheme, Disableable, IconName, Root, Selectable, Sizable, Theme, ThemeMode, ThemeStyled,
    TitleBar, WindowExt,
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
use crate::state::settings;
use crate::state::store::{banner, store};
use crate::state::update;
use crate::templates;
use crate::ui::code_sheet::{CODE_SHEET_WIDTH, CodeSheet};
use crate::ui::curl_sheet::{CURL_SHEET_WIDTH, CurlSheet, import_button};
use crate::ui::method_color;
use crate::ui::settings_dialog::{SettingsPage, open_settings, open_settings_page};
use crate::{
    CloseTab, DuplicateTab, FindInResponse, NewTab, OpenSettings, SaveRequest, SendRequest,
    ToggleSidebar,
};

/// 侧栏默认宽度（spec §7.1）。
pub const SIDEBAR_DEFAULT_WIDTH: f32 = 300.;

/// 标签栏箭头一次滚动的距离，约一个标签宽（标签上限 200 px）。
const TAB_SCROLL_STEP: f32 = 180.;

/// 图标栏上的功能；展开的面板显示其中一个的内容。
/// 判别值即图标栏上的下标（rail 的 Button id 用的是 `section as usize`），
/// 所以 `ALL` 的顺序必须与变体声明顺序一致，有测试钉住。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SidebarSection {
    #[default]
    Saved,
    Templates,
}

impl SidebarSection {
    pub const ALL: [SidebarSection; 2] = [SidebarSection::Saved, SidebarSection::Templates];
}

/// 右侧图标栏上的功能；点一下从右侧滑出对应的抽屉。
/// 与 [`SidebarSection`] 同款约定：判别值即图标栏上的下标（按钮 id 用的是 `section as usize`），
/// 所以 `ALL` 的顺序必须与变体声明顺序一致，有测试钉住。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ToolSection {
    #[default]
    CodeGen,
    ImportCurl,
}

impl ToolSection {
    pub const ALL: [ToolSection; 2] = [ToolSection::CodeGen, ToolSection::ImportCurl];
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
    /// 标签栏的横向滚动句柄：左右箭头与「激活项滚入视口」都靠它。
    tab_scroll: ScrollHandle,
    focus_handle: FocusHandle,
    /// 全局更新器状态的副本（观察到变化时同步），状态栏提示与「关于」页据此渲染。
    update_status: UpdateStatus,
    /// 「生成代码」抽屉的正文。**必须是独立实体**：它由 `Sheet` 的 builder 在
    /// `Workspace::render` 内部渲染，做成 Workspace 上的 render 方法会二次借用本实体而 panic
    /// （详见 [`crate::ui::code_sheet`] 的模块注释）。抽屉是窗口级单例，全局一份。
    pub(crate) code_sheet: Entity<CodeSheet>,
    /// 「导入 cURL」抽屉的正文。与 `code_sheet` 同样的约束：必须是独立实体。
    pub(crate) curl_sheet: Entity<CurlSheet>,
    /// 当前打开着的是哪个抽屉。`Sheet` 是窗口级单例，只靠 `has_active_sheet`
    /// 分不清「点的是同一个（该收起）」还是「点的是另一个（该换内容）」。
    open_tool: Option<ToolSection>,
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
            tab_scroll: ScrollHandle::new(),
            focus_handle: cx.focus_handle(),
            update_status: update::status(cx),
            code_sheet: cx.new(|cx| CodeSheet::new(window, cx)),
            curl_sheet: cx.new(|cx| CurlSheet::new(window, cx)),
            open_tool: None,
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
            self.reset_workspace_panels(cx);
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

    /// 测试用：`ResizableState` 记下的各 panel 宽度，用来钉住侧栏不会被比例重分配压窄。
    #[cfg(test)]
    pub fn sidebar_panel_sizes(&self, cx: &App) -> Vec<f32> {
        self.sidebar_state
            .read(cx)
            .sizes()
            .iter()
            .copied()
            .map(f32::from)
            .collect()
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
        self.reveal_active_tab();
        self.persist_workspace(cx);
        cx.notify();
    }

    /// 复制当前 Tab：把已填内容原样开一份新的，插在源 Tab 右侧。
    ///
    /// 走 `draft()` 快照而不是 clone —— `RequestTab` 持有 `InputState` / `KvTable`
    /// 等实体，clone 只会让两个 Tab 共享同一份状态、跟着一起改。
    ///
    /// 副本永远是「未保存 + 有改动」：不继承 `saved_id`，否则两个 Tab 指向同一条
    /// 已保存请求，谁先按保存谁覆盖对方。
    ///
    /// 快照口径与「保存」一致：`draft()` 只取当前 Body 模式对应的内容，
    /// 停在 none 时 raw 编辑器里的草稿文字不会被带过去。
    pub fn duplicate_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let source_ix = self.active;
        let (draft, raw_format) = {
            let t = self.active_tab().read(cx);
            (t.draft(cx), t.raw_format)
        };

        // 复用 new_tab 的整套流程（建实体、带上 split、立即落草稿），它固定追加到末尾
        self.new_tab(window, cx);
        let tab = self.active_tab().clone();
        tab.update(cx, |t, cx| {
            t.load_draft(&draft, window, cx);
            // load_draft 只在 body 是 raw 时才设格式；停在别的模式时同步一下，
            // 用户切回 raw 才会看到和源 Tab 一样的格式
            t.raw_format = raw_format;
            t.saved_id = None;
            // load_draft 走的是不发事件的程序化写入，不会置脏，这里手动标记
            t.mark_dirty(cx);
            t.save_draft_now(cx);
            cx.notify();
        });

        // new_tab 把副本追加在了末尾，挪到源 Tab 右边才符合「复制」的直觉
        let from = self.tabs.len() - 1;
        let to = source_ix + 1;
        if to < from {
            let tab = self.tabs.remove(from);
            self.tabs.insert(to, tab);
            self.active = to;
            self.reveal_active_tab();
            self.persist_workspace(cx);
        }
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
        self.reveal_active_tab();
        self.persist_workspace(cx);
        cx.notify();
    }

    pub fn activate(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix < self.tabs.len() && ix != self.active {
            self.active = ix;
            self.reveal_active_tab();
            self.persist_workspace(cx);
            cx.notify();
        }
    }

    /// 让激活的标签滚进视口。标签多到溢出时，不这么做就会出现「新建了标签却看不见」。
    ///
    /// `scroll_to_item` 的下标是滚动容器的直接子元素下标；当前用的 `TabVariant::Tab`
    /// 没有滑动指示器，子元素就是「全部标签 + 末尾的空白占位」，所以与标签下标一一对应。
    fn reveal_active_tab(&self) {
        self.tab_scroll.scroll_to_item(self.active);
    }

    /// 标签栏横向滚一步；`dir` 为 -1 看左边、+1 看右边。
    /// gpui 的约定是向右滚时 `offset.x` 变负。
    pub fn scroll_tabs(&mut self, dir: f32, cx: &mut Context<Self>) {
        let max = self.tab_scroll.max_offset().x;
        let x = (self.tab_scroll.offset().x - px(TAB_SCROLL_STEP) * dir).clamp(-max, px(0.));
        self.tab_scroll.set_offset(point(x, px(0.)));
        cx.notify();
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
        if !self.sidebar_collapsed {
            self.reset_workspace_panels(cx);
        }
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

    /// 拖拽分隔条松手：记录侧栏宽度并写回，然后把 panel 交还给 `sidebar_width` 驱动。
    fn on_sidebar_resized(&mut self, state: &Entity<ResizableState>, cx: &mut Context<Self>) {
        let Some(width) = state.read(cx).sizes().first().copied().map(f32::from) else {
            return;
        };
        if self.sidebar_width != Some(width) {
            self.sidebar_width = Some(width);
            self.persist_workspace(cx);
        }
        // 拖拽把 panel 的 size 写成了 Some(...)，从此它盖过 `initial_size`。宽度已经记进
        // `sidebar_width` 了，这里立刻还原成 None，维持下面那条不变量。
        self.reset_workspace_panels(cx);
    }

    /// 把侧栏 panel 的尺寸交还给 [`Self::sidebar_width`]——它才是侧栏宽度的唯一权威，
    /// `ResizableState` 只负责拖拽交互本身。
    ///
    /// 不这么做的话侧栏会「展开几秒后自己缩一点」：gpui-component 的 `ResizablePanel`
    /// 在 `visible(false)` 时 render 第一件事就是 `return div()`，既不 prepaint 也不写
    /// `ResizableState::sizes`。于是收起期间 sizes 停在陈旧的 `[侧栏宽, 容器全宽]`，
    /// 其总和远大于容器宽度；而 `ResizablePanelGroup` 只要容器尺寸一变就会按
    /// `size / total` 的比例重分配（启动后更新检查回来改变状态栏、窗口 resize 都算），
    /// 侧栏于是被按 `300 / 1500` 这样的错误比例压窄。
    ///
    /// `reset_panel` 把 panel 的 size 置回 `None`，于是 render 重新采用 `initial_size`
    /// （侧栏用的就是渲染时传下去的 `sidebar_width`；主工作区没有 initial_size，回到
    /// 由 flex 填满剩余空间）。而 `adjust_to_container_size` 只要见到任一 panel 的 size
    /// 是 `None` 就早退——上游正是靠这条保护「有尺寸偏好的 panel 不被没偏好的拖着走」。
    ///
    /// **两个 panel 都要 reset**：只 reset 侧栏是不够的。侧栏展开后的第一帧，
    /// `update_panel_size` 会把它的 size 从 `None` 写回 `Some`（`sizes[0]` 此时还是
    /// 占位的 `PANEL_MIN_SIZE`），而主工作区的 `sizes[1]` 早在收起那一帧就被量成了
    /// 容器全宽、此后再不更新。两个 `Some` 一凑齐，早退失效，比例重分配照样发生。
    /// 主工作区被 reset 后 `sizes[1]` 不再等于 `PANEL_MIN_SIZE`，它的 size 就会一直
    /// 保持 `None`，这条保护才真正立住。
    fn reset_workspace_panels(&self, cx: &mut App) {
        // 首帧之前 panels 还是空的，`reset_panel` 会直接索引 `sizes[ix]` 而 panic
        let count = self.sidebar_state.read(cx).sizes().len();
        self.sidebar_state.update(cx, |state, cx| {
            for ix in 0..count {
                state.reset_panel(ix, cx);
            }
        });
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

    /// 模板列表点击：总是开一个新 Tab（不像 open_saved 那样去重——同一个模板
    /// 允许并排开好几份改着用）。产出的是全新未保存请求，所以不设 `saved_id`：
    /// `saved_name` 在 restore 时只从 `saved_id` 反查，设了会在重启后丢，
    /// 标题因此统一走 URL 末段推导。
    pub fn open_template(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(template) = templates::find(id) else {
            return;
        };
        let draft = template.draft();
        self.new_tab(window, cx);
        let tab = self.active_tab();
        tab.update(cx, |t, cx| {
            t.load_draft(&draft, window, cx);
            t.saved_id = None;
            // load_draft 走的是不发事件的程序化写入，不会置脏，这里手动标记
            t.mark_dirty(cx);
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
        let titles: Vec<(Method, SharedString, bool)> = self
            .tabs
            .iter()
            .map(|t| {
                let t = t.read(cx);
                (t.current_method(cx), t.title(cx), t.dirty)
            })
            .collect();
        // max_offset / bounds 只在 prepaint 阶段写入：首帧、以及标签装得下时都是 0。
        let max_x = self.tab_scroll.max_offset().x;
        let overflowing = max_x > px(0.);
        // 必须自己再夹一次：set_offset 不夹值，div 只在 prepaint 夹，
        // 直接读回的瞬时越界值会被误判成「还能接着滚」
        let offset_x = self.tab_scroll.offset().x.clamp(-max_x, px(0.));
        TabBar::new("request-tabs")
            .w_full()
            .pl_1()
            // 标题是 URL 路径，长起来能顶到 /v1/chat/completions；封顶后 label 自己
            // 省略号截断，prefix 的角标与 suffix 的关闭按钮保持原尺寸。
            // 上限比 spec 的 180 宽 20 px：prefix 多了 40 px 的 method 角标，
            // 不放宽的话 label 只剩不到半个词的位置。
            .max_width(px(200.))
            .track_scroll(&self.tab_scroll)
            // 官方内置的溢出菜单：∨ 展开全部标签、当前项打勾，点一下直接跳。
            // 它只切换选中不负责滚动，滚动由 activate 里的 reveal_active_tab 兜住。
            .menu(true)
            // 箭头只在真装不下时出现。max_offset 要等布局算完才有值，
            // 所以刚好溢出的那一帧还看不到，下一次重绘就正常了。
            .when(overflowing, |bar| {
                bar.suffix(
                    h_flex()
                        .items_center()
                        .gap_0p5()
                        .pr_1()
                        .child(
                            Button::new("tabs-scroll-left")
                                .ghost()
                                .xsmall()
                                .icon(IconName::ChevronLeft)
                                .disabled(offset_x >= px(-0.5))
                                .tooltip(tr!("tab.scroll_left"))
                                .on_click(cx.listener(|this, _, _, cx| this.scroll_tabs(-1., cx))),
                        )
                        .child(
                            Button::new("tabs-scroll-right")
                                .ghost()
                                .xsmall()
                                .icon(IconName::ChevronRight)
                                .disabled(offset_x <= -max_x + px(0.5))
                                .tooltip(tr!("tab.scroll_right"))
                                .on_click(cx.listener(|this, _, _, cx| this.scroll_tabs(1., cx))),
                        ),
                )
            })
            .prefix(
                div().pr_2().child(
                    Button::new("new-tab")
                        .outline()
                        .small()
                        .icon(IconName::Plus)
                        .tooltip_with_action(tr!("tab.new"), &NewTab, None)
                        .on_click(cx.listener(|this, _, window, cx| this.new_tab(window, cx))),
                ),
            )
            .selected_index(self.active)
            .on_click(cx.listener(|this, ix: &usize, _, cx| this.activate(*ix, cx)))
            .children(
                titles
                    .into_iter()
                    .enumerate()
                    .map(|(ix, (method, title, dirty))| {
                        let aria = if dirty {
                            tr!("tab.aria_dirty", title = title)
                        } else {
                            tr!("tab.aria", title = title)
                        };
                        // 角标走 prefix 而不是拼进 label：max_width 下 prefix 被包了
                        // flex_shrink_0，只有 label 会被省略号截断；而且溢出菜单与 aria
                        // 用的都是 label 文本，method 不该混进去。
                        // prefix 只能设一次，未保存的圆点必须并进同一个元素。
                        // Tab 根节点没有水平 padding，不补 pl 角标会贴死左边缘。
                        Tab::new()
                            .label(title)
                            .aria_label(aria)
                            .pl_2()
                            .prefix(
                                h_flex()
                                    .gap_1()
                                    .items_center()
                                    .child(
                                        div()
                                            .w_10()
                                            .flex_none()
                                            .text_xs()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(method_color(method, cx))
                                            .child(method.short()),
                                    )
                                    .when(dirty, |h| {
                                        // 未保存改动：标题前的圆点（spec §7.1）
                                        h.child(
                                            div()
                                                .size_1p5()
                                                .rounded_full_style(cx)
                                                .bg(cx.theme().primary),
                                        )
                                    }),
                            )
                            .suffix(
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
                    }),
            )
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

/// 右侧图标栏的功能入口与「生成代码」抽屉的开合。
impl Workspace {
    /// 右侧图标栏的点击入口。
    pub fn open_tool_section(
        &mut self,
        section: ToolSection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 点当前开着的那个 → 收起；点另一个 → 换内容（与左侧栏同款手感）
        if self.open_tool == Some(section) {
            self.close_tool_sheet(window, cx);
            return;
        }
        match section {
            ToolSection::CodeGen => self.open_code_sheet(window, cx),
            ToolSection::ImportCurl => self.open_curl_sheet(window, cx),
        }
    }

    /// 收起当前抽屉。
    ///
    /// 用 `open_tool` 而不是 `window.has_active_sheet` 判断有没有抽屉开着：后者内部走
    /// `Root::read`，窗口根不是 `gpui_component::Root` 时直接 unwrap 一个 None
    /// （`#[gpui::test]` 的测试窗口就是这样）。`open_tool` 本来就完整记录了这件事。
    fn close_tool_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.open_tool.take().is_some() {
            window.close_sheet(cx);
        }
        cx.notify();
    }

    /// 打开（或收起）代码抽屉。再点一次同一个图标就收起，与左侧栏「点当前功能收起」同款手感。
    ///
    /// builder 里**只把 `Entity<CodeSheet>` 传进去**，不碰 `self`：这个闭包是 `Fn`，
    /// 每帧在本实体的 `render` 内部执行，动一下 `self` 就会二次借用而 panic。
    pub fn open_code_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.refresh_code_sheet(window, cx);
        self.open_tool = Some(ToolSection::CodeGen);

        let code_sheet = self.code_sheet.clone();
        window.open_sheet(cx, move |sheet, _, _| {
            sheet
                .size(px(CODE_SHEET_WIDTH))
                .title(div().child(tr!("tools.code.title")))
                .child(code_sheet.clone())
        });
        cx.notify();
    }

    /// 打开「导入 cURL」抽屉。
    ///
    /// builder 里只放 `Entity<CurlSheet>` 与一个弱引用闭包，**绝不碰 `self`**：
    /// 它是 `Fn`、每帧在本实体的 `render` 内部执行，动一下就二次借用 panic。
    pub fn open_curl_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_tool = Some(ToolSection::ImportCurl);
        let curl_sheet = self.curl_sheet.clone();
        let ws = cx.entity().downgrade();

        window.open_sheet(cx, move |sheet, _, cx| {
            let body = curl_sheet.clone();
            let ws = ws.clone();
            // 解析不出来就没得导，按钮置灰而不是点了没反应
            let ready = curl_sheet.read(cx).draft().is_some();
            sheet
                .size(px(CURL_SHEET_WIDTH))
                .title(div().child(tr!("tools.import_curl.title")))
                .footer(import_button(ready, move |_, window, cx| {
                    if let Some(ws) = ws.upgrade() {
                        ws.update(cx, |ws, cx| ws.import_curl(window, cx));
                    }
                }))
                .child(body)
        });
        // 聚焦必须等到 Sheet 挂上之后：`open_sheet` 自己会接管焦点
        // （`Root` 会记下打开前的焦点以便关闭时还原），在它之前聚焦一定被盖掉。
        // 这个抽屉的第一动作就是粘贴，光标不在输入框里等于每次都要先点一下。
        let sheet = self.curl_sheet.clone();
        window.on_next_frame(move |window, cx| {
            sheet.update(cx, |sheet, cx| sheet.focus(window, cx));
        });
        cx.notify();
    }

    /// 把抽屉里解析好的草稿开成一个新 Tab。
    ///
    /// 开新 Tab 而不是改当前那个：当前 Tab 多半正编到一半，导入不该把它冲掉。
    pub fn import_curl(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(draft) = self.curl_sheet.read(cx).draft().cloned() else {
            return;
        };
        // 与 duplicate_tab 同一条路径：建实体 → 装草稿 → 置脏 → 立即落草稿
        self.new_tab(window, cx);
        let tab = self.active_tab().clone();
        tab.update(cx, |t, cx| {
            t.load_draft(&draft, window, cx);
            t.saved_id = None;
            // load_draft 是不发事件的程序化写入，不会置脏，这里手动标记
            t.mark_dirty(cx);
            t.save_draft_now(cx);
            cx.notify();
        });
        self.curl_sheet
            .update(cx, |sheet, cx| sheet.clear(window, cx));
        self.close_tool_sheet(window, cx);
    }

    /// 把当前 Tab 的草稿与默认头开关交给抽屉，重新生成代码。
    ///
    /// 默认请求头的开关是全局设置，所以每次都现取——用户刚在设置里关掉 User-Agent，
    /// 下一次打开抽屉就该看到它消失。
    pub(crate) fn refresh_code_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let draft = self.active_tab().read(cx).draft(cx);
        let disabled = settings::settings(cx).request.disabled_default_headers;
        self.code_sheet.update(cx, |sheet, cx| {
            sheet.load(draft, disabled, window, cx);
        });
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
        // 同理，Sheet 层也得自己画出来：不挂这一句，window.open_sheet 只会更新状态，
        // 画面上什么都不会出现。
        let sheet_layer = Root::render_sheet_layer(window, cx);
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
            .on_action(
                cx.listener(|this, _: &DuplicateTab, window, cx| this.duplicate_active(window, cx)),
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
                    )
                    // 与最左的图标栏对称，固定在最右、从不移动
                    .child(self.render_tool_rail(cx)),
            )
            .child(self.render_status_bar(cx))
            .children(sheet_layer)
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
