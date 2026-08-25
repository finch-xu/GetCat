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

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, ClickEvent, Context, FontWeight, InteractiveElement, IntoElement, MouseButton,
    ParentElement, Role, SharedString, StatefulInteractiveElement, Styled, WeakEntity, div, px,
};
use gpui_component::{
    ActiveTheme, Disableable, ElementExt as _, Icon, IconName, Selectable, Sizable, ThemeStyled,
    button::{Button, ButtonVariants},
    h_flex,
    menu::{ContextMenuExt as _, PopupMenu, PopupMenuItem},
    v_flex,
};

use getcat_core::model::{MAX_TAB_ROWS, Method};

use crate::NewTab;
use crate::assets::ICON_ROWS_3;
use crate::i18n::tr;
use crate::state::workspace::Workspace;
use crate::ui::method_color;

/// 标签宽度。单行模式下是上限（label 自己省略号截断），多行模式下是定值——
/// 分页要靠它算每行装几个。
///
/// 比 spec 的 180 宽 20 px：prefix 多了 40 px 的 method 角标，不放宽的话 label
/// 只剩不到半个词的位置。
pub const TAB_WIDTH: f32 = 200.;

/// 标签高度，与 gpui-component `TabVariant::Tab` 的默认尺寸对齐。
const TAB_HEIGHT: f32 = 30.;

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
    /// 标签栏。行数由 [`Workspace::tab_rows`] 决定：1 走横向滚动，>1 走分页。
    pub fn render_tab_strip(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = self.tab_rows();
        let width = self.strip_width_cell();
        let multi_row = rows > 1;
        let tabs: Vec<TabMeta> = self
            .tabs_meta(cx)
            .into_iter()
            .enumerate()
            .map(|(ix, (method, title, dirty))| TabMeta {
                ix,
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
                            .flex_1()
                            .min_w_0()
                            .child(body)
                            // 分页要知道「一行装得下几个」，而这只有布局算完才知道。
                            // 写进 Cell 而不是 `&mut self`：prepaint 里改状态会让每帧
                            // 的测量都触发一次重绘。
                            .on_prepaint(move |bounds, _, _| width.set(bounds.size.width.into())),
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
        let ix = tab.ix;
        let selected = tab.selected;
        let aria = if tab.dirty {
            tr!("tab.aria_dirty", title = tab.title.clone())
        } else {
            tr!("tab.aria", title = tab.title.clone())
        };
        let active_bg = *cx.theme().tokens.tab_active;
        // 菜单回调只拿得到 `&mut App`，握弱引用而不是 `self`（照 `ui::sidebar` 的写法）
        let owner = cx.entity().downgrade();

        div()
            .id(("tab", ix))
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
                Button::new(("close-tab", ix))
                    .ghost()
                    .xsmall()
                    .icon(IconName::Close)
                    .tooltip(tr!("tab.close"))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        // 阻止冒泡到标签自身的 on_click，否则关闭后会再触发 activate(ix)
                        cx.stop_propagation();
                        this.close_tab(ix, window, cx)
                    })),
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
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.activate(ix, cx)))
            // 右键先选中再弹菜单：菜单项说的都是「当前 Tab」，选中态必须先对上
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, _, _, cx| this.activate(ix, cx)),
            )
            .context_menu(move |menu, _, _| tab_menu(menu, ix, owner.clone()))
            .into_any_element()
    }

    /// 标签栏右侧的一组控件：翻页 / 滚动箭头，以及行数切换。
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

        h_flex()
            .flex_none()
            .items_center()
            .gap_0p5()
            .px_1()
            .py_1()
            .when(show_arrows, |h| {
                h.child(
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
}

/// 标签的右键菜单。对象级命令：与行内的关闭按钮是同一组动作，
/// 键盘与辅助技术用户多一条可达路径（设计指南）。
///
/// 「关闭其他 / 关闭所有」不弹二次确认——草稿本来就随时落盘，关闭即删草稿是
/// [`Workspace::close_tab`] 早就定下的语义，这里跟着走。
fn tab_menu(menu: PopupMenu, ix: usize, owner: WeakEntity<Workspace>) -> PopupMenu {
    let duplicate = owner.clone();
    let others = owner.clone();
    menu.item(
        PopupMenuItem::new(tr!("tab.menu_duplicate")).on_click(move |_, window, cx| {
            if let Some(ws) = duplicate.upgrade() {
                ws.update(cx, |ws, cx| ws.duplicate_tab(ix, window, cx));
            }
        }),
    )
    .separator()
    .item(
        PopupMenuItem::new(tr!("tab.menu_close_others")).on_click(move |_, _, cx| {
            if let Some(ws) = others.upgrade() {
                ws.update(cx, |ws, cx| ws.close_other_tabs(ix, cx));
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
    ix: usize,
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
}
