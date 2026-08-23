//! 请求面板：Params / Headers / Body 三段。

use getcat_core::http::{DEFAULT_HEADERS, default_header_enabled};
use getcat_core::model::RawFormat;
use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    ActiveTheme, Sizable,
    button::{Button, ButtonVariants},
    checkbox::Checkbox,
    h_flex,
    input::Editor,
    tab::{Tab, TabBar},
    tag::Tag,
    v_flex,
};

use crate::i18n::tr;
use crate::state::request_tab::{BodyMode, RequestSection, RequestTab};
use crate::state::settings;
use crate::ui::format_bytes;
use crate::ui::kv_table::{DEFAULT_COLUMN_FRACTIONS, TABLE_SIZE};

fn section_label(text: SharedString, cx: &App) -> impl IntoElement {
    div()
        .text_xs()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(cx.theme().muted_foreground)
        .child(text)
}

/// 增删全局的默认头禁用清单。存禁用项而非启用项，往 `DEFAULT_HEADERS` 里加条目时
/// 老的 settings.json 不必迁移。
fn toggle_default_header(key_lower: &str, checked: bool, cx: &mut App) {
    settings::update(cx, |s| {
        let list = &mut s.request.disabled_default_headers;
        if checked {
            list.retain(|d| !d.eq_ignore_ascii_case(key_lower));
        } else if !list.iter().any(|d| d.eq_ignore_ascii_case(key_lower)) {
            list.push(key_lower.to_string());
        }
    });
}

impl RequestTab {
    pub fn render_request_pane(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let params_count = self.params.read(cx).count(cx) + self.path_params.read(cx).count(cx);
        let headers_count = self.headers.read(cx).count(cx);
        let section = self.request_section;
        let label = |name: &str, n: usize| -> SharedString {
            if n > 0 {
                format!("{name} ({n})").into()
            } else {
                name.to_string().into()
            }
        };

        v_flex()
            .size_full()
            .min_h_0()
            .child(
                TabBar::new("request-sections")
                    .underline()
                    .small()
                    .px_3()
                    .selected_index(section.index())
                    .on_click(cx.listener(|this, ix: &usize, _, cx| {
                        this.request_section = RequestSection::from_index(*ix);
                        cx.notify();
                    }))
                    .child(Tab::new().label(label("Params", params_count)))
                    .child(Tab::new().label(label("Headers", headers_count)))
                    .child(
                        Tab::new()
                            .label(label("Body", usize::from(self.body_mode != BodyMode::None))),
                    ),
            )
            .child(
                div()
                    .id("request-section")
                    .flex_1()
                    .min_h_0()
                    .px_3()
                    .py_3()
                    .when(section != RequestSection::Body, |d| d.overflow_y_scroll())
                    .child(match section {
                        RequestSection::Params => v_flex()
                            .gap_3()
                            .when(self.has_path_params(cx), |v| {
                                v.child(section_label(tr!("request.path_params"), cx))
                                    .child(self.path_params.clone())
                            })
                            .child(section_label(tr!("request.query_params"), cx))
                            .child(self.params.clone())
                            .into_any_element(),
                        RequestSection::Headers => v_flex()
                            .gap_3()
                            .child(self.render_default_headers(cx))
                            .child(self.headers.clone())
                            .into_any_element(),
                        RequestSection::Body => self.render_body_section(cx),
                    }),
            )
    }

    /// 「默认参数（Header）」：每次请求都自动带上的那几个头。
    ///
    /// 这块存在的意义是把原本隐形的客户端行为摆到台面上——在此之前 User-Agent 由
    /// client 层写死、Accept-Encoding 由解压中间件在背后补，界面上完全看不出请求
    /// 究竟发了什么，连 Accept 压根没发都不知道。
    ///
    /// 值固定不可编辑，只能整条开关；开关是**全局**的（落在 settings.json 的 request
    /// 段），所以标题旁挂了「全局」标签，避免被当成本请求的参数。
    fn render_default_headers(&self, cx: &mut Context<Self>) -> AnyElement {
        let disabled = settings::settings(cx).request.disabled_default_headers;
        // 本请求自己填了同名 header 时，默认那条不会生效——reqwest 是以 vacant-entry
        // 语义合并 client 级默认头的。界面上得说清楚，否则「我明明把 Accept 改成
        // application/json 了，上面怎么还写 */*」。
        let table = self.headers.read(cx);
        let overridden: Vec<String> = table
            .values(cx)
            .into_iter()
            .filter(|kv| kv.enabled && !kv.key.trim().is_empty())
            .map(|kv| kv.key.trim().to_ascii_lowercase())
            .collect();

        let muted = cx.theme().muted_foreground;
        let row_border = cx.theme().table_row_border;
        let last = DEFAULT_HEADERS.len().saturating_sub(1);

        v_flex()
            .gap_2()
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(section_label(tr!("request.default_headers"), cx))
                    .child(
                        Tag::secondary()
                            .xsmall()
                            .child(tr!("request.default_headers_global")),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(tr!("request.default_headers_desc")),
            )
            .child(
                // 外框与 KvTable 保持同一套圆角 / 边框 / 底色，读起来是同一类东西
                v_flex()
                    .w_full()
                    .rounded(cx.theme().radius)
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().table)
                    .overflow_hidden()
                    .children(
                        DEFAULT_HEADERS
                            .iter()
                            .enumerate()
                            .map(|(ix, (key, value))| {
                                let key_lower = key.to_ascii_lowercase();
                                let enabled = default_header_enabled(&disabled, key);
                                let is_overridden = enabled && overridden.contains(&key_lower);
                                // 关掉的和被覆盖的都发不出去，一律淡化；被覆盖的额外加删除线
                                let inactive = !enabled || is_overridden;

                                h_flex()
                                    .w_full()
                                    .h(TABLE_SIZE.table_row_height())
                                    .flex_none()
                                    .when(ix < last, |d| d.border_b_1().border_color(row_border))
                                    .text_sm()
                                    .child(
                                        div()
                                            .w_8()
                                            .flex_none()
                                            .h_full()
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .child(
                                                Checkbox::new(("default-header", ix))
                                                    .checked(enabled)
                                                    .tooltip(tr!("kv.enabled"))
                                                    .on_click(cx.listener(
                                                        move |_, checked: &bool, _, cx| {
                                                            toggle_default_header(
                                                                &key_lower, *checked, cx,
                                                            );
                                                            cx.notify();
                                                        },
                                                    )),
                                            ),
                                    )
                                    // 中间这段的结构照抄 KvTable：两侧各让出 w_8，列宽比例算在
                                    // 剩下的这块上。不这么套的话两张表的列边界会差出 64 px。
                                    .child(
                                        h_flex()
                                            .flex_1()
                                            .min_w_0()
                                            .h_full()
                                            .child(
                                                div()
                                                    .flex_none()
                                                    .flex_basis(relative(
                                                        DEFAULT_COLUMN_FRACTIONS[0],
                                                    ))
                                                    .min_w_0()
                                                    .h_full()
                                                    .flex()
                                                    .items_center()
                                                    .border_r_1()
                                                    .border_color(row_border)
                                                    .px_2()
                                                    .child(
                                                        div()
                                                            .truncate()
                                                            .when(inactive, |d| d.text_color(muted))
                                                            .when(is_overridden, |d| {
                                                                d.line_through()
                                                            })
                                                            .child(*key),
                                                    ),
                                            )
                                            .child(
                                                h_flex()
                                                    .flex_1()
                                                    .min_w_0()
                                                    .h_full()
                                                    .gap_2()
                                                    .px_2()
                                                    .text_color(muted)
                                                    .child(
                                                        div()
                                                            .min_w_0()
                                                            .truncate()
                                                            .when(is_overridden, |d| {
                                                                d.line_through()
                                                            })
                                                            .child(*value),
                                                    )
                                                    .when(is_overridden, |h| {
                                                        h.child(div().flex_none().text_xs().child(
                                                            tr!(
                                                                "request.default_headers_overridden"
                                                            ),
                                                        ))
                                                    }),
                                            ),
                                    )
                                    // 与 KvTable 右侧那一列删除按钮对齐，两张表的列不会错位
                                    .child(div().w_8().flex_none())
                            }),
                    ),
            )
            .into_any_element()
    }

    pub fn render_body_section(&self, cx: &mut Context<Self>) -> AnyElement {
        let mode = self.body_mode;
        let current_format = self.raw_format;
        v_flex()
            .size_full()
            .gap_3()
            // 两行工具条：Body 类型一行，raw 的格式与格式化按钮另起一行。
            // 挤在一行时窄面板下会把分段控件裁掉，拆开后各自都有完整宽度。
            .child(
                v_flex()
                    .gap_2()
                    // 包一层 h_flex 让分段控件按内容收缩：TabBar 自身不带宽度约束，
                    // 直接挂在 v_flex（无 items_*）下会被交叉轴 stretch 撑满整行，
                    // segmented 的背景条一路铺到右边缘。h_flex 自带 items_center，
                    // 与下面 raw 格式那行结构一致。
                    .child(
                        h_flex().child(
                            TabBar::new("body-mode")
                                .segmented()
                                .small()
                                .selected_index(mode.index())
                                .on_click(cx.listener(|this, ix: &usize, _, cx| {
                                    this.body_mode = BodyMode::from_index(*ix);
                                    this.refresh_body_hint(cx);
                                    this.mark_dirty(cx);
                                }))
                                .child("none")
                                .child("form-data")
                                // 标签缩短；发出的 Content-Type 仍是 application/x-www-form-urlencoded
                                .child("urlencoded")
                                .child("raw")
                                .child("binary"),
                        ),
                    )
                    .when(mode == BodyMode::Raw, |v| {
                        v.child(
                            h_flex()
                                .items_center()
                                .gap_2()
                                .child(
                                    TabBar::new("raw-format")
                                        .segmented()
                                        .small()
                                        .selected_index(current_format.index())
                                        .on_click(cx.listener(|this, ix: &usize, _, cx| {
                                            this.raw_format = RawFormat::from_index(*ix);
                                            this.refresh_body_hint(cx);
                                            this.mark_dirty(cx);
                                        }))
                                        .children(RawFormat::ALL.map(|f| f.label())),
                                )
                                // 只有 JSON 能格式化：core 里只有 JSON 美化器，
                                // 与其给个点不动的灰按钮，不如在别的格式下不显示
                                .when(current_format == RawFormat::Json, |h| {
                                    h.child(
                                        Button::new("format-json")
                                            .ghost()
                                            .small()
                                            .label(tr!("request.format_json"))
                                            .tooltip(tr!("request.format_json_tooltip"))
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.format_body(window, cx)
                                            })),
                                    )
                                }),
                        )
                    }),
            )
            .child(match mode {
                BodyMode::None => div()
                    .py_2()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(tr!("request.no_body"))
                    .into_any_element(),
                BodyMode::Raw => v_flex()
                    .flex_1()
                    .min_h_0()
                    .gap_1()
                    .child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .rounded(cx.theme().radius)
                            .border_1()
                            .border_color(cx.theme().border)
                            .overflow_hidden()
                            .child(
                                Editor::new(self.editor_for(self.raw_format))
                                    .aria_label(tr!("request.body_editor_aria"))
                                    .font_family(cx.theme().mono_font_family.clone())
                                    .text_size(cx.theme().mono_font_size)
                                    .size_full(),
                            ),
                    )
                    .when_some(self.body_hint, |v, hint| {
                        v.child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().warning)
                                .child(hint.text()),
                        )
                    })
                    .into_any_element(),
                BodyMode::FormData => v_flex()
                    .flex_1()
                    .min_h_0()
                    .gap_1()
                    .child(
                        div()
                            .id("form-data-body")
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .child(self.form_data.clone()),
                    )
                    .when_some(self.body_hint, |v, hint| {
                        v.child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().warning)
                                .child(hint.text()),
                        )
                    })
                    .into_any_element(),
                BodyMode::FormUrlEncoded => div()
                    .id("form-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(self.form.clone())
                    .into_any_element(),
                BodyMode::Binary => self.render_file_body(cx),
            })
            .into_any_element()
    }

    fn render_file_body(&self, cx: &mut Context<Self>) -> AnyElement {
        let muted = cx.theme().muted_foreground;
        v_flex()
            .gap_2()
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        Button::new("choose-file")
                            .outline()
                            .small()
                            .label(tr!("common.choose_file"))
                            .on_click(
                                cx.listener(|this, _, window, cx| this.choose_file(window, cx)),
                            ),
                    )
                    .when(self.file_path.is_some(), |h| {
                        h.child(
                            Button::new("clear-file")
                                .ghost()
                                .small()
                                .label(tr!("common.clear"))
                                .on_click(cx.listener(|this, _, _, cx| this.clear_file(cx))),
                        )
                    }),
            )
            .child(match &self.file_path {
                Some(path) => h_flex()
                    .gap_3()
                    .text_sm()
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .font_family(cx.theme().mono_font_family.clone())
                            .child(path.display().to_string()),
                    )
                    .when_some(self.file_size, |h, size| {
                        h.child(
                            div()
                                .flex_none()
                                .text_color(muted)
                                .child(format_bytes(size)),
                        )
                    })
                    .into_any_element(),
                None => div()
                    .text_sm()
                    .text_color(muted)
                    .child(tr!("request.no_file"))
                    .into_any_element(),
            })
            .into_any_element()
    }
}
