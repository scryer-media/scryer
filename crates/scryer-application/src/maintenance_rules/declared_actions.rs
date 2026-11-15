//! The actions a maintenance rule declares in its own Rego source.
//!
//! A rule names what happens to a subject by defining action heads
//! (`unmonitor if { … }`, `add_tags := ["stale"] if { … }`). Two things are
//! derived from that here:
//!
//! - [`MaintenanceDeclaredActions`] is what a revision *can* do, read off the
//!   source without running it. It is what a revision stores in its action
//!   column, what arming is derived from, and what authoring shows.
//! - [`build_declared_action_sequence`] turns what one evaluation *decided*
//!   for one subject into the step sequence the sequence executor runs. The
//!   order of steps is fixed by the host, never by the author.

use std::collections::HashSet;

use scryer_rules::maintenance_actions::{
    MaintenanceActionHead, MaintenanceActionScan, MaintenanceActions,
};
use serde::{Deserialize, Serialize};

use scryer_domain::MaintenanceRuleSubjectKind;

use super::MaintenanceSubjectKind;
use super::action_sequence::{
    MaintenanceActionSequence, MaintenanceActionSequenceError, MaintenanceActionStep,
    MaintenanceActionStepKind, MaintenanceActionStepParameters, MaintenanceSearchCondition,
};

/// Schema of the stored declared-actions document. Revisions written before
/// it hold a v1 action spec or a v2 action sequence instead.
pub const MAINTENANCE_DECLARED_ACTIONS_SCHEMA_VERSION: u32 = 3;

/// What a revision's source can do, as stored on the revision.
///
/// Every list keeps source order and holds each value once.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MaintenanceDeclaredActions {
    pub schema_version: u32,
    /// Every action head the source defines.
    pub heads: Vec<MaintenanceActionHead>,
    /// Whether any defined head deletes media.
    pub destructive: bool,
    /// Every profile name written for `set_quality_profile`.
    #[serde(default)]
    pub quality_profiles: Vec<String>,
    /// Every tag written for `add_tags`.
    #[serde(default)]
    pub add_tags: Vec<String>,
    /// Every tag written for `remove_tags`.
    #[serde(default)]
    pub remove_tags: Vec<String>,
}

impl MaintenanceDeclaredActions {
    /// The document for a source's static scan.
    pub fn from_scan(scan: &MaintenanceActionScan) -> Self {
        let mut heads: Vec<MaintenanceActionHead> = scan.defined_heads.iter().copied().collect();
        heads.sort_by_key(|head| {
            scan.head_positions
                .get(head)
                .copied()
                .unwrap_or((usize::MAX, usize::MAX))
        });
        fn distinct(
            literals: &[scryer_rules::maintenance_actions::MaintenanceActionLiteral],
        ) -> Vec<String> {
            let mut seen = HashSet::new();
            literals
                .iter()
                .filter(|literal| seen.insert(literal.value.as_str()))
                .map(|literal| literal.value.clone())
                .collect()
        }
        Self {
            schema_version: MAINTENANCE_DECLARED_ACTIONS_SCHEMA_VERSION,
            destructive: heads.iter().any(|head| head.is_destructive()),
            heads,
            quality_profiles: distinct(&scan.quality_profiles),
            add_tags: distinct(&scan.add_tags),
            remove_tags: distinct(&scan.remove_tags),
        }
    }

    pub fn defines(&self, head: MaintenanceActionHead) -> bool {
        self.heads.contains(&head)
    }

    /// Whether any defined head is a reversible action of its own. The
    /// unmonitor a delete head implies is part of that delete and rides the
    /// destructive gate, the way the single delete actions always did.
    pub fn defines_reversible_action(&self) -> bool {
        self.heads.iter().any(|head| !head.is_destructive())
    }
}

/// Whether a rule of this scope can run this head. Seasons, episodes, and
/// storage-root rules act only on monitoring and files; everything else is a
/// whole-title operation.
pub fn head_supported_for_scope(
    head: MaintenanceActionHead,
    scope: MaintenanceRuleSubjectKind,
    storage_root_selected: bool,
) -> bool {
    let narrow = storage_root_selected || scope != MaintenanceRuleSubjectKind::Title;
    if !narrow {
        return true;
    }
    if head == MaintenanceActionHead::DeleteTitle && scope != MaintenanceRuleSubjectKind::Title {
        return false;
    }
    matches!(
        head,
        MaintenanceActionHead::Unmonitor | MaintenanceActionHead::DeleteFiles
    )
}

/// The heads one evaluation decided, in execution order, as shown to the
/// operator. Deleting files always unmonitors first, so it shows both.
pub fn decided_heads_in_execution_order(
    actions: &MaintenanceActions,
) -> Vec<MaintenanceActionHead> {
    let mut heads = Vec::new();
    if !actions.add_tags.is_empty() {
        heads.push(MaintenanceActionHead::AddTags);
    }
    if !actions.remove_tags.is_empty() {
        heads.push(MaintenanceActionHead::RemoveTags);
    }
    if actions.set_quality_profile.is_some() {
        heads.push(MaintenanceActionHead::SetQualityProfile);
    }
    if actions.search {
        heads.push(MaintenanceActionHead::Search);
    }
    if actions.unmonitor || (actions.delete_files && !actions.delete_title) {
        heads.push(MaintenanceActionHead::Unmonitor);
    }
    if actions.delete_files && !actions.delete_title {
        heads.push(MaintenanceActionHead::DeleteFiles);
    }
    if actions.delete_title {
        heads.push(MaintenanceActionHead::DeleteTitle);
    }
    heads
}

/// The step ids [`build_declared_action_sequence`] produces for these
/// actions, without resolving anything. A finished sequence records its step
/// ids, so this tells whether a later decision would run different steps.
pub fn declared_step_ids(actions: &MaintenanceActions) -> Vec<String> {
    let delete_files = actions.delete_files && !actions.delete_title;
    [
        (
            !actions.add_tags.is_empty(),
            MaintenanceActionStepKind::AddTags,
        ),
        (
            !actions.remove_tags.is_empty(),
            MaintenanceActionStepKind::RemoveTags,
        ),
        (
            actions.set_quality_profile.is_some(),
            MaintenanceActionStepKind::ChangeQualityProfile,
        ),
        (actions.search, MaintenanceActionStepKind::Search),
        (
            actions.unmonitor || delete_files,
            MaintenanceActionStepKind::Unmonitor,
        ),
        (delete_files, MaintenanceActionStepKind::DeleteFiles),
        (
            actions.delete_title,
            MaintenanceActionStepKind::DeleteTitleAndFiles,
        ),
    ]
    .into_iter()
    .filter(|(present, _)| *present)
    .map(|(_, kind)| kind.descriptor().id.to_string())
    .collect()
}

/// Read the decided actions a candidate stored. A candidate written before
/// rules declared their own actions has none.
pub fn decided_actions_from_json(json: Option<&str>) -> Option<MaintenanceActions> {
    json.and_then(|json| serde_json::from_str(json).ok())
}

/// The decided actions as stored on a candidate.
pub fn decided_actions_to_json(actions: &MaintenanceActions) -> String {
    serde_json::to_string(actions).expect("maintenance actions always serialize")
}

fn step(
    kind: MaintenanceActionStepKind,
    parameters: MaintenanceActionStepParameters,
) -> MaintenanceActionStep {
    MaintenanceActionStep {
        id: kind.descriptor().id.to_string(),
        kind,
        parameters,
    }
}

/// The step sequence for what one evaluation decided for one subject.
///
/// Steps run in a fixed order: add tags, remove tags, set quality profile,
/// search, unmonitor, delete files, delete title. Deleting files unmonitors
/// the subject first. Deleting the title removes its files with it, so a
/// subject that is also told to delete files gets the title deletion alone.
/// A search decided together with a profile change runs only when the change
/// took effect.
///
/// `quality_profile_id` is the id the decided profile name resolved to; it
/// must be present when a profile was decided. The result is validated with
/// the same checks a stored sequence passes, so a combination the executor
/// cannot run safely is an error here, never a partial run.
pub fn build_declared_action_sequence(
    actions: &MaintenanceActions,
    subject: MaintenanceSubjectKind,
    quality_profile_id: Option<&str>,
) -> Result<MaintenanceActionSequence, MaintenanceActionSequenceError> {
    let mut steps = Vec::new();
    if !actions.add_tags.is_empty() {
        steps.push(step(
            MaintenanceActionStepKind::AddTags,
            MaintenanceActionStepParameters::Tags {
                tags: actions.add_tags.clone(),
            },
        ));
    }
    if !actions.remove_tags.is_empty() {
        steps.push(step(
            MaintenanceActionStepKind::RemoveTags,
            MaintenanceActionStepParameters::Tags {
                tags: actions.remove_tags.clone(),
            },
        ));
    }
    let profile_changes = actions.set_quality_profile.is_some();
    if profile_changes {
        let Some(target) = quality_profile_id else {
            return Err(MaintenanceActionSequenceError::InvalidEnvelope(
                "the decided quality profile has no resolved id".into(),
            ));
        };
        steps.push(step(
            MaintenanceActionStepKind::ChangeQualityProfile,
            MaintenanceActionStepParameters::ChangeQualityProfile {
                target_quality_profile_id: target.to_string(),
            },
        ));
    }
    if actions.search {
        steps.push(step(
            MaintenanceActionStepKind::Search,
            MaintenanceActionStepParameters::Search {
                condition: if profile_changes {
                    MaintenanceSearchCondition::PreviousProfileChanged
                } else {
                    MaintenanceSearchCondition::Unconditional
                },
            },
        ));
    }
    let delete_files = actions.delete_files && !actions.delete_title;
    if actions.unmonitor || delete_files {
        steps.push(step(
            MaintenanceActionStepKind::Unmonitor,
            MaintenanceActionStepParameters::Unmonitor {
                include_descendants: true,
            },
        ));
    }
    if delete_files {
        steps.push(step(
            MaintenanceActionStepKind::DeleteFiles,
            MaintenanceActionStepParameters::None,
        ));
    }
    if actions.delete_title {
        steps.push(step(
            MaintenanceActionStepKind::DeleteTitleAndFiles,
            MaintenanceActionStepParameters::None,
        ));
    }
    let sequence = MaintenanceActionSequence::new(steps);
    sequence.validate(subject)?;
    Ok(sequence)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(sequence: &MaintenanceActionSequence) -> Vec<MaintenanceActionStepKind> {
        sequence.steps.iter().map(|step| step.kind).collect()
    }

    #[test]
    fn steps_run_in_the_fixed_order_whatever_the_author_wrote() {
        let actions = MaintenanceActions {
            unmonitor: true,
            search: true,
            set_quality_profile: Some("Fixture HD".into()),
            add_tags: vec!["stale".into()],
            remove_tags: vec!["fresh".into()],
            ..MaintenanceActions::default()
        };
        let sequence =
            build_declared_action_sequence(&actions, MaintenanceSubjectKind::Show, Some("7"))
                .expect("valid");
        assert_eq!(
            kinds(&sequence),
            vec![
                MaintenanceActionStepKind::AddTags,
                MaintenanceActionStepKind::RemoveTags,
                MaintenanceActionStepKind::ChangeQualityProfile,
                MaintenanceActionStepKind::Search,
                MaintenanceActionStepKind::Unmonitor,
            ]
        );
        assert_eq!(
            sequence.search_condition(3),
            Some(MaintenanceSearchCondition::PreviousProfileChanged)
        );
    }

    #[test]
    fn step_ids_match_the_built_sequence_for_every_head_combination() {
        for bits in 0u32..(1 << 7) {
            let actions = MaintenanceActions {
                unmonitor: bits & 1 != 0,
                delete_files: bits & 2 != 0,
                delete_title: bits & 4 != 0,
                search: bits & 8 != 0,
                set_quality_profile: (bits & 16 != 0).then(|| "Fixture HD".to_string()),
                add_tags: if bits & 32 != 0 {
                    vec!["stale".into()]
                } else {
                    Vec::new()
                },
                remove_tags: if bits & 64 != 0 {
                    vec!["fresh".into()]
                } else {
                    Vec::new()
                },
            };
            let Ok(sequence) =
                build_declared_action_sequence(&actions, MaintenanceSubjectKind::Show, Some("7"))
            else {
                continue;
            };
            let built: Vec<String> = sequence.steps.iter().map(|step| step.id.clone()).collect();
            assert_eq!(built, declared_step_ids(&actions), "{actions:?}");
        }
    }

    #[test]
    fn a_search_without_a_profile_change_is_unconditional() {
        let actions = MaintenanceActions {
            search: true,
            ..MaintenanceActions::default()
        };
        let sequence =
            build_declared_action_sequence(&actions, MaintenanceSubjectKind::Movie, None)
                .expect("valid");
        assert_eq!(
            sequence.search_condition(0),
            Some(MaintenanceSearchCondition::Unconditional)
        );
    }

    #[test]
    fn deleting_files_unmonitors_first() {
        let actions = MaintenanceActions {
            delete_files: true,
            ..MaintenanceActions::default()
        };
        for subject in MaintenanceSubjectKind::ALL {
            let sequence = build_declared_action_sequence(&actions, *subject, None)
                .unwrap_or_else(|error| panic!("{subject:?}: {error}"));
            assert_eq!(
                kinds(&sequence),
                vec![
                    MaintenanceActionStepKind::Unmonitor,
                    MaintenanceActionStepKind::DeleteFiles
                ],
                "{subject:?}"
            );
        }
        assert_eq!(
            decided_heads_in_execution_order(&actions),
            vec![
                MaintenanceActionHead::Unmonitor,
                MaintenanceActionHead::DeleteFiles
            ]
        );
    }

    #[test]
    fn deleting_the_title_alone_is_one_step_even_with_delete_files() {
        for actions in [
            MaintenanceActions {
                delete_title: true,
                ..MaintenanceActions::default()
            },
            MaintenanceActions {
                delete_title: true,
                delete_files: true,
                ..MaintenanceActions::default()
            },
        ] {
            let sequence =
                build_declared_action_sequence(&actions, MaintenanceSubjectKind::Movie, None)
                    .expect("valid");
            assert_eq!(
                kinds(&sequence),
                vec![MaintenanceActionStepKind::DeleteTitleAndFiles]
            );
            assert_eq!(
                decided_heads_in_execution_order(&actions),
                vec![MaintenanceActionHead::DeleteTitle]
            );
        }
    }

    #[test]
    fn a_combination_the_executor_cannot_run_is_an_error() {
        // Adding and removing the same tag on one subject.
        let opposing = MaintenanceActions {
            add_tags: vec!["stale".into()],
            remove_tags: vec!["stale".into()],
            ..MaintenanceActions::default()
        };
        assert!(
            build_declared_action_sequence(&opposing, MaintenanceSubjectKind::Movie, None).is_err()
        );
        // A title-only action on an episode.
        let search = MaintenanceActions {
            search: true,
            ..MaintenanceActions::default()
        };
        assert!(
            build_declared_action_sequence(&search, MaintenanceSubjectKind::Episode, None).is_err()
        );
        // A profile with no resolved id.
        let profile = MaintenanceActions {
            set_quality_profile: Some("Fixture HD".into()),
            ..MaintenanceActions::default()
        };
        assert!(
            build_declared_action_sequence(&profile, MaintenanceSubjectKind::Movie, None).is_err()
        );
    }

    #[test]
    fn narrow_scopes_run_only_monitoring_and_file_heads() {
        for scope in [
            MaintenanceRuleSubjectKind::Season,
            MaintenanceRuleSubjectKind::Episode,
        ] {
            for head in MaintenanceActionHead::ALL {
                assert_eq!(
                    head_supported_for_scope(head, scope, false),
                    matches!(
                        head,
                        MaintenanceActionHead::Unmonitor | MaintenanceActionHead::DeleteFiles
                    ),
                    "{scope:?} {head:?}"
                );
            }
        }
        for head in MaintenanceActionHead::ALL {
            assert!(head_supported_for_scope(
                head,
                MaintenanceRuleSubjectKind::Title,
                false
            ));
            assert_eq!(
                head_supported_for_scope(head, MaintenanceRuleSubjectKind::Title, true),
                matches!(
                    head,
                    MaintenanceActionHead::Unmonitor | MaintenanceActionHead::DeleteFiles
                ),
                "storage root {head:?}"
            );
        }
    }

    #[test]
    fn the_declared_document_keeps_source_order_and_distinct_values() {
        let scan = scryer_rules::maintenance_actions::scan_maintenance_action_heads(
            "package scryer.maintenance.user.fixture_rule\nimport rego.v1\n\
             add_tags := [\"stale\", \"stale\"] if input.facts.has_file\n\
             delete_files if input.facts.has_file\n\
             set_quality_profile := \"Fixture HD\" if input.facts.has_file\n\
             set_quality_profile := \"Fixture HD\" if input.facts.monitored\n\
             unmonitor if input.facts.monitored\n",
            "fixture_rule",
        )
        .expect("parses");
        let declared = MaintenanceDeclaredActions::from_scan(&scan);
        assert_eq!(
            declared.heads,
            vec![
                MaintenanceActionHead::AddTags,
                MaintenanceActionHead::DeleteFiles,
                MaintenanceActionHead::SetQualityProfile,
                MaintenanceActionHead::Unmonitor,
            ]
        );
        assert!(declared.destructive);
        assert_eq!(declared.quality_profiles, vec!["Fixture HD".to_string()]);
        assert_eq!(declared.add_tags, vec!["stale".to_string()]);
        let json = serde_json::to_string(&declared).unwrap();
        let read: MaintenanceDeclaredActions = serde_json::from_str(&json).unwrap();
        assert_eq!(read, declared);
    }

    #[test]
    fn decided_actions_round_trip_and_tolerate_absence() {
        let actions = MaintenanceActions {
            unmonitor: true,
            add_tags: vec!["stale".into()],
            ..MaintenanceActions::default()
        };
        let json = decided_actions_to_json(&actions);
        assert_eq!(decided_actions_from_json(Some(&json)), Some(actions));
        assert_eq!(decided_actions_from_json(None), None);
    }
}
