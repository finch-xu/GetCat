//! 请求标签栏：单行横向滚动，或最多三行分页。
//!
//! # 为什么自绘而不用 gpui-component 的 `TabBar`
//!
//! 三件事它都做不到：
//!
//! 1. **多行**：`TabBar` 把标签区写死成 `h_flex().overflow_x_scroll()`，而它的
//!    `refine_style` 只作用在最外层容器上，改不成换行布局。
//! 2. **右键菜单**：`Tab` 的根节点是 `overflow_hidden()` 的，挂在它内部的
//!    `ContextMenu`（绝对定位的子元素）会被整个裁掉；而 `TabBar::children` 只收
//!    `Into<Tab>`，在外面包一层带菜单的容器也传不进去。
//! 3. **单独复用 `Tab`**：`Tab::ix()` 是 `pub(crate)`，外部构造的 `Tab` 全部落在
//!    element id `0` 上，hover / 点击状态会串成一片。
//!
//! 所以单行与多行共用这一套自绘实现：样式天然一致，也省得两套渲染各自漂移。
//! 配色全部走主题 token（`tokens.tab_active` / `tab_foreground` / `border` …），
//! 与 `TabVariant::Tab` 的观感对齐——原始色值只允许出现在 `theme.rs`。
//!
//! # 分页为什么不用 flex-wrap
//!
//! wrap 的换行点是布局期才定的，Rust 侧算不出「每行装得下几个」，也就没法分页。
//! 多行模式改成给每个标签固定宽度 [`TAB_WIDTH`]，每行个数、页数、当前页全是纯计算：
//! 确定、可测，翻页按钮的禁用条件也算得准。

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, IconName, Selectable, Sizable, ThemeStyled,
    button::{Button, ButtonVariants},
    h_flex,
    menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenu, PopupMenuItem},
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, AppContext, Bounds, ClickEvent, Context, DragMoveEvent, FontWeight,
    InteractiveElement, IntoElement, MouseButton, ParentElement, Pixels, Point, Render, Role,
    SharedString, StatefulInteractiveElement, Styled, Task, WeakEntity, Window, canvas, div, point,
    px,
};

use getcat_core::model::{MAX_TAB_ROWS, Method, TabId, Ulid};

use crate::NewTab;
use crate::assets::ICON_ROWS_3;
use crate::i18n::tr;
use crate::state::variables;
use crate::state::workspace::Workspace;
use crate::ui::method_color;
use crate::ui::variables_sheet::SheetScope;

/// 标签宽度。单行模式下是上限（label 自己省略号截断），多行模式下是定值——
/// 分页要靠它算每行装几个。
///
/// 比 spec 的 180 宽 20 px：prefix 多了 40 px 的 method 角标，不放宽的话 label
/// 只剩不到半个词的位置。
pub const TAB_WIDTH: f32 = 200.;

/// 标签高度，与 gpui-component `TabVariant::Tab` 的默认尺寸对齐。
const TAB_HEIGHT: f32 = 30.;

const DRAG_EDGE: f32 = 24.;
const DRAG_SCROLL_STEP: f32 = 12.;
const DRAG_SCROLL_INTERVAL: Duration = Duration::from_millis(16);
const DRAG_PAGE_INTERVAL: Duration = Duration::from_millis(500);

/// 只保存上一帧的几何信息；prepaint 回填时不更新 Workspace 实体。
#[derive(Default)]
struct TabDragLayout {
    viewport: Option<Bounds<Pixels>>,
    tabs: Vec<(TabId, Bounds<Pixels>)>,
    previous: Option<Bounds<Pixels>>,
    next: Option<Bounds<Pixels>>,
    valid: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TabDragNavigation {
    Scroll(i32),
    Page(i32),
}

#[derive(Default)]
pub(crate) struct TabDragState {
    layout: Rc<RefCell<TabDragLayout>>,
    source: Option<TabId>,
    session: Option<Rc<()>>,
    pointer: Option<Point<Pixels>>,
    navigation: Option<TabDragNavigation>,
    navigation_task: Option<Task<()>>,
}

#[derive(Clone)]
struct TabDrag {
    id: TabId,
    owner: WeakEntity<Workspace>,
    session: Rc<()>,
    method: Method,
    title: SharedString,
    dirty: bool,
}

struct TabDragPreview(TabDrag);

/// 显式贴齐父元素的左上角，避免 flex 的静态定位和标签内边距偏移命中区域。
fn measure_drag_bounds(callback: impl FnOnce(Bounds<Pixels>) + 'static) -> impl IntoElement {
    canvas(move |bounds, _, _| callback(bounds), |_, _, _, _| {})
        .absolute()
        .top_0()
        .left_0()
        .size_full()
}

impl Render for TabDragPreview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .w(px(TAB_WIDTH))
            .h(px(TAB_HEIGHT))
            .px_2()
            .gap_1()
            .border_1()
            .border_color(cx.theme().border)
            .bg(*cx.theme().tokens.tab_active)
            .text_color(cx.theme().tab_active_foreground)
            .shadow_md()
            .child(
                div()
                    .w_11()
                    .flex_none()
                    .whitespace_nowrap()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(method_color(self.0.method, cx))
                    .child(self.0.method.short()),
            )
            .when(self.0.dirty, |d| {
                d.child(
                    div()
                        .size_1p5()
                        .flex_none()
                        .rounded_full_style(cx)
                        .bg(cx.theme().primary),
                )
            })
            .child(
                div()
                    .min_w_0()
                    .text_sm()
                    .truncate()
                    .child(self.0.title.clone()),
            )
    }
}

/// 每行能装下几个标签。`width` 为 0（首帧还没量到）时回落到 1，
/// 此时 `page_count` 也就是 1，全部标签当作一页——下一帧量到真实宽度后自动修正。
pub fn tabs_per_row(width: f32) -> usize {
    ((width / TAB_WIDTH).floor() as usize).max(1)
}

/// 分页：每页标签数。
pub fn tabs_per_page(width: f32, rows: u8) -> usize {
    tabs_per_row(width) * rows.max(1) as usize
}

/// 总页数（至少 1 页，空标签栏不存在）。
pub fn page_count(total: usize, per_page: usize) -> usize {
    total.div_ceil(per_page.max(1)).max(1)
}

impl Workspace {
    fn begin_tab_drag(&mut self, drag: &TabDrag, window: &mut Window, cx: &mut Context<Self>) {
        if self.tab_index(drag.id, cx).is_none() {
            return;
        }
        self.tab_drag.navigation_task = None;
        self.tab_drag.navigation = None;
        self.tab_drag.source = Some(drag.id);
        self.tab_drag.session = Some(drag.session.clone());
        self.tab_drag.pointer = Some(window.mouse_position());
        self.set_tab_drag_navigation(
            self.tab_drag_navigation(window.mouse_position()),
            window,
            cx,
        );
        cx.notify();
    }

    fn finish_tab_drag(&mut self, session: &Rc<()>, cx: &mut Context<Self>) {
        if !self
            .tab_drag
            .session
            .as_ref()
            .is_some_and(|current| Rc::ptr_eq(current, session))
        {
            return;
        }
        self.tab_drag.navigation_task = None;
        self.tab_drag.navigation = None;
        self.tab_drag.source = None;
        self.tab_drag.session = None;
        self.tab_drag.pointer = None;
        cx.notify();
    }

    fn reorder_dragged_tab(
        &mut self,
        position: Point<Pixels>,
        previous: Option<Point<Pixels>>,
        scroll_direction: Option<i32>,
        cx: &mut Context<Self>,
    ) {
        let Some(source) = self.tab_drag.source else {
            return;
        };
        let (target, bounds, source_bounds) = {
            let layout = self.tab_drag.layout.borrow();
            if !layout.valid || !layout.viewport.is_some_and(|b| b.contains(&position)) {
                return;
            }
            let Some(&(target, bounds)) = layout.tabs.iter().find(|(_, b)| b.contains(&position))
            else {
                return;
            };
            (
                target,
                bounds,
                layout
                    .tabs
                    .iter()
                    .find(|(id, _)| *id == source)
                    .map(|(_, b)| *b),
            )
        };
        let (Some(from), Some(to)) = (self.tab_index(source, cx), self.tab_index(target, cx))
        else {
            return;
        };
        if from == to {
            return;
        }
        let forward = to > from;
        let midpoint = bounds.origin.x + bounds.size.width / 2.;
        if (forward && position.x < midpoint) || (!forward && position.x > midpoint) {
            return;
        }
        if let Some(direction) = scroll_direction {
            if forward != (direction > 0) {
                return;
            }
        } else if let (Some(previous), Some(source_bounds)) = (previous, source_bounds) {
            // 标签宽度不一，换位后新的邻居可能立刻出现在鼠标下面。
            // 同行只接受实际移动方向；跨行则用纵向，允许行末绕回下一行开头。
            let same_row = (source_bounds.origin.y - bounds.origin.y).abs() < px(1.);
            let delta = if same_row {
                position.x - previous.x
            } else {
                position.y - previous.y
            };
            if (same_row && delta == px(0.))
                || (forward && delta < px(0.))
                || (!forward && delta > px(0.))
            {
                return;
            }
        }
        self.move_tab(source, target, cx);
        // 同一帧的后续鼠标事件不能继续使用换位前的坐标。
        self.tab_drag.layout.borrow_mut().valid = false;
    }

    fn tab_drag_navigation(&self, position: Point<Pixels>) -> Option<TabDragNavigation> {
        let layout = self.tab_drag.layout.borrow();
        if self.tab_rows() > 1 {
            if layout.previous.is_some_and(|b| b.contains(&position)) {
                return Some(TabDragNavigation::Page(-1));
            }
            if layout.next.is_some_and(|b| b.contains(&position)) {
                return Some(TabDragNavigation::Page(1));
            }
        } else if let Some(bounds) = layout.viewport.filter(|b| b.contains(&position)) {
            let left = position.x - bounds.origin.x;
            let right = bounds.right() - position.x;
            let offset = self.tab_scroll().offset().x;
            let max = self.tab_scroll().max_offset().x;
            if left < px(DRAG_EDGE) && left <= right && offset < px(0.) {
                return Some(TabDragNavigation::Scroll(-1));
            }
            if right < px(DRAG_EDGE) && right < left && offset > -max {
                return Some(TabDragNavigation::Scroll(1));
            }
        }
        None
    }

    fn set_tab_drag_navigation(
        &mut self,
        navigation: Option<TabDragNavigation>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.tab_drag.navigation == navigation {
            return;
        }
        self.tab_drag.navigation_task = None;
        self.tab_drag.navigation = navigation;
        let Some(navigation) = navigation else { return };
        let interval = match navigation {
            TabDragNavigation::Scroll(_) => DRAG_SCROLL_INTERVAL,
            TabDragNavigation::Page(_) => DRAG_PAGE_INTERVAL,
        };
        self.tab_drag.navigation_task = Some(cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(interval).await;
                let keep_going = this
                    .update_in(cx, |this, window, cx| {
                        let position = window.mouse_position();
                        let source_exists = this
                            .tab_drag
                            .source
                            .is_some_and(|id| this.tab_index(id, cx).is_some());
                        if !cx.has_active_drag()
                            || !source_exists
                            || !window.is_window_active()
                            || !window.is_window_hovered()
                        {
                            this.tab_drag.navigation = None;
                            return false;
                        }
                        if let TabDragNavigation::Scroll(direction) = navigation {
                            // 到达边界后仍处理最后一步滚动暴露出来的目标。
                            this.reorder_dragged_tab(position, None, Some(direction), cx);
                        }
                        if this.tab_drag_navigation(position) != Some(navigation) {
                            this.tab_drag.navigation = None;
                            return false;
                        }
                        match navigation {
                            TabDragNavigation::Scroll(direction) => {
                                let scroll = this.tab_scroll();
                                let max = scroll.max_offset().x;
                                let x = (scroll.offset().x
                                    - px(DRAG_SCROLL_STEP) * direction as f32)
                                    .clamp(-max, px(0.));
                                scroll.set_offset(point(x, px(0.)));
                                cx.notify();
                            }
                            TabDragNavigation::Page(direction) => {
                                this.step_tabs(direction, cx);
                            }
                        }
                        this.tab_drag.layout.borrow_mut().valid = false;
                        true
                    })
                    .unwrap_or(false);
                if !keep_going {
                    break;
                }
            }
        }));
    }

    fn update_tab_drag(
        &mut self,
        drag: &TabDrag,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self
            .tab_drag
            .session
            .as_ref()
            .is_some_and(|session| Rc::ptr_eq(session, &drag.session))
        {
            return;
        }
        if self.tab_index(drag.id, cx).is_none() {
            self.finish_tab_drag(&drag.session, cx);
            return;
        }
        let previous = self.tab_drag.pointer.replace(position);
        self.reorder_dragged_tab(position, previous, None, cx);
        self.set_tab_drag_navigation(self.tab_drag_navigation(position), window, cx);
    }

    /// 标签栏。行数由 [`Workspace::tab_rows`] 决定：1 走横向滚动，>1 走分页。
    pub fn render_tab_strip(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = self.tab_rows();
        let width = self.strip_width_cell();
        let layout = self.tab_drag.layout.clone();
        *layout.borrow_mut() = TabDragLayout::default();
        let multi_row = rows > 1;
        let tabs: Vec<TabMeta> = self
            .tabs_meta(cx)
            .into_iter()
            .enumerate()
            .map(|(ix, (id, method, title, dirty))| TabMeta {
                id,
                method,
                title,
                dirty,
                selected: ix == self.active_index(),
            })
            .collect();

        let body = if multi_row {
            self.render_paged_rows(&tabs, rows, cx).into_any_element()
        } else {
            self.render_single_row(&tabs, cx).into_any_element()
        };

        v_flex()
            .id("tab-strip")
            .role(Role::TabList)
            .aria_label(tr!("tab.strip_aria"))
            .w_full()
            .flex_none()
            .bg(cx.theme().tokens.tab_bar)
            .border_b_1()
            .border_color(cx.theme().border)
            .on_drag_move(
                cx.listener(|this, event: &DragMoveEvent<TabDrag>, window, cx| {
                    let drag = event.drag(cx).clone();
                    this.update_tab_drag(&drag, event.event.position, window, cx);
                }),
            )
            .child(
                h_flex()
                    .w_full()
                    .items_start()
                    .child(
                        // 新建按钮固定在最左，不跟着翻页跑
                        div().flex_none().px_1().py_1().child(
                            Button::new("new-tab")
                                .outline()
                                .small()
                                .icon(IconName::Plus)
                                .tooltip_with_action(tr!("tab.new"), &NewTab, None)
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.new_tab(window, cx)),
                                ),
                        ),
                    )
                    .child(
                        div()
                            .relative()
                            .flex_1()
                            .min_w_0()
                            .child(body)
                            // 分页要知道「一行装得下几个」，而这只有布局算完才知道。
                            // 写进 Cell 而不是 `&mut self`：prepaint 里改状态会让每帧
                            // 的测量都触发一次重绘。
                            .child(measure_drag_bounds(move |bounds| {
                                width.set(bounds.size.width.into());
                                let mut layout = layout.borrow_mut();
                                layout.viewport = Some(bounds);
                                layout.valid = true;
                            })),
                    )
                    .child(self.render_strip_controls(tabs.len(), rows, cx)),
            )
    }

    /// 单行：横向滚动，与改动前的手感一致。
    fn render_single_row(&self, tabs: &[TabMeta], cx: &mut Context<Self>) -> AnyElement {
        let mut children = Vec::with_capacity(tabs.len());
        for tab in tabs {
            children.push(self.render_tab(tab, false, cx));
        }
        h_flex()
            .id("tab-row")
            .w_full()
            .overflow_x_scroll()
            .track_scroll(self.tab_scroll())
            .children(children)
            .into_any_element()
    }

    /// 多行：把当前页的标签按每行 `per_row` 个铺开。
    fn render_paged_rows(&self, tabs: &[TabMeta], rows: u8, cx: &mut Context<Self>) -> AnyElement {
        let per_row = tabs_per_row(self.strip_width());
        let per_page = per_row * rows as usize;
        let page = self.tab_page().min(page_count(tabs.len(), per_page) - 1);

        let mut rendered = Vec::new();
        for tab in tabs.iter().skip(page * per_page).take(per_page) {
            rendered.push(self.render_tab(tab, true, cx));
        }

        // AnyElement 不是 Copy 也没有 take，按 per_row 个一批消费掉这个迭代器
        let mut rows_out = Vec::new();
        let mut iter = rendered.into_iter().peekable();
        while iter.peek().is_some() {
            let row: Vec<_> = iter.by_ref().take(per_row).collect();
            rows_out.push(h_flex().children(row));
        }
        v_flex().w_full().children(rows_out).into_any_element()
    }

    /// 一个标签。`fixed_width` 为真时宽度定死（多行模式靠它分页），否则是上限。
    fn render_tab(&self, tab: &TabMeta, fixed_width: bool, cx: &mut Context<Self>) -> AnyElement {
        let id = tab.id;
        let selected = tab.selected;
        let aria = if tab.dirty {
            tr!("tab.aria_dirty", title = tab.title.clone())
        } else {
            tr!("tab.aria", title = tab.title.clone())
        };
        let active_bg = *cx.theme().tokens.tab_active;
        // 菜单回调只拿得到 `&mut App`，握弱引用而不是 `self`（照 `ui::sidebar` 的写法）
        let owner = cx.entity().downgrade();
        let layout = self.tab_drag.layout.clone();
        let drag = TabDrag {
            id,
            owner: owner.clone(),
            session: Rc::new(()),
            method: tab.method,
            title: tab.title.clone(),
            dirty: tab.dirty,
        };

        div()
            .id(format!("tab-{id}"))
            .role(Role::Tab)
            .aria_label(aria)
            .aria_selected(selected)
            // 菜单是绝对定位的子元素：这一层绝不能 overflow_hidden，否则弹出来就被裁掉。
            // 需要裁剪的只有标题，交给内层那个 truncate 的 div。
            .relative()
            .flex()
            .items_center()
            .gap_1()
            .flex_none()
            .h(px(TAB_HEIGHT))
            .px_2()
            .when(self.tab_drag.source == Some(id), |d| d.opacity(0.5))
            .on_drag(drag, |drag, _, window, cx| {
                if let Some(owner) = drag.owner.upgrade() {
                    owner.update(cx, |this, cx| this.begin_tab_drag(drag, window, cx));
                }
                let drag = drag.clone();
                cx.new(|cx| {
                    let owner = drag.owner.clone();
                    let session = drag.session.clone();
                    // 预览由 GPUI 在松手、取消或窗口关闭时释放，包含标签栏外结束拖动。
                    cx.on_release(move |_, cx| {
                        if let Some(owner) = owner.upgrade() {
                            owner.update(cx, |this, cx| this.finish_tab_drag(&session, cx));
                        }
                    })
                    .detach();
                    TabDragPreview(drag)
                })
            })
            .map(|d| {
                if fixed_width {
                    d.w(px(TAB_WIDTH))
                } else {
                    d.max_w(px(TAB_WIDTH))
                }
            })
            // 左右各一条细线把标签隔开，与 TabVariant::Tab 的处理一致
            .border_l_1()
            .border_r_1()
            .map(|d| {
                if selected {
                    d.bg(active_bg)
                        .border_color(cx.theme().border)
                        .text_color(cx.theme().tab_active_foreground)
                } else {
                    d.border_color(cx.theme().transparent)
                        .text_color(cx.theme().tab_foreground)
                        // 未选中的悬停给一层更淡的同色，好过只改文字色
                        .hover(|s| s.bg(active_bg.opacity(0.5)))
                }
            })
            .child(
                // 定宽让各行的标题左边缘对齐。44 而不是 40：`PATCH` 是 `Method::short()`
                // 允许的最长缩写（5 个字符），40 px 下会折成两行、把整行撑高。
                // `whitespace_nowrap` 是第二道保险，字号变化时也不会再折。
                div()
                    .w_11()
                    .flex_none()
                    .whitespace_nowrap()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(method_color(tab.method, cx))
                    .child(tab.method.short()),
            )
            .when(tab.dirty, |d| {
                // 未保存改动：标题前的圆点（spec §7.1）
                d.child(
                    div()
                        .size_1p5()
                        .flex_none()
                        .rounded_full_style(cx)
                        .bg(cx.theme().primary),
                )
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_sm()
                    .truncate()
                    .child(tab.title.clone()),
            )
            .child(
                div()
                    .flex_none()
                    // Button 自己先收到按下，随后在包装层截断，标签不会启动拖动。
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        Button::new(format!("close-tab-{id}"))
                            .ghost()
                            .xsmall()
                            .icon(IconName::Close)
                            .tooltip(tr!("tab.close"))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                if let Some(ix) = this.tab_index(id, cx) {
                                    this.close_tab(ix, window, cx);
                                }
                            })),
                    ),
            )
            .when(selected, |d| {
                // 顶部主题色指示条。只靠底色是不够的：底色能加深的幅度被方法角标的
                // 对比度卡死（再深一档 PUT 就跌破 4.8:1），而这条线是纯增量的信号——
                // 一眼就能定位当前标签，又不占用任何对比度预算。VS Code / Chrome 同款。
                d.child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .right_0()
                        .h(px(2.))
                        .bg(cx.theme().primary),
                )
            })
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                if let Some(ix) = this.tab_index(id, cx) {
                    this.activate(ix, cx);
                }
            }))
            // 右键先选中再弹菜单：菜单项说的都是「当前 Tab」，选中态必须先对上
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, _, _, cx| {
                    if let Some(ix) = this.tab_index(id, cx) {
                        this.activate(ix, cx);
                    }
                }),
            )
            .context_menu(move |menu, _, _| tab_menu(menu, id, owner.clone()))
            .child(measure_drag_bounds(move |bounds| {
                layout.borrow_mut().tabs.push((id, bounds))
            }))
            .into_any_element()
    }

    /// 标签栏右侧的一组控件：环境切换器、翻页 / 滚动箭头，以及行数切换。
    fn render_strip_controls(
        &self,
        total: usize,
        rows: u8,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let multi_row = rows > 1;
        let per_page = tabs_per_page(self.strip_width(), rows);
        let pages = page_count(total, per_page);
        let page = self.tab_page().min(pages - 1);

        // 单行下箭头只在真装不下时出现（max_offset 要等布局算完才有值，所以刚好
        // 溢出的那一帧还看不到，下一次重绘就正常了）；多行下按钮常驻，翻页是它的
        // 主要导航方式，藏起来反而让人找不到别的页。
        let max_x = self.tab_scroll().max_offset().x;
        let show_arrows = multi_row || max_x > px(0.);
        // set_offset 不夹值、div 只在 prepaint 夹，直接读回的瞬时越界值会被误判成「还能滚」
        let offset_x = self.tab_scroll().offset().x.clamp(-max_x, px(0.));
        let (at_start, at_end) = if multi_row {
            (page == 0, page + 1 >= pages)
        } else {
            (offset_x >= px(-0.5), offset_x <= -max_x + px(0.5))
        };

        let switcher = self.render_env_switcher(cx);
        let previous_layout = self.tab_drag.layout.clone();
        let next_layout = self.tab_drag.layout.clone();

        h_flex()
            .flex_none()
            .items_center()
            .gap_0p5()
            .px_1()
            .py_1()
            .child(switcher)
            .when(show_arrows, |h| {
                h.child(
                    div()
                        .relative()
                        .flex_none()
                        .child(
                            Button::new("tabs-prev")
                                .ghost()
                                .xsmall()
                                .icon(IconName::ChevronLeft)
                                .disabled(at_start)
                                .tooltip(if multi_row {
                                    tr!("tab.prev_page")
                                } else {
                                    tr!("tab.scroll_left")
                                })
                                .on_click(cx.listener(|this, _, _, cx| this.step_tabs(-1, cx))),
                        )
                        .child(measure_drag_bounds(move |bounds| {
                            previous_layout.borrow_mut().previous = (!at_start).then_some(bounds);
                        })),
                )
                .child(
                    div()
                        .relative()
                        .flex_none()
                        .child(
                            Button::new("tabs-next")
                                .ghost()
                                .xsmall()
                                .icon(IconName::ChevronRight)
                                .disabled(at_end)
                                .tooltip(if multi_row {
                                    tr!("tab.next_page")
                                } else {
                                    tr!("tab.scroll_right")
                                })
                                .on_click(cx.listener(|this, _, _, cx| this.step_tabs(1, cx))),
                        )
                        .child(measure_drag_bounds(move |bounds| {
                            next_layout.borrow_mut().next = (!at_end).then_some(bounds);
                        })),
                )
            })
            .child(
                Button::new("tabs-rows")
                    .ghost()
                    .xsmall()
                    .icon(Icon::empty().path(ICON_ROWS_3))
                    .selected(multi_row)
                    .tooltip(if multi_row {
                        tr!("tab.rows_toggle_off")
                    } else {
                        tr!("tab.rows_toggle")
                    })
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_tab_rows(cx))),
            )
    }

    /// 环境切换器：当前环境名 + 下拉菜单（选环境 / 「无环境」/「管理变量…」）。
    ///
    /// builder 里只握弱引用，不碰 `self`——同款约束：`dropdown_menu` 是 `Fn`，
    /// 每帧都可能在本实体的 `render` 内部被重新求值。
    fn render_env_switcher(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let owner = cx.entity().downgrade();
        let full_label = self.environment_label(cx);
        let aria = tr!("env_switcher.aria", name = full_label.clone());
        let envs: Vec<(Ulid, SharedString)> = variables::variables(cx)
            .environments
            .iter()
            .map(|e| (e.id, SharedString::from(e.name.clone())))
            .collect();
        let active = variables::variables(cx).active_environment;

        Button::new("env-switcher")
            .ghost()
            .xsmall()
            .icon(IconName::Globe)
            .label(truncate_env_label(&full_label))
            .dropdown_caret(true)
            .accessibility_label(aria.clone())
            .tooltip(aria)
            .dropdown_menu(move |menu, _, _| {
                let mut menu = menu;
                let none_owner = owner.clone();
                menu = menu.item(
                    PopupMenuItem::new(tr!("env_switcher.none"))
                        .checked(active.is_none())
                        .on_click(move |_, window, cx| {
                            if let Some(ws) = none_owner.upgrade() {
                                ws.update(cx, |ws, cx| ws.select_environment(None, window, cx));
                            }
                        }),
                );
                for (id, name) in envs.clone() {
                    let item_owner = owner.clone();
                    menu = menu.item(
                        PopupMenuItem::new(name)
                            .checked(active == Some(id))
                            .on_click(move |_, window, cx| {
                                if let Some(ws) = item_owner.upgrade() {
                                    ws.update(cx, |ws, cx| {
                                        ws.select_environment(Some(id), window, cx)
                                    });
                                }
                            }),
                    );
                }
                let manage_owner = owner.clone();
                menu.separator()
                    .item(PopupMenuItem::new(tr!("env_switcher.manage")).on_click(
                        move |_, window, cx| {
                            if let Some(ws) = manage_owner.upgrade() {
                                ws.update(cx, |ws, cx| {
                                    ws.open_variables_sheet(
                                        SheetScope::Environment,
                                        None,
                                        window,
                                        cx,
                                    )
                                });
                            }
                        },
                    ))
            })
    }
}

/// 切换器按钮上的可见文字：过长的环境名截到这么多字符（`Button` 的 `text_ellipsis`
/// 只在容器有宽度上限时才生效，而这个按钮的宽度就是内容撑出来的，所以自己先截）。
/// 可访问名称与 tooltip 不受影响，仍是全名（[`Workspace::render_env_switcher`]）。
const ENV_LABEL_MAX_CHARS: usize = 16;

fn truncate_env_label(name: &str) -> SharedString {
    if name.chars().count() <= ENV_LABEL_MAX_CHARS {
        return SharedString::from(name.to_string());
    }
    let short: String = name.chars().take(ENV_LABEL_MAX_CHARS).collect();
    SharedString::from(format!("{short}…"))
}

/// 标签的右键菜单。对象级命令：与行内的关闭按钮是同一组动作，
/// 键盘与辅助技术用户多一条可达路径（设计指南）。
///
/// 「关闭其他 / 关闭所有」不弹二次确认——草稿本来就随时落盘，关闭即删草稿是
/// [`Workspace::close_tab`] 早就定下的语义，这里跟着走。
fn tab_menu(menu: PopupMenu, id: TabId, owner: WeakEntity<Workspace>) -> PopupMenu {
    let duplicate = owner.clone();
    let others = owner.clone();
    menu.item(
        PopupMenuItem::new(tr!("tab.menu_duplicate")).on_click(move |_, window, cx| {
            if let Some(ws) = duplicate.upgrade() {
                ws.update(cx, |ws, cx| {
                    if let Some(ix) = ws.tab_index(id, cx) {
                        ws.duplicate_tab(ix, window, cx);
                    }
                });
            }
        }),
    )
    .separator()
    .item(
        PopupMenuItem::new(tr!("tab.menu_close_others")).on_click(move |_, _, cx| {
            if let Some(ws) = others.upgrade() {
                ws.update(cx, |ws, cx| {
                    if let Some(ix) = ws.tab_index(id, cx) {
                        ws.close_other_tabs(ix, cx);
                    }
                });
            }
        }),
    )
    .item(
        PopupMenuItem::new(tr!("tab.menu_close_all")).on_click(move |_, window, cx| {
            if let Some(ws) = owner.upgrade() {
                ws.update(cx, |ws, cx| ws.close_all_tabs(window, cx));
            }
        }),
    )
}

/// 渲染一个标签需要的全部信息，从各 Tab 实体里一次性读出来。
struct TabMeta {
    id: TabId,
    method: Method,
    title: SharedString,
    dirty: bool,
    selected: bool,
}

/// 行数在 1 与 [`MAX_TAB_ROWS`] 之间切换。
pub fn next_tab_rows(current: u8) -> u8 {
    if current > 1 { 1 } else { MAX_TAB_ROWS }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn per_row_falls_back_to_one_before_the_first_layout() {
        // 首帧还没量到宽度，全部标签当作一页；下一帧量到真实宽度后自动修正
        assert_eq!(tabs_per_row(0.), 1);
        assert_eq!(tabs_per_page(0., 3), 3);
        // 比一个标签还窄也至少放一个，否则 per_page 为 0、页数除零
        assert_eq!(tabs_per_row(120.), 1);
    }

    #[test]
    fn per_row_divides_the_available_width() {
        assert_eq!(tabs_per_row(TAB_WIDTH), 1);
        assert_eq!(tabs_per_row(TAB_WIDTH * 3.), 3);
        // 差一点点装不下第四个，就只算三个
        assert_eq!(tabs_per_row(TAB_WIDTH * 4. - 1.), 3);
        assert_eq!(tabs_per_page(TAB_WIDTH * 4., 3), 12);
    }

    #[test]
    fn page_count_never_reports_zero_pages() {
        // 空标签栏不存在，但页数是除法结果，得挡住 0 免得 `pages - 1` 下溢
        assert_eq!(page_count(0, 12), 1);
        assert_eq!(page_count(1, 12), 1);
        assert_eq!(page_count(12, 12), 1);
        assert_eq!(page_count(13, 12), 2);
        assert_eq!(page_count(24, 12), 2);
        assert_eq!(page_count(25, 12), 3);
        // per_page 为 0 时不能除零
        assert_eq!(page_count(5, 0), 5);
    }

    #[test]
    fn rows_toggle_flips_between_one_and_the_maximum() {
        assert_eq!(next_tab_rows(1), MAX_TAB_ROWS);
        assert_eq!(next_tab_rows(MAX_TAB_ROWS), 1);
        // 越界值一律当多行处理，切回单行
        assert_eq!(next_tab_rows(9), 1);
    }

    #[test]
    fn env_label_truncates_only_past_the_limit() {
        assert_eq!(truncate_env_label("dev").as_ref(), "dev");
        let exactly = "a".repeat(ENV_LABEL_MAX_CHARS);
        assert_eq!(truncate_env_label(&exactly).as_ref(), exactly);
        let long = "a".repeat(ENV_LABEL_MAX_CHARS + 5);
        let got = truncate_env_label(&long);
        assert_eq!(
            got.chars().count(),
            ENV_LABEL_MAX_CHARS + 1,
            "截断内容 + 一个省略号"
        );
        assert!(got.ends_with('…'));
    }
}
