//! 响应面板：状态行、Pretty/Raw 与 Body/Headers 切换、按档位分派的 Body 视图、虚拟化 Headers 列表。

use getcat_core::body::tier::ViewTier;
use getcat_core::http::{BodyStore, RequestError};
use getcat_core::model::HttpVersionPref;
use getcat_core::tls::CertificateInfo;
use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    ActiveTheme, Disableable, Icon, IconName, Selectable, Sizable,
    alert::Alert,
    button::{Button, ButtonVariants},
    description_list::DescriptionList,
    h_flex,
    input::Editor,
    kbd::Kbd,
    tab::{Tab, TabBar},
    tag::Tag,
    v_flex,
};

use crate::assets::ICON_WRAP_TEXT;
use crate::i18n::tr;
use crate::state::request_tab::{RequestTab, ResponseSection};
use crate::state::response::{ResponseState, ResponseView};
use crate::state::settings;
use crate::ui::body_view::{render_header_rows, render_text_lines};
use crate::ui::text::{
    cert_warning_label, content_kind_label, error_detail, error_kind, tier_notice,
};
use crate::ui::{format_bytes, format_duration, status_color};
use crate::{FindInResponse, SendRequest};

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

impl RequestTab {
    pub fn render_response_pane(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        // 只取用得上的两样（一个 bool、一个 Copy 枚举），不整份 clone 证书——
        // 那是八个 String，每帧重绘都要重新分配一遍
        let (is_done, has_pretty, headers_count, has_certificate, banner) = match &self.response {
            ResponseState::Done { view, .. } => {
                let cert = view.meta.certificate.as_deref();
                (
                    true,
                    view.has_pretty(),
                    view.header_rows.len(),
                    cert.is_some(),
                    // 证书没问题时不打扰，只有体检出结果才挂横幅
                    cert.filter(|info| !info.is_trustworthy())
                        .and_then(|info| info.warnings.first().copied()),
                )
            }
            _ => (false, false, 0, false, None),
        };
        let section = self.response_section;
        // 页签随响应变：http 请求没有证书，那一页就不该出现
        let sections = ResponseSection::visible(has_certificate);
        let selected_section = sections.iter().position(|s| *s == section).unwrap_or(0);
        let clicked_sections = sections.clone();
        // Idle 下没东西可清；InFlight 归 URL 栏的取消按钮管，这里不掺和
        let can_clear = matches!(
            self.response,
            ResponseState::Done { .. } | ResponseState::Failed { .. }
        );
        let wrap_response = settings::settings(cx).wrap_response_body;
        let wrap_available = self.response_wrap_available();

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
                            // tooltip 就是这三个图标按钮对外的可读名字，一个都不能省
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
                            })),
                    )
                    // 只有存在美化文本时才提供 Pretty/Raw 切换
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
                received, total, ..
            } => {
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
            } => empty_state(tr!("response.cancelled"), cx),
            ResponseState::Failed { error } => v_flex()
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
                .into_any_element(),
            ResponseState::Done { body, view } => match section {
                ResponseSection::Body => self.render_body_view(body, view, cx),
                ResponseSection::Headers => {
                    render_header_rows(view.header_rows.clone(), &self.headers_scroll, cx)
                        .into_any_element()
                }
                ResponseSection::Certificate => match &view.meta.certificate {
                    Some(info) => render_certificate(info),
                    // 上一条响应有证书、这一条没有：页签已经消失，内容跟着回落到 Body
                    None => self.render_body_view(body, view, cx),
                },
            },
        }
    }

    /// 按档位分派：A 档只读 Editor；B 档 uniform_list 行视图；C 档摘要 + 前 1 MiB 行视图；二进制只有摘要。
    fn render_body_view(
        &self,
        body: &BodyStore,
        view: &ResponseView,
        cx: &mut Context<Self>,
    ) -> AnyElement {
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
                ViewTier::Virtual | ViewTier::Preview => {
                    render_text_lines("response-lines", doc.doc.clone(), &self.body_scroll, cx)
                        .into_any_element()
                }
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
            ResponseState::Failed { error } => {
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
