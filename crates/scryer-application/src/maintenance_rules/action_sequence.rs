//! Closed, versioned definitions for composable maintenance actions.
//!
//! A v2 sequence is deliberately separate from the v1
//! [`MaintenanceActionSpec`](super::MaintenanceActionSpec) wire shape. Stored
//! v1 revisions remain raw v1 JSON; a v2 revision is recognized only by its
//! schema-2 sequence envelope. Policy evaluates membership only and never gets
//! to choose an action or supply a parameter.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use super::{
    MaintenanceActionKind, MaintenanceActionParameters, MaintenanceActionSpec,
    MaintenanceActionSpecError, MaintenanceEffectClass, MaintenanceRiskClass,
    MaintenanceSubjectKind,
};

/// Persisted candidate and legacy-projection value for a v2 sequence.
///
/// This is an output-only sentinel. It is intentionally absent from the v1
/// [`MaintenanceActionKind`] enum, so a legacy action payload cannot turn into
/// an empty sequence by naming this value.
pub const ACTION_SEQUENCE_KIND: &str = "action_sequence";

/// The only accepted v2 sequence schema.
pub const MAINTENANCE_ACTION_SEQUENCE_SCHEMA_VERSION: u32 = 2;

/// A sequence is intentionally compact: every supported primitive may appear
/// once, and the closed catalog currently contains seven primitives.
pub const MAINTENANCE_MAX_ACTION_SEQUENCE_STEPS: usize = 7;

/// Whether a durable action job is complete when it is accepted or only when
/// later reconciliation proves its postcondition.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MaintenanceActionCompletionPolicy {
    Accepted,
    Completed,
}

/// The closed v2 primitive catalog. These variants are not aliases for v1
/// presets: v1 retains its serialized shape and execution behavior.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MaintenanceActionStepKind {
    Unmonitor,
    DeleteFiles,
    ChangeQualityProfile,
    Search,
    AddTags,
    RemoveTags,
    DeleteTitleAndFiles,
}

/// Search execution condition. A sequence author must make conditional search
/// explicit so a profile/search preset cannot silently become an unconditional
/// search after a reorder.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MaintenanceSearchCondition {
    Unconditional,
    PreviousProfileChanged,
}

impl MaintenanceActionStepKind {
    pub const ALL: &'static [Self] = &[
        Self::Unmonitor,
        Self::DeleteFiles,
        Self::ChangeQualityProfile,
        Self::Search,
        Self::AddTags,
        Self::RemoveTags,
        Self::DeleteTitleAndFiles,
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
        }
    }

    pub const fn descriptor(self) -> &'static MaintenanceActionStepDescriptor {
        action_sequence_descriptor_for(self)
    }
}

/// Typed configuration for a v2 primitive. There is no free-form JSON escape
/// hatch: a mismatched kind/parameter pair is rejected before persistence.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum MaintenanceActionStepParameters {
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
        condition: MaintenanceSearchCondition,
    },
    Tags {
        tags: Vec<String>,
    },
}

impl MaintenanceActionStepParameters {
    fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }

    fn matches_kind(&self, kind: MaintenanceActionStepKind) -> bool {
        match kind {
            MaintenanceActionStepKind::Unmonitor => matches!(self, Self::Unmonitor { .. }),
            MaintenanceActionStepKind::ChangeQualityProfile => {
                matches!(self, Self::ChangeQualityProfile { .. })
            }
            MaintenanceActionStepKind::Search => matches!(self, Self::Search { .. }),
            MaintenanceActionStepKind::AddTags | MaintenanceActionStepKind::RemoveTags => {
                matches!(self, Self::Tags { .. })
            }
            MaintenanceActionStepKind::DeleteFiles
            | MaintenanceActionStepKind::DeleteTitleAndFiles => self.is_none(),
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
pub struct MaintenanceActionStep {
    pub id: String,
    pub kind: MaintenanceActionStepKind,
    #[serde(
        default,
        skip_serializing_if = "MaintenanceActionStepParameters::is_none"
    )]
    pub parameters: MaintenanceActionStepParameters,
}

impl MaintenanceActionStep {
    pub fn descriptor(&self) -> &'static MaintenanceActionStepDescriptor {
        action_sequence_descriptor_for(self.kind)
    }
}

/// The explicit schema-2 persisted envelope. `content_hash` is computed from
/// this canonical ordered content and is never accepted from a client payload.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct MaintenanceActionSequence {
    pub schema_version: u32,
    #[serde(default)]
    pub steps: Vec<MaintenanceActionStep>,
}

impl MaintenanceActionSequence {
    pub fn new(steps: Vec<MaintenanceActionStep>) -> Self {
        Self {
            schema_version: MAINTENANCE_ACTION_SEQUENCE_SCHEMA_VERSION,
            steps,
        }
    }

    /// Computes the revision-bound identity from ordered content. A reorder is
    /// a new hash even when it contains the same individual steps.
    pub fn content_hash(&self) -> Result<String, MaintenanceActionSequenceError> {
        let bytes = serde_json::to_vec(self)
            .map_err(|error| MaintenanceActionSequenceError::Serialization(error.to_string()))?;
        Ok(blake3::hash(&bytes).to_hex().to_string())
    }

    pub fn validate(
        &self,
        subject: MaintenanceSubjectKind,
    ) -> Result<(), MaintenanceActionSequenceError> {
        if self.schema_version != MAINTENANCE_ACTION_SEQUENCE_SCHEMA_VERSION {
            return Err(MaintenanceActionSequenceError::UnsupportedSchemaVersion {
                found: self.schema_version,
                expected: MAINTENANCE_ACTION_SEQUENCE_SCHEMA_VERSION,
            });
        }
        if self.steps.len() > MAINTENANCE_MAX_ACTION_SEQUENCE_STEPS {
            return Err(MaintenanceActionSequenceError::TooManySteps {
                found: self.steps.len(),
                maximum: MAINTENANCE_MAX_ACTION_SEQUENCE_STEPS,
            });
        }

        let mut ids = HashSet::new();
        let mut kinds = HashSet::new();
        let mut added_tags = HashSet::new();
        let mut removed_tags = HashSet::new();
        let mut prior_covering_unmonitor = false;
        let mut includes_delete_files = false;

        for (index, step) in self.steps.iter().enumerate() {
            validate_step_id(&step.id)?;
            if !ids.insert(step.id.as_str()) {
                return Err(MaintenanceActionSequenceError::DuplicateStepId {
                    id: step.id.clone(),
                });
            }
            if !kinds.insert(step.kind) {
                return Err(MaintenanceActionSequenceError::DuplicateStepKind { kind: step.kind });
            }
            let descriptor = step.descriptor();
            if !descriptor.supports_subject(subject) {
                return Err(MaintenanceActionSequenceError::UnsupportedSubject {
                    kind: step.kind,
                    subject,
                });
            }
            if !step.parameters.matches_kind(step.kind) {
                return Err(MaintenanceActionSequenceError::ParameterShapeMismatch {
                    kind: step.kind,
                });
            }
            validate_step_parameters(step, subject)?;

            if step.kind == MaintenanceActionStepKind::DeleteFiles && !prior_covering_unmonitor {
                return Err(MaintenanceActionSequenceError::DeleteFilesRequiresUnmonitor);
            }
            if step.kind == MaintenanceActionStepKind::DeleteFiles {
                includes_delete_files = true;
            }
            if step.kind == MaintenanceActionStepKind::DeleteTitleAndFiles && includes_delete_files
            {
                return Err(MaintenanceActionSequenceError::RedundantDeleteFilesBeforeDeleteTitle);
            }
            if matches!(
                &step.parameters,
                MaintenanceActionStepParameters::Search {
                    condition: MaintenanceSearchCondition::PreviousProfileChanged
                }
            ) && !self.steps[..index].last().is_some_and(|previous| {
                previous.kind == MaintenanceActionStepKind::ChangeQualityProfile
            }) {
                return Err(
                    MaintenanceActionSequenceError::ConditionalSearchRequiresPreviousProfile,
                );
            }
            if step.kind == MaintenanceActionStepKind::Unmonitor
                && unmonitor_covers_scope(&step.parameters, subject)
            {
                prior_covering_unmonitor = true;
            }
            if step.kind == MaintenanceActionStepKind::AddTags {
                added_tags.extend(step.parameters.tag_labels().iter().cloned());
            }
            if step.kind == MaintenanceActionStepKind::RemoveTags {
                removed_tags.extend(step.parameters.tag_labels().iter().cloned());
            }
            if descriptor.terminal && index + 1 != self.steps.len() {
                return Err(MaintenanceActionSequenceError::TerminalStepMustBeLast {
                    kind: step.kind,
                });
            }
        }

        if let Some(label) = added_tags.intersection(&removed_tags).next() {
            return Err(MaintenanceActionSequenceError::OpposingTagOperations {
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
        subject: MaintenanceSubjectKind,
    ) -> Result<(), MaintenanceActionSequenceError> {
        self.validate(subject)?;
        if let Some(step) = self
            .steps
            .iter()
            .find(|step| !step.descriptor().storage_root_allowed)
        {
            return Err(MaintenanceActionSequenceError::UnsupportedStorageRootStep {
                kind: step.kind,
            });
        }
        Ok(())
    }

    /// Whether a Search is the preset form that depends on the immediately
    /// preceding profile mutation. A standalone Search is unconditional.
    pub fn search_condition(&self, step_index: usize) -> Option<MaintenanceSearchCondition> {
        match &self.steps.get(step_index)?.parameters {
            MaintenanceActionStepParameters::Search { condition } => Some(*condition),
            _ => None,
        }
    }

    /// The only profile step a conditional Search may consume. Validation
    /// already guarantees it is immediately preceding the Search.
    pub fn previous_profile_step(&self, step_index: usize) -> Option<&MaintenanceActionStep> {
        self.steps
            .get(step_index.checked_sub(1)?)
            .filter(|step| step.kind == MaintenanceActionStepKind::ChangeQualityProfile)
    }
}

/// Runtime-friendly representation of persisted revision configuration.
///
/// This enum intentionally has no serde representation. Persistence uses a
/// raw v1 spec or a raw v2 sequence so serializing the enum itself can never
/// rewrite legacy rows into a tagged envelope.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MaintenanceActionDefinition {
    Legacy(MaintenanceActionSpec),
    Sequence(MaintenanceActionSequence),
}

impl MaintenanceActionDefinition {
    pub fn action_kind(&self) -> &'static str {
        match self {
            Self::Legacy(spec) => spec.kind.as_wire_str(),
            Self::Sequence(_) => ACTION_SEQUENCE_KIND,
        }
    }

    pub fn validate(
        &self,
        subject: MaintenanceSubjectKind,
    ) -> Result<(), MaintenanceActionSequenceError> {
        match self {
            Self::Legacy(spec) => spec.validate(subject).map_err(Into::into),
            Self::Sequence(sequence) => sequence.validate(subject),
        }
    }

    pub fn validate_for_storage_root(
        &self,
        subject: MaintenanceSubjectKind,
    ) -> Result<(), MaintenanceActionSequenceError> {
        match self {
            Self::Legacy(spec) => spec.validate(subject).map_err(Into::into),
            Self::Sequence(sequence) => sequence.validate_for_storage_root(subject),
        }
    }

    pub fn to_persisted_json(&self) -> Result<String, MaintenanceActionSequenceError> {
        match self {
            Self::Legacy(spec) => serde_json::to_string(spec)
                .map_err(|error| MaintenanceActionSequenceError::Serialization(error.to_string())),
            Self::Sequence(sequence) => serde_json::to_string(sequence)
                .map_err(|error| MaintenanceActionSequenceError::Serialization(error.to_string())),
        }
    }
}

/// Reads either an unchanged raw v1 spec or the explicit raw v2 envelope.
pub fn action_definition_from_persisted_json(
    json: &str,
) -> Result<MaintenanceActionDefinition, MaintenanceActionSequenceError> {
    let value: serde_json::Value = serde_json::from_str(json)
        .map_err(|error| MaintenanceActionSequenceError::InvalidEnvelope(error.to_string()))?;
    let object = value.as_object().ok_or_else(|| {
        MaintenanceActionSequenceError::InvalidEnvelope(
            "maintenance action must be an object".into(),
        )
    })?;
    match object
        .get("schema_version")
        .and_then(serde_json::Value::as_u64)
    {
        Some(version) if version == MAINTENANCE_ACTION_SEQUENCE_SCHEMA_VERSION as u64 => {
            let sequence: MaintenanceActionSequence =
                serde_json::from_value(value).map_err(|error| {
                    MaintenanceActionSequenceError::InvalidEnvelope(error.to_string())
                })?;
            Ok(MaintenanceActionDefinition::Sequence(sequence))
        }
        Some(_) | None => {
            let spec: MaintenanceActionSpec = serde_json::from_value(value).map_err(|error| {
                MaintenanceActionSequenceError::InvalidEnvelope(error.to_string())
            })?;
            if spec.kind.as_wire_str() == ACTION_SEQUENCE_KIND {
                return Err(MaintenanceActionSequenceError::ActionSequenceSentinelRequiresEnvelope);
            }
            Ok(MaintenanceActionDefinition::Legacy(spec))
        }
    }
}

/// A dependency surfaced to authoring and execution. The executor verifies the
/// persisted evidence afresh; this descriptor only describes the rule shape.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MaintenanceActionStepRequirement {
    PriorCompleteCoveringUnmonitor,
}

/// Static backend-owned data for one v2 primitive.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MaintenanceActionStepDescriptor {
    pub id: &'static str,
    pub kind: MaintenanceActionStepKind,
    pub label: &'static str,
    pub supported_subjects: &'static [MaintenanceSubjectKind],
    /// A stable typed parameter discriminator for authoring clients.
    pub parameter_schema: &'static str,
    pub effect_classes: &'static [MaintenanceEffectClass],
    pub risk_class: MaintenanceRiskClass,
    pub requires: &'static [MaintenanceActionStepRequirement],
    pub terminal: bool,
    pub completion_policy: MaintenanceActionCompletionPolicy,
    pub storage_root_allowed: bool,
}

impl MaintenanceActionStepDescriptor {
    pub fn supports_subject(&self, subject: MaintenanceSubjectKind) -> bool {
        self.supported_subjects.contains(&subject)
    }
}

const ALL_SUBJECTS: &[MaintenanceSubjectKind] = MaintenanceSubjectKind::ALL;
const TITLE_SUBJECTS: &[MaintenanceSubjectKind] =
    &[MaintenanceSubjectKind::Movie, MaintenanceSubjectKind::Show];
const CATALOG_INTENT: &[MaintenanceEffectClass] = &[MaintenanceEffectClass::CatalogIntent];
const ACQUISITION: &[MaintenanceEffectClass] = &[MaintenanceEffectClass::Acquisition];
const DESTRUCTIVE_STORAGE: &[MaintenanceEffectClass] =
    &[MaintenanceEffectClass::DestructiveStorage];
const DELETE_FILES_REQUIREMENT: &[MaintenanceActionStepRequirement] =
    &[MaintenanceActionStepRequirement::PriorCompleteCoveringUnmonitor];
const NO_REQUIREMENTS: &[MaintenanceActionStepRequirement] = &[];

const UNMONITOR: MaintenanceActionStepDescriptor = MaintenanceActionStepDescriptor {
    id: "unmonitor",
    kind: MaintenanceActionStepKind::Unmonitor,
    label: "Unmonitor",
    supported_subjects: ALL_SUBJECTS,
    parameter_schema: "unmonitor",
    effect_classes: CATALOG_INTENT,
    risk_class: MaintenanceRiskClass::Medium,
    requires: NO_REQUIREMENTS,
    terminal: false,
    completion_policy: MaintenanceActionCompletionPolicy::Completed,
    storage_root_allowed: true,
};
const DELETE_FILES: MaintenanceActionStepDescriptor = MaintenanceActionStepDescriptor {
    id: "delete_files",
    kind: MaintenanceActionStepKind::DeleteFiles,
    label: "Delete files",
    supported_subjects: ALL_SUBJECTS,
    parameter_schema: "none",
    effect_classes: DESTRUCTIVE_STORAGE,
    risk_class: MaintenanceRiskClass::High,
    requires: DELETE_FILES_REQUIREMENT,
    terminal: false,
    completion_policy: MaintenanceActionCompletionPolicy::Completed,
    storage_root_allowed: true,
};
const CHANGE_QUALITY_PROFILE: MaintenanceActionStepDescriptor = MaintenanceActionStepDescriptor {
    id: "change_quality_profile",
    kind: MaintenanceActionStepKind::ChangeQualityProfile,
    label: "Change quality profile",
    supported_subjects: TITLE_SUBJECTS,
    parameter_schema: "change_quality_profile",
    effect_classes: CATALOG_INTENT,
    risk_class: MaintenanceRiskClass::Medium,
    requires: NO_REQUIREMENTS,
    terminal: false,
    completion_policy: MaintenanceActionCompletionPolicy::Completed,
    storage_root_allowed: false,
};
const SEARCH: MaintenanceActionStepDescriptor = MaintenanceActionStepDescriptor {
    id: "search",
    kind: MaintenanceActionStepKind::Search,
    label: "Search",
    supported_subjects: TITLE_SUBJECTS,
    parameter_schema: "search",
    effect_classes: ACQUISITION,
    risk_class: MaintenanceRiskClass::Medium,
    requires: NO_REQUIREMENTS,
    terminal: false,
    completion_policy: MaintenanceActionCompletionPolicy::Accepted,
    storage_root_allowed: false,
};
const ADD_TAGS: MaintenanceActionStepDescriptor = MaintenanceActionStepDescriptor {
    id: "add_tags",
    kind: MaintenanceActionStepKind::AddTags,
    label: "Add tags",
    supported_subjects: TITLE_SUBJECTS,
    parameter_schema: "tags",
    effect_classes: CATALOG_INTENT,
    risk_class: MaintenanceRiskClass::Low,
    requires: NO_REQUIREMENTS,
    terminal: false,
    completion_policy: MaintenanceActionCompletionPolicy::Completed,
    storage_root_allowed: false,
};
const REMOVE_TAGS: MaintenanceActionStepDescriptor = MaintenanceActionStepDescriptor {
    id: "remove_tags",
    kind: MaintenanceActionStepKind::RemoveTags,
    label: "Remove tags",
    supported_subjects: TITLE_SUBJECTS,
    parameter_schema: "tags",
    effect_classes: CATALOG_INTENT,
    risk_class: MaintenanceRiskClass::Low,
    requires: NO_REQUIREMENTS,
    terminal: false,
    completion_policy: MaintenanceActionCompletionPolicy::Completed,
    storage_root_allowed: false,
};
const DELETE_TITLE_AND_FILES: MaintenanceActionStepDescriptor = MaintenanceActionStepDescriptor {
    id: "delete_title_and_files",
    kind: MaintenanceActionStepKind::DeleteTitleAndFiles,
    label: "Delete title and files",
    supported_subjects: TITLE_SUBJECTS,
    parameter_schema: "none",
    effect_classes: DESTRUCTIVE_STORAGE,
    risk_class: MaintenanceRiskClass::High,
    requires: NO_REQUIREMENTS,
    terminal: true,
    completion_policy: MaintenanceActionCompletionPolicy::Completed,
    storage_root_allowed: false,
};

const ACTION_SEQUENCE_CATALOG: &[MaintenanceActionStepDescriptor] = &[
    UNMONITOR,
    DELETE_FILES,
    CHANGE_QUALITY_PROFILE,
    SEARCH,
    ADD_TAGS,
    REMOVE_TAGS,
    DELETE_TITLE_AND_FILES,
];

pub fn action_sequence_catalog() -> &'static [MaintenanceActionStepDescriptor] {
    ACTION_SEQUENCE_CATALOG
}

pub const fn action_sequence_descriptor_for(
    kind: MaintenanceActionStepKind,
) -> &'static MaintenanceActionStepDescriptor {
    match kind {
        MaintenanceActionStepKind::Unmonitor => &UNMONITOR,
        MaintenanceActionStepKind::DeleteFiles => &DELETE_FILES,
        MaintenanceActionStepKind::ChangeQualityProfile => &CHANGE_QUALITY_PROFILE,
        MaintenanceActionStepKind::Search => &SEARCH,
        MaintenanceActionStepKind::AddTags => &ADD_TAGS,
        MaintenanceActionStepKind::RemoveTags => &REMOVE_TAGS,
        MaintenanceActionStepKind::DeleteTitleAndFiles => &DELETE_TITLE_AND_FILES,
    }
}

fn validate_step_id(id: &str) -> Result<(), MaintenanceActionSequenceError> {
    if id.is_empty() || id.len() > 64 {
        return Err(MaintenanceActionSequenceError::InvalidStepId { id: id.to_string() });
    }
    if !id
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err(MaintenanceActionSequenceError::InvalidStepId { id: id.to_string() });
    }
    Ok(())
}

fn validate_step_parameters(
    step: &MaintenanceActionStep,
    subject: MaintenanceSubjectKind,
) -> Result<(), MaintenanceActionSequenceError> {
    match (&step.kind, &step.parameters) {
        (
            MaintenanceActionStepKind::Unmonitor,
            MaintenanceActionStepParameters::Unmonitor {
                include_descendants: false,
            },
        ) if subject == MaintenanceSubjectKind::Season => {
            Err(MaintenanceActionSequenceError::SeasonUnmonitorRequiresDescendants)
        }
        (
            MaintenanceActionStepKind::ChangeQualityProfile,
            MaintenanceActionStepParameters::ChangeQualityProfile {
                target_quality_profile_id,
            },
        ) => MaintenanceActionSpec::change_quality_profile(target_quality_profile_id.clone())
            .validate(subject)
            .map_err(Into::into),
        (
            MaintenanceActionStepKind::AddTags | MaintenanceActionStepKind::RemoveTags,
            MaintenanceActionStepParameters::Tags { tags },
        ) => MaintenanceActionSpec {
            kind: match step.kind {
                MaintenanceActionStepKind::AddTags => MaintenanceActionKind::AddTags,
                MaintenanceActionStepKind::RemoveTags => MaintenanceActionKind::RemoveTags,
                _ => unreachable!("matched tag step kind"),
            },
            schema_version: super::MAINTENANCE_ACTION_SCHEMA_VERSION,
            parameters: MaintenanceActionParameters::Tags { tags: tags.clone() },
        }
        .validate(subject)
        .map_err(Into::into),
        _ => Ok(()),
    }
}

fn unmonitor_covers_scope(
    parameters: &MaintenanceActionStepParameters,
    subject: MaintenanceSubjectKind,
) -> bool {
    match subject {
        MaintenanceSubjectKind::Show => matches!(
            parameters,
            MaintenanceActionStepParameters::Unmonitor {
                include_descendants: true
            }
        ),
        MaintenanceSubjectKind::Movie
        | MaintenanceSubjectKind::Season
        | MaintenanceSubjectKind::Episode => true,
    }
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum MaintenanceActionSequenceError {
    #[error(
        "maintenance action sequence schema version {found} is unsupported; expected {expected}"
    )]
    UnsupportedSchemaVersion { found: u32, expected: u32 },

    #[error("maintenance action sequence has {found} steps; at most {maximum} are allowed")]
    TooManySteps { found: usize, maximum: usize },

    #[error("'{id}' is not a valid stable maintenance action step id")]
    InvalidStepId { id: String },

    #[error("maintenance action sequence repeats step id '{id}'")]
    DuplicateStepId { id: String },

    #[error("maintenance action sequence repeats primitive '{kind:?}'")]
    DuplicateStepKind { kind: MaintenanceActionStepKind },

    #[error("maintenance action step '{kind:?}' does not support subject '{subject:?}'")]
    UnsupportedSubject {
        kind: MaintenanceActionStepKind,
        subject: MaintenanceSubjectKind,
    },

    #[error("maintenance action step '{kind:?}' has parameters of the wrong shape")]
    ParameterShapeMismatch { kind: MaintenanceActionStepKind },

    #[error("a season unmonitor step must include its episodes")]
    SeasonUnmonitorRequiresDescendants,

    #[error("delete_files requires an earlier complete-covering unmonitor step")]
    DeleteFilesRequiresUnmonitor,

    #[error("delete_files cannot be combined with terminal delete_title_and_files")]
    RedundantDeleteFilesBeforeDeleteTitle,

    #[error("a conditional search must immediately follow change_quality_profile")]
    ConditionalSearchRequiresPreviousProfile,

    #[error("terminal maintenance action '{kind:?}' must be the final step")]
    TerminalStepMustBeLast { kind: MaintenanceActionStepKind },

    #[error("tag '{label}' is both added and removed by the same sequence")]
    OpposingTagOperations { label: String },

    #[error("maintenance action step '{kind:?}' is not allowed on a storage-root rule")]
    UnsupportedStorageRootStep { kind: MaintenanceActionStepKind },

    #[error("the action_sequence sentinel is only valid with a schema-2 sequence envelope")]
    ActionSequenceSentinelRequiresEnvelope,

    #[error("invalid maintenance action envelope: {0}")]
    InvalidEnvelope(String),

    #[error("could not serialize maintenance action content: {0}")]
    Serialization(String),

    #[error(transparent)]
    Legacy(#[from] MaintenanceActionSpecError),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(
        id: &str,
        kind: MaintenanceActionStepKind,
        parameters: MaintenanceActionStepParameters,
    ) -> MaintenanceActionStep {
        MaintenanceActionStep {
            id: id.into(),
            kind,
            parameters,
        }
    }

    #[test]
    fn legacy_json_is_preserved_and_schema_two_is_explicit() {
        let legacy = r#"{"kind":"delete_title_and_files","schema_version":1}"#;
        let definition = action_definition_from_persisted_json(legacy).unwrap();
        assert_eq!(definition.to_persisted_json().unwrap(), legacy);

        let sequence = MaintenanceActionSequence::new(vec![]);
        let json = sequence_json(&sequence);
        assert_eq!(
            action_definition_from_persisted_json(&json).unwrap(),
            MaintenanceActionDefinition::Sequence(sequence)
        );
    }

    #[test]
    fn sentinel_never_decodes_as_an_empty_legacy_action() {
        assert!(
            action_definition_from_persisted_json(
                r#"{"kind":"action_sequence","schema_version":1}"#
            )
            .is_err()
        );
    }

    #[test]
    fn sequence_validation_rejects_closed_catalog_violations() {
        let duplicate_ids = MaintenanceActionSequence::new(vec![
            step(
                "same",
                MaintenanceActionStepKind::Unmonitor,
                MaintenanceActionStepParameters::Unmonitor {
                    include_descendants: true,
                },
            ),
            step(
                "same",
                MaintenanceActionStepKind::DeleteFiles,
                MaintenanceActionStepParameters::None,
            ),
        ]);
        assert!(matches!(
            duplicate_ids.validate(MaintenanceSubjectKind::Show),
            Err(MaintenanceActionSequenceError::DuplicateStepId { .. })
        ));

        let terminal_not_last = MaintenanceActionSequence::new(vec![
            step(
                "delete",
                MaintenanceActionStepKind::DeleteTitleAndFiles,
                MaintenanceActionStepParameters::None,
            ),
            step(
                "search",
                MaintenanceActionStepKind::Search,
                MaintenanceActionStepParameters::Search {
                    condition: MaintenanceSearchCondition::Unconditional,
                },
            ),
        ]);
        assert!(matches!(
            terminal_not_last.validate(MaintenanceSubjectKind::Movie),
            Err(MaintenanceActionSequenceError::TerminalStepMustBeLast { .. })
        ));

        let without_unmonitor = MaintenanceActionSequence::new(vec![step(
            "delete-files",
            MaintenanceActionStepKind::DeleteFiles,
            MaintenanceActionStepParameters::None,
        )]);
        assert_eq!(
            without_unmonitor.validate(MaintenanceSubjectKind::Episode),
            Err(MaintenanceActionSequenceError::DeleteFilesRequiresUnmonitor)
        );
    }

    #[test]
    fn sequence_hash_is_order_bound_and_storage_scope_is_closed() {
        let unmonitor = step(
            "unmonitor",
            MaintenanceActionStepKind::Unmonitor,
            MaintenanceActionStepParameters::Unmonitor {
                include_descendants: true,
            },
        );
        let delete = step(
            "delete-files",
            MaintenanceActionStepKind::DeleteFiles,
            MaintenanceActionStepParameters::None,
        );
        let forward = MaintenanceActionSequence::new(vec![unmonitor.clone(), delete.clone()]);
        let reverse = MaintenanceActionSequence::new(vec![delete, unmonitor]);
        assert_ne!(
            forward.content_hash().unwrap(),
            reverse.content_hash().unwrap()
        );
        assert!(
            forward
                .validate_for_storage_root(MaintenanceSubjectKind::Show)
                .is_ok()
        );
    }

    #[test]
    fn opposing_tag_operations_and_extra_fields_are_rejected() {
        let sequence = MaintenanceActionSequence::new(vec![
            step(
                "add",
                MaintenanceActionStepKind::AddTags,
                MaintenanceActionStepParameters::Tags {
                    tags: vec!["keep".into()],
                },
            ),
            step(
                "remove",
                MaintenanceActionStepKind::RemoveTags,
                MaintenanceActionStepParameters::Tags {
                    tags: vec!["keep".into()],
                },
            ),
        ]);
        assert!(matches!(
            sequence.validate(MaintenanceSubjectKind::Movie),
            Err(MaintenanceActionSequenceError::OpposingTagOperations { .. })
        ));
        assert!(
            serde_json::from_str::<MaintenanceActionSequence>(
                r#"{"schema_version":2,"steps":[],"extra":true}"#
            )
            .is_err()
        );
    }

    #[test]
    fn conditional_search_is_closed_to_the_profile_step_and_delete_pair_is_rejected() {
        let conditional = MaintenanceActionSequence::new(vec![step(
            "search",
            MaintenanceActionStepKind::Search,
            MaintenanceActionStepParameters::Search {
                condition: MaintenanceSearchCondition::PreviousProfileChanged,
            },
        )]);
        assert_eq!(
            conditional.validate(MaintenanceSubjectKind::Movie),
            Err(MaintenanceActionSequenceError::ConditionalSearchRequiresPreviousProfile)
        );

        let redundant_delete = MaintenanceActionSequence::new(vec![
            step(
                "unmonitor",
                MaintenanceActionStepKind::Unmonitor,
                MaintenanceActionStepParameters::Unmonitor {
                    include_descendants: true,
                },
            ),
            step(
                "delete-files",
                MaintenanceActionStepKind::DeleteFiles,
                MaintenanceActionStepParameters::None,
            ),
            step(
                "delete-title",
                MaintenanceActionStepKind::DeleteTitleAndFiles,
                MaintenanceActionStepParameters::None,
            ),
        ]);
        assert_eq!(
            redundant_delete.validate(MaintenanceSubjectKind::Movie),
            Err(MaintenanceActionSequenceError::RedundantDeleteFilesBeforeDeleteTitle)
        );
    }

    #[test]
    fn season_unmonitor_cannot_claim_a_narrower_effect_than_execution_performs() {
        let sequence = MaintenanceActionSequence::new(vec![step(
            "unmonitor",
            MaintenanceActionStepKind::Unmonitor,
            MaintenanceActionStepParameters::Unmonitor {
                include_descendants: false,
            },
        )]);
        assert_eq!(
            sequence.validate(MaintenanceSubjectKind::Season),
            Err(MaintenanceActionSequenceError::SeasonUnmonitorRequiresDescendants)
        );
        assert!(sequence.validate(MaintenanceSubjectKind::Episode).is_ok());
    }

    #[test]
    fn sequence_rejects_more_than_the_bounded_catalog_limit() {
        let steps = (0..MAINTENANCE_MAX_ACTION_SEQUENCE_STEPS + 1)
            .map(|index| {
                step(
                    &format!("step-{index}"),
                    MaintenanceActionStepKind::Search,
                    MaintenanceActionStepParameters::Search {
                        condition: MaintenanceSearchCondition::Unconditional,
                    },
                )
            })
            .collect();
        assert!(matches!(
            MaintenanceActionSequence::new(steps).validate(MaintenanceSubjectKind::Movie),
            Err(MaintenanceActionSequenceError::TooManySteps { .. })
        ));
    }

    fn sequence_json(sequence: &MaintenanceActionSequence) -> String {
        serde_json::to_string(sequence).unwrap()
    }
}
