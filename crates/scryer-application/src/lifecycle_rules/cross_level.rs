//! Conflicting actions one rule decides for a title and a season or episode
//! of it.
//!
//! A decision holds the heads of one level only, so the same-level check on
//! [`scryer_rules::lifecycle_actions::LifecycleActions::conflict`] never sees
//! a title that is monitored while one of its seasons is deleted. This module
//! pairs the decisions of one title's subjects instead. A subject is paired
//! only with the subjects that enclose it or that it encloses: the title
//! encloses every season and episode, and a season encloses its own episodes.

use std::collections::HashMap;

use scryer_domain::LifecycleActionsConflict;
use scryer_rules::lifecycle::LifecycleSubjectKind as Level;
use scryer_rules::lifecycle_actions::{LifecycleActionHead, LifecycleActions};

use super::declared_actions::LifecycleDeclaredActions;

/// Whether a head the rule declares at `a` conflicts with one it declares at
/// `b`. Read off the declared heads alone, so a rule that cannot pair two
/// levels does no work for them.
pub(crate) fn levels_can_conflict(declared: &LifecycleDeclaredActions, a: Level, b: Level) -> bool {
    a != b
        && declared
            .heads
            .iter()
            .filter(|head| head.level() == a)
            .any(|head| {
                declared
                    .heads
                    .iter()
                    .filter(|other| other.level() == b)
                    .any(|other| head.conflicts(*other))
            })
}

/// Whether any of `heads` conflicts with a head the rule declares at `level`.
pub(crate) fn heads_conflict_with_declared(
    heads: &[LifecycleActionHead],
    declared: &LifecycleDeclaredActions,
    level: Level,
) -> bool {
    heads.iter().any(|head| {
        declared
            .heads
            .iter()
            .filter(|other| other.level() == level)
            .any(|other| head.conflicts(*other))
    })
}

/// Whether any two of the rule's declared heads, at different levels, conflict.
/// A rule acting at one level only never does.
pub(crate) fn cross_level_conflict_possible(declared: &LifecycleDeclaredActions) -> bool {
    [
        (Level::Title, Level::Season),
        (Level::Title, Level::Episode),
        (Level::Season, Level::Episode),
    ]
    .into_iter()
    .any(|(a, b)| levels_can_conflict(declared, a, b))
}

/// The heads a decision at `level` applies, named at that level.
pub(crate) fn heads_at_level(actions: &LifecycleActions, level: Level) -> Vec<LifecycleActionHead> {
    actions
        .heads()
        .into_iter()
        .filter_map(|head| head.at_level(level))
        .collect()
}

/// The first of `own` heads, in execution order, that conflicts with one of
/// `other`, with that head.
pub(crate) fn first_conflict(
    own: &[LifecycleActionHead],
    other: &[LifecycleActionHead],
) -> Option<(LifecycleActionHead, LifecycleActionHead)> {
    LifecycleActionHead::EXECUTION_ORDER
        .into_iter()
        .filter(|head| own.contains(head))
        .find_map(|head| {
            LifecycleActionHead::EXECUTION_ORDER
                .into_iter()
                .filter(|candidate| other.contains(candidate))
                .find(|candidate| head.conflicts(*candidate))
                .map(|candidate| (head, candidate))
        })
}

pub(crate) const fn level_storage_str(level: Level) -> &'static str {
    match level {
        Level::Title => "title",
        Level::Season => "season",
        Level::Episode => "episode",
    }
}

/// The live candidate a subject's decision belongs to.
#[derive(Clone, Debug)]
pub(crate) struct LifecycleDecidedCandidate {
    pub id: String,
    /// The marker the candidate carries now.
    pub actions_conflict: Option<LifecycleActionsConflict>,
}

/// What one evaluation of a title decided for one of its subjects.
#[derive(Clone, Debug)]
pub(crate) struct LifecycleSubjectDecision {
    pub level: Level,
    pub subject_id: String,
    /// The season a season is, or the season an episode is in.
    pub season_id: Option<String>,
    pub heads: Vec<LifecycleActionHead>,
    pub candidate: Option<LifecycleDecidedCandidate>,
}

/// The marker each decision's candidate should carry, by index into
/// `decisions`, for the decisions that have a candidate.
///
/// The title is paired with every season and episode, a season with the title
/// and its own episodes, and an episode with the title and its own season. The
/// first enclosing or contained subject with a conflicting head names the
/// marker, title first, then in the order the subjects were decided. Seasons
/// and episodes are indexed by season once, so the work is linear in the
/// number of subjects.
pub(crate) fn cross_level_markers(
    decisions: &[LifecycleSubjectDecision],
) -> Vec<(usize, Option<LifecycleActionsConflict>)> {
    let title = decisions
        .iter()
        .position(|decision| decision.level == Level::Title);
    let mut seasons: HashMap<&str, usize> = HashMap::new();
    let mut episodes_by_season: HashMap<&str, Vec<usize>> = HashMap::new();
    let mut narrow: Vec<usize> = Vec::new();
    for (index, decision) in decisions.iter().enumerate() {
        match decision.level {
            Level::Title => {}
            Level::Season => {
                narrow.push(index);
                if let Some(season) = decision.season_id.as_deref() {
                    seasons.entry(season).or_insert(index);
                }
            }
            Level::Episode => {
                narrow.push(index);
                if let Some(season) = decision.season_id.as_deref() {
                    episodes_by_season.entry(season).or_default().push(index);
                }
            }
        }
    }
    // Seasons before episodes, each in the order decided.
    narrow.sort_by_key(|index| (decisions[*index].level != Level::Season, *index));

    let marker = |own: usize, other: usize| -> Option<LifecycleActionsConflict> {
        let (head, other_head) = first_conflict(&decisions[own].heads, &decisions[other].heads)?;
        Some(LifecycleActionsConflict {
            head: head.rule_name().to_string(),
            other_subject_kind: level_storage_str(decisions[other].level).to_string(),
            other_subject_id: decisions[other].subject_id.clone(),
            other_head: other_head.rule_name().to_string(),
        })
    };

    decisions
        .iter()
        .enumerate()
        .filter(|(_, decision)| decision.candidate.is_some())
        .map(|(index, decision)| {
            let found = match decision.level {
                Level::Title => narrow.iter().find_map(|other| marker(index, *other)),
                Level::Season => title.and_then(|title| marker(index, title)).or_else(|| {
                    decision
                        .season_id
                        .as_deref()
                        .and_then(|season| episodes_by_season.get(season))
                        .and_then(|episodes| {
                            episodes.iter().find_map(|other| marker(index, *other))
                        })
                }),
                Level::Episode => title.and_then(|title| marker(index, title)).or_else(|| {
                    decision
                        .season_id
                        .as_deref()
                        .and_then(|season| seasons.get(season))
                        .and_then(|other| marker(index, *other))
                }),
            };
            (index, found)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decision(
        level: Level,
        id: &str,
        season: Option<&str>,
        heads: &[LifecycleActionHead],
    ) -> LifecycleSubjectDecision {
        LifecycleSubjectDecision {
            level,
            subject_id: id.to_string(),
            season_id: season.map(str::to_string),
            heads: heads.to_vec(),
            candidate: Some(LifecycleDecidedCandidate {
                id: format!("candidate-{id}"),
                actions_conflict: None,
            }),
        }
    }

    fn marked(decisions: &[LifecycleSubjectDecision]) -> Vec<(String, Option<(String, String)>)> {
        cross_level_markers(decisions)
            .into_iter()
            .map(|(index, marker)| {
                (
                    decisions[index].subject_id.clone(),
                    marker.map(|marker| (marker.other_subject_id, marker.other_head)),
                )
            })
            .collect()
    }

    #[test]
    fn a_title_and_a_season_with_conflicting_heads_mark_each_other() {
        use LifecycleActionHead::*;
        let decisions = [
            decision(Level::Title, "show", None, &[Monitor]),
            decision(Level::Season, "s1", Some("s1"), &[DeleteSeasonFiles]),
            decision(Level::Season, "s2", Some("s2"), &[NotifySeason]),
        ];
        assert_eq!(
            marked(&decisions),
            vec![
                (
                    "show".into(),
                    Some(("s1".into(), "delete_season_files".into()))
                ),
                ("s1".into(), Some(("show".into(), "monitor".into()))),
                ("s2".into(), None),
            ]
        );
    }

    #[test]
    fn a_season_pairs_only_with_its_own_episodes() {
        use LifecycleActionHead::*;
        let decisions = [
            decision(Level::Season, "s1", Some("s1"), &[MonitorSeason]),
            decision(Level::Episode, "e1", Some("s1"), &[DeleteEpisodeFiles]),
            decision(Level::Episode, "e2", Some("s2"), &[DeleteEpisodeFiles]),
        ];
        assert_eq!(
            marked(&decisions),
            vec![
                (
                    "s1".into(),
                    Some(("e1".into(), "delete_episode_files".into()))
                ),
                ("e1".into(), Some(("s1".into(), "monitor_season".into()))),
                ("e2".into(), None),
            ]
        );
    }

    #[test]
    fn allowed_mixes_mark_nothing() {
        use LifecycleActionHead::*;
        let decisions = [
            decision(Level::Title, "show", None, &[DeleteFiles]),
            decision(Level::Season, "s1", Some("s1"), &[DeleteSeasonFiles]),
            decision(Level::Episode, "e1", Some("s1"), &[UnmonitorEpisode]),
        ];
        assert!(
            marked(&decisions)
                .iter()
                .all(|(_, marker)| marker.is_none())
        );
        let decisions = [
            decision(Level::Title, "show", None, &[Unmonitor]),
            decision(Level::Episode, "e1", Some("s1"), &[DeleteEpisodeFiles]),
        ];
        assert!(
            marked(&decisions)
                .iter()
                .all(|(_, marker)| marker.is_none())
        );
    }

    #[test]
    fn a_subject_without_a_candidate_marks_others_but_is_not_marked() {
        use LifecycleActionHead::*;
        let mut title = decision(Level::Title, "show", None, &[Monitor]);
        title.candidate = None;
        let decisions = [
            title,
            decision(Level::Episode, "e1", Some("s1"), &[DeleteEpisodeFiles]),
        ];
        assert_eq!(
            marked(&decisions),
            vec![("e1".into(), Some(("show".into(), "monitor".into())))]
        );
    }

    #[test]
    fn the_static_skip_reads_the_declared_heads() {
        use LifecycleActionHead::*;
        let mut declared = LifecycleDeclaredActions::none();
        declared.heads = vec![Monitor, Unmonitor];
        assert!(!cross_level_conflict_possible(&declared));
        declared.heads = vec![DeleteFiles, DeleteSeasonFiles, UnmonitorEpisode];
        assert!(!cross_level_conflict_possible(&declared));
        declared.heads = vec![Monitor, DeleteSeasonFiles];
        assert!(cross_level_conflict_possible(&declared));
        assert!(levels_can_conflict(&declared, Level::Title, Level::Season));
        assert!(!levels_can_conflict(
            &declared,
            Level::Title,
            Level::Episode
        ));
    }
}
