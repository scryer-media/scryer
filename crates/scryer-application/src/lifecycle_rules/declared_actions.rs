//! The actions a lifecycle rule declares in its own Rego source.
//!
//! A rule names what happens to a subject by defining action heads
//! (`unmonitor if { … }`, `add_tags := ["stale"] if { … }`). Two things are
//! derived from that here:
//!
//! - [`LifecycleDeclaredActions`] is what a revision *can* do, read off the
//!   source without running it. It is what a revision stores in its action
//!   column, what arming is derived from, and what authoring shows.
//! - [`build_declared_action_sequence`] turns what one evaluation *decided*
//!   for one subject into the step sequence the sequence executor runs. The
//!   order of steps is fixed by the host, never by the author.

use std::collections::HashSet;

use scryer_rules::lifecycle_actions::{LifecycleActionHead, LifecycleActionScan, LifecycleActions};
use serde::{Deserialize, Serialize};

use scryer_rules::lifecycle::LifecycleSubjectKind as ScryerSubjectKind;

use super::LifecycleSubjectKind;
use super::action_sequence::{
    LifecycleActionSequence, LifecycleActionSequenceError, LifecycleActionStep,
    LifecycleActionStepKind, LifecycleActionStepParameters, LifecycleSearchCondition,
};

/// Schema of the stored declared-actions document. Revisions written before
/// it hold an older document, which reads as declaring no action.
pub const LIFECYCLE_DECLARED_ACTIONS_SCHEMA_VERSION: u32 = 3;

/// What a revision's source can do, as stored on the revision.
///
/// Every list keeps source order and holds each value once.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LifecycleDeclaredActions {
    pub schema_version: u32,
    /// Every action head the source defines.
    pub heads: Vec<LifecycleActionHead>,
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

impl LifecycleDeclaredActions {
    /// The document for a source's static scan.
    pub fn from_scan(scan: &LifecycleActionScan) -> Self {
        let mut heads: Vec<LifecycleActionHead> = scan.defined_heads.iter().copied().collect();
        heads.sort_by_key(|head| {
            scan.head_positions
                .get(head)
                .copied()
                .unwrap_or((usize::MAX, usize::MAX))
        });
        fn distinct(
            literals: &[scryer_rules::lifecycle_actions::LifecycleActionLiteral],
        ) -> Vec<String> {
            let mut seen = HashSet::new();
            literals
                .iter()
                .filter(|literal| seen.insert(literal.value.as_str()))
                .map(|literal| literal.value.clone())
                .collect()
        }
        Self {
            schema_version: LIFECYCLE_DECLARED_ACTIONS_SCHEMA_VERSION,
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
            schema_version: LIFECYCLE_DECLARED_ACTIONS_SCHEMA_VERSION,
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
    pub fn from_persisted_json(json: &str) -> Result<Option<Self>, LifecycleActionSequenceError> {
        let value: serde_json::Value = serde_json::from_str(json)
            .map_err(|error| LifecycleActionSequenceError::InvalidEnvelope(error.to_string()))?;
        let Some(object) = value.as_object() else {
            return Err(LifecycleActionSequenceError::InvalidEnvelope(
                "lifecycle action must be an object".into(),
            ));
        };
        let current = object
            .get("schema_version")
            .and_then(serde_json::Value::as_u64)
            == Some(u64::from(LIFECYCLE_DECLARED_ACTIONS_SCHEMA_VERSION));
        if !current {
            return Ok(None);
        }
        serde_json::from_value(value)
            .map(Some)
            .map_err(|error| LifecycleActionSequenceError::InvalidEnvelope(error.to_string()))
    }

    /// The document as a revision stores it.
    pub fn to_persisted_json(&self) -> String {
        serde_json::to_string(self).expect("declared lifecycle actions always serialize")
    }

    pub fn defines(&self, head: LifecycleActionHead) -> bool {
        self.heads.contains(&head)
    }

    /// Whether the source defines the head that applies `base`'s action at
    /// `level`. Decided actions name title heads; the level they were decided
    /// at is the subject's.
    pub fn defines_at(&self, base: LifecycleActionHead, level: ScryerSubjectKind) -> bool {
        base.at_level(level).is_some_and(|head| self.defines(head))
    }

    /// The levels the source acts at, read off the heads it defines.
    pub fn levels(&self) -> std::collections::BTreeSet<ScryerSubjectKind> {
        self.heads.iter().map(|head| head.level()).collect()
    }

    /// Whether any defined head is a reversible action of its own. The
    /// unmonitor a delete head implies is part of that delete and rides the
    /// destructive gate, the way the single delete actions always did.
    pub fn defines_reversible_action(&self) -> bool {
        self.heads.iter().any(|head| !head.is_destructive())
    }
}

/// One decided action in the place it runs: the head it shows as, and the step
/// it runs as. A notification is not a step: it is sent once the whole
/// sequence has succeeded, so it can follow even a title deletion.
struct OrderedAction {
    head: LifecycleActionHead,
    step: Option<LifecycleActionStepKind>,
}

/// What one evaluation decided, in execution order. This is the only place
/// that order is written.
///
/// Steps run in a fixed order: add tags, remove tags, set quality profile,
/// monitor, search, fetch subtitles, move, unmonitor, delete files, delete
/// title, and the notification last. Deleting files unmonitors the subject
/// first. Deleting the title removes its files with it, so a subject that is
/// also told to delete files gets the title deletion alone.
fn actions_in_execution_order(actions: &LifecycleActions) -> Vec<OrderedAction> {
    use LifecycleActionHead as Head;
    use LifecycleActionStepKind as Step;
    let delete_files = actions.delete_files && !actions.delete_title;
    [
        (
            !actions.add_tags.is_empty(),
            Head::AddTags,
            Some(Step::AddTags),
        ),
        (
            !actions.remove_tags.is_empty(),
            Head::RemoveTags,
            Some(Step::RemoveTags),
        ),
        (
            actions.set_quality_profile.is_some(),
            Head::SetQualityProfile,
            Some(Step::ChangeQualityProfile),
        ),
        (actions.monitor, Head::Monitor, Some(Step::Monitor)),
        (actions.search, Head::Search, Some(Step::Search)),
        (
            actions.fetch_subtitles,
            Head::FetchSubtitles,
            Some(Step::FetchSubtitles),
        ),
        (
            actions.move_to_library.is_some(),
            Head::MoveToLibrary,
            Some(Step::MoveToLibrary),
        ),
        (
            actions.move_to_root.is_some(),
            Head::MoveToRoot,
            Some(Step::MoveToRoot),
        ),
        (
            actions.unmonitor || delete_files,
            Head::Unmonitor,
            Some(Step::Unmonitor),
        ),
        (delete_files, Head::DeleteFiles, Some(Step::DeleteFiles)),
        (
            actions.delete_title,
            Head::DeleteTitle,
            Some(Step::DeleteTitleAndFiles),
        ),
        (actions.notify.is_some(), Head::Notify, None),
    ]
    .into_iter()
    .filter(|(present, _, _)| *present)
    .map(|(_, head, step)| OrderedAction { head, step })
    .collect()
}

/// The heads one evaluation decided, in execution order, as shown to the
/// operator. Deleting files always unmonitors first, so it shows both.
pub fn decided_heads_in_execution_order(actions: &LifecycleActions) -> Vec<LifecycleActionHead> {
    actions_in_execution_order(actions)
        .into_iter()
        .map(|action| action.head)
        .collect()
}

/// The step ids [`build_declared_action_sequence`] produces for these
/// actions, without resolving anything. A finished sequence records its step
/// ids, so this tells whether a later decision would run different steps.
pub fn declared_step_ids(actions: &LifecycleActions) -> Vec<String> {
    actions_in_execution_order(actions)
        .into_iter()
        .filter_map(|action| action.step)
        .map(|kind| kind.descriptor().id.to_string())
        .collect()
}

/// Read the decided actions a candidate stored. A candidate written before
/// rules declared their own actions has none.
pub fn decided_actions_from_json(json: Option<&str>) -> Option<LifecycleActions> {
    json.and_then(|json| serde_json::from_str(json).ok())
}

/// The decided actions as stored on a candidate.
pub fn decided_actions_to_json(actions: &LifecycleActions) -> String {
    serde_json::to_string(actions).expect("lifecycle actions always serialize")
}

fn step(
    kind: LifecycleActionStepKind,
    parameters: LifecycleActionStepParameters,
) -> LifecycleActionStep {
    LifecycleActionStep {
        id: kind.descriptor().id.to_string(),
        kind,
        parameters,
    }
}

/// The step sequence for what one evaluation decided for one subject, in the
/// order [`actions_in_execution_order`] fixes. A search decided together with
/// a profile change runs only when the change took effect.
///
/// `quality_profile_id` is the id the decided profile name resolved to; it
/// must be present when a profile was decided. The result is validated with
/// the same checks a stored sequence passes, so a combination the executor
/// cannot run safely is an error here, never a partial run.
pub fn build_declared_action_sequence(
    actions: &LifecycleActions,
    subject: LifecycleSubjectKind,
    quality_profile_id: Option<&str>,
) -> Result<LifecycleActionSequence, LifecycleActionSequenceError> {
    let profile_changes = actions.set_quality_profile.is_some();
    let mut steps = Vec::new();
    for kind in actions_in_execution_order(actions)
        .into_iter()
        .filter_map(|action| action.step)
    {
        let parameters = match kind {
            LifecycleActionStepKind::AddTags => LifecycleActionStepParameters::Tags {
                tags: actions.add_tags.clone(),
            },
            LifecycleActionStepKind::RemoveTags => LifecycleActionStepParameters::Tags {
                tags: actions.remove_tags.clone(),
            },
            LifecycleActionStepKind::ChangeQualityProfile => {
                let Some(target) = quality_profile_id else {
                    return Err(LifecycleActionSequenceError::InvalidEnvelope(
                        "the decided quality profile has no resolved id".into(),
                    ));
                };
                LifecycleActionStepParameters::ChangeQualityProfile {
                    target_quality_profile_id: target.to_string(),
                }
            }
            LifecycleActionStepKind::Search => LifecycleActionStepParameters::Search {
                condition: if profile_changes {
                    LifecycleSearchCondition::PreviousProfileChanged
                } else {
                    LifecycleSearchCondition::Unconditional
                },
            },
            LifecycleActionStepKind::MoveToLibrary => LifecycleActionStepParameters::Library {
                library: actions.move_to_library.clone().unwrap_or_default(),
            },
            LifecycleActionStepKind::MoveToRoot => LifecycleActionStepParameters::Root {
                root: actions.move_to_root.clone().unwrap_or_default(),
            },
            LifecycleActionStepKind::Unmonitor => LifecycleActionStepParameters::Unmonitor {
                include_descendants: true,
            },
            LifecycleActionStepKind::Monitor
            | LifecycleActionStepKind::FetchSubtitles
            | LifecycleActionStepKind::DeleteFiles
            | LifecycleActionStepKind::DeleteTitleAndFiles => LifecycleActionStepParameters::None,
        };
        steps.push(step(kind, parameters));
    }
    let sequence = LifecycleActionSequence::new(steps);
    sequence.validate(subject)?;
    Ok(sequence)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(sequence: &LifecycleActionSequence) -> Vec<LifecycleActionStepKind> {
        sequence.steps.iter().map(|step| step.kind).collect()
    }

    #[test]
    fn steps_run_in_the_fixed_order_whatever_the_author_wrote() {
        let actions = LifecycleActions {
            unmonitor: true,
            search: true,
            set_quality_profile: Some("Fixture HD".into()),
            add_tags: vec!["stale".into()],
            remove_tags: vec!["fresh".into()],
            ..LifecycleActions::default()
        };
        let sequence =
            build_declared_action_sequence(&actions, LifecycleSubjectKind::Show, Some("7"))
                .expect("valid");
        assert_eq!(
            kinds(&sequence),
            vec![
                LifecycleActionStepKind::AddTags,
                LifecycleActionStepKind::RemoveTags,
                LifecycleActionStepKind::ChangeQualityProfile,
                LifecycleActionStepKind::Search,
                LifecycleActionStepKind::Unmonitor,
            ]
        );
        assert_eq!(
            sequence.search_condition(3),
            Some(LifecycleSearchCondition::PreviousProfileChanged)
        );
    }

    #[test]
    fn step_ids_match_the_built_sequence_for_every_head_combination() {
        for bits in 0u32..(1 << 12) {
            let actions = LifecycleActions {
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
                build_declared_action_sequence(&actions, LifecycleSubjectKind::Show, Some("7"))
            else {
                continue;
            };
            let built: Vec<String> = sequence.steps.iter().map(|step| step.id.clone()).collect();
            assert_eq!(built, declared_step_ids(&actions), "{actions:?}");
        }
    }

    /// Every combination of decided actions, at every subject level, with the
    /// heads shown, the step ids recorded and the sequence built for it,
    /// folded into one digest. A change to the execution order, to a step's
    /// parameters or to what is refused changes the digest.
    #[test]
    fn execution_order_is_pinned_for_every_head_combination() {
        let mut hasher = blake3::Hasher::new();
        for bits in 0u32..(1 << 12) {
            let actions = LifecycleActions {
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
            hasher.update(
                format!(
                    "{bits}|{:?}|{:?}\n",
                    decided_heads_in_execution_order(&actions),
                    declared_step_ids(&actions)
                )
                .as_bytes(),
            );
            for subject in LifecycleSubjectKind::ALL {
                for profile_id in [None, Some("7")] {
                    let built = build_declared_action_sequence(&actions, *subject, profile_id);
                    hasher.update(format!("  {subject:?} {profile_id:?} {built:?}\n").as_bytes());
                }
            }
        }
        assert_eq!(
            hasher.finalize().to_hex().to_string(),
            "f2aadf61628607b596a0280d087b762dc53a950ececdcb07a31b9de138bdd235",
            "the execution order changed"
        );
    }

    #[test]
    fn the_newer_heads_run_in_their_fixed_places_and_notify_is_not_a_step() {
        let actions = LifecycleActions {
            unmonitor: true,
            search: true,
            add_tags: vec!["stale".into()],
            monitor: false,
            fetch_subtitles: true,
            move_to_root: Some("/fixture/archive".into()),
            notify: Some("Fixture archived".into()),
            ..LifecycleActions::default()
        };
        let sequence = build_declared_action_sequence(&actions, LifecycleSubjectKind::Movie, None)
            .expect("valid");
        assert_eq!(
            kinds(&sequence),
            vec![
                LifecycleActionStepKind::AddTags,
                LifecycleActionStepKind::Search,
                LifecycleActionStepKind::FetchSubtitles,
                LifecycleActionStepKind::MoveToRoot,
                LifecycleActionStepKind::Unmonitor,
            ]
        );
        assert_eq!(
            sequence.steps[3].parameters,
            LifecycleActionStepParameters::Root {
                root: "/fixture/archive".into()
            }
        );
        assert_eq!(
            decided_heads_in_execution_order(&actions),
            vec![
                LifecycleActionHead::AddTags,
                LifecycleActionHead::Search,
                LifecycleActionHead::FetchSubtitles,
                LifecycleActionHead::MoveToRoot,
                LifecycleActionHead::Unmonitor,
                LifecycleActionHead::Notify,
            ]
        );

        // A rule that only notifies runs no step at all.
        let notify_only = LifecycleActions {
            notify: Some("Fixture".into()),
            ..LifecycleActions::default()
        };
        assert!(
            build_declared_action_sequence(&notify_only, LifecycleSubjectKind::Episode, None)
                .expect("valid")
                .steps
                .is_empty()
        );

        let monitor_and_delete = LifecycleActions {
            monitor: true,
            delete_files: true,
            ..LifecycleActions::default()
        };
        assert!(matches!(
            build_declared_action_sequence(&monitor_and_delete, LifecycleSubjectKind::Movie, None),
            Err(LifecycleActionSequenceError::ConflictingSteps { .. })
        ));
    }

    #[test]
    fn a_declared_document_without_the_newer_lists_keeps_its_stored_form() {
        let stored = r#"{"schema_version":3,"heads":["unmonitor"],"destructive":false,"quality_profiles":[],"add_tags":[],"remove_tags":[]}"#;
        let declared = LifecycleDeclaredActions::from_persisted_json(stored)
            .expect("reads")
            .expect("current schema");
        assert!(declared.libraries.is_empty());
        assert_eq!(declared.to_persisted_json(), stored);
    }

    #[test]
    fn a_search_without_a_profile_change_is_unconditional() {
        let actions = LifecycleActions {
            search: true,
            ..LifecycleActions::default()
        };
        let sequence = build_declared_action_sequence(&actions, LifecycleSubjectKind::Movie, None)
            .expect("valid");
        assert_eq!(
            sequence.search_condition(0),
            Some(LifecycleSearchCondition::Unconditional)
        );
    }

    #[test]
    fn deleting_files_unmonitors_first() {
        let actions = LifecycleActions {
            delete_files: true,
            ..LifecycleActions::default()
        };
        for subject in LifecycleSubjectKind::ALL {
            let sequence = build_declared_action_sequence(&actions, *subject, None)
                .unwrap_or_else(|error| panic!("{subject:?}: {error}"));
            assert_eq!(
                kinds(&sequence),
                vec![
                    LifecycleActionStepKind::Unmonitor,
                    LifecycleActionStepKind::DeleteFiles
                ],
                "{subject:?}"
            );
        }
        assert_eq!(
            decided_heads_in_execution_order(&actions),
            vec![
                LifecycleActionHead::Unmonitor,
                LifecycleActionHead::DeleteFiles
            ]
        );
    }

    #[test]
    fn deleting_the_title_alone_is_one_step_even_with_delete_files() {
        for actions in [
            LifecycleActions {
                delete_title: true,
                ..LifecycleActions::default()
            },
            LifecycleActions {
                delete_title: true,
                delete_files: true,
                ..LifecycleActions::default()
            },
        ] {
            let sequence =
                build_declared_action_sequence(&actions, LifecycleSubjectKind::Movie, None)
                    .expect("valid");
            assert_eq!(
                kinds(&sequence),
                vec![LifecycleActionStepKind::DeleteTitleAndFiles]
            );
            assert_eq!(
                decided_heads_in_execution_order(&actions),
                vec![LifecycleActionHead::DeleteTitle]
            );
        }
    }

    #[test]
    fn a_combination_the_executor_cannot_run_is_an_error() {
        // Adding and removing the same tag on one subject.
        let opposing = LifecycleActions {
            add_tags: vec!["stale".into()],
            remove_tags: vec!["stale".into()],
            ..LifecycleActions::default()
        };
        assert!(
            build_declared_action_sequence(&opposing, LifecycleSubjectKind::Movie, None).is_err()
        );
        // A title-only action on an episode.
        let search = LifecycleActions {
            search: true,
            ..LifecycleActions::default()
        };
        assert!(
            build_declared_action_sequence(&search, LifecycleSubjectKind::Episode, None).is_err()
        );
        // A profile with no resolved id.
        let profile = LifecycleActions {
            set_quality_profile: Some("Fixture HD".into()),
            ..LifecycleActions::default()
        };
        assert!(
            build_declared_action_sequence(&profile, LifecycleSubjectKind::Movie, None).is_err()
        );
    }

    #[test]
    fn a_decided_action_is_defined_only_at_the_level_its_head_acts_at() {
        let scan = scryer_rules::lifecycle_actions::scan_lifecycle_action_heads(
            "package scryer.maintenance.user.fixture_rule\nimport rego.v1\n\
             unmonitor if input.facts.monitored\n\
             delete_season_files if input.season.facts.has_file\n",
            "fixture_rule",
        )
        .expect("parses");
        let declared = LifecycleDeclaredActions::from_scan(&scan);
        assert_eq!(
            declared.levels().into_iter().collect::<Vec<_>>(),
            vec![ScryerSubjectKind::Title, ScryerSubjectKind::Season]
        );
        assert!(declared.defines_at(LifecycleActionHead::Unmonitor, ScryerSubjectKind::Title));
        assert!(!declared.defines_at(LifecycleActionHead::Unmonitor, ScryerSubjectKind::Season));
        assert!(declared.defines_at(LifecycleActionHead::DeleteFiles, ScryerSubjectKind::Season));
        assert!(!declared.defines_at(LifecycleActionHead::DeleteFiles, ScryerSubjectKind::Title));
        assert!(!declared.defines_at(LifecycleActionHead::DeleteFiles, ScryerSubjectKind::Episode));
        // A title-only action has no season head at all.
        assert!(!declared.defines_at(LifecycleActionHead::Search, ScryerSubjectKind::Season));
    }

    #[test]
    fn the_declared_document_keeps_source_order_and_distinct_values() {
        let scan = scryer_rules::lifecycle_actions::scan_lifecycle_action_heads(
            "package scryer.maintenance.user.fixture_rule\nimport rego.v1\n\
             add_tags := [\"stale\", \"stale\"] if input.facts.has_file\n\
             delete_files if input.facts.has_file\n\
             set_quality_profile := \"Fixture HD\" if input.facts.has_file\n\
             set_quality_profile := \"Fixture HD\" if input.facts.monitored\n\
             unmonitor if input.facts.monitored\n",
            "fixture_rule",
        )
        .expect("parses");
        let declared = LifecycleDeclaredActions::from_scan(&scan);
        assert_eq!(
            declared.heads,
            vec![
                LifecycleActionHead::AddTags,
                LifecycleActionHead::DeleteFiles,
                LifecycleActionHead::SetQualityProfile,
                LifecycleActionHead::Unmonitor,
            ]
        );
        assert!(declared.destructive);
        assert_eq!(declared.quality_profiles, vec!["Fixture HD".to_string()]);
        assert_eq!(declared.add_tags, vec!["stale".to_string()]);
        let json = serde_json::to_string(&declared).unwrap();
        let read: LifecycleDeclaredActions = serde_json::from_str(&json).unwrap();
        assert_eq!(read, declared);
    }

    #[test]
    fn decided_actions_round_trip_and_tolerate_absence() {
        let actions = LifecycleActions {
            unmonitor: true,
            add_tags: vec!["stale".into()],
            ..LifecycleActions::default()
        };
        let json = decided_actions_to_json(&actions);
        assert_eq!(decided_actions_from_json(Some(&json)), Some(actions));
        assert_eq!(decided_actions_from_json(None), None);
    }
}
