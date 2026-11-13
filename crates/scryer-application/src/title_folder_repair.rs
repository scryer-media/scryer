//! Untangling title folders that collide, so every title folder is unique and
//! none lies inside another.
//!
//! Overlapping titles form connected components: two titles are connected
//! when their folders are the same folder or one lies inside the other. Inside
//! a component the decision is made one pair at a time:
//!
//! 1. the title with strictly more of its own primary media files under its
//!    own folder keeps it;
//! 2. on a tie over the same folder, the earliest-added title keeps it;
//! 3. on a tie over a nested pair, the inner title keeps its folder and the
//!    outer one is cleared.
//!
//! Titles claim their folders strongest first; a title whose folder collides
//! with one already kept is cleared, so nothing left overlaps. Clearing only ever forgets a recorded folder; no
//! file and no media row is involved.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};

use crate::title_folder_keys::{
    TitleFolderKeyRelation, title_folder_key, title_folder_key_ancestors,
    title_folder_key_descendant_bounds, title_folder_key_relation,
};

/// Why a title's recorded folder was cleared, stored beside the title.
pub mod reasons {
    /// Another title recorded the same folder and kept it.
    pub const SHARED_FOLDER: &str = "TITLE_FOLDER_CONFLICT_SHARED_FOLDER";
    /// The folder lay inside another title's folder, which was kept.
    pub const INSIDE_KEPT_FOLDER: &str = "TITLE_FOLDER_CONFLICT_INSIDE_KEPT_FOLDER";
    /// The folder held another title's folder, which was kept.
    pub const HOLDS_KEPT_FOLDER: &str = "TITLE_FOLDER_CONFLICT_HOLDS_KEPT_FOLDER";
    /// The folder was a library root, or held one.
    pub const LIBRARY_ROOT: &str = "TITLE_FOLDER_CONFLICT_LIBRARY_ROOT";
    /// Every reason a title folder can be cleared for.
    pub const ALL: &[&str] = &[
        SHARED_FOLDER,
        INSIDE_KEPT_FOLDER,
        HOLDS_KEPT_FOLDER,
        LIBRARY_ROOT,
    ];
}

/// One title taking part in the repair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TitleFolderRepairCandidate {
    pub title_id: String,
    pub folder_path: String,
    pub key: String,
    /// The title's own primary media files under its own folder.
    pub own_file_count: u64,
    pub created_at: DateTime<Utc>,
}

/// A title that loses its recorded folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClearedTitleFolder {
    pub title_id: String,
    pub lost_folder: String,
    pub reason: &'static str,
    /// The title it lost to; for a library root conflict, the root's key.
    pub kept_by: String,
}

/// One connected group of overlapping titles and what the repair does to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TitleFolderRepairComponent {
    /// Every title in the group, ordered by key then id.
    pub members: Vec<TitleFolderRepairCandidate>,
    /// The titles that lose their folder, in the order they were cleared.
    pub cleared: Vec<ClearedTitleFolder>,
}

/// The groups of titles whose recorded folders overlap. Titles that overlap
/// nothing are not returned.
pub fn title_folder_overlap_components(titles: &[TitleFolderRepairCandidate]) -> Vec<Vec<usize>> {
    let mut by_key: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (index, title) in titles.iter().enumerate() {
        by_key.entry(title.key.as_str()).or_default().push(index);
    }
    let mut parent = (0..titles.len()).collect::<Vec<_>>();
    fn find(parent: &mut [usize], index: usize) -> usize {
        let mut root = index;
        while parent[root] != root {
            root = parent[root];
        }
        let mut current = index;
        while parent[current] != root {
            let next = parent[current];
            parent[current] = root;
            current = next;
        }
        root
    }
    let union = |parent: &mut Vec<usize>, left: usize, right: usize| {
        let (left, right) = (find(parent, left), find(parent, right));
        if left != right {
            parent[left.max(right)] = left.min(right);
        }
    };
    for indices in by_key.values() {
        for pair in indices.windows(2) {
            union(&mut parent, pair[0], pair[1]);
        }
    }
    for (index, title) in titles.iter().enumerate() {
        for ancestor in title_folder_key_ancestors(&title.key) {
            if let Some(owners) = by_key.get(ancestor.as_str()) {
                union(&mut parent, index, owners[0]);
            }
        }
    }
    let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for index in 0..titles.len() {
        let root = find(&mut parent, index);
        groups.entry(root).or_default().push(index);
    }
    groups
        .into_values()
        .filter(|group| group.len() > 1)
        .collect()
}

/// The order titles claim folders in: most own files first; among equals the
/// deeper folder first (so the inner title of a nested pair wins a tie), then
/// the earliest added, then the title id.
fn claim_order(
    left: &(usize, &TitleFolderRepairCandidate),
    right: &(usize, &TitleFolderRepairCandidate),
) -> std::cmp::Ordering {
    let (left_depth, left) = left;
    let (right_depth, right) = right;
    right
        .own_file_count
        .cmp(&left.own_file_count)
        .then_with(|| right_depth.cmp(left_depth))
        .then_with(|| left.created_at.cmp(&right.created_at))
        .then_with(|| left.title_id.cmp(&right.title_id))
}

/// The kept title `key` collides with, and how `key` sits relative to it: the
/// same folder first, then the nearest kept folder holding it, then the first
/// kept folder (by key) inside it.
fn kept_overlap(
    kept: &BTreeMap<String, usize>,
    key: &str,
) -> Option<(usize, TitleFolderKeyRelation)> {
    if let Some(&index) = kept.get(key) {
        return Some((index, TitleFolderKeyRelation::Same));
    }
    if let Some(&index) = title_folder_key_ancestors(key)
        .iter()
        .rev()
        .find_map(|ancestor| kept.get(ancestor.as_str()))
    {
        return Some((index, TitleFolderKeyRelation::Inside));
    }
    let (low, high) = title_folder_key_descendant_bounds(key)?;
    kept.range(low..high)
        .next()
        .map(|(_, &index)| (index, TitleFolderKeyRelation::Contains))
}

fn reason_for_loser(relation_of_loser: TitleFolderKeyRelation) -> &'static str {
    match relation_of_loser {
        TitleFolderKeyRelation::Same => reasons::SHARED_FOLDER,
        TitleFolderKeyRelation::Inside => reasons::INSIDE_KEPT_FOLDER,
        TitleFolderKeyRelation::Contains => reasons::HOLDS_KEPT_FOLDER,
    }
}

/// Decide one component. Titles claim their folders strongest first, in
/// [`claim_order`]; a title keeps its folder only when no folder already kept
/// is the same folder, holds it or lies inside it, and is otherwise cleared
/// against that kept title. For any two overlapping titles this is the pair
/// rule above, the kept folders never overlap, and every cleared title names a
/// title that really kept its folder. Each lookup goes through an ordered map,
/// so a component of `n` titles costs about `n log n`.
pub fn resolve_title_folder_component(
    mut members: Vec<TitleFolderRepairCandidate>,
) -> TitleFolderRepairComponent {
    members.sort_by(|left, right| (&left.key, &left.title_id).cmp(&(&right.key, &right.title_id)));
    let mut order = members
        .iter()
        .enumerate()
        .map(|(index, member)| (index, title_folder_key_ancestors(&member.key).len()))
        .collect::<Vec<_>>();
    order.sort_by(|(left, left_depth), (right, right_depth)| {
        claim_order(
            &(*left_depth, &members[*left]),
            &(*right_depth, &members[*right]),
        )
    });
    let mut kept = BTreeMap::<String, usize>::new();
    let mut cleared = Vec::new();
    for (index, _) in order {
        let member = &members[index];
        match kept_overlap(&kept, &member.key) {
            None => {
                kept.insert(member.key.clone(), index);
            }
            Some((winner, relation)) => cleared.push(ClearedTitleFolder {
                title_id: member.title_id.clone(),
                lost_folder: member.folder_path.clone(),
                reason: reason_for_loser(relation),
                kept_by: members[winner].title_id.clone(),
            }),
        }
    }
    TitleFolderRepairComponent { members, cleared }
}

/// The titles whose folder is a library root or holds one, in `titles` order.
/// Each is reported against the first such root by key. These are decided
/// before any overlap between titles, so a root-sized folder never wins a
/// shared or nested folder from a real title folder.
pub fn title_folder_library_root_conflicts(
    titles: &[TitleFolderRepairCandidate],
    root_paths: &[String],
) -> Vec<ClearedTitleFolder> {
    let mut root_keys = root_paths
        .iter()
        .filter_map(|root| title_folder_key(root))
        .collect::<Vec<_>>();
    root_keys.sort();
    root_keys.dedup();
    titles
        .iter()
        .filter_map(|title| {
            let root = root_keys.iter().find(|root| {
                matches!(
                    title_folder_key_relation(&title.key, root),
                    Some(TitleFolderKeyRelation::Same | TitleFolderKeyRelation::Contains)
                )
            })?;
            Some(ClearedTitleFolder {
                title_id: title.title_id.clone(),
                lost_folder: title.folder_path.clone(),
                reason: reasons::LIBRARY_ROOT,
                kept_by: root.clone(),
            })
        })
        .collect()
}

/// Every overlapping component among `titles`, decided.
pub fn plan_title_folder_repair(
    titles: &[TitleFolderRepairCandidate],
) -> Vec<TitleFolderRepairComponent> {
    title_folder_overlap_components(titles)
        .into_iter()
        .map(|group| {
            resolve_title_folder_component(
                group
                    .into_iter()
                    .map(|index| titles[index].clone())
                    .collect(),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn candidate(id: &str, folder: &str, files: u64, day: u32) -> TitleFolderRepairCandidate {
        TitleFolderRepairCandidate {
            title_id: id.into(),
            folder_path: folder.into(),
            key: title_folder_key(folder).unwrap(),
            own_file_count: files,
            created_at: Utc.with_ymd_and_hms(2026, 1, day, 0, 0, 0).unwrap(),
        }
    }

    fn cleared(plan: &[TitleFolderRepairComponent]) -> Vec<(String, &'static str, String)> {
        plan.iter()
            .flat_map(|component| component.cleared.iter())
            .map(|cleared| {
                (
                    cleared.title_id.clone(),
                    cleared.reason,
                    cleared.kept_by.clone(),
                )
            })
            .collect()
    }

    #[test]
    fn reasons_are_distinct_prefixed_and_screaming_snake_case() {
        let mut seen = std::collections::BTreeSet::new();
        for reason in reasons::ALL {
            assert!(seen.insert(*reason), "duplicate {reason}");
            assert!(reason.starts_with("TITLE_FOLDER_CONFLICT_"), "{reason}");
            assert!(
                reason
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'),
                "{reason}"
            );
        }
    }

    #[test]
    fn duplicate_folder_goes_to_the_title_with_more_files() {
        let plan = plan_title_folder_repair(&[
            candidate("emberfall-a", "/library/Emberfall", 1, 1),
            candidate("emberfall-b", "/library/Emberfall/", 3, 2),
        ]);
        assert_eq!(
            cleared(&plan),
            [(
                "emberfall-a".into(),
                reasons::SHARED_FOLDER,
                "emberfall-b".into()
            )]
        );
    }

    #[test]
    fn duplicate_folder_tie_goes_to_the_earliest_added_title() {
        let plan = plan_title_folder_repair(&[
            candidate("emberfall-late", "/library/Emberfall", 2, 5),
            candidate("emberfall-early", "/library/Emberfall", 2, 3),
        ]);
        assert_eq!(
            cleared(&plan),
            [(
                "emberfall-late".into(),
                reasons::SHARED_FOLDER,
                "emberfall-early".into()
            )]
        );
    }

    #[test]
    fn nested_pair_goes_to_the_title_with_more_files() {
        let plan = plan_title_folder_repair(&[
            candidate("outer", "/library/Cinderwake", 4, 1),
            candidate("inner", "/library/Cinderwake/Cinderwake II", 1, 2),
        ]);
        assert_eq!(
            cleared(&plan),
            [("inner".into(), reasons::INSIDE_KEPT_FOLDER, "outer".into())]
        );
    }

    #[test]
    fn nested_tie_keeps_the_inner_title() {
        let plan = plan_title_folder_repair(&[
            candidate("outer", "/library/Cinderwake", 0, 1),
            candidate("inner", "/library/Cinderwake/Cinderwake II", 0, 2),
        ]);
        assert_eq!(
            cleared(&plan),
            [("outer".into(), reasons::HOLDS_KEPT_FOLDER, "inner".into())]
        );
    }

    #[test]
    fn three_title_chain_is_one_component_and_ends_without_overlap() {
        let titles = [
            candidate("top", "/library/Ashgrove", 5, 1),
            candidate("middle", "/library/Ashgrove/Part", 0, 2),
            candidate("bottom", "/library/Ashgrove/Part/Deep", 3, 3),
            candidate("elsewhere", "/library/Emberfall", 0, 4),
        ];
        let plan = plan_title_folder_repair(&titles);
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].members.len(), 3);
        assert_eq!(
            cleared(&plan),
            [
                ("bottom".into(), reasons::INSIDE_KEPT_FOLDER, "top".into()),
                ("middle".into(), reasons::INSIDE_KEPT_FOLDER, "top".into()),
            ]
        );
    }

    #[test]
    fn a_kept_inner_title_survives_when_its_outer_loses() {
        let plan = plan_title_folder_repair(&[
            candidate("outer", "/library/Ashgrove", 0, 1),
            candidate("inner-one", "/library/Ashgrove/One", 2, 2),
            candidate("inner-two", "/library/Ashgrove/Two", 1, 3),
        ]);
        assert_eq!(
            cleared(&plan),
            [(
                "outer".into(),
                reasons::HOLDS_KEPT_FOLDER,
                "inner-one".into()
            )]
        );
    }

    #[test]
    fn disjoint_and_case_distinct_posix_folders_are_left_alone() {
        assert!(
            plan_title_folder_repair(&[
                candidate("a", "/library/Emberfall", 0, 1),
                candidate("b", "/library/emberfall", 0, 2),
                candidate("c", "/library/Emberfall 2", 0, 3),
            ])
            .is_empty()
        );
    }

    #[test]
    fn the_decision_does_not_depend_on_input_order() {
        let titles = vec![
            candidate("top", "/library/Ashgrove", 1, 1),
            candidate("middle", "/library/Ashgrove/Part", 1, 2),
            candidate("twin", "/library/Ashgrove/Part", 1, 2),
            candidate("bottom", "/library/Ashgrove/Part/Deep", 1, 3),
        ];
        let forward = cleared(&plan_title_folder_repair(&titles));
        let mut reversed = titles.clone();
        reversed.reverse();
        assert_eq!(forward, cleared(&plan_title_folder_repair(&reversed)));
    }

    #[test]
    fn a_folder_equal_to_or_holding_a_root_is_cleared() {
        let titles = [
            candidate("emberfall-root", "/srv/media/", 3, 1),
            candidate("cinderwake-parent", "/srv", 2, 1),
            candidate("ashgrove-inside", "/srv/media/Ashgrove", 1, 1),
            candidate("ashgrove-elsewhere", "/archive/Ashgrove", 0, 1),
        ];
        let roots = ["/srv/media".to_string(), "/srv/media/".to_string()];
        let cleared = title_folder_library_root_conflicts(&titles, &roots)
            .into_iter()
            .map(|cleared| {
                (
                    cleared.title_id,
                    cleared.reason,
                    cleared.lost_folder,
                    cleared.kept_by,
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            cleared,
            [
                (
                    "emberfall-root".to_string(),
                    reasons::LIBRARY_ROOT,
                    "/srv/media/".to_string(),
                    "/srv/media".to_string()
                ),
                (
                    "cinderwake-parent".to_string(),
                    reasons::LIBRARY_ROOT,
                    "/srv".to_string(),
                    "/srv/media".to_string()
                ),
            ]
        );
    }

    #[test]
    fn root_conflicts_follow_the_path_shape() {
        let titles = [
            candidate("emberfall-windows", r"D:\MEDIA", 0, 1),
            candidate("cinderwake-posix", "/Media", 0, 1),
        ];
        let roots = [r"d:\media\".to_string(), "/media".to_string()];
        let cleared = title_folder_library_root_conflicts(&titles, &roots)
            .into_iter()
            .map(|cleared| cleared.title_id)
            .collect::<Vec<_>>();
        assert_eq!(cleared, ["emberfall-windows"]);
    }

    #[test]
    fn no_roots_or_blank_roots_clear_nothing() {
        let titles = [candidate("emberfall", "/srv/media/Emberfall", 0, 1)];
        assert!(title_folder_library_root_conflicts(&titles, &[]).is_empty());
        assert!(title_folder_library_root_conflicts(&titles, &["  ".to_string()]).is_empty());
    }

    #[test]
    fn an_outer_title_beaten_by_one_inner_does_not_take_another_inner_with_it() {
        let plan = plan_title_folder_repair(&[
            candidate("ashgrove-outer", "/tv/Ashgrove", 3, 1),
            candidate("ashgrove-specials", "/tv/Ashgrove/Specials", 2, 2),
            candidate("ashgrove-season", "/tv/Ashgrove/Season 1", 5, 3),
        ]);
        assert_eq!(
            cleared(&plan),
            [(
                "ashgrove-outer".into(),
                reasons::HOLDS_KEPT_FOLDER,
                "ashgrove-season".into()
            )]
        );
    }

    #[test]
    fn twenty_thousand_children_under_one_outer_resolve() {
        let mut titles = vec![candidate("ashgrove-outer", "/tv/Ashgrove", 1, 1)];
        for child in 0..20_000 {
            titles.push(candidate(
                &format!("ashgrove-{child:05}"),
                &format!("/tv/Ashgrove/Child {child:05}"),
                0,
                2,
            ));
        }
        let plan = plan_title_folder_repair(&titles);
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].members.len(), 20_001);
        assert_eq!(plan[0].cleared.len(), 20_000);
        assert!(plan[0].cleared.iter().all(|cleared| {
            cleared.reason == reasons::INSIDE_KEPT_FOLDER && cleared.kept_by == "ashgrove-outer"
        }));
    }

    #[test]
    fn twenty_thousand_titles_on_one_folder_resolve() {
        let titles = (0..20_000)
            .map(|index| candidate(&format!("cinderwake-{index:05}"), "/tv/Cinderwake", 0, 1))
            .collect::<Vec<_>>();
        let plan = plan_title_folder_repair(&titles);
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].cleared.len(), 19_999);
        assert!(plan[0].cleared.iter().all(|cleared| {
            cleared.reason == reasons::SHARED_FOLDER && cleared.kept_by == "cinderwake-00000"
        }));
    }

    #[test]
    fn kept_folders_never_overlap() {
        let titles = [
            candidate("a", "/tv/Emberfall", 2, 1),
            candidate("b", "/tv/Emberfall/One", 2, 2),
            candidate("c", "/tv/Emberfall/One/Deep", 7, 3),
            candidate("d", "/tv/Emberfall/Two", 1, 4),
            candidate("e", "/tv/Emberfall", 4, 5),
        ];
        let plan = plan_title_folder_repair(&titles);
        let cleared_ids = cleared(&plan)
            .into_iter()
            .map(|(id, _, _)| id)
            .collect::<Vec<_>>();
        let kept = titles
            .iter()
            .filter(|title| !cleared_ids.contains(&title.title_id))
            .collect::<Vec<_>>();
        for (index, left) in kept.iter().enumerate() {
            for right in &kept[index + 1..] {
                assert_eq!(title_folder_key_relation(&left.key, &right.key), None);
            }
        }
        for (_, _, kept_by) in cleared(&plan) {
            assert!(!cleared_ids.contains(&kept_by), "{kept_by} was cleared");
        }
    }
}
