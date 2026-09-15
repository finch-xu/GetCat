//! 变量（全局）：内存里一份 [`VariableSets`]，改动后同步写 `variables.json`。
//! 与 [`crate::state::settings`] 同款：`update` 在副本上改、比对、落盘、`set_global`；
//! 只读模式下 `store(cx)` 为 None，写盘自动变 no-op。

use std::collections::BTreeMap;

use getcat_core::model::{PreOp, RequestDraft, Ulid, VariableSets};
use getcat_core::ops::{self, OpOutcome, PostReport};
use getcat_core::vars::{self, Resolved};
use gpui_kit::{App, Global};

use crate::state::store::store;

pub struct VariablesHandle {
    sets: VariableSets,
}

impl Global for VariablesHandle {}

/// 全局未安装（只有测试会这样）时 `variables()` 返回它。
static EMPTY: VariableSets = VariableSets {
    globals: Vec::new(),
    environments: Vec::new(),
    active_environment: None,
    groups: BTreeMap::new(),
};

/// 启动时安装：`loaded` 为 `variables.json` 的内容（没有文件则为空）。
pub fn install(cx: &mut App, loaded: Option<VariableSets>) {
    cx.set_global(VariablesHandle {
        sets: loaded.unwrap_or_default(),
    });
}

pub fn variables(cx: &App) -> &VariableSets {
    cx.try_global::<VariablesHandle>()
        .map(|h| &h.sets)
        .unwrap_or(&EMPTY)
}

/// 修改变量：`f` 在副本上改；没变化则什么都不做，否则落盘 + 装回全局。
pub fn update(cx: &mut App, f: impl FnOnce(&mut VariableSets)) {
    let before = variables(cx);
    let mut next = before.clone();
    f(&mut next);
    if next == *before {
        return;
    }
    if let Some(store) = store(cx) {
        store.write_variables(next.clone());
    }
    cx.set_global(VariablesHandle { sets: next });
}

// 计划 3 的环境切换器（UI）调用；本计划只有测试在用
#[allow(dead_code)]
pub fn set_active_environment(cx: &mut App, id: Option<Ulid>) {
    update(cx, |s| s.active_environment = id);
}

/// 按当前变量表替换一份草稿；`group` 是这条请求所属的分类（未保存 / 未分类为 None）。
/// 按值接收：调用方手里都是刚取出、用完即丢的草稿，省掉一次多 MB body 的克隆。
pub fn resolve(cx: &App, group: Option<&str>, draft: RequestDraft) -> Resolved {
    vars::resolve_draft(draft, &variables(cx).context(group))
}

/// 执行前置操作并落盘。没有启用的操作时不碰变量表（也就不写盘）。
pub fn run_pre_ops(
    cx: &mut App,
    group: Option<&str>,
    pre_ops: &[PreOp],
) -> Vec<(PreOp, OpOutcome)> {
    if !pre_ops.iter().any(|o| o.enabled) {
        return Vec::new();
    }
    let mut results = Vec::new();
    update(cx, |sets| results = ops::run_pre_ops(pre_ops, sets, group));
    results
}

/// 把后置提取的值写进变量表，并把写不进去的提取行改写为「跳过」。
/// 调用方负责先做 generation 校验；渲染「操作」页签之前必须先调它。
pub fn apply_extracted(cx: &mut App, group: Option<&str>, report: &mut PostReport) {
    if report.extracted.is_empty() {
        return;
    }
    update(cx, |sets| ops::apply_extracted(report, sets, group));
}
