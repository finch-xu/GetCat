//! 键值表：Params / Headers / Form 共用。最后一行永远保留一个空行用于新增。

use getcat_core::model::KeyValue;
use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    ActiveTheme, Disableable, IconName, Sizable,
    button::{Button, ButtonVariants},
    checkbox::Checkbox,
    h_flex,
    input::{Input, InputEvent, InputState},
    v_flex,
};

pub enum KvTableEvent {
    Changed,
}

struct KvRow {
    key: Entity<InputState>,
    value: Entity<InputState>,
    enabled: bool,
    _subs: Vec<Subscription>,
}

pub struct KvTable {
    key_placeholder: SharedString,
    value_placeholder: SharedString,
    rows: Vec<KvRow>,
    /// Path 参数模式：key 只读、由 URL 驱动，不自动追加空行。
    locked_keys: bool,
}

impl EventEmitter<KvTableEvent> for KvTable {}

impl KvTable {
    pub fn new(
        key_placeholder: impl Into<SharedString>,
        value_placeholder: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut this = Self {
            key_placeholder: key_placeholder.into(),
            value_placeholder: value_placeholder.into(),
            rows: Vec::new(),
            locked_keys: false,
        };
        this.push_row("", "", true, window, cx);
        this
    }

    pub fn locked_keys(mut self, locked: bool) -> Self {
        self.locked_keys = locked;
        if locked {
            self.rows.clear();
        }
        self
    }

    fn push_row(
        &mut self,
        key: &str,
        value: &str,
        enabled: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key_ph = self.key_placeholder.clone();
        let value_ph = self.value_placeholder.clone();
        let key_state = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(key_ph)
                .default_value(key.to_string())
        });
        let value_state = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(value_ph)
                .default_value(value.to_string())
        });
        let subs = vec![
            cx.subscribe_in(&key_state, window, Self::on_input_event),
            cx.subscribe_in(&value_state, window, Self::on_input_event),
        ];
        self.rows.push(KvRow {
            key: key_state,
            value: value_state,
            enabled,
            _subs: subs,
        });
    }

    fn on_input_event(
        &mut self,
        _: &Entity<InputState>,
        ev: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(ev, InputEvent::Change) {
            self.ensure_trailing_empty_row(window, cx);
            cx.emit(KvTableEvent::Changed);
        }
    }

    fn ensure_trailing_empty_row(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.locked_keys {
            return;
        }
        let last_is_empty = self
            .rows
            .last()
            .map(|r| r.key.read(cx).value().is_empty() && r.value.read(cx).value().is_empty())
            .unwrap_or(false);
        if !last_is_empty {
            self.push_row("", "", true, window, cx);
            cx.notify();
        }
    }

    pub fn values(&self, cx: &App) -> Vec<KeyValue> {
        self.rows
            .iter()
            .filter_map(|r| {
                let key = r.key.read(cx).value().to_string();
                let value = r.value.read(cx).value().to_string();
                (!key.is_empty() || !value.is_empty()).then_some(KeyValue {
                    key,
                    value,
                    enabled: r.enabled,
                })
            })
            .collect()
    }

    pub fn count(&self, cx: &App) -> usize {
        self.values(cx)
            .iter()
            .filter(|kv| kv.enabled && !kv.key.is_empty())
            .count()
    }

    /// 让行的 key 集合等于 `names`（保持顺序），保留同名行已有的 value 与 enabled。
    pub fn sync_keys(&mut self, names: &[String], window: &mut Window, cx: &mut Context<Self>) {
        let existing: Vec<(String, String, bool)> = self
            .rows
            .iter()
            .map(|r| {
                (
                    r.key.read(cx).value().to_string(),
                    r.value.read(cx).value().to_string(),
                    r.enabled,
                )
            })
            .collect();
        if existing
            .iter()
            .map(|(k, _, _)| k.as_str())
            .eq(names.iter().map(String::as_str))
        {
            return;
        }
        self.rows.clear();
        for name in names {
            let (value, enabled) = existing
                .iter()
                .find(|(k, _, _)| k == name)
                .map(|(_, v, e)| (v.clone(), *e))
                .unwrap_or_else(|| (String::new(), true));
            self.push_row(name, &value, enabled, window, cx);
        }
        cx.emit(KvTableEvent::Changed);
        cx.notify();
    }

    fn remove_row(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if ix < self.rows.len() {
            self.rows.remove(ix);
        }
        self.ensure_trailing_empty_row(window, cx);
        cx.emit(KvTableEvent::Changed);
        cx.notify();
    }

    fn toggle_row(&mut self, ix: usize, checked: bool, cx: &mut Context<Self>) {
        if let Some(r) = self.rows.get_mut(ix) {
            r.enabled = checked;
        }
        cx.emit(KvTableEvent::Changed);
        cx.notify();
    }
}

impl Render for KvTable {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let locked = self.locked_keys;
        v_flex()
            .w_full()
            .gap_1()
            .child(
                h_flex()
                    .px_1()
                    .gap_2()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(div().w(px(28.)))
                    .child(div().flex_1().child("Key"))
                    .child(div().flex_1().child("Value"))
                    .child(div().w(px(28.))),
            )
            .when(locked && self.rows.is_empty(), |v| {
                v.child(
                    div()
                        .px_1()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child("URL 中没有 {name} 形式的 Path 参数"),
                )
            })
            .children(self.rows.iter().enumerate().map(|(ix, row)| {
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        div().w(px(28.)).flex().justify_center().child(
                            // 可访问名称：gpui-component Checkbox 只有可见 label 会进入 a11y 树（tooltip 不算），
                            // 而每行加可见文字会让表格变宽、噪音大。Plan 2 决定保留现状；后续方向是给上游
                            // Checkbox 加 aria_label，或自绘带 role(Checkbox) 的开关（见 Plan 1 Ruling 2 更正）。
                            Checkbox::new(("kv-enabled", ix))
                                .checked(row.enabled)
                                .tooltip("启用此行")
                                .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                                    this.toggle_row(ix, *checked, cx)
                                })),
                        ),
                    )
                    .child(
                        div().flex_1().child(
                            Input::new(&row.key)
                                .small()
                                .readonly(locked)
                                .aria_label(self.key_placeholder.clone()),
                        ),
                    )
                    .child(
                        div().flex_1().child(
                            Input::new(&row.value)
                                .small()
                                .aria_label(self.value_placeholder.clone()),
                        ),
                    )
                    .child(
                        div().w(px(28.)).child(
                            Button::new(("kv-remove", ix))
                                .ghost()
                                .xsmall()
                                .icon(IconName::Close)
                                .tooltip("删除此行")
                                .disabled(locked)
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.remove_row(ix, window, cx)
                                })),
                        ),
                    )
            }))
    }
}
