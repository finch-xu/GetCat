//! 响应面板：状态行、Pretty/Raw 与 Body/Headers 切换、按档位分派的 Body 视图、虚拟化 Headers 列表。

use getcat_core::body::tier::ViewTier;
use getcat_core::http::{BodyStore, RequestError};
use getcat_core::model::HttpVersionPref;
use getcat_core::tls::CertificateInfo;
use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, IconName, Selectable, Sizable,
    alert::Alert,
    button::{Button, ButtonVariants},
    clipboard::Clipboard,
    description_list::DescriptionList,
    h_flex,
    input::{Editor, SelectAll},
    kbd::Kbd,
    tab::{Tab, TabBar},
    tag::Tag,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
// 显式导入而非 `use gpui_kit::*`：本文件含 `#[cfg(test)] mod tests`，通配符会引入 gpui 重导出的
// `#[test]` 属性宏并与标准库同名冲突。编译器报"找不到 X"时把 X 加进这里，不要改回通配符。
use gpui_kit::{
    AnyElement, App, Context, Div, FontWeight, InteractiveElement, IntoElement, ParentElement,
    Role, SharedString, StatefulInteractiveElement, Styled, Window, div, rems,
};

use crate::assets::ICON_WRAP_TEXT;
use crate::i18n::tr;
use crate::state::request_tab::{RequestTab, ResponseSection, SseBodyMode};
use crate::state::response::{OpsReport, ResponseState, ResponseView, SseLive, SseView};
use crate::state::settings;
use crate::state::variables;
use crate::ui::body_view::{render_header_rows, render_sse_events, render_text_lines};
use crate::ui::ops_table::{PostRowKind, row_texts, scope_label};
use crate::ui::text::{
    cert_warning_label, content_kind_label, error_detail, error_kind, op_detail, op_outcome_label,
    tier_notice,
};
use crate::ui::{format_bytes, format_duration, status_color};
use crate::{FindInResponse, SendRequest};
use getcat_core::model::{PreOpKind, VarScope, VariableSets};
use getcat_core::ops::OpOutcome;

fn empty_state(text: impl Into<SharedString>, cx: &App) -> AnyElement {
    empty_state_frame(cx).child(text.into()).into_any_element()
}

fn empty_state_frame(cx: &App) -> Div {
    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .text_sm()
        .text_color(cx.theme().muted_foreground)
}

/// 「证书」页签：体检结论在前，证书原文字段在后。
///
/// 字段值一律原样展示（主体、颁发者、SAN 都是证书里的原文），只有体检结论
/// 是我们下的判断、需要翻译——与 `ui::text` 的规则一致。
fn render_certificate(info: &CertificateInfo) -> AnyElement {
    let san = if info.san.is_empty() {
        tr!("cert.san_empty").to_string()
    } else {
        info.san.join("\n")
    };
    let list = DescriptionList::new()
        .columns(1)
        .bordered(false)
        .small()
        .label_width(rems(7.))
        .item(tr!("cert.subject"), info.subject.clone(), 1)
        .item(tr!("cert.issuer"), info.issuer.clone(), 1)
        .item(tr!("cert.not_before"), info.not_before.clone(), 1)
        .item(tr!("cert.not_after"), info.not_after.clone(), 1)
        .item(tr!("cert.san"), san, 1)
        .item(tr!("cert.serial"), info.serial.clone(), 1)
        .item(
            tr!("cert.signature_algorithm"),
            info.signature_algorithm.clone(),
            1,
        )
        .item(tr!("cert.fingerprint"), info.sha256_fingerprint.clone(), 1);

    v_flex()
        .id("certificate-section")
        .size_full()
        .min_h_0()
        .overflow_y_scroll()
        .p_3()
        .gap_3()
        .children(
            info.warnings.iter().enumerate().map(|(ix, w)| {
                Alert::warning(("cert-warning", ix), cert_warning_label(*w)).xsmall()
            }),
        )
        .child(list)
        .into_any_element()
}

/// 档位提示：官方 `Alert` 的 banner 形态，底色 / 边框 / 图标全部来自主题。
fn notice_bar(text: impl Into<SharedString>) -> AnyElement {
    Alert::warning("tier-notice", text.into())
        .banner()
        .xsmall()
        .into_any_element()
}

/// 全局 / 每个环境 / 每个分类里所有非空的 secret 变量值，去重后按字节长度**降序**排列。
///
/// 长度必须降序：如果 secret A = "abc"、secret B = "abcXYZ" 同时存在，先换 A 会把 B 切掉
/// 一角——"abcXYZ" 变成 "••••••XYZ"，"XYZ" 这部分本属于 B、却没被掩码，明文露了出来。
/// 从最长的开始替换就不会有这个问题：位置上如果两个 secret 都能匹配，长的一定先被处理。
fn secret_values(sets: &VariableSets) -> Vec<String> {
    // 用 BTreeSet 去重（`dedup()` 只去掉相邻重复，排序后长度相同的重复值不一定相邻）
    let unique: std::collections::BTreeSet<String> = sets
        .globals
        .iter()
        .chain(sets.environments.iter().flat_map(|e| e.variables.iter()))
        .chain(sets.groups.values().flat_map(|vars| vars.iter()))
        .filter(|v| v.secret && !v.value.is_empty())
        .map(|v| v.value.clone())
        .collect();
    let mut values: Vec<String> = unique.into_iter().collect();
    values.sort_by_key(|v| std::cmp::Reverse(v.len()));
    values
}

/// 把 `secrets`（须已按 [`secret_values`] 的长度降序排列）里在 `text` 中出现的子串
/// 依次换成掩码。断言失败详情里的载荷（如 `Mismatch` 的 actual/expected）可能夹带
/// 发送时用 `{{secret_var}}` 替换出来的明文，渲染前统一在这里掩掉。
fn mask_with(text: &str, secrets: &[String]) -> String {
    let mut masked = text.to_string();
    for value in secrets {
        if masked.contains(value.as_str()) {
            masked = masked.replace(value.as_str(), "••••••");
        }
    }
    masked
}

/// 提取到的变量当前是否标了 secret（决定「操作」页签里这条提取值要不要掩码显示）。
fn is_extracted_var_secret(
    sets: &VariableSets,
    scope: VarScope,
    group: Option<&str>,
    key: &str,
) -> bool {
    let vars = match scope {
        VarScope::Global => sets.globals.as_slice(),
        VarScope::Environment => sets
            .active_env()
            .map(|e| e.variables.as_slice())
            .unwrap_or(&[]),
        VarScope::Group => sets.group_vars(group),
    };
    vars.iter().any(|v| v.key == key && v.secret)
}

/// 「操作」页签：前置在前、后置在后，每条一行：状态色圆点 + 操作描述 + 原因 / 提取值。
fn render_ops_report(report: &OpsReport, group: Option<&str>, cx: &App) -> AnyElement {
    let sets = variables::variables(cx);
    // 每次渲染只算一遍、排一次序，逐行复用——secret 列表与行数无关
    let secrets = secret_values(sets);
    let line =
        |ix: usize, title: SharedString, outcome: &OpOutcome, extra: Option<SharedString>| {
            let color = match outcome {
                OpOutcome::Passed => cx.theme().success,
                OpOutcome::Failed(_) => cx.theme().danger,
                OpOutcome::Skipped(_) => cx.theme().warning,
            };
            let detail =
                op_detail(outcome).map(|text| SharedString::from(mask_with(&text, &secrets)));
            v_flex()
                .id(("op-result", ix))
                .py_1()
                .gap_0p5()
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(div().w_2().h_2().rounded_full().bg(color).flex_none())
                        .child(div().text_sm().child(title))
                        .child(
                            div()
                                .text_xs()
                                .text_color(color)
                                .child(op_outcome_label(outcome)),
                        ),
                )
                .when_some(detail.or(extra), |v, text| {
                    v.child(
                        div()
                            .pl_4()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(text),
                    )
                })
        };
    let mut rows: Vec<AnyElement> = Vec::new();
    for (ix, (op, outcome)) in report.pre.iter().enumerate() {
        let PreOpKind::SetVariable { scope, key, .. } = &op.kind;
        let title = tr!(
            "ops.set_line",
            scope = scope_label(*scope),
            key = key.clone()
        );
        rows.push(line(ix, title, outcome, None).into_any_element());
    }
    let offset = report.pre.len();
    for (ix, (op, outcome)) in report.post.results.iter().enumerate() {
        let title: SharedString =
            format!("{} · {}", PostRowKind::from_op(op).label(), row_texts(op).0).into();
        // 提取值按结果行号对应（core 的 `Extracted.index`），不按 key 反查——同一个 key
        // 提取两次也对得上；`apply_extracted` 已把写不进去的行从 extracted 移除，
        // 找得到的都是真正写入的。
        let extra = report
            .post
            .extracted
            .iter()
            .find(|e| e.index == ix)
            .map(|e| {
                let value = if is_extracted_var_secret(sets, e.scope, group, &e.key) {
                    "••••••".to_string()
                } else {
                    e.value.clone()
                };
                tr!("ops.extracted_line", key = e.key.clone(), value = value)
            });
        rows.push(line(offset + ix, title, outcome, extra).into_any_element());
    }
    v_flex()
        .id("ops-section")
        .size_full()
        .min_h_0()
        .overflow_y_scroll()
        .p_3()
        .children(rows)
        .into_any_element()
}

impl RequestTab {
    pub fn render_response_pane(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        // 只取用得上的两样（一个 bool、一个 Copy 枚举），不整份 clone 证书——
        // 那是八个 String，每帧重绘都要重新分配一遍
        let (is_done, has_pretty, sse_done, headers_count, has_certificate, banner) =
            match &self.response {
                ResponseState::Done { view, .. } => {
                    let cert = view.meta.certificate.as_deref();
                    (
                        true,
                        view.has_pretty(),
                        view.sse.is_some(),
                        view.header_rows.len(),
                        cert.is_some(),
                        // 证书没问题时不打扰，只有体检出结果才挂横幅
                        cert.filter(|info| !info.is_trustworthy())
                            .and_then(|info| info.warnings.first().copied()),
                    )
                }
                _ => (false, false, false, 0, false, None),
            };
        let section = self.response_section;
        let ops_report = self.ops_report();
        // 页签随响应变：http 请求没有证书、没有启用操作时那两页就不该出现
        let sections = ResponseSection::visible(has_certificate, ops_report.is_some());
        let selected_section = sections.iter().position(|s| *s == section).unwrap_or(0);
        let clicked_sections = sections.clone();
        // Idle 下没东西可清；InFlight 归 URL 栏的取消按钮管，这里不掺和
        let can_clear = matches!(
            self.response,
            ResponseState::Done { .. } | ResponseState::Failed { .. }
        );
        let wrap_response = settings::settings(cx).wrap_response_body;
        let wrap_available = self.response_wrap_available();
        let copy_target = self.copy_target();
        let tab = cx.entity();

        v_flex()
            .size_full()
            .min_h_0()
            // 顶栏拆成两行：状态元数据与操作一行、页签与 Pretty/Raw 一行。全挤在一行时，
            // 右侧按钮组 flex_none 不收缩、左侧又没裁剪，窄面板下状态文字会直接画到按钮上。
            // 请求侧的 Body 工具条早就是这么拆的（见 request_pane::render_body_section）。
            .child(
                h_flex()
                    .h_9()
                    .px_3()
                    .gap_3()
                    .items_center()
                    .justify_between()
                    // 可压缩 + 裁剪：再窄也只是把 HTTP 版本这类次要信息切掉，不会溢出压到按钮上
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .child(self.render_status_line(cx)),
                    )
                    .child(
                        h_flex()
                            .flex_none()
                            .gap_1()
                            .items_center()
                            // Button 没有 aria_label（只实现 InteractiveElement），
                            // tooltip 就是这几个图标按钮对外的可读名字，一个都不能省
                            // 复制当前页：Body 页整份正文、Headers 页全部响应头。文本在点击时才拼，
                            // 渲染期只看有没有目标（响应体可能几十 MB）。
                            .when_some(copy_target, |h, target| {
                                let tab = tab.clone();
                                h.child(
                                    Clipboard::new("copy-response")
                                        .value_fn(move |_, cx| {
                                            tab.read(cx)
                                                .copy_target_text()
                                                .unwrap_or_default()
                                                .into()
                                        })
                                        .tooltip(target.tooltip()),
                                )
                            })
                            .when(is_done, |h| {
                                h.child(
                                    Button::new("find-in-response")
                                        .ghost()
                                        .xsmall()
                                        .icon(IconName::Search)
                                        .tooltip_with_action(
                                            tr!("response.find"),
                                            &FindInResponse,
                                            None,
                                        )
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.find_in_response(window, cx)
                                        })),
                                )
                                // 换行只对 A 档的只读 Editor 有意义：B/C 档走 uniform_list，
                                // 等高行是它的硬前提，换行会直接把虚拟化算法弄乱。那两档
                                // 下按钮置灰并在 tooltip 里说明去处（横向滚动条现在常驻）。
                                .child(
                                    Button::new("wrap-response-body")
                                        .ghost()
                                        .xsmall()
                                        .icon(Icon::empty().path(ICON_WRAP_TEXT))
                                        .selected(wrap_response && wrap_available)
                                        .disabled(!wrap_available)
                                        .tooltip(if wrap_available {
                                            tr!("response.wrap_lines")
                                        } else {
                                            tr!("response.wrap_unavailable")
                                        })
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.toggle_response_wrap(cx)
                                        })),
                                )
                                .child(
                                    Button::new("save-body")
                                        .ghost()
                                        .xsmall()
                                        .icon(IconName::HardDrive)
                                        .tooltip(tr!("response.save_to_file"))
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.save_body(window, cx)
                                        })),
                                )
                            })
                            .when(can_clear, |h| {
                                h.child(
                                    Button::new("clear-response")
                                        .ghost()
                                        .xsmall()
                                        .icon(IconName::Delete)
                                        .tooltip(tr!("response.clear"))
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.clear_response(window, cx)
                                        })),
                                )
                            }),
                    ),
            )
            .child(
                h_flex()
                    .h_9()
                    .px_3()
                    .gap_3()
                    .items_center()
                    .justify_between()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        TabBar::new("response-sections")
                            .underline()
                            .xsmall()
                            .selected_index(selected_section)
                            .on_click(cx.listener(move |this, ix: &usize, _, cx| {
                                if let Some(next) = clicked_sections.get(*ix) {
                                    this.response_section = *next;
                                    cx.notify();
                                }
                            }))
                            .children(sections.iter().map(|s| match s {
                                ResponseSection::Body => Tab::new().label("Body"),
                                ResponseSection::Headers => {
                                    Tab::new().label(if headers_count > 0 {
                                        format!("Headers ({headers_count})")
                                    } else {
                                        "Headers".to_string()
                                    })
                                }
                                ResponseSection::Certificate => {
                                    Tab::new().label(tr!("response.section_certificate"))
                                }
                                ResponseSection::Ops => Tab::new().label(match ops_report {
                                    Some(r) => tr!(
                                        "ops.section_title",
                                        passed = r.passed(),
                                        total = r.total()
                                    ),
                                    None => tr!("ops.section_title", passed = 0, total = 0),
                                }),
                            })),
                    )
                    // 只有存在美化文本时才提供 Pretty/Raw 切换（SSE 响应没有 Pretty，
                    // 段控换成 文本 / 事件流 / 原始 三视图）
                    .when(has_pretty, |h| {
                        h.child(
                            TabBar::new("pretty-raw")
                                .segmented()
                                .xsmall()
                                .selected_index(if self.pretty { 0 } else { 1 })
                                .on_click(cx.listener(|this, ix: &usize, window, cx| {
                                    this.set_pretty(*ix == 0, window, cx)
                                }))
                                .child("Pretty")
                                .child("Raw"),
                        )
                    })
                    .when(sse_done, |h| {
                        h.child(
                            TabBar::new("sse-mode")
                                .segmented()
                                .xsmall()
                                .selected_index(self.sse_mode.index())
                                .on_click(cx.listener(|this, ix: &usize, window, cx| {
                                    this.set_sse_mode(SseBodyMode::from_index(*ix), window, cx)
                                }))
                                .children(SseBodyMode::ALL.iter().map(|m| m.label())),
                        )
                    }),
            )
            // 搜索提示原本挤在顶栏右侧（max_w_96），是把那一行撑爆的主因之一。
            // 挪成横幅后既不再抢顶栏的宽度，文字也不用截断了。
            .when_some(self.notice.clone(), |v, notice| {
                v.child(
                    Alert::info("search-notice", notice.text())
                        .banner()
                        .xsmall(),
                )
            })
            .when_some(banner, |v, warning| {
                v.child(self.render_certificate_banner(warning, cx))
            })
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .child(self.render_response_body(section, window, cx)),
            )
    }

    /// 证书体检出问题时挂在响应上方的常驻横幅。
    ///
    /// 这条只在请求**成功**时出现——校验关着，自签名 / 过期证书照样能连上，
    /// 用户需要知道「连是连上了，但这张证书不可信」。
    fn render_certificate_banner(
        &self,
        warning: getcat_core::tls::CertWarning,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        h_flex()
            .items_center()
            .gap_2()
            .px_3()
            .py_1()
            .border_b_1()
            .border_color(cx.theme().warning.opacity(0.3))
            .bg(cx.theme().warning.opacity(0.08))
            .text_xs()
            .child(div().flex_1().min_w_0().child(tr!(
                "response.cert_banner",
                issue = cert_warning_label(warning)
            )))
            .child(
                Button::new("view-certificate")
                    .ghost()
                    .xsmall()
                    .label(tr!("response.cert_view"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.response_section = ResponseSection::Certificate;
                        cx.notify();
                    })),
            )
    }

    fn render_response_body(
        &self,
        section: ResponseSection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match &self.response {
            ResponseState::Idle => {
                let send_key = Kbd::binding_for_action(&SendRequest, None, window);
                empty_state_frame(cx)
                    .child(
                        h_flex()
                            .gap_1()
                            .items_center()
                            .child(tr!("response.idle_prefix"))
                            .children(send_key),
                    )
                    .into_any_element()
            }
            ResponseState::InFlight {
                received,
                total,
                live,
                ..
            } => {
                // SSE：收到就展示。拼出的文本（或原始流）随 Chunk 事件实时增长。
                if let Some(live) = live {
                    return self.render_sse_live(live, cx);
                }
                let text = match total {
                    Some(t) => tr!(
                        "response.in_flight",
                        received = format_bytes(*received),
                        total = format_bytes(*t)
                    ),
                    None => tr!(
                        "response.in_flight_unknown",
                        received = format_bytes(*received)
                    ),
                };
                empty_state(text, cx)
            }
            ResponseState::Failed {
                error: RequestError::Cancelled,
                ..
            } => empty_state(tr!("response.cancelled"), cx),
            ResponseState::Failed { error, ops } => {
                // 「操作」页签且这次失败挂了报告：前置结果 + 后置「请求失败，未执行」照常可看
                if section == ResponseSection::Ops
                    && let Some(report) = ops
                {
                    return render_ops_report(report, self.saved_group.as_deref(), cx);
                }
                v_flex()
                    .size_full()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(cx.theme().danger)
                            .child(error_kind(error)),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(error_detail(error)),
                    )
                    // 显式挑了版本又失败，多半是服务端不支持这一版——reqwest 的原话
                    // （"frame with invalid size" 之类）指不到这一点，补一句去处
                    .when(self.http_version != HttpVersionPref::Auto, |v| {
                        v.child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(tr!(
                                    "response.forced_version_hint",
                                    version = self.http_version.label()
                                )),
                        )
                    })
                    .into_any_element()
            }
            ResponseState::Done { body, view, ops } => match section {
                ResponseSection::Body => self.render_body_view(body, view, cx),
                ResponseSection::Headers => {
                    render_header_rows(view.header_rows.clone(), &self.headers_list, cx)
                        .into_any_element()
                }
                ResponseSection::Certificate => match &view.meta.certificate {
                    Some(info) => render_certificate(info),
                    // 上一条响应有证书、这一条没有：页签已经消失，内容跟着回落到 Body
                    None => self.render_body_view(body, view, cx),
                },
                ResponseSection::Ops => match ops {
                    Some(report) => render_ops_report(report, self.saved_group.as_deref(), cx),
                    // 没有报告时页签本就不出现，防御性地回落到 Body
                    None => self.render_body_view(body, view, cx),
                },
            },
        }
    }

    /// SSE 在途的实时视图：顶部一行接收状态，正文是逐 chunk 增长的拼装文本 / 原始流。
    fn render_sse_live(&self, live: &SseLive, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .size_full()
            .child(
                h_flex()
                    .px_3()
                    .py_1()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(tr!("response.sse_streaming", events = live.event_count)),
            )
            .child(
                div()
                    .id("sse-live-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p_3()
                    .role(Role::Group)
                    .aria_label(tr!("response.sse_live_aria"))
                    .font_family(cx.theme().mono_font_family.clone())
                    .text_size(cx.theme().mono_font_size)
                    .child(live.display_text()),
            )
            .into_any_element()
    }

    /// SSE 响应完成后的 Body 区：统计条 + 按 `sse_mode` 分派的三视图。
    fn render_sse_body(
        &self,
        view: &ResponseView,
        sse: &SseView,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let content = match self.effective_sse_mode(sse) {
            SseBodyMode::Events => {
                render_sse_events(sse.events.clone(), &self.body_scroll, cx).into_any_element()
            }
            // Text / Raw：与普通响应一样按档位分派（current_doc 已按模式选好文档）
            _ => match self.current_doc(view) {
                Some(doc) if doc.doc.line_count() > 0 => match doc.tier {
                    ViewTier::Editor => {
                        Editor::new(self.response_editor_for(view.kind.editor_language()))
                            .aria_label(tr!("response.body_aria"))
                            .font_family(cx.theme().mono_font_family.clone())
                            .text_size(cx.theme().mono_font_size)
                            .readonly(true)
                            .size_full()
                            .into_any_element()
                    }
                    ViewTier::Virtual | ViewTier::Preview => render_text_lines(
                        "response-lines",
                        doc.doc.clone(),
                        &self.body_scroll,
                        &self.lines_selection,
                        cx.listener(|this, _: &SelectAll, window, cx| {
                            this.select_all_response_lines(window, cx)
                        }),
                        cx,
                    )
                    .into_any_element(),
                },
                _ => empty_state(tr!("response.empty_body"), cx),
            },
        };
        v_flex()
            .size_full()
            .child(self.render_sse_stats(view, sse, cx))
            .child(div().flex_1().min_h_0().child(content))
            .into_any_element()
    }

    /// SSE 统计条：TTFT、事件数、token 用量与生成速率。
    ///
    /// TTFT 优先取首个内容 delta 的时刻（在途解析记录），拼不出 delta 的流退回
    /// 首字节时刻（TTFB）。tok/s 按 output_tokens / (总耗时 − TTFT) 计。
    fn render_sse_stats(
        &self,
        view: &ResponseView,
        sse: &SseView,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let ttft = sse.first_delta.or(view.meta.ttfb);
        let rate = match (sse.usage.output_tokens, ttft) {
            (Some(tokens), Some(t)) if view.meta.duration > t => {
                let secs = (view.meta.duration - t).as_secs_f64();
                (secs > 0.).then(|| format!("{:.1}", tokens as f64 / secs))
            }
            _ => None,
        };
        h_flex()
            .flex_wrap()
            .items_center()
            .gap_x_3()
            .px_3()
            .py_1()
            .border_b_1()
            .border_color(cx.theme().border)
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .when_some(ttft, |h, t| {
                h.child(tr!("response.sse_stats_ttft", value = format_duration(t)))
            })
            .child(tr!("response.sse_stats_events", count = sse.events.len()))
            .when_some(sse.usage.input_tokens, |h, n| {
                h.child(tr!("response.sse_stats_input_tokens", count = n))
            })
            .when_some(sse.usage.output_tokens, |h, n| {
                h.child(tr!("response.sse_stats_output_tokens", count = n))
            })
            .when_some(rate, |h, r| {
                h.child(tr!("response.sse_stats_rate", rate = r))
            })
            .into_any_element()
    }

    /// 按档位分派：A 档只读 Editor；B 档 uniform_list 行视图；C 档摘要 + 前 1 MiB 行视图；二进制只有摘要。
    fn render_body_view(
        &self,
        body: &BodyStore,
        view: &ResponseView,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        // SSE 响应整块换成 统计条 + 三视图
        if let Some(sse) = &view.sse {
            return self.render_sse_body(view, sse, cx);
        }
        let Some(doc) = view.doc(self.pretty) else {
            return v_flex()
                .size_full()
                .child(self.render_preview_summary(body, view, cx))
                .child(empty_state(tr!("response.binary_no_preview"), cx))
                .into_any_element();
        };
        let lines = if doc.doc.line_count() == 0 {
            empty_state(tr!("response.empty_body"), cx)
        } else {
            match doc.tier {
                ViewTier::Editor => {
                    Editor::new(self.response_editor_for(view.kind.editor_language()))
                        .aria_label(tr!("response.body_aria"))
                        .font_family(cx.theme().mono_font_family.clone())
                        .text_size(cx.theme().mono_font_size)
                        .readonly(true)
                        .size_full()
                        .into_any_element()
                }
                ViewTier::Virtual | ViewTier::Preview => render_text_lines(
                    "response-lines",
                    doc.doc.clone(),
                    &self.body_scroll,
                    &self.lines_selection,
                    cx.listener(|this, _: &SelectAll, window, cx| {
                        this.select_all_response_lines(window, cx)
                    }),
                    cx,
                )
                .into_any_element(),
            }
        };
        v_flex()
            .size_full()
            .when_some(tier_notice(doc.tier), |v, text| v.child(notice_bar(text)))
            .when(view.is_preview(), |v| {
                v.child(self.render_preview_summary(body, view, cx))
            })
            .child(div().flex_1().min_h_0().child(lines))
            .into_any_element()
    }

    /// C 档 / 二进制的摘要块：大小、类型、耗时、临时文件路径与"用系统程序打开"。
    fn render_preview_summary(
        &self,
        body: &BodyStore,
        view: &ResponseView,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        // 标签 / 值成对的元数据用官方 DescriptionList：标签列宽、间距与字色都由组件定
        let list = DescriptionList::new()
            .columns(1)
            .bordered(false)
            .small()
            .label_width(rems(4.5))
            .item(
                tr!("response.summary.size"),
                format_bytes(view.meta.body_len),
                1,
            )
            .item(
                tr!("response.summary.type"),
                view.meta
                    .content_type
                    .clone()
                    .map(SharedString::from)
                    .unwrap_or_else(|| content_kind_label(view.kind)),
                1,
            )
            .item(
                tr!("response.summary.duration"),
                format_duration(view.meta.duration),
                1,
            )
            .when_some(body.path(), |list, path| {
                list.item(
                    tr!("response.summary.temp_file"),
                    path.display().to_string(),
                    1,
                )
            });
        v_flex()
            .gap_2()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(list)
            .when(body.path().is_some(), |v| {
                v.child(
                    h_flex().child(
                        Button::new("open-with-system")
                            .outline()
                            .xsmall()
                            .label(tr!("response.open_with_system"))
                            .on_click(cx.listener(|this, _, _, cx| this.open_body_with_system(cx))),
                    ),
                )
            })
            .into_any_element()
    }

    pub fn render_status_line(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        match &self.response {
            ResponseState::Idle => h_flex()
                .text_sm()
                .text_color(muted)
                .child(tr!("response.status_idle"))
                .into_any_element(),
            ResponseState::InFlight {
                started, received, ..
            } => h_flex()
                .gap_3()
                .text_sm()
                .text_color(muted)
                .child(tr!(
                    "response.status_in_flight",
                    elapsed = format_duration(started.elapsed())
                ))
                .child(format_bytes(*received))
                .into_any_element(),
            ResponseState::Failed { error, .. } => {
                let cancelled = matches!(error, RequestError::Cancelled);
                h_flex()
                    .text_sm()
                    .text_color(if cancelled { muted } else { cx.theme().danger })
                    .child(if cancelled {
                        tr!("response.status_cancelled")
                    } else {
                        tr!("response.status_failed")
                    })
                    .into_any_element()
            }
            ResponseState::Done { view, .. } => {
                let color = status_color(view.meta.status, cx);
                h_flex()
                    .gap_3()
                    .items_center()
                    .text_sm()
                    .child(
                        // 状态码用官方 Tag 的描边形态：圆角与内边距跟主题走，不再手调透明度
                        Tag::custom(color, color, color)
                            .outline()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(format!("{} {}", view.meta.status, view.meta.status_text)),
                    )
                    .child(
                        div()
                            .text_color(muted)
                            .child(format_duration(view.meta.duration)),
                    )
                    .child(
                        div()
                            .text_color(muted)
                            .child(format_bytes(view.meta.body_len)),
                    )
                    .child(div().text_color(muted).child(content_kind_label(view.kind)))
                    // 选了 Auto 时，这是唯一能看出到底走了 h1 还是 h2 的地方
                    .when_some(view.meta.http_version.clone(), |h, version| {
                        h.child(div().text_color(muted).child(version))
                    })
                    .into_any_element()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use getcat_core::model::{Environment, Variable};

    fn secret(key: &str, value: &str) -> Variable {
        let mut v = Variable::new(key, value);
        v.secret = true;
        v
    }

    /// 短 secret 是长 secret 的前缀时，必须先换长的，否则长值被切掉一角、
    /// 残留的后半截（这里是 "XYZ"）会露在外面。
    #[test]
    fn secret_values_sort_longer_first_so_substrings_dont_leak() {
        let mut sets = VariableSets::default();
        sets.globals.push(secret("short", "abc"));
        let mut env = Environment::new("dev");
        env.variables.push(secret("long", "abcXYZ"));
        sets.active_environment = Some(env.id);
        sets.environments.push(env);

        let values = secret_values(&sets);
        assert_eq!(values, vec!["abcXYZ".to_string(), "abc".to_string()]);
        assert_eq!(
            mask_with("got abcXYZ and abc", &values),
            "got •••••• and ••••••"
        );
    }

    /// 环境作用域与分类作用域的 secret 都要认得到、都能掩码。
    #[test]
    fn secret_values_cover_environment_and_group_scopes() {
        let mut sets = VariableSets::default();
        let mut env = Environment::new("dev");
        env.variables.push(secret("env_token", "ENVSECRET"));
        sets.active_environment = Some(env.id);
        sets.environments.push(env);
        sets.groups
            .insert("g".into(), vec![secret("grp_token", "GRPSECRET")]);

        let values = secret_values(&sets);
        assert!(values.contains(&"ENVSECRET".to_string()));
        assert!(values.contains(&"GRPSECRET".to_string()));
        assert_eq!(
            mask_with("env=ENVSECRET grp=GRPSECRET", &values),
            "env=•••••• grp=••••••"
        );
    }

    /// 既有场景：全局 secret 被掩码，非 secret 值原样保留。
    #[test]
    fn mask_with_redacts_secret_variable_values() {
        let mut sets = VariableSets::default();
        sets.globals.push(secret("token", "T0K"));
        sets.globals.push(Variable::new("plain", "P"));
        let values = secret_values(&sets);

        assert_eq!(
            mask_with("Expected 200, got T0K", &values),
            "Expected 200, got ••••••"
        );
        // 非 secret 的值原样保留
        assert_eq!(mask_with("value is P", &values), "value is P");
    }
}
