//! The closed step sequence the lifecycle executor runs.
//!
//! A rule declares its actions in its own Rego source; what one evaluation
//! decides for one subject is turned into one of these sequences, in an order
//! the host fixes. The sequence is the executor's representation only: a
//! revision stores the actions its source declares, never a sequence.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use super::{LifecycleEffectClass, LifecycleRiskClass, LifecycleSubjectKind};

/// The action kind every candidate and sequence run records.
///
/// It is intentionally absent from [`super::LifecycleActionKind`], the names
/// earlier builds recorded, so history never reads it as one of those.
pub const ACTION_SEQUENCE_KIND: &str = "action_sequence";

/// The only accepted v2 sequence schema.
pub const LIFECYCLE_ACTION_SEQUENCE_SCHEMA_VERSION: u32 = 2;

/// A sequence is intentionally compact: every supported primitive may appear
/// once, and the closed catalog currently contains eleven primitives.
pub const LIFECYCLE_MAX_ACTION_SEQUENCE_STEPS: usize = 11;

/// Whether a durable action job is complete when it is accepted or only when
/// later reconciliation proves its postcondition.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleActionCompletionPolicy {
    Accepted,
    Completed,
}

/// The closed v2 primitive catalog. These variants are not aliases for v1
/// presets: v1 retains its serialized shape and execution behavior.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleActionStepKind {
    Unmonitor,
    DeleteFiles,
    ChangeQualityProfile,
    Search,
    AddTags,
    RemoveTags,
    DeleteTitleAndFiles,
    Monitor,
    FetchSubtitles,
    MoveToLibrary,
    MoveToRoot,
}

/// Search execution condition. A sequence author must make conditional search
/// explicit so a profile/search preset cannot silently become an unconditional
/// search after a reorder.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleSearchCondition {
    Unconditional,
    PreviousProfileChanged,
}

impl LifecycleActionStepKind {
    pub const ALL: &'static [Self] = &[
        Self::Unmonitor,
        Self::DeleteFiles,
        Self::ChangeQualityProfile,
        Self::Search,
        Self::AddTags,
        Self::RemoveTags,
        Self::DeleteTitleAndFiles,
        Self::Monitor,
        Self::FetchSubtitles,
        Self::MoveToLibrary,
        Self::MoveToRoot,
    ];

    pub const fn as_wire_str(self) -> &'static str {
        match self {
            Self::Unmonitor => "unmonitor",
            Self::DeleteFiles => "delete_files",
            Self::ChangeQualityProfile => "change_quality_profile",
            Self::Search => "search",
            Self::AddTags => "add_tags",
            Self::RemoveTags => "remove_tags",
            Self::DeleteTitleAndFiles => "delete_title_and_files",
            Self::Monitor => "monitor",
            Self::FetchSubtitles => "fetch_subtitles",
            Self::MoveToLibrary => "move_to_library",
            Self::MoveToRoot => "move_to_root",
        }
    }

    /// Whether this step moves the title's files to another place.
    pub const fn is_move(self) -> bool {
        matches!(self, Self::MoveToLibrary | Self::MoveToRoot)
    }

    pub const fn descriptor(self) -> &'static LifecycleActionStepDescriptor {
        action_sequence_descriptor_for(self)
    }
}

/// Typed configuration for a v2 primitive. There is no free-form JSON escape
/// hatch: a mismatched kind/parameter pair is rejected before persistence.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum LifecycleActionStepParameters {
    #[default]
    None,
    /// Explicit at title scope. For a show, `true` covers its descendants; a
    /// season always covers its episodes and an episode covers only itself.
    Unmonitor {
        include_descendants: bool,
    },
    ChangeQualityProfile {
        target_quality_profile_id: String,
    },
    Search {
        condition: LifecycleSearchCondition,
    },
    Tags {
        tags: Vec<String>,
    },
    /// The library a move names, as the rule wrote it. It is resolved to a
    /// configured library of the title's kind when the step runs.
    Library {
        library: String,
    },
    /// The root path a move names, as the rule wrote it. It is resolved
    /// among the title's own library's roots when the step runs.
    Root {
        root: String,
    },
}

impl LifecycleActionStepParameters {
    fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }

    fn matches_kind(&self, kind: LifecycleActionStepKind) -> bool {
        match kind {
            LifecycleActionStepKind::Unmonitor => matches!(self, Self::Unmonitor { .. }),
            LifecycleActionStepKind::ChangeQualityProfile => {
                matches!(self, Self::ChangeQualityProfile { .. })
            }
            LifecycleActionStepKind::Search => matches!(self, Self::Search { .. }),
            LifecycleActionStepKind::AddTags | LifecycleActionStepKind::RemoveTags => {
                matches!(self, Self::Tags { .. })
            }
            LifecycleActionStepKind::MoveToLibrary => matches!(self, Self::Library { .. }),
            LifecycleActionStepKind::MoveToRoot => matches!(self, Self::Root { .. }),
            LifecycleActionStepKind::DeleteFiles
            | LifecycleActionStepKind::DeleteTitleAndFiles
            | LifecycleActionStepKind::Monitor
            | LifecycleActionStepKind::FetchSubtitles => self.is_none(),
        }
    }

    pub fn tag_labels(&self) -> &[String] {
        match self {
            Self::Tags { tags } => tags,
            _ => &[],
        }
    }
}

/// One user-authored, stable-addressable v2 operation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct LifecycleActionStep {
    pub id: String,
    pub kind: LifecycleActionStepKind,
    #[serde(
        default,
        skip_serializing_if = "LifecycleActionStepParameters::is_none"
    )]
    pub parameters: LifecycleActionStepParameters,
}

impl LifecycleActionStep {
    pub fn descriptor(&self) -> &'static LifecycleActionStepDescriptor {
        action_sequence_descriptor_for(self.kind)
    }
}

/// The explicit schema-2 persisted envelope. `content_hash` is computed from
/// this canonical ordered content and is never accepted from a client payload.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct LifecycleActionSequence {
    pub schema_version: u32,
    #[serde(default)]
    pub steps: Vec<LifecycleActionStep>,
}

impl LifecycleActionSequence {
    pub fn new(steps: Vec<LifecycleActionStep>) -> Self {
        Self {
            schema_version: LIFECYCLE_ACTION_SEQUENCE_SCHEMA_VERSION,
            steps,
        }
    }

    /// Whether running this sequence needs the reversible-effects gate. The
    /// unmonitor that comes with a file delete is part of that delete and
    /// rides the destructive gate, the way the single delete actions did.
    pub fn requires_reversible_gate(&self) -> bool {
        let deletes_files = self
            .steps
            .iter()
            .any(|step| step.kind == LifecycleActionStepKind::DeleteFiles);
        self.steps.iter().any(|step| {
            step.descriptor().risk_class != LifecycleRiskClass::High
                && !(deletes_files && step.kind == LifecycleActionStepKind::Unmonitor)
        })
    }

    /// Computes the revision-bound identity from ordered content. A reorder is
    /// a new hash even when it contains the same individual steps.
    pub fn content_hash(&self) -> Result<String, LifecycleActionSequenceError> {
        let bytes = serde_json::to_vec(self)
            .map_err(|error| LifecycleActionSequenceError::Serialization(error.to_string()))?;
        Ok(blake3::hash(&bytes).to_hex().to_string())
    }

    pub fn validate(
        &self,
        subject: LifecycleSubjectKind,
    ) -> Result<(), LifecycleActionSequenceError> {
        if self.schema_version != LIFECYCLE_ACTION_SEQUENCE_SCHEMA_VERSION {
            return Err(LifecycleActionSequenceError::UnsupportedSchemaVersion {
                found: self.schema_version,
                expected: LIFECYCLE_ACTION_SEQUENCE_SCHEMA_VERSION,
            });
        }
        if self.steps.len() > LIFECYCLE_MAX_ACTION_SEQUENCE_STEPS {
            return Err(LifecycleActionSequenceError::TooManySteps {
                found: self.steps.len(),
                maximum: LIFECYCLE_MAX_ACTION_SEQUENCE_STEPS,
            });
        }

        let mut ids = HashSet::new();
        let mut kinds = HashSet::new();
        let mut added_tags = HashSet::new();
        let mut removed_tags = HashSet::new();
        let mut prior_covering_unmonitor = false;
        let mut includes_delete_files = false;
        let present: HashSet<LifecycleActionStepKind> =
            self.steps.iter().map(|step| step.kind).collect();
        if let Some((first, second)) = conflicting_step_kinds(&present) {
            return Err(LifecycleActionSequenceError::ConflictingSteps { first, second });
        }

        for (index, step) in self.steps.iter().enumerate() {
            validate_step_id(&step.id)?;
            if !ids.insert(step.id.as_str()) {
                return Err(LifecycleActionSequenceError::DuplicateStepId {
                    id: step.id.clone(),
                });
            }
            if !kinds.insert(step.kind) {
                return Err(LifecycleActionSequenceError::DuplicateStepKind { kind: step.kind });
            }
            let descriptor = step.descriptor();
            if !descriptor.supports_subject(subject) {
                return Err(LifecycleActionSequenceError::UnsupportedSubject {
                    kind: step.kind,
                    subject,
                });
            }
            if !step.parameters.matches_kind(step.kind) {
                return Err(LifecycleActionSequenceError::ParameterShapeMismatch {
                    kind: step.kind,
                });
            }
            validate_step_parameters(step, subject)?;

            if step.kind == LifecycleActionStepKind::DeleteFiles && !prior_covering_unmonitor {
                return Err(LifecycleActionSequenceError::DeleteFilesRequiresUnmonitor);
            }
            if step.kind == LifecycleActionStepKind::DeleteFiles {
                includes_delete_files = true;
            }
            if step.kind == LifecycleActionStepKind::DeleteTitleAndFiles && includes_delete_files {
                return Err(LifecycleActionSequenceError::RedundantDeleteFilesBeforeDeleteTitle);
            }
            if matches!(
                &step.parameters,
                LifecycleActionStepParameters::Search {
                    condition: LifecycleSearchCondition::PreviousProfileChanged
                }
            ) && !self.steps[..index].last().is_some_and(|previous| {
                previous.kind == LifecycleActionStepKind::ChangeQualityProfile
            }) {
                return Err(LifecycleActionSequenceError::ConditionalSearchRequiresPreviousProfile);
            }
            if step.kind == LifecycleActionStepKind::Unmonitor
                && unmonitor_covers_scope(&step.parameters, subject)
            {
                prior_covering_unmonitor = true;
            }
            if step.kind == LifecycleActionStepKind::AddTags {
                added_tags.extend(step.parameters.tag_labels().iter().cloned());
            }
            if step.kind == LifecycleActionStepKind::RemoveTags {
                removed_tags.extend(step.parameters.tag_labels().iter().cloned());
            }
            if descriptor.terminal && index + 1 != self.steps.len() {
                return Err(LifecycleActionSequenceError::TerminalStepMustBeLast {
                    kind: step.kind,
                });
            }
        }

        if let Some(label) = added_tags.intersection(&removed_tags).next() {
            return Err(LifecycleActionSequenceError::OpposingTagOperations {
                label: (*label).clone(),
            });
        }
        Ok(())
    }

    /// Storage-root sequences are deliberately limited to the filesystem-safe
    /// subset. This is a second validation pass because ordinary title rules
    /// are allowed to combine catalog and acquisition primitives.
    pub fn validate_for_storage_root(
        &self,
        subject: LifecycleSubjectKind,
    ) -> Result<(), LifecycleActionSequenceError> {
        self.validate(subject)?;
        if let Some(step) = self
            .steps
            .iter()
            .find(|step| !step.descriptor().storage_root_allowed)
        {
            return Err(LifecycleActionSequenceError::UnsupportedStorageRootStep {
                kind: step.kind,
            });
        }
        Ok(())
    }

    /// Whether a Search is the preset form that depends on the immediately
    /// preceding profile mutation. A standalone Search is unconditional.
    pub fn search_condition(&self, step_index: usize) -> Option<LifecycleSearchCondition> {
        match &self.steps.get(step_index)?.parameters {
            LifecycleActionStepParameters::Search { condition } => Some(*condition),
            _ => None,
        }
    }

    /// The only profile step a conditional Search may consume. Validation
    /// already guarantees it is immediately preceding the Search.
    pub fn previous_profile_step(&self, step_index: usize) -> Option<&LifecycleActionStep> {
        self.steps
            .get(step_index.checked_sub(1)?)
            .filter(|step| step.kind == LifecycleActionStepKind::ChangeQualityProfile)
    }
}

/// A dependency surfaced to authoring and execution. The executor verifies the
/// persisted evidence afresh; this descriptor only describes the rule shape.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleActionStepRequirement {
    PriorCompleteCoveringUnmonitor,
}

/// Static backend-owned data for one v2 primitive.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LifecycleActionStepDescriptor {
    pub id: &'static str,
    pub kind: LifecycleActionStepKind,
    pub label: &'static str,
    pub supported_subjects: &'static [LifecycleSubjectKind],
    /// A stable typed parameter discriminator for authoring clients.
    pub parameter_schema: &'static str,
    pub effect_classes: &'static [LifecycleEffectClass],
    pub risk_class: LifecycleRiskClass,
    pub requires: &'static [LifecycleActionStepRequirement],
    pub terminal: bool,
    pub completion_policy: LifecycleActionCompletionPolicy,
    pub storage_root_allowed: bool,
}

impl LifecycleActionStepDescriptor {
    pub fn supports_subject(&self, subject: LifecycleSubjectKind) -> bool {
        self.supported_subjects.contains(&subject)
    }
}

const ALL_SUBJECTS: &[LifecycleSubjectKind] = LifecycleSubjectKind::ALL;
const TITLE_SUBJECTS: &[LifecycleSubjectKind] =
    &[LifecycleSubjectKind::Movie, LifecycleSubjectKind::Show];
const CATALOG_INTENT: &[LifecycleEffectClass] = &[LifecycleEffectClass::CatalogIntent];
const ACQUISITION: &[LifecycleEffectClass] = &[LifecycleEffectClass::Acquisition];
const FILE_ORGANIZATION: &[LifecycleEffectClass] = &[LifecycleEffectClass::FileOrganization];
const DESTRUCTIVE_STORAGE: &[LifecycleEffectClass] = &[LifecycleEffectClass::DestructiveStorage];
const DELETE_FILES_REQUIREMENT: &[LifecycleActionStepRequirement] =
    &[LifecycleActionStepRequirement::PriorCompleteCoveringUnmonitor];
const NO_REQUIREMENTS: &[LifecycleActionStepRequirement] = &[];

const UNMONITOR: LifecycleActionStepDescriptor = LifecycleActionStepDescriptor {
    id: "unmonitor",
    kind: LifecycleActionStepKind::Unmonitor,
    label: "Unmonitor",
    supported_subjects: ALL_SUBJECTS,
    parameter_schema: "unmonitor",
    effect_classes: CATALOG_INTENT,
    risk_class: LifecycleRiskClass::Medium,
    requires: NO_REQUIREMENTS,
    terminal: false,
    completion_policy: LifecycleActionCompletionPolicy::Completed,
    storage_root_allowed: true,
};
const DELETE_FILES: LifecycleActionStepDescriptor = LifecycleActionStepDescriptor {
    id: "delete_files",
    kind: LifecycleActionStepKind::DeleteFiles,
    label: "Delete files",
    supported_subjects: ALL_SUBJECTS,
    parameter_schema: "none",
    effect_classes: DESTRUCTIVE_STORAGE,
    risk_class: LifecycleRiskClass::High,
    requires: DELETE_FILES_REQUIREMENT,
    terminal: false,
    completion_policy: LifecycleActionCompletionPolicy::Completed,
    storage_root_allowed: true,
};
const CHANGE_QUALITY_PROFILE: LifecycleActionStepDescriptor = LifecycleActionStepDescriptor {
    id: "change_quality_profile",
    kind: LifecycleActionStepKind::ChangeQualityProfile,
    label: "Change quality profile",
    supported_subjects: TITLE_SUBJECTS,
    parameter_schema: "change_quality_profile",
    effect_classes: CATALOG_INTENT,
    risk_class: LifecycleRiskClass::Medium,
    requires: NO_REQUIREMENTS,
    terminal: false,
    completion_policy: LifecycleActionCompletionPolicy::Completed,
    storage_root_allowed: false,
};
const SEARCH: LifecycleActionStepDescriptor = LifecycleActionStepDescriptor {
    id: "search",
    kind: LifecycleActionStepKind::Search,
    label: "Search",
    supported_subjects: TITLE_SUBJECTS,
    parameter_schema: "search",
    effect_classes: ACQUISITION,
    risk_class: LifecycleRiskClass::Medium,
    requires: NO_REQUIREMENTS,
    terminal: false,
    completion_policy: LifecycleActionCompletionPolicy::Accepted,
    storage_root_allowed: false,
};
const ADD_TAGS: LifecycleActionStepDescriptor = LifecycleActionStepDescriptor {
    id: "add_tags",
    kind: LifecycleActionStepKind::AddTags,
    label: "Add tags",
    supported_subjects: TITLE_SUBJECTS,
    parameter_schema: "tags",
    effect_classes: CATALOG_INTENT,
    risk_class: LifecycleRiskClass::Low,
    requires: NO_REQUIREMENTS,
    terminal: false,
    completion_policy: LifecycleActionCompletionPolicy::Completed,
    storage_root_allowed: false,
};
const REMOVE_TAGS: LifecycleActionStepDescriptor = LifecycleActionStepDescriptor {
    id: "remove_tags",
    kind: LifecycleActionStepKind::RemoveTags,
    label: "Remove tags",
    supported_subjects: TITLE_SUBJECTS,
    parameter_schema: "tags",
    effect_classes: CATALOG_INTENT,
    risk_class: LifecycleRiskClass::Low,
    requires: NO_REQUIREMENTS,
    terminal: false,
    completion_policy: LifecycleActionCompletionPolicy::Completed,
    storage_root_allowed: false,
};
const DELETE_TITLE_AND_FILES: LifecycleActionStepDescriptor = LifecycleActionStepDescriptor {
    id: "delete_title_and_files",
    kind: LifecycleActionStepKind::DeleteTitleAndFiles,
    label: "Delete title and files",
    supported_subjects: TITLE_SUBJECTS,
    parameter_schema: "none",
    effect_classes: DESTRUCTIVE_STORAGE,
    risk_class: LifecycleRiskClass::High,
    requires: NO_REQUIREMENTS,
    terminal: true,
    completion_policy: LifecycleActionCompletionPolicy::Completed,
    storage_root_allowed: false,
};

const MONITOR: LifecycleActionStepDescriptor = LifecycleActionStepDescriptor {
    id: "monitor",
    kind: LifecycleActionStepKind::Monitor,
    label: "Monitor",
    supported_subjects: ALL_SUBJECTS,
    parameter_schema: "none",
    effect_classes: CATALOG_INTENT,
    risk_class: LifecycleRiskClass::Medium,
    requires: NO_REQUIREMENTS,
    terminal: false,
    completion_policy: LifecycleActionCompletionPolicy::Completed,
    storage_root_allowed: false,
};
const FETCH_SUBTITLES: LifecycleActionStepDescriptor = LifecycleActionStepDescriptor {
    id: "fetch_subtitles",
    kind: LifecycleActionStepKind::FetchSubtitles,
    label: "Fetch subtitles",
    supported_subjects: ALL_SUBJECTS,
    parameter_schema: "none",
    effect_classes: ACQUISITION,
    risk_class: LifecycleRiskClass::Low,
    requires: NO_REQUIREMENTS,
    terminal: false,
    completion_policy: LifecycleActionCompletionPolicy::Accepted,
    storage_root_allowed: false,
};
const MOVE_TO_LIBRARY: LifecycleActionStepDescriptor = LifecycleActionStepDescriptor {
    id: "move_to_library",
    kind: LifecycleActionStepKind::MoveToLibrary,
    label: "Move to library",
    supported_subjects: TITLE_SUBJECTS,
    parameter_schema: "library",
    effect_classes: FILE_ORGANIZATION,
    risk_class: LifecycleRiskClass::Medium,
    requires: NO_REQUIREMENTS,
    terminal: false,
    completion_policy: LifecycleActionCompletionPolicy::Accepted,
    storage_root_allowed: false,
};
const MOVE_TO_ROOT: LifecycleActionStepDescriptor = LifecycleActionStepDescriptor {
    id: "move_to_root",
    kind: LifecycleActionStepKind::MoveToRoot,
    label: "Move to root",
    supported_subjects: TITLE_SUBJECTS,
    parameter_schema: "root",
    effect_classes: FILE_ORGANIZATION,
    risk_class: LifecycleRiskClass::Medium,
    requires: NO_REQUIREMENTS,
    terminal: false,
    completion_policy: LifecycleActionCompletionPolicy::Accepted,
    storage_root_allowed: false,
};

const ACTION_SEQUENCE_CATALOG: &[LifecycleActionStepDescriptor] = &[
    UNMONITOR,
    DELETE_FILES,
    CHANGE_QUALITY_PROFILE,
    SEARCH,
    ADD_TAGS,
    REMOVE_TAGS,
    DELETE_TITLE_AND_FILES,
    MONITOR,
    FETCH_SUBTITLES,
    MOVE_TO_LIBRARY,
    MOVE_TO_ROOT,
];

pub fn action_sequence_catalog() -> &'static [LifecycleActionStepDescriptor] {
    ACTION_SEQUENCE_CATALOG
}

pub const fn action_sequence_descriptor_for(
    kind: LifecycleActionStepKind,
) -> &'static LifecycleActionStepDescriptor {
    match kind {
        LifecycleActionStepKind::Unmonitor => &UNMONITOR,
        LifecycleActionStepKind::DeleteFiles => &DELETE_FILES,
        LifecycleActionStepKind::ChangeQualityProfile => &CHANGE_QUALITY_PROFILE,
        LifecycleActionStepKind::Search => &SEARCH,
        LifecycleActionStepKind::AddTags => &ADD_TAGS,
        LifecycleActionStepKind::RemoveTags => &REMOVE_TAGS,
        LifecycleActionStepKind::DeleteTitleAndFiles => &DELETE_TITLE_AND_FILES,
        LifecycleActionStepKind::Monitor => &MONITOR,
        LifecycleActionStepKind::FetchSubtitles => &FETCH_SUBTITLES,
        LifecycleActionStepKind::MoveToLibrary => &MOVE_TO_LIBRARY,
        LifecycleActionStepKind::MoveToRoot => &MOVE_TO_ROOT,
    }
}

fn validate_step_id(id: &str) -> Result<(), LifecycleActionSequenceError> {
    if id.is_empty() || id.len() > 64 {
        return Err(LifecycleActionSequenceError::InvalidStepId { id: id.to_string() });
    }
    if !id
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err(LifecycleActionSequenceError::InvalidStepId { id: id.to_string() });
    }
    Ok(())
}

fn validate_step_parameters(
    step: &LifecycleActionStep,
    subject: LifecycleSubjectKind,
) -> Result<(), LifecycleActionSequenceError> {
    match (&step.kind, &step.parameters) {
        (
            LifecycleActionStepKind::Unmonitor,
            LifecycleActionStepParameters::Unmonitor {
                include_descendants: false,
            },
        ) if subject == LifecycleSubjectKind::Season => {
            Err(LifecycleActionSequenceError::SeasonUnmonitorRequiresDescendants)
        }
        (
            LifecycleActionStepKind::ChangeQualityProfile,
            LifecycleActionStepParameters::ChangeQualityProfile {
                target_quality_profile_id,
            },
        ) if target_quality_profile_id.trim().is_empty() => {
            Err(LifecycleActionSequenceError::EmptyQualityProfileTarget)
        }
        (
            LifecycleActionStepKind::AddTags | LifecycleActionStepKind::RemoveTags,
            LifecycleActionStepParameters::Tags { tags },
        ) => validate_action_tag_labels(tags),
        (
            LifecycleActionStepKind::MoveToLibrary,
            LifecycleActionStepParameters::Library { library: target },
        )
        | (
            LifecycleActionStepKind::MoveToRoot,
            LifecycleActionStepParameters::Root { root: target },
        ) if target.trim().is_empty() => Err(LifecycleActionSequenceError::EmptyMoveTarget),
        _ => Ok(()),
    }
}

/// Static half of tag-parameter validation: shape, normalization, and the
/// per-title ceiling.
///
/// It is deliberately the same normalizer the assignment path uses, so a label
/// that a human could type into the tag picker is exactly the set of labels a
/// step can carry, and a step can never carry a spelling that would never
/// match a bag. Whether the label is *defined* is the other half of the check
/// and cannot live here: it needs the registry, so the executor performs it
/// before it acts.
fn validate_action_tag_labels(tags: &[String]) -> Result<(), LifecycleActionSequenceError> {
    if tags.is_empty() {
        return Err(LifecycleActionSequenceError::EmptyTagList);
    }
    if tags.len() > crate::MAX_USER_TAGS_PER_TITLE {
        return Err(LifecycleActionSequenceError::TooManyTags {
            found: tags.len(),
            maximum: crate::MAX_USER_TAGS_PER_TITLE,
        });
    }
    let mut seen = HashSet::new();
    for label in tags {
        let normalized = crate::normalize_user_title_tag(label).map_err(|reason| {
            LifecycleActionSequenceError::InvalidTagLabel {
                label: label.clone(),
                reason,
            }
        })?;
        if &normalized != label {
            // Steps carry the normal form only. Accepting `Needs  Review` here
            // and normalizing it silently would let the rule and the step
            // disagree about what the rule says.
            return Err(LifecycleActionSequenceError::InvalidTagLabel {
                label: label.clone(),
                reason: format!(
                    "tags are stored lowercase and trimmed, so this must be '{normalized}'"
                ),
            });
        }
        if !seen.insert(normalized) {
            return Err(LifecycleActionSequenceError::DuplicateTagLabel {
                label: label.clone(),
            });
        }
    }
    Ok(())
}

/// Step kinds that cannot run in one sequence: monitoring with unmonitoring or
/// either delete, both moves, and a move with either delete.
fn conflicting_step_kinds(
    present: &HashSet<LifecycleActionStepKind>,
) -> Option<(LifecycleActionStepKind, LifecycleActionStepKind)> {
    use LifecycleActionStepKind as K;
    const PAIRS: [(LifecycleActionStepKind, LifecycleActionStepKind); 8] = [
        (K::Monitor, K::Unmonitor),
        (K::Monitor, K::DeleteFiles),
        (K::Monitor, K::DeleteTitleAndFiles),
        (K::MoveToLibrary, K::MoveToRoot),
        (K::MoveToLibrary, K::DeleteFiles),
        (K::MoveToLibrary, K::DeleteTitleAndFiles),
        (K::MoveToRoot, K::DeleteFiles),
        (K::MoveToRoot, K::DeleteTitleAndFiles),
    ];
    PAIRS
        .into_iter()
        .find(|(first, second)| present.contains(first) && present.contains(second))
}

fn unmonitor_covers_scope(
    parameters: &LifecycleActionStepParameters,
    subject: LifecycleSubjectKind,
) -> bool {
    match subject {
        LifecycleSubjectKind::Show => matches!(
            parameters,
            LifecycleActionStepParameters::Unmonitor {
                include_descendants: true
            }
        ),
        LifecycleSubjectKind::Movie
        | LifecycleSubjectKind::Season
        | LifecycleSubjectKind::Episode => true,
    }
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum LifecycleActionSequenceError {
    #[error("lifecycle action sequence schema version {found} is unsupported; expected {expected}")]
    UnsupportedSchemaVersion { found: u32, expected: u32 },

    #[error("lifecycle action sequence has {found} steps; at most {maximum} are allowed")]
    TooManySteps { found: usize, maximum: usize },

    #[error("'{id}' is not a valid stable lifecycle action step id")]
    InvalidStepId { id: String },

    #[error("lifecycle action sequence repeats step id '{id}'")]
    DuplicateStepId { id: String },

    #[error("lifecycle action sequence repeats primitive '{kind:?}'")]
    DuplicateStepKind { kind: LifecycleActionStepKind },

    #[error("lifecycle action step '{kind:?}' does not support subject '{subject:?}'")]
    UnsupportedSubject {
        kind: LifecycleActionStepKind,
        subject: LifecycleSubjectKind,
    },

    #[error("lifecycle action step '{kind:?}' has parameters of the wrong shape")]
    ParameterShapeMismatch { kind: LifecycleActionStepKind },

    #[error("a season unmonitor step must include its episodes")]
    SeasonUnmonitorRequiresDescendants,

    #[error("delete_files requires an earlier complete-covering unmonitor step")]
    DeleteFilesRequiresUnmonitor,

    #[error("delete_files cannot be combined with terminal delete_title_and_files")]
    RedundantDeleteFilesBeforeDeleteTitle,

    #[error("a conditional search must immediately follow change_quality_profile")]
    ConditionalSearchRequiresPreviousProfile,

    #[error("terminal lifecycle action '{kind:?}' must be the final step")]
    TerminalStepMustBeLast { kind: LifecycleActionStepKind },

    #[error("tag '{label}' is both added and removed by the same sequence")]
    OpposingTagOperations { label: String },

    #[error("lifecycle action step '{kind:?}' is not allowed on a storage-root rule")]
    UnsupportedStorageRootStep { kind: LifecycleActionStepKind },

    #[error("target quality profile id must not be empty")]
    EmptyQualityProfileTarget,

    #[error("a tag action must name at least one tag")]
    EmptyTagList,

    #[error("'{label}' is not a usable tag: {reason}")]
    InvalidTagLabel { label: String, reason: String },

    #[error("a tag action may name at most {maximum} tags, and this one names {found}")]
    TooManyTags { found: usize, maximum: usize },

    #[error("'{label}' is named twice in the same tag action")]
    DuplicateTagLabel { label: String },

    #[error("lifecycle actions '{first:?}' and '{second:?}' cannot run together")]
    ConflictingSteps {
        first: LifecycleActionStepKind,
        second: LifecycleActionStepKind,
    },

    #[error("a move must name its library or root")]
    EmptyMoveTarget,

    #[error("invalid lifecycle action envelope: {0}")]
    InvalidEnvelope(String),

    #[error("could not serialize lifecycle action content: {0}")]
    Serialization(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(
        id: &str,
        kind: LifecycleActionStepKind,
        parameters: LifecycleActionStepParameters,
    ) -> LifecycleActionStep {
        LifecycleActionStep {
            id: id.into(),
            kind,
            parameters,
        }
    }

    #[test]
    fn a_sequence_names_its_schema_explicitly() {
        let sequence = LifecycleActionSequence::new(vec![]);
        let json = sequence_json(&sequence);
        assert_eq!(json, r#"{"schema_version":2,"steps":[]}"#);
        assert_eq!(
            serde_json::from_str::<LifecycleActionSequence>(&json).unwrap(),
            sequence
        );
    }

    fn tags_step(kind: LifecycleActionStepKind, tags: &[&str]) -> LifecycleActionSequence {
        LifecycleActionSequence::new(vec![step(
            "tags",
            kind,
            LifecycleActionStepParameters::Tags {
                tags: tags.iter().map(|tag| tag.to_string()).collect(),
            },
        )])
    }

    #[test]
    fn steps_refuse_unusable_profile_and_tag_parameters() {
        for blank in ["", "   "] {
            let sequence = LifecycleActionSequence::new(vec![step(
                "profile",
                LifecycleActionStepKind::ChangeQualityProfile,
                LifecycleActionStepParameters::ChangeQualityProfile {
                    target_quality_profile_id: blank.to_string(),
                },
            )]);
            assert_eq!(
                sequence.validate(LifecycleSubjectKind::Movie),
                Err(LifecycleActionSequenceError::EmptyQualityProfileTarget)
            );
        }

        use LifecycleActionStepKind::{AddTags, RemoveTags};
        for kind in [AddTags, RemoveTags] {
            // A tag step with no tags does nothing at all; that is a mistake,
            // not a no-op to run.
            assert_eq!(
                tags_step(kind, &[]).validate(LifecycleSubjectKind::Movie),
                Err(LifecycleActionSequenceError::EmptyTagList)
            );
        }
        // Membership is by exact label, so a spelling the assignment path would
        // normalize is refused rather than silently rewritten.
        assert!(matches!(
            tags_step(AddTags, &["Needs  Review"]).validate(LifecycleSubjectKind::Show),
            Err(LifecycleActionSequenceError::InvalidTagLabel { .. })
        ));
        // The reserved namespace is settings, never a user tag.
        assert!(matches!(
            tags_step(AddTags, &["scryer:monitor-type:all"]).validate(LifecycleSubjectKind::Show),
            Err(LifecycleActionSequenceError::InvalidTagLabel { .. })
        ));
        assert_eq!(
            tags_step(RemoveTags, &["keep", "keep"]).validate(LifecycleSubjectKind::Movie),
            Err(LifecycleActionSequenceError::DuplicateTagLabel {
                label: "keep".to_string()
            })
        );
        let too_many: Vec<String> = (0..=crate::MAX_USER_TAGS_PER_TITLE)
            .map(|index| format!("tag-{index}"))
            .collect();
        let too_many: Vec<&str> = too_many.iter().map(String::as_str).collect();
        assert_eq!(
            tags_step(AddTags, &too_many).validate(LifecycleSubjectKind::Movie),
            Err(LifecycleActionSequenceError::TooManyTags {
                found: too_many.len(),
                maximum: crate::MAX_USER_TAGS_PER_TITLE,
            })
        );
        assert!(
            tags_step(AddTags, &["needs review"])
                .validate(LifecycleSubjectKind::Movie)
                .is_ok()
        );
    }

    #[test]
    fn sequence_validation_rejects_closed_catalog_violations() {
        let duplicate_ids = LifecycleActionSequence::new(vec![
            step(
                "same",
                LifecycleActionStepKind::Unmonitor,
                LifecycleActionStepParameters::Unmonitor {
                    include_descendants: true,
                },
            ),
            step(
                "same",
                LifecycleActionStepKind::DeleteFiles,
                LifecycleActionStepParameters::None,
            ),
        ]);
        assert!(matches!(
            duplicate_ids.validate(LifecycleSubjectKind::Show),
            Err(LifecycleActionSequenceError::DuplicateStepId { .. })
        ));

        let terminal_not_last = LifecycleActionSequence::new(vec![
            step(
                "delete",
                LifecycleActionStepKind::DeleteTitleAndFiles,
                LifecycleActionStepParameters::None,
            ),
            step(
                "search",
                LifecycleActionStepKind::Search,
                LifecycleActionStepParameters::Search {
                    condition: LifecycleSearchCondition::Unconditional,
                },
            ),
        ]);
        assert!(matches!(
            terminal_not_last.validate(LifecycleSubjectKind::Movie),
            Err(LifecycleActionSequenceError::TerminalStepMustBeLast { .. })
        ));

        let without_unmonitor = LifecycleActionSequence::new(vec![step(
            "delete-files",
            LifecycleActionStepKind::DeleteFiles,
            LifecycleActionStepParameters::None,
        )]);
        assert_eq!(
            without_unmonitor.validate(LifecycleSubjectKind::Episode),
            Err(LifecycleActionSequenceError::DeleteFilesRequiresUnmonitor)
        );
    }

    #[test]
    fn sequence_hash_is_order_bound_and_storage_scope_is_closed() {
        let unmonitor = step(
            "unmonitor",
            LifecycleActionStepKind::Unmonitor,
            LifecycleActionStepParameters::Unmonitor {
                include_descendants: true,
            },
        );
        let delete = step(
            "delete-files",
            LifecycleActionStepKind::DeleteFiles,
            LifecycleActionStepParameters::None,
        );
        let forward = LifecycleActionSequence::new(vec![unmonitor.clone(), delete.clone()]);
        let reverse = LifecycleActionSequence::new(vec![delete, unmonitor]);
        assert_ne!(
            forward.content_hash().unwrap(),
            reverse.content_hash().unwrap()
        );
        assert!(
            forward
                .validate_for_storage_root(LifecycleSubjectKind::Show)
                .is_ok()
        );
    }

    #[test]
    fn opposing_tag_operations_and_extra_fields_are_rejected() {
        let sequence = LifecycleActionSequence::new(vec![
            step(
                "add",
                LifecycleActionStepKind::AddTags,
                LifecycleActionStepParameters::Tags {
                    tags: vec!["keep".into()],
                },
            ),
            step(
                "remove",
                LifecycleActionStepKind::RemoveTags,
                LifecycleActionStepParameters::Tags {
                    tags: vec!["keep".into()],
                },
            ),
        ]);
        assert!(matches!(
            sequence.validate(LifecycleSubjectKind::Movie),
            Err(LifecycleActionSequenceError::OpposingTagOperations { .. })
        ));
        assert!(
            serde_json::from_str::<LifecycleActionSequence>(
                r#"{"schema_version":2,"steps":[],"extra":true}"#
            )
            .is_err()
        );
    }

    #[test]
    fn conditional_search_is_closed_to_the_profile_step_and_delete_pair_is_rejected() {
        let conditional = LifecycleActionSequence::new(vec![step(
            "search",
            LifecycleActionStepKind::Search,
            LifecycleActionStepParameters::Search {
                condition: LifecycleSearchCondition::PreviousProfileChanged,
            },
        )]);
        assert_eq!(
            conditional.validate(LifecycleSubjectKind::Movie),
            Err(LifecycleActionSequenceError::ConditionalSearchRequiresPreviousProfile)
        );

        let redundant_delete = LifecycleActionSequence::new(vec![
            step(
                "unmonitor",
                LifecycleActionStepKind::Unmonitor,
                LifecycleActionStepParameters::Unmonitor {
                    include_descendants: true,
                },
            ),
            step(
                "delete-files",
                LifecycleActionStepKind::DeleteFiles,
                LifecycleActionStepParameters::None,
            ),
            step(
                "delete-title",
                LifecycleActionStepKind::DeleteTitleAndFiles,
                LifecycleActionStepParameters::None,
            ),
        ]);
        assert_eq!(
            redundant_delete.validate(LifecycleSubjectKind::Movie),
            Err(LifecycleActionSequenceError::RedundantDeleteFilesBeforeDeleteTitle)
        );
    }

    #[test]
    fn season_unmonitor_cannot_claim_a_narrower_effect_than_execution_performs() {
        let sequence = LifecycleActionSequence::new(vec![step(
            "unmonitor",
            LifecycleActionStepKind::Unmonitor,
            LifecycleActionStepParameters::Unmonitor {
                include_descendants: false,
            },
        )]);
        assert_eq!(
            sequence.validate(LifecycleSubjectKind::Season),
            Err(LifecycleActionSequenceError::SeasonUnmonitorRequiresDescendants)
        );
        assert!(sequence.validate(LifecycleSubjectKind::Episode).is_ok());
    }

    #[test]
    fn conflicting_steps_are_refused_as_a_whole_sequence() {
        let monitor = step(
            "monitor",
            LifecycleActionStepKind::Monitor,
            LifecycleActionStepParameters::None,
        );
        let unmonitor = step(
            "unmonitor",
            LifecycleActionStepKind::Unmonitor,
            LifecycleActionStepParameters::Unmonitor {
                include_descendants: true,
            },
        );
        let to_library = step(
            "move_to_library",
            LifecycleActionStepKind::MoveToLibrary,
            LifecycleActionStepParameters::Library {
                library: "Fixture Archive".into(),
            },
        );
        let to_root = step(
            "move_to_root",
            LifecycleActionStepKind::MoveToRoot,
            LifecycleActionStepParameters::Root {
                root: "/fixture/archive".into(),
            },
        );
        let delete_title = step(
            "delete_title_and_files",
            LifecycleActionStepKind::DeleteTitleAndFiles,
            LifecycleActionStepParameters::None,
        );
        for (steps, first, second) in [
            (
                vec![monitor.clone(), unmonitor.clone()],
                LifecycleActionStepKind::Monitor,
                LifecycleActionStepKind::Unmonitor,
            ),
            (
                vec![to_library.clone(), to_root.clone()],
                LifecycleActionStepKind::MoveToLibrary,
                LifecycleActionStepKind::MoveToRoot,
            ),
            (
                vec![to_root.clone(), delete_title],
                LifecycleActionStepKind::MoveToRoot,
                LifecycleActionStepKind::DeleteTitleAndFiles,
            ),
        ] {
            assert_eq!(
                LifecycleActionSequence::new(steps).validate(LifecycleSubjectKind::Movie),
                Err(LifecycleActionSequenceError::ConflictingSteps { first, second })
            );
        }
        assert!(
            LifecycleActionSequence::new(vec![monitor, to_library, unmonitor.clone()])
                .validate(LifecycleSubjectKind::Show)
                .is_err(),
            "monitor and unmonitor conflict wherever they sit"
        );
        assert!(
            LifecycleActionSequence::new(vec![to_root, unmonitor])
                .validate(LifecycleSubjectKind::Show)
                .is_ok()
        );
    }

    #[test]
    fn moves_are_whole_title_steps_with_a_named_target_and_never_storage_root_steps() {
        let to_library = |library: &str| {
            LifecycleActionSequence::new(vec![step(
                "move_to_library",
                LifecycleActionStepKind::MoveToLibrary,
                LifecycleActionStepParameters::Library {
                    library: library.into(),
                },
            )])
        };
        assert!(
            to_library("Fixture Archive")
                .validate(LifecycleSubjectKind::Movie)
                .is_ok()
        );
        assert_eq!(
            to_library(" ").validate(LifecycleSubjectKind::Movie),
            Err(LifecycleActionSequenceError::EmptyMoveTarget)
        );
        assert!(matches!(
            to_library("Fixture Archive").validate(LifecycleSubjectKind::Episode),
            Err(LifecycleActionSequenceError::UnsupportedSubject { .. })
        ));
        assert!(matches!(
            to_library("Fixture Archive").validate_for_storage_root(LifecycleSubjectKind::Movie),
            Err(LifecycleActionSequenceError::UnsupportedStorageRootStep { .. })
        ));
        let mismatched = LifecycleActionSequence::new(vec![step(
            "move_to_root",
            LifecycleActionStepKind::MoveToRoot,
            LifecycleActionStepParameters::Library {
                library: "Fixture Archive".into(),
            },
        )]);
        assert!(matches!(
            mismatched.validate(LifecycleSubjectKind::Movie),
            Err(LifecycleActionSequenceError::ParameterShapeMismatch { .. })
        ));
        let monitor_episode = LifecycleActionSequence::new(vec![
            step(
                "monitor",
                LifecycleActionStepKind::Monitor,
                LifecycleActionStepParameters::None,
            ),
            step(
                "fetch_subtitles",
                LifecycleActionStepKind::FetchSubtitles,
                LifecycleActionStepParameters::None,
            ),
        ]);
        assert!(
            monitor_episode
                .validate(LifecycleSubjectKind::Episode)
                .is_ok()
        );
    }

    /// Sequences stored before the newer step kinds existed keep their exact
    /// form, so their content hash and every recorded step stay valid.
    #[test]
    fn a_stored_sequence_of_the_original_kinds_reads_and_hashes_unchanged() {
        let stored = r#"{"schema_version":2,"steps":[{"id":"unmonitor","kind":"unmonitor","parameters":{"unmonitor":{"include_descendants":true}}},{"id":"delete_files","kind":"delete_files"}]}"#;
        let sequence: LifecycleActionSequence =
            serde_json::from_str(stored).expect("stored sequence reads");
        assert_eq!(
            serde_json::to_string(&sequence).expect("serializes"),
            stored
        );
        assert_eq!(
            sequence.content_hash().expect("hash"),
            blake3::hash(stored.as_bytes()).to_hex().to_string()
        );
    }

    #[test]
    fn sequence_rejects_more_than_the_bounded_catalog_limit() {
        let steps = (0..LIFECYCLE_MAX_ACTION_SEQUENCE_STEPS + 1)
            .map(|index| {
                step(
                    &format!("step-{index}"),
                    LifecycleActionStepKind::Search,
                    LifecycleActionStepParameters::Search {
                        condition: LifecycleSearchCondition::Unconditional,
                    },
                )
            })
            .collect();
        assert!(matches!(
            LifecycleActionSequence::new(steps).validate(LifecycleSubjectKind::Movie),
            Err(LifecycleActionSequenceError::TooManySteps { .. })
        ));
    }

    fn sequence_json(sequence: &LifecycleActionSequence) -> String {
        serde_json::to_string(sequence).unwrap()
    }
}
