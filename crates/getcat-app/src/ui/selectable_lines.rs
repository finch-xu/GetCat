//! B/C 档行视图的文本选择：接入 gpui-base 的窗口级文本选择引擎
//! （官方 Text Selection 文档里的"自定义参与者"路径，`TextSelectionLayer` 已由 `Root` 挂在窗口根，
//! ⌘C 也由 `Root` 统一处理：它把 `TextSelection::selected_text` 写进剪贴板）。
//!
//! 整份响应体是**一个**参与者（[`LinesSelection`] 持有稳定的 `TextSelectionHandle`）。每帧：
//! [`participant_layer`] 在 prepaint 上报视口几何、滚动偏移与命中框；可见的每一行由 [`LineText`]
//! 在 prepaint 上报排版后的文本 run，第一行 paint 时把整帧的 run 一次交给引擎投影成每行的
//! 字节区间，各行在文字底下画选中底色。
//!
//! uniform_list 只渲染可见行，所以复制文本不能只靠 run：行号来自快照里的内容坐标（内容键 = 行号，
//! 不随滚动变化），两个端点所在行的列则在它们可见时从引擎的投影结果里记下来（拖选开始时锚点行
//! 一定可见，拖动中光标行一定可见），之后即使滚出视口也还在；`copy_with` 回调据此在 `TextDoc`
//! 上切片，中间的离屏行自然包含在内。

use std::cell::{Cell, RefCell};
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use getcat_core::body::text::{LinePos, TextDoc};
use gpui_kit::base::{
    TextSelection, TextSelectionContentKey, TextSelectionCoverage, TextSelectionEndpoint,
    TextSelectionEvent, TextSelectionHandle, TextSelectionRegistration, TextSelectionRun,
    TextSelectionSnapshot,
};
use gpui_kit::{
    App, BorderStyle, Bounds, Corners, Edges, Element, ElementId, EntityId, FocusHandle,
    GlobalElementId, Hsla, InspectorElementId, IntoElement, LayoutId, PaintQuad, Pixels, Point,
    SharedString, Styled, StyledText, Subscription, UniformListScrollHandle, Window, canvas, point,
    transparent_black,
};

use crate::ui::body_view::LINE_HEIGHT_PX;

/// 一份响应体作为选择参与者的稳定状态；随 `RequestTab` 存活。
pub struct LinesSelection {
    handle: TextSelectionHandle,
    /// 选区变化时重绘本窗口；快照也在这里记下来供复制用
    _subscriptions: Vec<Subscription>,
    /// 拖选开始时把焦点交给行视图，⌘A 才会落到它身上
    pub focus: FocusHandle,
    shared: Rc<Shared>,
}

#[derive(Default)]
struct Shared {
    doc: RefCell<Option<Arc<TextDoc>>>,
    /// 引擎最近一次发布的快照（端点是内容坐标，不随滚动变化）
    snapshot: Cell<Option<TextSelectionSnapshot>>,
    /// ⌘A 全选：引擎侧用 `set_local_selection` 标记，这里记一份给复制与绘制用
    select_all: Cell<bool>,
    /// 参与者视口在窗口里的位置（每帧刷新；复制时把窗口 x 换算成行内 x）
    bounds: Cell<Option<Bounds<Pixels>>>,
    /// 两个端点所在行的列，在端点行可见时由投影结果更新；`(行号, 列)`
    anchor_col: Cell<Option<(usize, usize)>>,
    cursor_col: Cell<Option<(usize, usize)>>,
    /// 本帧可见行上报的 run 与投影结果
    frame: RefCell<Frame>,
}

/// 一帧内可见行的 run，按 prepaint 顺序收集；`ranges` 在第一行 paint 时一次算出。
#[derive(Default)]
struct Frame {
    lines: Vec<usize>,
    runs: Vec<TextSelectionRun>,
    ranges: Option<Vec<Option<Range<usize>>>>,
}

impl Shared {
    fn line_count(&self) -> usize {
        self.doc.borrow().as_ref().map_or(0, |d| d.line_count())
    }

    /// 内容 y → 行号（夹在有效范围内）；空文档为 None。
    fn line_at(&self, content: Point<Pixels>) -> Option<usize> {
        let count = self.line_count();
        if count == 0 {
            return None;
        }
        let ix = (f32::from(content.y) / LINE_HEIGHT_PX).floor().max(0.) as usize;
        Some(ix.min(count - 1))
    }

    /// 端点所在行：优先用引擎在手势时记下的内容键，没有就按内容 y 算。
    fn line_of(&self, doc: &TextDoc, endpoint: &TextSelectionEndpoint) -> usize {
        endpoint
            .content_key()
            .map(|key| key.value() as usize)
            .or_else(|| self.line_at(endpoint.content_point()))
            .unwrap_or(0)
            .min(doc.line_count().saturating_sub(1))
    }

    /// 锚点是否在光标之前（阅读顺序）：先比行，同一行比窗口 x。
    fn anchor_is_start(&self, doc: &TextDoc, snapshot: &TextSelectionSnapshot) -> bool {
        let (a, c) = (
            self.line_of(doc, &snapshot.anchor()),
            self.line_of(doc, &snapshot.cursor()),
        );
        if a != c {
            return a < c;
        }
        snapshot
            .window_points()
            .is_none_or(|p| p.anchor().x <= p.cursor().x)
    }

    /// 缓存的端点列；没缓存到（端点行从未在选区存在期间可见）就退到该行的行首 / 行尾。
    fn col_or(&self, cached: Option<(usize, usize)>, line: usize, is_start: bool) -> usize {
        match cached {
            Some((l, col)) if l == line => col,
            _ if is_start => 0,
            _ => usize::MAX,
        }
    }

    /// 投影出本帧各行的选中区间后，把端点行的列记下来。
    fn remember_endpoint_cols(&self, doc: &TextDoc, frame: &Frame) {
        let Some(snapshot) = self.snapshot.get() else {
            return;
        };
        let Some(ranges) = frame.ranges.as_ref() else {
            return;
        };
        let anchor_line = self.line_of(doc, &snapshot.anchor());
        let cursor_line = self.line_of(doc, &snapshot.cursor());
        let anchor_first = self.anchor_is_start(doc, &snapshot);
        for (line, range) in frame.lines.iter().zip(ranges) {
            let Some(range) = range else {
                continue;
            };
            if *line == anchor_line {
                let col = if anchor_first { range.start } else { range.end };
                self.anchor_col.set(Some((*line, col)));
            }
            if *line == cursor_line {
                let col = if anchor_first { range.end } else { range.start };
                self.cursor_col.set(Some((*line, col)));
            }
        }
    }

    /// 复制文本：全选给全文；否则按快照的覆盖类型在文档上定位两端。
    fn copy_text(&self, me: EntityId) -> String {
        let Some(doc) = self.doc.borrow().clone() else {
            return String::new();
        };
        if self.select_all.get() {
            return doc.text().to_string();
        }
        let Some(snapshot) = self.snapshot.get() else {
            return String::new();
        };
        let (anchor, cursor) = (snapshot.anchor(), snapshot.cursor());
        let anchor_first = self.anchor_is_start(&doc, &snapshot);
        let anchor_pos = |is_start: bool| {
            let line = self.line_of(&doc, &anchor);
            LinePos {
                line,
                col: self.col_or(self.anchor_col.get(), line, is_start),
            }
        };
        let cursor_pos = |is_start: bool| {
            let line = self.line_of(&doc, &cursor);
            LinePos {
                line,
                col: self.col_or(self.cursor_col.get(), line, is_start),
            }
        };
        // 只有一端落在本参与者里时取那一端；`is_start` 说明它是选区的起点还是终点
        let own_pos = |is_start: bool| {
            if anchor.entity_id() == Some(me) {
                anchor_pos(is_start)
            } else {
                cursor_pos(is_start)
            }
        };
        let start = LinePos { line: 0, col: 0 };
        let end = LinePos {
            line: usize::MAX,
            col: usize::MAX,
        };
        match snapshot.coverage() {
            TextSelectionCoverage::Full => doc.text().to_string(),
            TextSelectionCoverage::FromStart => doc.slice(start, own_pos(false)).to_string(),
            TextSelectionCoverage::ToEnd => doc.slice(own_pos(true), end).to_string(),
            TextSelectionCoverage::Bounded => doc
                .slice(anchor_pos(anchor_first), cursor_pos(!anchor_first))
                .to_string(),
        }
    }
}

impl LinesSelection {
    pub fn new(window: &Window, cx: &mut App) -> Self {
        let handle = TextSelectionHandle::new("", cx);
        let shared = Rc::new(Shared::default());
        let focus = cx.focus_handle();
        let me = handle.entity_id();

        // 回调都存在引擎的实体里，只能捕获 Rc<Shared> 与 FocusHandle，不能捕获 handle 本身（会自引用不释放）
        let s = shared.clone();
        handle.copy_with(move |_| s.copy_text(me), cx);
        let s = shared.clone();
        handle.resolve_content_key_with(
            move |content, _| {
                s.line_at(content)
                    .map(|ix| TextSelectionContentKey::new(ix as u64))
            },
            cx,
        );
        let s = shared.clone();
        handle.clear_with(
            move |_| {
                s.select_all.set(false);
                s.anchor_col.set(None);
                s.cursor_col.set(None);
            },
            cx,
        );
        let f = focus.clone();
        handle.focus_with(move |window, cx| window.focus(&f, cx), cx);

        let refresh = handle.refresh_window_on_change(window, cx);
        let s = shared.clone();
        let track = handle.subscribe(
            move |event, _| {
                if let TextSelectionEvent::SelectionChanged(snapshot) = event {
                    s.snapshot.set(*snapshot);
                }
            },
            cx,
        );
        Self {
            handle,
            _subscriptions: vec![refresh, track],
            focus,
            shared,
        }
    }

    /// 当前显示的文档；换文档时清掉窗口选区（旧选区的坐标对新文本没有意义）。
    pub fn set_doc(&self, doc: Option<Arc<TextDoc>>, window: &mut Window, cx: &mut App) {
        let changed = {
            let current = self.shared.doc.borrow();
            match (current.as_ref(), doc.as_ref()) {
                (None, None) => false,
                (Some(a), Some(b)) => !Arc::ptr_eq(a, b),
                _ => true,
            }
        };
        if !changed {
            return;
        }
        *self.shared.doc.borrow_mut() = doc;
        self.shared.snapshot.set(None);
        self.shared.select_all.set(false);
        self.shared.anchor_col.set(None);
        self.shared.cursor_col.set(None);
        TextSelection::clear(window, cx);
    }

    /// ⌘A：整份文档进入选中态（引擎用它决定 ⌘C 时要不要问本参与者要文本）。
    pub fn select_all(&self, window: &mut Window, cx: &mut App) {
        if self.shared.doc.borrow().is_none() {
            return;
        }
        TextSelection::clear(window, cx);
        self.shared.select_all.set(true);
        self.handle.set_local_selection(true, cx);
        window.refresh();
    }

    /// 参与者视口最近一帧的位置（测试用来把行号换算成鼠标坐标）。
    #[cfg(test)]
    pub fn bounds(&self) -> Option<Bounds<Pixels>> {
        self.shared.bounds.get()
    }

    /// 给 uniform_list 的行闭包用的轻量副本（闭包是 `'static` 的，拿不到 `&LinesSelection`）。
    pub fn clone_for_rows(&self) -> RowsSelection {
        RowsSelection {
            handle: self.handle.clone(),
            shared: self.shared.clone(),
        }
    }
}

/// [`LinesSelection`] 里行元素需要的那一部分。
#[derive(Clone)]
pub struct RowsSelection {
    handle: TextSelectionHandle,
    shared: Rc<Shared>,
}

impl RowsSelection {
    /// 一行文字：上报 run、画选中底色、再画文字。`text` 必须是行里实际显示的（已截断的）文本。
    pub fn line_text(&self, line: usize, text: SharedString, selection_color: Hsla) -> LineText {
        LineText {
            line,
            styled: StyledText::new(text.clone()),
            text,
            handle: self.handle.clone(),
            shared: self.shared.clone(),
            selection_color,
        }
    }
}

/// 覆盖整个行视图的零绘制层：每帧上报参与者几何。放在列表**之前**作为第一个子元素，
/// 它的 prepaint 先于所有行执行，顺便把上一帧收集的 run 清掉。
pub fn participant_layer(
    selection: &LinesSelection,
    scroll: &UniformListScrollHandle,
) -> impl IntoElement {
    let handle = selection.handle.clone();
    let shared = selection.shared.clone();
    let scroll = scroll.clone();
    canvas(
        move |bounds, window, cx| {
            *shared.frame.borrow_mut() = Frame::default();
            shared.bounds.set(Some(bounds));
            let hitbox = window.insert_hitbox(bounds, gpui_kit::HitboxBehavior::Normal);
            // ScrollHandle 的 offset 向上滚动为负；引擎按 `窗口点 - 视口原点 - offset` 得到内容坐标
            let offset = scroll.0.borrow().base_handle.offset();
            handle.register(
                TextSelectionRegistration::new(hitbox, bounds)
                    .with_scroll_offset(offset)
                    .with_text_bounds(vec![bounds]),
                window,
                cx,
            );
        },
        |_, _, _, _| {},
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

pub struct LineText {
    line: usize,
    text: SharedString,
    styled: StyledText,
    handle: TextSelectionHandle,
    shared: Rc<Shared>,
    selection_color: Hsla,
}

impl LineText {
    /// 本行的选中区间：本帧第一次调用时把所有可见行的 run 交给引擎投影，之后各行直接查表。
    fn selected_range(&self, cx: &mut App) -> Option<Range<usize>> {
        let mut frame = self.shared.frame.borrow_mut();
        if frame.ranges.is_none() {
            let ranges = self.handle.update_runs(&frame.runs, cx).ranges().to_vec();
            frame.ranges = Some(ranges);
            if let Some(doc) = self.shared.doc.borrow().as_ref() {
                self.shared.remember_endpoint_cols(doc, &frame);
            }
        }
        let ix = frame.lines.iter().position(|l| *l == self.line)?;
        frame.ranges.as_ref()?.get(ix)?.clone()
    }

    fn paint_quad(bounds: Bounds<Pixels>, color: Hsla, window: &mut Window) {
        window.paint_quad(PaintQuad {
            bounds,
            background: color.into(),
            corner_radii: Corners::default(),
            border_widths: Edges::default(),
            border_color: transparent_black(),
            border_style: BorderStyle::default(),
        });
    }
}

impl IntoElement for LineText {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for LineText {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        self.styled
            .request_layout(global_id, inspector_id, window, cx)
    }

    fn prepaint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        self.styled
            .prepaint(global_id, inspector_id, bounds, &mut (), window, cx);
        let mut frame = self.shared.frame.borrow_mut();
        frame.lines.push(self.line);
        frame.runs.push(
            TextSelectionRun::new(self.text.clone(), self.styled.layout().clone(), bounds)
                .with_document_order(self.line as u64),
        );
    }

    fn paint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        _: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let layout = self.styled.layout().clone();
        if self.shared.select_all.get() {
            Self::paint_quad(bounds, self.selection_color, window);
        } else if let Some(range) = self.selected_range(cx)
            && let (Some(start), Some(end)) = (
                layout.position_for_index(range.start),
                layout.position_for_index(range.end),
            )
        {
            // 行视图不换行，选中区永远是同一行上的一段
            Self::paint_quad(
                Bounds::from_corners(start, point(end.x, start.y + layout.line_height())),
                self.selection_color,
                window,
            );
        }
        self.styled.paint(
            global_id,
            inspector_id,
            bounds,
            &mut (),
            &mut (),
            window,
            cx,
        );
    }
}
