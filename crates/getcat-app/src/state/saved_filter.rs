//! 已保存请求的分类推导与过滤（spec 2026-08-26 §3）。
//!
//! 分类没有独立实体：由成员的 `group` 字段推导，成员删光分类自动消失，
//! 因此没有空分类、没有孤儿数据、没有迁移。本模块是纯逻辑，不依赖 UI。

use getcat_core::model::SavedRequest;

/// 侧栏「已保存」面板的过滤器。会话状态，不落盘，重启回 [`SavedFilter::All`]。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum SavedFilter {
    /// 全部请求（默认）。
    #[default]
    All,
    /// `group` 为 None 的请求。
    #[allow(dead_code)] // TODO(Task 3)：侧栏 UI 接入后删除
    Uncategorized,
    /// 某个分类。
    #[allow(dead_code)] // TODO(Task 3)：侧栏 UI 接入后删除
    Group(String),
}

/// 分类名入库前的归一化：trim，空串归 None（= 未分类）。
#[allow(dead_code)] // TODO(Task 3)：侧栏 UI 接入后删除
pub fn normalize_group(name: &str) -> Option<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// 推导分类清单：`(名称, 成员数)`，按不分大小写的字母序；
/// 只差大小写的名字是两个分类（原样保存），排序相邻。
#[allow(dead_code)] // TODO(Task 3)：侧栏 UI 接入后删除
pub fn derive_groups(saved: &[SavedRequest]) -> Vec<(String, usize)> {
    let mut groups: Vec<(String, usize)> = Vec::new();
    for request in saved {
        let Some(name) = &request.group else { continue };
        match groups.iter_mut().find(|(g, _)| g == name) {
            Some((_, count)) => *count += 1,
            None => groups.push((name.clone(), 1)),
        }
    }
    groups.sort_by(|(a, _), (b, _)| {
        a.to_lowercase()
            .cmp(&b.to_lowercase())
            .then_with(|| a.cmp(b))
    });
    groups
}

/// 未分类请求数（分类列决定要不要显示「未分类」项）。
pub fn uncategorized_count(saved: &[SavedRequest]) -> usize {
    saved.iter().filter(|r| r.group.is_none()).count()
}

/// 过滤后的**下标**集合（保持原有排序）。返回下标而不是克隆整条请求：
/// `SavedRequest` 里是完整的 `RequestDraft`（可能带大请求体），每次渲染克隆太贵；
/// 渲染闭包拿 `saved` 的 Rc 快照 + 这份下标，仍是每帧 O(可见行)。
#[allow(dead_code)] // TODO(Task 3)：侧栏 UI 接入后删除
pub fn filter_indices(filter: &SavedFilter, saved: &[SavedRequest]) -> Vec<usize> {
    saved
        .iter()
        .enumerate()
        .filter(|(_, r)| match filter {
            SavedFilter::All => true,
            SavedFilter::Uncategorized => r.group.is_none(),
            SavedFilter::Group(name) => r.group.as_deref() == Some(name.as_str()),
        })
        .map(|(ix, _)| ix)
        .collect()
}

/// 过滤器是否仍有成员。选中的分类被删光/解散/合并走后，调用方回退到 All（spec §3）。
pub fn filter_still_valid(filter: &SavedFilter, saved: &[SavedRequest]) -> bool {
    match filter {
        SavedFilter::All => true,
        SavedFilter::Uncategorized => uncategorized_count(saved) > 0,
        SavedFilter::Group(name) => saved
            .iter()
            .any(|r| r.group.as_deref() == Some(name.as_str())),
    }
}

#[cfg(test)]
mod tests {
    use getcat_core::model::{RequestDraft, SavedRequest};

    use super::*;

    fn saved(name: &str, group: Option<&str>) -> SavedRequest {
        let mut r = SavedRequest::new(name, RequestDraft::default());
        r.group = group.map(str::to_string);
        r
    }

    #[test]
    fn normalize_trims_and_maps_empty_to_none() {
        assert_eq!(normalize_group("  订单  "), Some("订单".to_string()));
        assert_eq!(normalize_group("   "), None);
        assert_eq!(normalize_group(""), None);
    }

    #[test]
    fn derive_groups_dedupes_counts_and_sorts_case_insensitively() {
        let list = vec![
            saved("a", Some("zeta")),
            saved("b", Some("Alpha")),
            saved("c", Some("zeta")),
            saved("d", None),
            // 只差大小写 = 两个分类（spec §7），排序相邻
            saved("e", Some("alpha")),
        ];
        let groups = derive_groups(&list);
        assert_eq!(
            groups,
            vec![
                ("Alpha".to_string(), 1),
                ("alpha".to_string(), 1),
                ("zeta".to_string(), 2),
            ]
        );
        assert_eq!(uncategorized_count(&list), 1);
    }

    #[test]
    fn filter_indices_covers_all_three_variants() {
        let list = vec![
            saved("a", Some("g")),
            saved("b", None),
            saved("c", Some("g")),
            saved("d", Some("h")),
        ];
        assert_eq!(filter_indices(&SavedFilter::All, &list), vec![0, 1, 2, 3]);
        assert_eq!(filter_indices(&SavedFilter::Uncategorized, &list), vec![1]);
        assert_eq!(
            filter_indices(&SavedFilter::Group("g".into()), &list),
            vec![0, 2]
        );
    }

    #[test]
    fn filter_validity_tracks_membership() {
        let list = vec![saved("a", Some("g"))];
        assert!(filter_still_valid(&SavedFilter::All, &list));
        assert!(filter_still_valid(&SavedFilter::Group("g".into()), &list));
        assert!(!filter_still_valid(
            &SavedFilter::Group("gone".into()),
            &list
        ));
        // 没有未分类请求时「未分类」也失效
        assert!(!filter_still_valid(&SavedFilter::Uncategorized, &list));
        assert!(filter_still_valid(
            &SavedFilter::Uncategorized,
            &[saved("b", None)]
        ));
    }
}
