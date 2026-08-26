//! 顶层工作区：Tab 列表、侧栏、主题、全局动作；负责从磁盘恢复，并把布局变化写回。

// 显式导入而非 `use gpui::*`：本文件含 `#[cfg(test)] mod tests`，通配符会引入
// gpui 重导出的 `#[proc_macro_attribute] test`，与标准库 `#[test]` 同名冲突，
// 导致该属性宏对自身生成的 `#[test]` 反复展开直至递归上限溢出。
use std::cell::Cell;
use std::rc::Rc;

use getcat_core::model::{
    MAX_TAB_ROWS, Method, SavedRequest, SplitDirection, TabDraft, TabId, ThemePref, Ulid,
    WorkspaceState, now_ms,
};
use getcat_core::store::Loaded;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    App, AppContext, Context, Entity, FocusHandle, FontWeight, InteractiveElement, IntoElement,
    ParentElement, Render, Role, ScrollHandle, SharedString, StatefulInteractiveElement, Styled,
    Subscription, UniformListScrollHandle, WeakEntity, Window, div, point, px,
};
use gpui_component::{
    ActiveTheme, IconName, Root, Selectable, Sizable, Theme, ThemeMode, TitleBar, WindowExt,
    alert::Alert,
    button::{Button, ButtonVariant, ButtonVariants},
    dialog::{DialogAction, DialogButtonProps, DialogClose, DialogFooter},
    h_flex,
    input::{Input, InputState},
    resizable::{ResizableState, h_resizable, resizable_panel},
    status_bar::StatusBar,
    v_flex,
};

use gpui_updater::UpdateStatus;

use crate::brand::APP_NAME;
use crate::i18n::tr;
use crate::state::request_tab::RequestTab;
use crate::state::saved_filter::{self, SavedFilter};
use crate::state::settings;
use crate::state::store::{banner, store};
use crate::state::update;
use crate::templates;
use crate::ui::code_sheet::{CODE_SHEET_WIDTH, CodeSheet};
use crate::ui::curl_sheet::{CURL_SHEET_WIDTH, CurlSheet, import_button};
use crate::ui::settings_dialog::{SettingsPage, open_settings, open_settings_page};
use crate::ui::tab_strip::{next_tab_rows, page_count, tabs_per_page};
use crate::{
    CloseTab, DuplicateTab, FindInResponse, NewTab, OpenSettings, SaveRequest, SendRequest,
    ToggleSidebar,
};

/// 侧栏默认宽度（spec §4.3：两栏主从式分类列上线后加宽，容纳分类列 + 请求列）。
pub const SIDEBAR_DEFAULT_WIDTH: f32 = 360.;

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
    /// 首帧之后把 panel 的尺寸偏好归还为 None（见 [`Self::reset_workspace_panels`]）。
    ///
    /// 首帧 prepaint 会把 `ResizableState` 里占位的 `PANEL_MIN_SIZE` 覆写成
    /// Some(实测宽)；启动即展开时**两个** panel 都会被写上，「任一 panel 为 None
    /// 就不做比例重分配」的保护随之失效，此后窗口宽度一变，侧栏就被按
    /// size/total 的比例缩放。首帧过后 sizes 已是真实值，占位覆写不会再发生，
    /// 归一化一次即可永久恢复保护。
    panels_normalize_pending: bool,
    theme: ThemePref,
    /// 请求 / 响应分栏方向（工作区级，写入 workspace.json）。
    split: SplitDirection,
    /// 已保存请求，按 updated_at 降序；Rc 让侧栏列表的渲染闭包每帧只 clone 指针。
    saved: Rc<Vec<SavedRequest>>,
    /// 侧栏列表的滚动句柄。
    saved_scroll: UniformListScrollHandle,
    /// 侧栏「已保存」当前的过滤器。会话状态，不落盘（spec §3），重启回「全部」。
    saved_filter: SavedFilter,
    /// 标签栏的横向滚动句柄：单行模式下左右箭头与「激活项滚入视口」都靠它。
    tab_scroll: ScrollHandle,
    /// 标签栏行数：1 = 单行横向滚动，`MAX_TAB_ROWS` = 多行分页。写入 workspace.json。
    tab_rows: u8,
    /// 多行模式当前页；纯会话状态，不落盘（重启后从激活标签重新算就够了）。
    tab_page: usize,
    /// 标签区在上一帧量到的可用宽度，分页用它算每行装几个。
    ///
    /// `Cell` 而不是普通字段：这是在 prepaint 里写的，走 `&mut self` 会让每帧的
    /// 布局测量都触发一次重绘（gpui-component 的 TabBar 出于同样理由用 `Rc<RefCell>`）。
    strip_width: Rc<Cell<f32>>,
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
            panels_normalize_pending: true,
            theme: state.theme,
            split: state.split,
            saved: Rc::new(saved),
            saved_scroll: UniformListScrollHandle::new(),
            saved_filter: SavedFilter::default(),
            tab_scroll: ScrollHandle::new(),
            tab_rows: state.tab_rows.clamp(1, MAX_TAB_ROWS),
            tab_page: 0,
            strip_width: Rc::new(Cell::new(0.)),
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

        // 恢复出来的激活标签可能在很靠后的位置：光设 `active` 是不够的，标签栏还得滚 / 翻过去。
        // 但这两件事都要等布局算完才做得准，所以推迟到首帧之后（见函数注释）。
        Self::reveal_active_tab_after_first_frame(window, cx);

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
        self.duplicate_tab(self.active, window, cx);
    }

    /// 复制指定 Tab；`duplicate_active` 与标签右键菜单都走这里。
    pub fn duplicate_tab(&mut self, source_ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if source_ix >= self.tabs.len() {
            return;
        }
        let (draft, raw_format) = {
            let t = self.tabs[source_ix].read(cx);
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

    /// 关闭除 `keep` 之外的所有 Tab。
    ///
    /// 从后往前删：正序删会让后面的下标一路前移，`keep` 自己也会跟着漂。
    /// 每个都要删草稿文件，语义与 [`Self::close_tab`] 一致。
    pub fn close_other_tabs(&mut self, keep: usize, cx: &mut Context<Self>) {
        if keep >= self.tabs.len() || self.tabs.len() == 1 {
            return;
        }
        for ix in (0..self.tabs.len()).rev() {
            if ix == keep {
                continue;
            }
            if let Some(store) = store(cx) {
                store.delete_draft(self.tabs[ix].read(cx).id);
            }
            self.tabs.remove(ix);
        }
        self.active = 0;
        self.reveal_active_tab();
        self.persist_workspace(cx);
        cx.notify();
    }

    /// 关闭所有 Tab。工作区不留空窗——照 [`Self::close_tab`] 关最后一个时的做法补一个新的。
    pub fn close_all_tabs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for tab in &self.tabs {
            if let Some(store) = store(cx) {
                store.delete_draft(tab.read(cx).id);
            }
        }
        self.tabs.clear();
        // new_tab 自己会 persist + notify
        self.new_tab(window, cx);
    }

    pub fn activate(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix < self.tabs.len() && ix != self.active {
            self.active = ix;
            self.reveal_active_tab();
            self.persist_workspace(cx);
            cx.notify();
        }
    }

    /// 让激活的标签露出来。标签多到装不下时，不这么做就会出现「新建了标签却看不见」。
    ///
    /// 单行滚动、多行翻页——两种模式各有各的「露出来」。
    fn reveal_active_tab(&mut self) {
        if self.tab_rows > 1 {
            let per_page = tabs_per_page(self.strip_width.get(), self.tab_rows);
            self.tab_page = self.active / per_page.max(1);
        } else {
            self.tab_scroll.scroll_to_item(self.active);
        }
    }

    /// 等首帧画完再 reveal 一次。恢复现场（[`Self::restore`]）专用。
    ///
    /// [`Self::reveal_active_tab`] 的两条路都要读布局结果，而 `restore` 跑在首帧之前：
    ///
    /// - 多行靠 `strip_width`，它由标签栏的 `on_prepaint` 回填，此刻还是 0，
    ///   `tabs_per_page` 退化成行数本身，页码算歪；
    /// - 单行靠 `ScrollHandle::scroll_to_item`，它只记一个待办项，真正滚动的
    ///   `scroll_to_active_item` 跑在 div 的 prepaint 里（zed e0931d5
    ///   `crates/gpui/src/elements/div.rs:1908`），而它**早于**同一次 prepaint 里给滚动句柄
    ///   写 `overflow` / `bounds` 的那两行（2337 / 2367）。首帧时 `overflow` 还是默认的
    ///   `Visible`，滚动分支整段跳过；可 `child_bounds` 已经在这次 prepaint 里填好了，
    ///   待办项于是被当成「已处理」清掉——请求就这么蒸发，且不会自动重试。
    ///
    /// 所以要嵌两层 `on_next_frame`：帧回调跑在 `window.draw` **之前**
    /// （zed e0931d5 `crates/gpui/src/window.rs:1592`），而 `restore` 本身发生在首帧之前，
    /// 第一层回调触发时依旧什么都没画。同样的理由见
    /// [`RequestTab::find_in_response`](crate::state::request_tab::RequestTab::find_in_response)。
    fn reveal_active_tab_after_first_frame(window: &mut Window, cx: &mut Context<Self>) {
        fn reveal(ws: &WeakEntity<Workspace>, cx: &mut App) {
            let Some(ws) = ws.upgrade() else { return };
            ws.update(cx, |ws, cx| {
                ws.reveal_active_tab();
                cx.notify();
            });
        }

        let ws = cx.entity().downgrade();
        window.on_next_frame(move |window, _| {
            let settled = ws.clone();
            // 第二帧：布局已经量到，滚 / 翻过去
            window.on_next_frame(move |window, cx| {
                reveal(&ws, cx);
                // 第三帧再校一次。单行刚溢出时滚动箭头是下一帧才出现的（`show_arrows` 要等
                // `max_offset` 有值），标签区会因此窄掉箭头那一截，而 gpui 算滚动偏移用的是
                // **上一帧**写进滚动句柄的 `bounds`——第一次算出来的偏移正好短这么点，
                // 激活标签的右边缘会被压在箭头底下。`reveal_active_tab` 是幂等的
                // （已经完整可见就什么都不动），补这一次只补差额，不会来回跳。
                window.on_next_frame(move |_, cx| reveal(&settled, cx));
            });
        });
    }

    /// 左右按钮：单行时横向滚一步，多行时翻一页。`dir` 为 -1 往左 / 上一页，+1 反之。
    pub fn step_tabs(&mut self, dir: i32, cx: &mut Context<Self>) {
        if self.tab_rows > 1 {
            let per_page = tabs_per_page(self.strip_width.get(), self.tab_rows);
            let pages = page_count(self.tabs.len(), per_page);
            let next = self.tab_page as i32 + dir;
            self.tab_page = next.clamp(0, pages as i32 - 1) as usize;
        } else {
            // gpui 的约定是向右滚时 `offset.x` 变负
            let max = self.tab_scroll.max_offset().x;
            let x =
                (self.tab_scroll.offset().x - px(TAB_SCROLL_STEP) * dir as f32).clamp(-max, px(0.));
            self.tab_scroll.set_offset(point(x, px(0.)));
        }
        cx.notify();
    }

    /// 单行 ⇄ 多行。切过去时把当前标签所在那页翻出来，免得切完发现选中的标签不见了。
    pub fn toggle_tab_rows(&mut self, cx: &mut Context<Self>) {
        self.tab_rows = next_tab_rows(self.tab_rows);
        self.reveal_active_tab();
        self.persist_workspace(cx);
        cx.notify();
    }

    pub fn tab_rows(&self) -> u8 {
        self.tab_rows
    }

    pub fn tab_page(&self) -> usize {
        self.tab_page
    }

    pub fn active_index(&self) -> usize {
        self.active
    }

    pub(crate) fn tab_scroll(&self) -> &ScrollHandle {
        &self.tab_scroll
    }

    /// 标签区上一帧量到的可用宽度。
    pub fn strip_width(&self) -> f32 {
        self.strip_width.get()
    }

    /// 供标签栏在 prepaint 里回填可用宽度的句柄。
    pub(crate) fn strip_width_cell(&self) -> Rc<Cell<f32>> {
        self.strip_width.clone()
    }

    /// 渲染标签栏需要的每个 Tab 的信息。
    pub(crate) fn tabs_meta(&self, cx: &App) -> Vec<(Method, SharedString, bool)> {
        self.tabs
            .iter()
            .map(|t| {
                let t = t.read(cx);
                (t.current_method(cx), t.title(cx), t.dirty)
            })
            .collect()
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
            tab_rows: self.tab_rows,
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
    /// 这条防的是**比例重分配**路径：gpui-component 的 `ResizablePanel` 在
    /// `visible(false)` 时 render 第一件事就是 `return div()`，既不 prepaint 也不写
    /// `ResizableState::sizes`。于是收起期间 sizes 停在陈旧的 `[侧栏宽, 容器全宽]`，
    /// 其总和远大于容器宽度；而 `ResizablePanelGroup` 只要容器宽度一变就会按
    /// `size / total` 的比例重分配（窗口 resize），侧栏于是被按 `300 / 1500`
    /// 这样的错误比例压窄。
    ///
    /// 注意「展开几秒后自己缩一点」的**可见**症状不走这条路，而是样式层的 flex
    /// 收缩——panel size 一旦变成 Some 就丢掉内部的 `flex_none`，修复是 render 里
    /// 调用方自己 `.flex_none()`（见那处注释）；「启动即展开」时首帧占位覆写造成
    /// 的保护失效则由 [`Self::panels_normalize_pending`] 兜底。三者互补。
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

    pub(crate) fn saved_filter(&self) -> &SavedFilter {
        &self.saved_filter
    }

    pub fn set_saved_filter(&mut self, filter: SavedFilter, cx: &mut Context<Self>) {
        if self.saved_filter != filter {
            self.saved_filter = filter;
            cx.notify();
        }
    }

    /// 当前过滤下的下标快照；渲染闭包拿它 + `saved_rc()`，每帧仍是 O(可见行)。
    pub(crate) fn filtered_saved_indices(&self) -> Vec<usize> {
        saved_filter::filter_indices(&self.saved_filter, &self.saved)
    }

    /// 组织操作共用：`retag` 返回 None = 这条不动，Some(g) = 把 group 设为 g。
    /// 只重写真正变化的文件；**不碰 `updated_at`**（spec §3，组织操作不算内容修改）。
    fn retag_saved(
        &mut self,
        mut retag: impl FnMut(&SavedRequest) -> Option<Option<String>>,
        cx: &mut Context<Self>,
    ) {
        let list = Rc::make_mut(&mut self.saved);
        let mut changed = Vec::new();
        for request in list.iter_mut() {
            if let Some(new_group) = retag(request)
                && request.group != new_group
            {
                request.group = new_group;
                changed.push(request.clone());
            }
        }
        if changed.is_empty() {
            return;
        }
        if let Some(store) = store(cx) {
            for request in changed {
                store.write_request(request);
            }
        }
        self.ensure_saved_filter_valid();
        cx.notify();
    }

    pub fn move_saved_to_group(&mut self, id: Ulid, group: Option<String>, cx: &mut Context<Self>) {
        self.retag_saved(|r| (r.id == id).then(|| group.clone()), cx);
    }

    /// 重命名分类；目标名已存在时自然合并（推导模型下同名即同类）。
    pub fn rename_group(&mut self, from: &str, to: &str, cx: &mut Context<Self>) {
        let Some(to) = saved_filter::normalize_group(to) else {
            return; // 空名不接受，对话框层已挡，这里兜底
        };
        if from == to {
            return;
        }
        // 正看着旧名就跟着切到新名，别让用户被甩回「全部」
        if self.saved_filter == SavedFilter::Group(from.to_string()) {
            self.saved_filter = SavedFilter::Group(to.clone());
        }
        self.retag_saved(
            |r| (r.group.as_deref() == Some(from)).then(|| Some(to.clone())),
            cx,
        );
    }

    /// 解散分类：成员回未分类，请求本身不删。
    pub fn dissolve_group(&mut self, name: &str, cx: &mut Context<Self>) {
        self.retag_saved(|r| (r.group.as_deref() == Some(name)).then_some(None), cx);
    }

    /// 选中的分类没有成员了（删光/解散/合并走）→ 回退「全部」。
    fn ensure_saved_filter_valid(&mut self) {
        if !saved_filter::filter_still_valid(&self.saved_filter, &self.saved) {
            self.saved_filter = SavedFilter::All;
        }
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
        self.ensure_saved_filter_valid();
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

    /// 重命名分类：输入框对话框；同名即合并（正文写明）。
    pub(crate) fn prompt_rename_group(
        &mut self,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if window.has_active_dialog(cx) {
            return;
        }
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(tr!("dialog.rename_group.name"))
                .default_value(SharedString::from(name.clone()))
        });
        let weak = cx.entity().downgrade();
        let input_for_focus = input.clone();
        window.open_dialog(cx, move |dialog, _, _| {
            let input_for_content = input.clone();
            let input_for_ok = input.clone();
            let weak = weak.clone();
            let from = name.clone();
            dialog
                .title(tr!("dialog.rename_group.title", name = from.clone()))
                .content(move |content, _, cx| {
                    content
                        .child(
                            Input::new(&input_for_content)
                                .aria_label(tr!("dialog.rename_group.name")),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(tr!("dialog.rename_group.body")),
                        )
                })
                .footer(
                    DialogFooter::new()
                        .child(
                            DialogClose::new().child(
                                Button::new("cancel-rename")
                                    .outline()
                                    .label(tr!("common.cancel")),
                            ),
                        )
                        .child(
                            DialogAction::new().child(
                                Button::new("ok-rename").primary().label(tr!("common.save")),
                            ),
                        ),
                )
                .on_ok(move |_, _, cx| {
                    let to = input_for_ok.read(cx).value().to_string();
                    if let Some(ws) = weak.upgrade() {
                        ws.update(cx, |ws, cx| ws.rename_group(&from, &to, cx));
                    }
                    true
                })
        });
        input_for_focus.update(cx, |s, cx| s.focus(window, cx));
    }

    /// 解散分类：AlertDialog 确认（与删除请求同款式样）。
    pub(crate) fn confirm_dissolve_group(
        &mut self,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if window.has_active_dialog(cx) {
            return;
        }
        let weak = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let weak = weak.clone();
            let name = name.clone();
            alert
                .title(tr!("dialog.dissolve_group.title", name = name.clone()))
                .description(tr!("dialog.dissolve_group.body"))
                .button_props(
                    DialogButtonProps::default()
                        .ok_text(tr!("dialog.dissolve_group.ok"))
                        .ok_variant(ButtonVariant::Danger)
                        .cancel_text(tr!("common.cancel"))
                        .show_cancel(true),
                )
                .on_ok(move |_, _, cx| {
                    if let Some(ws) = weak.upgrade() {
                        ws.update(cx, |ws, cx| ws.dissolve_group(&name, cx));
                    }
                    true
                })
        });
    }

    /// 「移动到分类 ▸ 新建分类…」：输入框对话框，确定后移动。
    pub(crate) fn prompt_move_to_new_group(
        &mut self,
        id: Ulid,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if window.has_active_dialog(cx) {
            return;
        }
        let input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(tr!("dialog.move_to_new_group.name"))
        });
        let weak = cx.entity().downgrade();
        let input_for_focus = input.clone();
        window.open_dialog(cx, move |dialog, _, _| {
            let input_for_content = input.clone();
            let input_for_ok = input.clone();
            let weak = weak.clone();
            dialog
                .title(tr!("dialog.move_to_new_group.title"))
                .content(move |content, _, _| {
                    content.child(
                        Input::new(&input_for_content)
                            .aria_label(tr!("dialog.move_to_new_group.name")),
                    )
                })
                .footer(
                    DialogFooter::new()
                        .child(
                            DialogClose::new().child(
                                Button::new("cancel-move")
                                    .outline()
                                    .label(tr!("common.cancel")),
                            ),
                        )
                        .child(
                            DialogAction::new()
                                .child(Button::new("ok-move").primary().label(tr!("common.save"))),
                        ),
                )
                .on_ok(move |_, _, cx| {
                    let group =
                        crate::state::saved_filter::normalize_group(&input_for_ok.read(cx).value());
                    if let Some(ws) = weak.upgrade() {
                        ws.update(cx, |ws, cx| ws.move_saved_to_group(id, group, cx));
                    }
                    true
                })
        });
        input_for_focus.update(cx, |s, cx| s.focus(window, cx));
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
        // 首帧 prepaint 把占位的 PANEL_MIN_SIZE 覆写成 Some(实测宽) 之后，归一化一次
        // （见 `panels_normalize_pending` 字段注释）。sizes 为空说明首帧还没画过——
        // 此时 `reset_panel` 会索引越界，留到下一次 render。
        if self.panels_normalize_pending && !self.sidebar_state.read(cx).sizes().is_empty() {
            self.panels_normalize_pending = false;
            self.reset_workspace_panels(cx);
        }
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
                                        // spec §4.3：拖拽范围随两栏分类列改宽（老 workspace.json 里
                                        // < 280 的 sidebar_width 由 panel 的 min_w 自然夹住，不迁移）。
                                        .size_range(px(280.)..px(560.))
                                        // 上游 `ResizablePanel` 内部无条件 `flex_grow_1`，且只在
                                        // panel size 为 None 时自带 `flex_none`。首次展开的那一帧
                                        // prepaint 会把占位的 `PANEL_MIN_SIZE` 覆写成 Some(实测宽)，
                                        // 此后若不由调用方钉死 flex，侧栏就带着默认 flex_shrink:1
                                        // 参与布局，下一次 render 被主区（basis=容器全宽）压掉约
                                        // 60 px——即「展开几秒后自己变窄」。上游文档点名要求有尺寸
                                        // 偏好的 panel 自行 `.flex_none()`；flex_basis 仍由内部的
                                        // 尺寸管理驱动，拖拽不受影响。
                                        .flex_none()
                                        .visible(!self.sidebar_collapsed)
                                        .child(self.render_sidebar(window, cx)),
                                )
                                .child(
                                    resizable_panel().child(
                                        v_flex()
                                            .size_full()
                                            .min_w_0()
                                            .child(self.render_tab_strip(cx))
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
