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
/// it hold an older document, which reads as declaring no action.
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
    // The lists below are left out while empty, so a document that names
    // none of them keeps the form it was stored in before they existed.
    /// Every library name written for `move_to_library`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub libraries: Vec<String>,
    /// Every root path written for `move_to_root`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roots: Vec<String>,
    /// Every message written for `notify`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notifications: Vec<String>,
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
            libraries: distinct(&scan.libraries),
            roots: distinct(&scan.roots),
            notifications: distinct(&scan.notifications),
        }
    }

    /// The document for a revision that declares no action.
    pub fn none() -> Self {
        Self {
            schema_version: MAINTENANCE_DECLARED_ACTIONS_SCHEMA_VERSION,
            heads: Vec::new(),
            destructive: false,
            quality_profiles: Vec::new(),
            add_tags: Vec::new(),
            remove_tags: Vec::new(),
            libraries: Vec::new(),
            roots: Vec::new(),
            notifications: Vec::new(),
        }
    }

    /// Read a revision's stored action column.
    ///
    /// `None` is a document of any other schema, written by a build from
    /// before rules declared their own actions. Nothing reads its contents any
    /// more, and such a revision is refused: it is never evaluated and nothing
    /// runs for it. Text that is not a JSON object at all is an error.
    pub fn from_persisted_json(json: &str) -> Result<Option<Self>, MaintenanceActionSequenceError> {
        let value: serde_json::Value = serde_json::from_str(json)
            .map_err(|error| MaintenanceActionSequenceError::InvalidEnvelope(error.to_string()))?;
        let Some(object) = value.as_object() else {
            return Err(MaintenanceActionSequenceError::InvalidEnvelope(
                "maintenance action must be an object".into(),
            ));
        };
        let current = object
            .get("schema_version")
            .and_then(serde_json::Value::as_u64)
            == Some(u64::from(MAINTENANCE_DECLARED_ACTIONS_SCHEMA_VERSION));
        if !current {
            return Ok(None);
        }
        serde_json::from_value(value)
            .map(Some)
            .map_err(|error| MaintenanceActionSequenceError::InvalidEnvelope(error.to_string()))
    }

    /// The document as a revision stores it.
    pub fn to_persisted_json(&self) -> String {
        serde_json::to_string(self).expect("declared maintenance actions always serialize")
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

/// Whether a rule of this scope can run this head. Storage-root rules act
/// only on monitoring and files. Seasons and episodes may also be monitored,
/// have subtitles fetched, and notify; everything else is a whole-title
/// operation.
pub fn head_supported_for_scope(
    head: MaintenanceActionHead,
    scope: MaintenanceRuleSubjectKind,
    storage_root_selected: bool,
) -> bool {
    if storage_root_selected {
        return matches!(
            head,
            MaintenanceActionHead::Unmonitor | MaintenanceActionHead::DeleteFiles
        );
    }
    scope == MaintenanceRuleSubjectKind::Title
        || scryer_rules::rule_builder_schema::maintenance_head_allows_narrow_subjects(head)
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
    if actions.monitor {
        heads.push(MaintenanceActionHead::Monitor);
    }
    if actions.search {
        heads.push(MaintenanceActionHead::Search);
    }
    if actions.fetch_subtitles {
        heads.push(MaintenanceActionHead::FetchSubtitles);
    }
    if actions.move_to_library.is_some() {
        heads.push(MaintenanceActionHead::MoveToLibrary);
    }
    if actions.move_to_root.is_some() {
        heads.push(MaintenanceActionHead::MoveToRoot);
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
    if actions.notify.is_some() {
        heads.push(MaintenanceActionHead::Notify);
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
        (actions.monitor, MaintenanceActionStepKind::Monitor),
        (actions.search, MaintenanceActionStepKind::Search),
        (
            actions.fetch_subtitles,
            MaintenanceActionStepKind::FetchSubtitles,
        ),
        (
            actions.move_to_library.is_some(),
            MaintenanceActionStepKind::MoveToLibrary,
        ),
        (
            actions.move_to_root.is_some(),
            MaintenanceActionStepKind::MoveToRoot,
        ),
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
/// monitor, search, fetch subtitles, move, unmonitor, delete files, delete
/// title. A notification is not a step: it is sent once the whole sequence
/// has succeeded, so it can follow even a title deletion. Deleting files unmonitors
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
    if actions.monitor {
        steps.push(step(
            MaintenanceActionStepKind::Monitor,
            MaintenanceActionStepParameters::None,
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
    if actions.fetch_subtitles {
        steps.push(step(
            MaintenanceActionStepKind::FetchSubtitles,
            MaintenanceActionStepParameters::None,
        ));
    }
    if let Some(library) = &actions.move_to_library {
        steps.push(step(
            MaintenanceActionStepKind::MoveToLibrary,
            MaintenanceActionStepParameters::Library {
                library: library.clone(),
            },
        ));
    }
    if let Some(root) = &actions.move_to_root {
        steps.push(step(
            MaintenanceActionStepKind::MoveToRoot,
            MaintenanceActionStepParameters::Root { root: root.clone() },
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
        for bits in 0u32..(1 << 12) {
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
                monitor: bits & 128 != 0,
                fetch_subtitles: bits & 256 != 0,
                move_to_library: (bits & 512 != 0).then(|| "Fixture Archive".to_string()),
                move_to_root: (bits & 1024 != 0).then(|| "/fixture/archive".to_string()),
                notify: (bits & 2048 != 0).then(|| "Fixture".to_string()),
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
    fn the_newer_heads_run_in_their_fixed_places_and_notify_is_not_a_step() {
        let actions = MaintenanceActions {
            unmonitor: true,
            search: true,
            add_tags: vec!["stale".into()],
            monitor: false,
            fetch_subtitles: true,
            move_to_root: Some("/fixture/archive".into()),
            notify: Some("Fixture archived".into()),
            ..MaintenanceActions::default()
        };
        let sequence =
            build_declared_action_sequence(&actions, MaintenanceSubjectKind::Movie, None)
                .expect("valid");
        assert_eq!(
            kinds(&sequence),
            vec![
                MaintenanceActionStepKind::AddTags,
                MaintenanceActionStepKind::Search,
                MaintenanceActionStepKind::FetchSubtitles,
                MaintenanceActionStepKind::MoveToRoot,
                MaintenanceActionStepKind::Unmonitor,
            ]
        );
        assert_eq!(
            sequence.steps[3].parameters,
            MaintenanceActionStepParameters::Root {
                root: "/fixture/archive".into()
            }
        );
        assert_eq!(
            decided_heads_in_execution_order(&actions),
            vec![
                MaintenanceActionHead::AddTags,
                MaintenanceActionHead::Search,
                MaintenanceActionHead::FetchSubtitles,
                MaintenanceActionHead::MoveToRoot,
                MaintenanceActionHead::Unmonitor,
                MaintenanceActionHead::Notify,
            ]
        );

        // A rule that only notifies runs no step at all.
        let notify_only = MaintenanceActions {
            notify: Some("Fixture".into()),
            ..MaintenanceActions::default()
        };
        assert!(
            build_declared_action_sequence(&notify_only, MaintenanceSubjectKind::Episode, None)
                .expect("valid")
                .steps
                .is_empty()
        );

        let monitor_and_delete = MaintenanceActions {
            monitor: true,
            delete_files: true,
            ..MaintenanceActions::default()
        };
        assert!(matches!(
            build_declared_action_sequence(
                &monitor_and_delete,
                MaintenanceSubjectKind::Movie,
                None
            ),
            Err(MaintenanceActionSequenceError::ConflictingSteps { .. })
        ));
    }

    #[test]
    fn a_declared_document_without_the_newer_lists_keeps_its_stored_form() {
        let stored = r#"{"schema_version":3,"heads":["unmonitor"],"destructive":false,"quality_profiles":[],"add_tags":[],"remove_tags":[]}"#;
        let declared = MaintenanceDeclaredActions::from_persisted_json(stored)
            .expect("reads")
            .expect("current schema");
        assert!(declared.libraries.is_empty());
        assert_eq!(declared.to_persisted_json(), stored);
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
    fn narrow_scopes_run_only_monitoring_subtitle_notify_and_file_heads() {
        for scope in [
            MaintenanceRuleSubjectKind::Season,
            MaintenanceRuleSubjectKind::Episode,
        ] {
            for head in MaintenanceActionHead::ALL {
                assert_eq!(
                    head_supported_for_scope(head, scope, false),
                    matches!(
                        head,
                        MaintenanceActionHead::Unmonitor
                            | MaintenanceActionHead::DeleteFiles
                            | MaintenanceActionHead::Monitor
                            | MaintenanceActionHead::FetchSubtitles
                            | MaintenanceActionHead::Notify
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
