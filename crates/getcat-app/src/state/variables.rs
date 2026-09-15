//! 变量（全局）：内存里一份 [`VariableSets`]，改动后同步写 `variables.json`。
//! 与 [`crate::state::settings`] 同款：`update` 在副本上改、比对、落盘、`set_global`；
//! 只读模式下 `store(cx)` 为 None，写盘自动变 no-op；`variables.json` 在但读不出来时
//! 以 `persist == false` 安装，同样只改内存（见 [`should_persist`]）。

use std::collections::BTreeMap;
use std::path::Path;

use getcat_core::model::{PreOp, RequestDraft, Ulid, VariableSets};
use getcat_core::ops::{self, OpOutcome, PostReport};
use getcat_core::store::LoadError;
use getcat_core::vars::{self, Resolved};
use gpui_kit::{App, Global};

use crate::state::store::store;

pub struct VariablesHandle {
    sets: VariableSets,
    /// 改动是否落盘。
    persist: bool,
}

impl Global for VariablesHandle {}

/// 全局未安装（只有测试会这样）时 `variables()` 返回它。
static EMPTY: VariableSets = VariableSets {
    globals: Vec::new(),
    environments: Vec::new(),
    active_environment: None,
    groups: BTreeMap::new(),
};

/// 启动时安装：`loaded` 为 `variables.json` 的内容（没有文件则为空）；
/// `persist` 为 false 时之后的改动只留在内存里，不写盘（由 [`should_persist`] 决定）。
pub fn install(cx: &mut App, loaded: Option<VariableSets>, persist: bool) {
    cx.set_global(VariablesHandle {
        sets: loaded.unwrap_or_default(),
        persist,
    });
}

/// 启动时判断变量改动能否落盘：`variables.json` 读取失败、但文件还在原处（例如权限问题）时
/// 返回 false——这时装上的是空表，第一次前置操作 / 提取就会把用户的环境与 secret 覆盖掉。
/// 解析失败的文件已被改名隔离、不在原处，照常可写。拿不准文件在不在（`try_exists` 出错）时按在处理。
pub fn should_persist(path: &Path, errors: &[LoadError]) -> bool {
    let unreadable =
        errors.iter().any(|e| e.path == path) && !matches!(path.try_exists(), Ok(false));
    if unreadable {
        tracing::warn!(
            path = %path.display(),
            "variables file exists but couldn't be read; variable changes will not be saved this session"
        );
    }
    !unreadable
}

pub fn variables(cx: &App) -> &VariableSets {
    cx.try_global::<VariablesHandle>()
        .map(|h| &h.sets)
        .unwrap_or(&EMPTY)
}

/// 修改变量：`f` 在副本上改；没变化则什么都不做，否则落盘（`persist` 为 false 时跳过）+ 装回全局。
pub fn update(cx: &mut App, f: impl FnOnce(&mut VariableSets)) {
    let before = variables(cx);
    let mut next = before.clone();
    f(&mut next);
    if next == *before {
        return;
    }
    // 没安装全局（只有测试会这样）时按可写处理
    let persist = cx.try_global::<VariablesHandle>().is_none_or(|h| h.persist);
    if persist && let Some(store) = store(cx) {
        store.write_variables(next.clone());
    }
    cx.set_global(VariablesHandle {
        sets: next,
        persist,
    });
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

/// 一次发送的准备结果，见 [`prepare_send`]。
pub struct PreparedSend {
    /// 执行过前置操作的变量表副本；没有启用的前置操作时为 None（不复制、也无需提交）。
    /// 调用方要等请求真正能发出（`http::prepare` 成功）后再用 [`update`] 装回全局——
    /// 发不出去就直接丢掉，全局不变、不写盘。
    pub next: Option<VariableSets>,
    /// 每条启用的前置操作一行。
    pub pre_results: Vec<(PreOp, OpOutcome)>,
    /// 按副本替换后的草稿；`unresolved` 已并入前置操作值里的未解析名。
    pub resolved: Resolved,
}

/// 发送前的变量处理：在全局变量表的**副本**上依次执行前置操作，再按副本替换草稿——
/// 顺序决定了 `{{ts}}` 能引用前置操作刚设的值。本函数不改全局、不写盘。
/// 按值接收草稿，理由同 [`resolve`]。
pub fn prepare_send(cx: &App, group: Option<&str>, draft: RequestDraft) -> PreparedSend {
    let current = variables(cx);
    if !draft.pre_ops.iter().any(|o| o.enabled) {
        return PreparedSend {
            next: None,
            pre_results: Vec::new(),
            resolved: vars::resolve_draft(draft, &current.context(group)),
        };
    }
    let mut next = current.clone();
    let pre = ops::run_pre_ops(&draft.pre_ops, &mut next, group);
    let mut resolved = vars::resolve_draft(draft, &next.context(group));
    resolved.unresolved.extend(pre.unresolved);
    PreparedSend {
        next: Some(next),
        pre_results: pre.results,
        resolved,
    }
}

/// 把后置提取的值写进变量表，并把写不进去的提取行改写为「跳过」。
/// 调用方负责先做 generation 校验；渲染「操作」页签之前必须先调它。
pub fn apply_extracted(cx: &mut App, group: Option<&str>, report: &mut PostReport) {
    if report.extracted.is_empty() {
        return;
    }
    update(cx, |sets| ops::apply_extracted(report, sets, group));
}
