//! The static maintenance action catalog (RFC 137 sections 9.1-9.3, 9.8, 9.9).
//!
//! The backend owns this registry. Clients — and policy evaluation — never
//! define new variants: Rego returns only `match`, `no_match`, or `unknown` and
//! never selects an action or supplies an action parameter (RFC 9.1). A rule
//! revision stores exactly one closed, schema-versioned
//! [`MaintenanceActionSpec`], validated by the host when the revision is
//! created.
//!
//! Nothing here is reachable outside tests yet; see the module docs for the
//! Track A2 dormancy contract.

use serde::{Deserialize, Serialize};

/// Current [`MaintenanceActionSpec`] schema version.
///
/// A stored spec is only accepted at this version; migrating a persisted
/// revision to a newer shape is an explicit forward step, never a silent
/// re-interpretation of old parameters.
pub const MAINTENANCE_ACTION_SCHEMA_VERSION: u32 = 1;

/// The closed set of persisted media-action variants for the Maintainerr-first
/// activation boundary (RFC 9.3 action table).
///
/// Post-parity candidates (`protect_from_lifecycle`, `refresh_title`,
/// `organize_files`, `ensure_subtitles`, standalone searches, …) have reserved
/// descriptor slots in the RFC but are deliberately absent from this enum: an
/// unknown wire name must fail to deserialize rather than land on a fallback.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MaintenanceActionKind {
    /// Membership/collection tracking only; no media mutation.
    DoNothing,
    /// Unmonitor the matched scope and keep its files.
    UnmonitorScopeKeepFiles,
    /// Delete the title and its files.
    DeleteTitleAndFiles,
    /// Unmonitor the title and delete its files while preserving the title.
    UnmonitorTitleDeleteAllFiles,
    /// Unmonitor the show and seasons and delete existing episode files.
    UnmonitorShowDeleteExistingFiles,
    /// Unmonitor the season or episode scope and delete its files.
    UnmonitorScopeDeleteFiles,
    /// Unmonitor the season and delete its files, then delete the show when a
    /// fresh parent check proves it is empty.
    UnmonitorSeasonDeleteFilesThenDeleteShowIfEmpty,
    /// Unmonitor the season, then unmonitor the show when a fresh parent check
    /// proves it is empty.
    UnmonitorSeasonThenUnmonitorShowIfEmpty,
    /// On the next action-handler pass, change the quality profile and search
    /// only when the current profile differs from the target.
    ChangeQualityProfileAndSearchIfChanged,
    /// Add the configured user tags to the matched title.
    AddTags,
    /// Remove the configured user tags from the matched title.
    RemoveTags,
}

impl MaintenanceActionKind {
    /// Every catalog kind, in catalog order.
    pub const ALL: &'static [Self] = &[
        Self::DoNothing,
        Self::UnmonitorScopeKeepFiles,
        Self::DeleteTitleAndFiles,
        Self::UnmonitorTitleDeleteAllFiles,
        Self::UnmonitorShowDeleteExistingFiles,
        Self::UnmonitorScopeDeleteFiles,
        Self::UnmonitorSeasonDeleteFilesThenDeleteShowIfEmpty,
        Self::UnmonitorSeasonThenUnmonitorShowIfEmpty,
        Self::ChangeQualityProfileAndSearchIfChanged,
        Self::AddTags,
        Self::RemoveTags,
    ];

    /// The static descriptor for this kind.
    pub fn descriptor(self) -> &'static MaintenanceActionDescriptor {
        descriptor_for(self)
    }

    /// Whether this kind may be configured against `subject` (RFC 9.3).
    pub fn supports_subject(self, subject: MaintenanceSubjectKind) -> bool {
        self.descriptor().supports_subject(subject)
    }

    /// The pinned wire name, identical to what serde serializes. A candidate
    /// row stores the action by this name rather than by a second encoding, so
    /// `action_kind_wire_names_are_pinned` covers both at once.
    pub const fn as_wire_str(self) -> &'static str {
        match self {
            Self::DoNothing => "do_nothing",
            Self::UnmonitorScopeKeepFiles => "unmonitor_scope_keep_files",
            Self::DeleteTitleAndFiles => "delete_title_and_files",
            Self::UnmonitorTitleDeleteAllFiles => "unmonitor_title_delete_all_files",
            Self::UnmonitorShowDeleteExistingFiles => "unmonitor_show_delete_existing_files",
            Self::UnmonitorScopeDeleteFiles => "unmonitor_scope_delete_files",
            Self::UnmonitorSeasonDeleteFilesThenDeleteShowIfEmpty => {
                "unmonitor_season_delete_files_then_delete_show_if_empty"
            }
            Self::UnmonitorSeasonThenUnmonitorShowIfEmpty => {
                "unmonitor_season_then_unmonitor_show_if_empty"
            }
            Self::ChangeQualityProfileAndSearchIfChanged => {
                "change_quality_profile_and_search_if_changed"
            }
            Self::AddTags => "add_tags",
            Self::RemoveTags => "remove_tags",
        }
    }

    /// Read a stored candidate's action back through the closed catalog. An
    /// unrecognized name is `None`: a row written by a newer build never
    /// collapses onto a fallback action here.
    pub fn parse_wire_str(value: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|kind| kind.as_wire_str() == value)
    }
}

/// The media subject a rule and its action are scoped to.
///
/// The source subject is preserved end to end: a season or episode match is
/// never widened into a title-level action (RFC 9.3).
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MaintenanceSubjectKind {
    Movie,
    Show,
    Season,
    Episode,
}

impl MaintenanceSubjectKind {
    /// Every subject kind.
    pub const ALL: &'static [Self] = &[Self::Movie, Self::Show, Self::Season, Self::Episode];
}

/// Risk class and its minimum control (RFC 9.2 risk table).
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MaintenanceRiskClass {
    /// No managed-media/catalog mutation; observe-only allowed.
    None,
    /// Reversible Scryer state, notification, or bounded refresh; normal arming
    /// and rate limit.
    Low,
    /// Changes acquisition intent, starts external work, or reorganizes files;
    /// affected-count preview and capability checks.
    Medium,
    /// Deletes, blocklists, or can make media unavailable; destructive
    /// confirmation, grace, current preview, cap, circuit breaker.
    High,
}

/// Effect classes used by the lifecycle coordinator's conflict arbitration
/// (RFC 9.9). Every action declares one or more.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MaintenanceEffectClass {
    Protect,
    Communicate,
    CatalogIntent,
    MetadataRepair,
    Acquisition,
    FileOrganization,
    DestructiveStorage,
}

/// When a due action becomes eligible to execute (RFC 9.3, Timing column).
///
/// The two "after grace and fresh parent checks" rows in the RFC table are
/// [`Self::AfterGrace`] here: the parent refetch is an execution precondition
/// enforced by the (not yet built) worker, not a distinct timing mode.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MaintenanceTimingMode {
    /// Membership/collection tracking only; nothing ever becomes due.
    MembershipTracking,
    /// Eligible after the configured grace from first continuous Scryer match;
    /// zero grace is immediately eligible but execution still waits for the
    /// action-handler schedule.
    AfterGrace,
    /// No grace window; evaluated on the next action-handler pass.
    ZeroGraceNextHandlerPass,
}

/// Repeat/idempotency semantics (RFC 9.8). The registry, not the user,
/// determines which modes are legal for a kind.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MaintenanceRepeatMode {
    /// Re-run only when the desired postcondition has drifted.
    EnsureState,
    /// Run once for a continuous match generation; a confirmed no-match and
    /// later rematch may create another.
    OncePerMatch,
    /// Run at a bounded configured cooldown while the match remains true.
    /// Reserved for post-parity actions.
    PeriodicWhileMatching,
    /// Reconcile a claim on match, hold on unknown, release on no-match.
    /// Reserved for post-parity actions.
    ContinuousClaim,
}

/// Static descriptor for one action kind (RFC 9.2).
///
/// `required_permissions`, `required_capabilities`, `preview_kind`,
/// `retry_class`, and `completion_postcondition` are also part of the RFC 9.2
/// descriptor shape. They arrive with the action executor wave (Track D) —
/// deliberately not stubbed here, because a placeholder type would have to be
/// re-designed once the permission-checked application operations and
/// postcondition probes exist.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MaintenanceActionDescriptor {
    pub kind: MaintenanceActionKind,
    pub schema_version: u32,
    pub supported_subjects: &'static [MaintenanceSubjectKind],
    pub effect_classes: &'static [MaintenanceEffectClass],
    pub risk_class: MaintenanceRiskClass,
    pub timing_mode: MaintenanceTimingMode,
    pub allowed_repeat_modes: &'static [MaintenanceRepeatMode],
}

impl MaintenanceActionDescriptor {
    /// Whether this action may be configured against `subject`.
    pub fn supports_subject(&self, subject: MaintenanceSubjectKind) -> bool {
        self.supported_subjects.contains(&subject)
    }

    /// Whether this action declares `effect`.
    pub fn has_effect(&self, effect: MaintenanceEffectClass) -> bool {
        self.effect_classes.contains(&effect)
    }
}

const ALL_SUBJECTS: &[MaintenanceSubjectKind] = MaintenanceSubjectKind::ALL;
const TITLE_SUBJECTS: &[MaintenanceSubjectKind] =
    &[MaintenanceSubjectKind::Movie, MaintenanceSubjectKind::Show];
const SHOW_ONLY: &[MaintenanceSubjectKind] = &[MaintenanceSubjectKind::Show];
const SEASON_ONLY: &[MaintenanceSubjectKind] = &[MaintenanceSubjectKind::Season];
const SEASON_OR_EPISODE: &[MaintenanceSubjectKind] = &[
    MaintenanceSubjectKind::Season,
    MaintenanceSubjectKind::Episode,
];

const ENSURE_STATE_ONLY: &[MaintenanceRepeatMode] = &[MaintenanceRepeatMode::EnsureState];
const ONCE_PER_MATCH_ONLY: &[MaintenanceRepeatMode] = &[MaintenanceRepeatMode::OncePerMatch];

const CATALOG_INTENT: &[MaintenanceEffectClass] = &[MaintenanceEffectClass::CatalogIntent];
const CATALOG_INTENT_AND_DESTRUCTIVE: &[MaintenanceEffectClass] = &[
    MaintenanceEffectClass::CatalogIntent,
    MaintenanceEffectClass::DestructiveStorage,
];
const DESTRUCTIVE_ONLY: &[MaintenanceEffectClass] = &[MaintenanceEffectClass::DestructiveStorage];
const CATALOG_INTENT_AND_ACQUISITION: &[MaintenanceEffectClass] = &[
    MaintenanceEffectClass::CatalogIntent,
    MaintenanceEffectClass::Acquisition,
];
/// `do_nothing` mutates no managed media at all; its only observable effect is
/// membership/collection tracking, which exists to feed the lifecycle
/// notification events. `communicate` is the one class in RFC 9.9 that carries
/// that meaning and coexists with every other result (arbitration rule 2).
const COMMUNICATE_ONLY: &[MaintenanceEffectClass] = &[MaintenanceEffectClass::Communicate];

const DO_NOTHING: MaintenanceActionDescriptor = MaintenanceActionDescriptor {
    kind: MaintenanceActionKind::DoNothing,
    schema_version: MAINTENANCE_ACTION_SCHEMA_VERSION,
    supported_subjects: ALL_SUBJECTS,
    effect_classes: COMMUNICATE_ONLY,
    risk_class: MaintenanceRiskClass::None,
    timing_mode: MaintenanceTimingMode::MembershipTracking,
    allowed_repeat_modes: ENSURE_STATE_ONLY,
};

const UNMONITOR_SCOPE_KEEP_FILES: MaintenanceActionDescriptor = MaintenanceActionDescriptor {
    kind: MaintenanceActionKind::UnmonitorScopeKeepFiles,
    schema_version: MAINTENANCE_ACTION_SCHEMA_VERSION,
    supported_subjects: ALL_SUBJECTS,
    effect_classes: CATALOG_INTENT,
    risk_class: MaintenanceRiskClass::Medium,
    timing_mode: MaintenanceTimingMode::AfterGrace,
    allowed_repeat_modes: ENSURE_STATE_ONLY,
};

const DELETE_TITLE_AND_FILES: MaintenanceActionDescriptor = MaintenanceActionDescriptor {
    kind: MaintenanceActionKind::DeleteTitleAndFiles,
    schema_version: MAINTENANCE_ACTION_SCHEMA_VERSION,
    supported_subjects: TITLE_SUBJECTS,
    effect_classes: DESTRUCTIVE_ONLY,
    risk_class: MaintenanceRiskClass::High,
    timing_mode: MaintenanceTimingMode::AfterGrace,
    allowed_repeat_modes: ONCE_PER_MATCH_ONLY,
};

const UNMONITOR_TITLE_DELETE_ALL_FILES: MaintenanceActionDescriptor = MaintenanceActionDescriptor {
    kind: MaintenanceActionKind::UnmonitorTitleDeleteAllFiles,
    schema_version: MAINTENANCE_ACTION_SCHEMA_VERSION,
    supported_subjects: TITLE_SUBJECTS,
    effect_classes: CATALOG_INTENT_AND_DESTRUCTIVE,
    risk_class: MaintenanceRiskClass::High,
    timing_mode: MaintenanceTimingMode::AfterGrace,
    allowed_repeat_modes: ONCE_PER_MATCH_ONLY,
};

const UNMONITOR_SHOW_DELETE_EXISTING_FILES: MaintenanceActionDescriptor =
    MaintenanceActionDescriptor {
        kind: MaintenanceActionKind::UnmonitorShowDeleteExistingFiles,
        schema_version: MAINTENANCE_ACTION_SCHEMA_VERSION,
        supported_subjects: SHOW_ONLY,
        effect_classes: CATALOG_INTENT_AND_DESTRUCTIVE,
        risk_class: MaintenanceRiskClass::High,
        timing_mode: MaintenanceTimingMode::AfterGrace,
        allowed_repeat_modes: ONCE_PER_MATCH_ONLY,
    };

const UNMONITOR_SCOPE_DELETE_FILES: MaintenanceActionDescriptor = MaintenanceActionDescriptor {
    kind: MaintenanceActionKind::UnmonitorScopeDeleteFiles,
    schema_version: MAINTENANCE_ACTION_SCHEMA_VERSION,
    supported_subjects: SEASON_OR_EPISODE,
    effect_classes: CATALOG_INTENT_AND_DESTRUCTIVE,
    risk_class: MaintenanceRiskClass::High,
    timing_mode: MaintenanceTimingMode::AfterGrace,
    allowed_repeat_modes: ONCE_PER_MATCH_ONLY,
};

const UNMONITOR_SEASON_DELETE_FILES_THEN_DELETE_SHOW_IF_EMPTY: MaintenanceActionDescriptor =
    MaintenanceActionDescriptor {
        kind: MaintenanceActionKind::UnmonitorSeasonDeleteFilesThenDeleteShowIfEmpty,
        schema_version: MAINTENANCE_ACTION_SCHEMA_VERSION,
        supported_subjects: SEASON_ONLY,
        effect_classes: CATALOG_INTENT_AND_DESTRUCTIVE,
        risk_class: MaintenanceRiskClass::High,
        timing_mode: MaintenanceTimingMode::AfterGrace,
        allowed_repeat_modes: ONCE_PER_MATCH_ONLY,
    };

const UNMONITOR_SEASON_THEN_UNMONITOR_SHOW_IF_EMPTY: MaintenanceActionDescriptor =
    MaintenanceActionDescriptor {
        kind: MaintenanceActionKind::UnmonitorSeasonThenUnmonitorShowIfEmpty,
        schema_version: MAINTENANCE_ACTION_SCHEMA_VERSION,
        supported_subjects: SEASON_ONLY,
        effect_classes: CATALOG_INTENT,
        risk_class: MaintenanceRiskClass::Medium,
        timing_mode: MaintenanceTimingMode::AfterGrace,
        allowed_repeat_modes: ENSURE_STATE_ONLY,
    };

const CHANGE_QUALITY_PROFILE_AND_SEARCH_IF_CHANGED: MaintenanceActionDescriptor =
    MaintenanceActionDescriptor {
        kind: MaintenanceActionKind::ChangeQualityProfileAndSearchIfChanged,
        schema_version: MAINTENANCE_ACTION_SCHEMA_VERSION,
        supported_subjects: TITLE_SUBJECTS,
        effect_classes: CATALOG_INTENT_AND_ACQUISITION,
        risk_class: MaintenanceRiskClass::Medium,
        timing_mode: MaintenanceTimingMode::ZeroGraceNextHandlerPass,
        allowed_repeat_modes: ONCE_PER_MATCH_ONLY,
    };

/// Tag writes are catalog state and nothing else: no file moves, no
/// acquisition, no monitoring change. `catalog_intent` is the effect class that
/// says "this changed something Scryer stores about the title", and `low` is the
/// risk class for reversible Scryer state — removing a tag the rule added is one
/// click, and no media becomes unavailable either way.
///
/// `ensure_state` rather than `once_per_match`: the desired postcondition is
/// "these labels are (not) on the title", so a title someone re-tagged by hand
/// should be corrected again on the next pass rather than left drifted because a
/// generation already ran.
const ADD_TAGS: MaintenanceActionDescriptor = MaintenanceActionDescriptor {
    kind: MaintenanceActionKind::AddTags,
    schema_version: MAINTENANCE_ACTION_SCHEMA_VERSION,
    supported_subjects: TITLE_SUBJECTS,
    effect_classes: CATALOG_INTENT,
    risk_class: MaintenanceRiskClass::Low,
    timing_mode: MaintenanceTimingMode::AfterGrace,
    allowed_repeat_modes: ENSURE_STATE_ONLY,
};

const REMOVE_TAGS: MaintenanceActionDescriptor = MaintenanceActionDescriptor {
    kind: MaintenanceActionKind::RemoveTags,
    schema_version: MAINTENANCE_ACTION_SCHEMA_VERSION,
    supported_subjects: TITLE_SUBJECTS,
    effect_classes: CATALOG_INTENT,
    risk_class: MaintenanceRiskClass::Low,
    timing_mode: MaintenanceTimingMode::AfterGrace,
    allowed_repeat_modes: ENSURE_STATE_ONLY,
};

static ACTION_CATALOG: &[MaintenanceActionDescriptor] = &[
    DO_NOTHING,
    UNMONITOR_SCOPE_KEEP_FILES,
    DELETE_TITLE_AND_FILES,
    UNMONITOR_TITLE_DELETE_ALL_FILES,
    UNMONITOR_SHOW_DELETE_EXISTING_FILES,
    UNMONITOR_SCOPE_DELETE_FILES,
    UNMONITOR_SEASON_DELETE_FILES_THEN_DELETE_SHOW_IF_EMPTY,
    UNMONITOR_SEASON_THEN_UNMONITOR_SHOW_IF_EMPTY,
    CHANGE_QUALITY_PROFILE_AND_SEARCH_IF_CHANGED,
    ADD_TAGS,
    REMOVE_TAGS,
];

/// The static action registry (RFC 9.2). GraphQL may later expose these
/// descriptors for the rule builder; clients never define new variants.
pub fn action_catalog() -> &'static [MaintenanceActionDescriptor] {
    ACTION_CATALOG
}

/// The descriptor for `kind`. Total by construction — the catalog is closed.
pub fn descriptor_for(kind: MaintenanceActionKind) -> &'static MaintenanceActionDescriptor {
    match kind {
        MaintenanceActionKind::DoNothing => &DO_NOTHING,
        MaintenanceActionKind::UnmonitorScopeKeepFiles => &UNMONITOR_SCOPE_KEEP_FILES,
        MaintenanceActionKind::DeleteTitleAndFiles => &DELETE_TITLE_AND_FILES,
        MaintenanceActionKind::UnmonitorTitleDeleteAllFiles => &UNMONITOR_TITLE_DELETE_ALL_FILES,
        MaintenanceActionKind::UnmonitorShowDeleteExistingFiles => {
            &UNMONITOR_SHOW_DELETE_EXISTING_FILES
        }
        MaintenanceActionKind::UnmonitorScopeDeleteFiles => &UNMONITOR_SCOPE_DELETE_FILES,
        MaintenanceActionKind::UnmonitorSeasonDeleteFilesThenDeleteShowIfEmpty => {
            &UNMONITOR_SEASON_DELETE_FILES_THEN_DELETE_SHOW_IF_EMPTY
        }
        MaintenanceActionKind::UnmonitorSeasonThenUnmonitorShowIfEmpty => {
            &UNMONITOR_SEASON_THEN_UNMONITOR_SHOW_IF_EMPTY
        }
        MaintenanceActionKind::ChangeQualityProfileAndSearchIfChanged => {
            &CHANGE_QUALITY_PROFILE_AND_SEARCH_IF_CHANGED
        }
        MaintenanceActionKind::AddTags => &ADD_TAGS,
        MaintenanceActionKind::RemoveTags => &REMOVE_TAGS,
    }
}

/// The closed parameter payload for a [`MaintenanceActionSpec`].
///
/// Parameters come from rule configuration and resolved subject identity, never
/// from free-form values returned by policy evaluation (RFC 9.1). For the
/// Maintainerr-parity catalog exactly one action is parameterized.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum MaintenanceActionParameters {
    /// The parameterless shape shared by every action except
    /// [`MaintenanceActionKind::ChangeQualityProfileAndSearchIfChanged`].
    #[default]
    None,
    ChangeQualityProfile {
        target_quality_profile_id: String,
    },
    /// Labels the tag actions add or remove. Which of the two a spec means is
    /// carried by its `kind`, so one payload shape serves both and a rule that
    /// adds `x` and one that removes `x` are the same object with a different
    /// verb — which is what makes the executor's conflict check a simple
    /// comparison.
    Tags {
        tags: Vec<String>,
    },
}

impl MaintenanceActionParameters {
    fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }

    /// The labels this payload carries, or an empty slice for every other
    /// shape. Callers that only need "what would this write" — the executor's
    /// conflict scan and the registry re-check — read this rather than
    /// destructuring.
    pub fn tag_labels(&self) -> &[String] {
        match self {
            Self::Tags { tags } => tags,
            _ => &[],
        }
    }

    /// Whether this payload's shape is the one `kind` requires.
    fn matches_kind(&self, kind: MaintenanceActionKind) -> bool {
        match kind {
            MaintenanceActionKind::ChangeQualityProfileAndSearchIfChanged => {
                matches!(self, Self::ChangeQualityProfile { .. })
            }
            MaintenanceActionKind::AddTags | MaintenanceActionKind::RemoveTags => {
                matches!(self, Self::Tags { .. })
            }
            _ => self.is_none(),
        }
    }
}

/// The closed, schema-versioned action configuration a rule revision stores
/// (RFC 9.1/9.2).
///
/// `deny_unknown_fields` is load-bearing: stored JSON — or anything shaped like
/// a policy result — cannot smuggle an extra parameter past validation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct MaintenanceActionSpec {
    pub kind: MaintenanceActionKind,
    pub schema_version: u32,
    #[serde(default, skip_serializing_if = "MaintenanceActionParameters::is_none")]
    pub parameters: MaintenanceActionParameters,
}

impl MaintenanceActionSpec {
    /// A parameterless spec at the current schema version.
    ///
    /// Passing [`MaintenanceActionKind::ChangeQualityProfileAndSearchIfChanged`],
    /// [`MaintenanceActionKind::AddTags`], or
    /// [`MaintenanceActionKind::RemoveTags`] builds a spec that
    /// [`Self::validate`] rejects; use [`Self::change_quality_profile`] or
    /// [`Self::tags`] instead.
    pub fn new(kind: MaintenanceActionKind) -> Self {
        Self {
            kind,
            schema_version: MAINTENANCE_ACTION_SCHEMA_VERSION,
            parameters: MaintenanceActionParameters::None,
        }
    }

    /// The quality-profile workflow spec at the current schema version.
    pub fn change_quality_profile(target_quality_profile_id: impl Into<String>) -> Self {
        Self {
            kind: MaintenanceActionKind::ChangeQualityProfileAndSearchIfChanged,
            schema_version: MAINTENANCE_ACTION_SCHEMA_VERSION,
            parameters: MaintenanceActionParameters::ChangeQualityProfile {
                target_quality_profile_id: target_quality_profile_id.into(),
            },
        }
    }

    /// A tag spec at the current schema version.
    ///
    /// `kind` must be [`MaintenanceActionKind::AddTags`] or
    /// [`MaintenanceActionKind::RemoveTags`]; any other kind produces a spec
    /// [`Self::validate`] refuses, exactly as the quality-profile constructor
    /// does.
    pub fn tags(kind: MaintenanceActionKind, tags: Vec<String>) -> Self {
        Self {
            kind,
            schema_version: MAINTENANCE_ACTION_SCHEMA_VERSION,
            parameters: MaintenanceActionParameters::Tags { tags },
        }
    }

    /// The static descriptor backing this spec.
    pub fn descriptor(&self) -> &'static MaintenanceActionDescriptor {
        descriptor_for(self.kind)
    }

    /// Host validation performed when a rule revision is created and again when
    /// a stored spec is read back (RFC 9.1: every parameter is validated when
    /// the revision is created and snapshotted onto a candidate).
    pub fn validate(
        &self,
        subject: MaintenanceSubjectKind,
    ) -> Result<(), MaintenanceActionSpecError> {
        if self.schema_version != MAINTENANCE_ACTION_SCHEMA_VERSION {
            return Err(MaintenanceActionSpecError::UnsupportedSchemaVersion {
                kind: self.kind,
                found: self.schema_version,
                expected: MAINTENANCE_ACTION_SCHEMA_VERSION,
            });
        }

        if !self.descriptor().supports_subject(subject) {
            return Err(MaintenanceActionSpecError::UnsupportedSubject {
                kind: self.kind,
                subject,
            });
        }

        if !self.parameters.matches_kind(self.kind) {
            return Err(MaintenanceActionSpecError::ParameterShapeMismatch { kind: self.kind });
        }

        if let MaintenanceActionParameters::ChangeQualityProfile {
            target_quality_profile_id,
        } = &self.parameters
            && target_quality_profile_id.trim().is_empty()
        {
            return Err(MaintenanceActionSpecError::EmptyQualityProfileTarget);
        }

        if let MaintenanceActionParameters::Tags { tags } = &self.parameters {
            validate_action_tag_labels(tags)?;
        }

        Ok(())
    }
}

/// Why a [`MaintenanceActionSpec`] is not a valid rule-revision configuration.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum MaintenanceActionSpecError {
    #[error("maintenance action '{kind:?}' does not support subject '{subject:?}'")]
    UnsupportedSubject {
        kind: MaintenanceActionKind,
        subject: MaintenanceSubjectKind,
    },

    #[error("maintenance action '{kind:?}' has schema version {found}, expected {expected}")]
    UnsupportedSchemaVersion {
        kind: MaintenanceActionKind,
        found: u32,
        expected: u32,
    },

    #[error("maintenance action '{kind:?}' was given parameters of the wrong shape")]
    ParameterShapeMismatch { kind: MaintenanceActionKind },

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
}

/// Static half of tag-parameter validation: shape, normalization, and the
/// per-title ceiling.
///
/// It is deliberately the same normalizer the assignment path uses, so a label
/// that a human could type into the tag picker is exactly the set of labels a
/// rule can be authored against, and a stored spec can never carry a spelling
/// that would never match a bag. Whether the label is *defined* is the other
/// half of the check and cannot live here: it needs the registry, so the
/// application service performs it before a revision is written and the
/// executor re-performs it before it acts.
fn validate_action_tag_labels(tags: &[String]) -> Result<(), MaintenanceActionSpecError> {
    if tags.is_empty() {
        return Err(MaintenanceActionSpecError::EmptyTagList);
    }
    if tags.len() > crate::MAX_USER_TAGS_PER_TITLE {
        return Err(MaintenanceActionSpecError::TooManyTags {
            found: tags.len(),
            maximum: crate::MAX_USER_TAGS_PER_TITLE,
        });
    }
    let mut seen = std::collections::HashSet::new();
    for label in tags {
        let normalized = crate::normalize_user_title_tag(label).map_err(|reason| {
            MaintenanceActionSpecError::InvalidTagLabel {
                label: label.clone(),
                reason,
            }
        })?;
        if &normalized != label {
            // Stored specs carry the normal form only. Accepting `Needs  Review`
            // here and normalizing it silently would leave the operator's editor
            // and the stored rule disagreeing about what the rule says.
            return Err(MaintenanceActionSpecError::InvalidTagLabel {
                label: label.clone(),
                reason: format!(
                    "tags are stored lowercase and trimmed, so this must be '{normalized}'"
                ),
            });
        }
        if !seen.insert(normalized) {
            return Err(MaintenanceActionSpecError::DuplicateTagLabel {
                label: label.clone(),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The RFC 9.3 subject matrix, restated independently of the catalog table
    /// so a descriptor edit cannot silently move the boundary.
    fn expected_subjects(kind: MaintenanceActionKind) -> &'static [MaintenanceSubjectKind] {
        use MaintenanceActionKind as K;
        use MaintenanceSubjectKind::{Episode, Movie, Season, Show};
        match kind {
            K::DoNothing => &[Movie, Show, Season, Episode],
            K::UnmonitorScopeKeepFiles => &[Movie, Show, Season, Episode],
            K::DeleteTitleAndFiles => &[Movie, Show],
            K::UnmonitorTitleDeleteAllFiles => &[Movie, Show],
            K::UnmonitorShowDeleteExistingFiles => &[Show],
            K::UnmonitorScopeDeleteFiles => &[Season, Episode],
            K::UnmonitorSeasonDeleteFilesThenDeleteShowIfEmpty => &[Season],
            K::UnmonitorSeasonThenUnmonitorShowIfEmpty => &[Season],
            K::ChangeQualityProfileAndSearchIfChanged => &[Movie, Show],
            K::AddTags => &[Movie, Show],
            K::RemoveTags => &[Movie, Show],
        }
    }

    fn spec_for(kind: MaintenanceActionKind) -> MaintenanceActionSpec {
        match kind {
            MaintenanceActionKind::ChangeQualityProfileAndSearchIfChanged => {
                MaintenanceActionSpec::change_quality_profile("profile-1")
            }
            MaintenanceActionKind::AddTags | MaintenanceActionKind::RemoveTags => {
                MaintenanceActionSpec::tags(kind, vec!["needs review".to_string()])
            }
            _ => MaintenanceActionSpec::new(kind),
        }
    }

    // ---- (a) descriptor invariants ------------------------------------

    #[test]
    fn every_kind_appears_exactly_once_in_the_catalog() {
        assert_eq!(action_catalog().len(), MaintenanceActionKind::ALL.len());
        for kind in MaintenanceActionKind::ALL {
            let matches = action_catalog()
                .iter()
                .filter(|descriptor| descriptor.kind == *kind)
                .count();
            assert_eq!(matches, 1, "{kind:?} must appear exactly once");
            assert_eq!(descriptor_for(*kind).kind, *kind);
        }
    }

    #[test]
    fn descriptors_declare_subjects_effects_and_repeat_modes() {
        for descriptor in action_catalog() {
            assert!(
                !descriptor.supported_subjects.is_empty(),
                "{:?} must support at least one subject",
                descriptor.kind
            );
            assert!(
                !descriptor.effect_classes.is_empty(),
                "{:?} must declare at least one effect class (RFC 9.9)",
                descriptor.kind
            );
            assert!(
                !descriptor.allowed_repeat_modes.is_empty(),
                "{:?} must allow at least one repeat mode (RFC 9.8)",
                descriptor.kind
            );
            assert_eq!(descriptor.schema_version, MAINTENANCE_ACTION_SCHEMA_VERSION);
        }
    }

    #[test]
    fn subjects_match_the_rfc_matrix() {
        for descriptor in action_catalog() {
            assert_eq!(
                descriptor.supported_subjects,
                expected_subjects(descriptor.kind),
                "subject matrix drift for {:?}",
                descriptor.kind
            );
        }
    }

    #[test]
    fn high_risk_actions_are_destructive_storage() {
        for descriptor in action_catalog() {
            if descriptor.risk_class == MaintenanceRiskClass::High {
                assert!(
                    descriptor.has_effect(MaintenanceEffectClass::DestructiveStorage),
                    "{:?} is High risk and must declare destructive_storage",
                    descriptor.kind
                );
            } else {
                assert!(
                    !descriptor.has_effect(MaintenanceEffectClass::DestructiveStorage),
                    "{:?} declares destructive_storage but is not High risk",
                    descriptor.kind
                );
            }
        }
    }

    #[test]
    fn do_nothing_is_the_only_risk_none_action() {
        let risk_none: Vec<_> = action_catalog()
            .iter()
            .filter(|descriptor| descriptor.risk_class == MaintenanceRiskClass::None)
            .map(|descriptor| descriptor.kind)
            .collect();
        assert_eq!(risk_none, vec![MaintenanceActionKind::DoNothing]);
        assert_eq!(
            DO_NOTHING.timing_mode,
            MaintenanceTimingMode::MembershipTracking
        );
    }

    #[test]
    fn only_do_nothing_is_membership_tracking_and_after_grace_repeats_are_bounded() {
        for descriptor in action_catalog() {
            match descriptor.timing_mode {
                MaintenanceTimingMode::MembershipTracking => {
                    assert_eq!(descriptor.kind, MaintenanceActionKind::DoNothing);
                }
                MaintenanceTimingMode::AfterGrace => {
                    assert!(
                        !descriptor.allowed_repeat_modes.is_empty(),
                        "{:?} is after_grace and must declare repeat modes",
                        descriptor.kind
                    );
                }
                MaintenanceTimingMode::ZeroGraceNextHandlerPass => {
                    assert_eq!(
                        descriptor.kind,
                        MaintenanceActionKind::ChangeQualityProfileAndSearchIfChanged
                    );
                }
            }
        }
    }

    #[test]
    fn parity_catalog_uses_only_ensure_state_and_once_per_match() {
        // RFC 9.8: `periodic_while_matching` and `continuous_claim` are
        // reserved for post-parity actions.
        for descriptor in action_catalog() {
            for mode in descriptor.allowed_repeat_modes {
                assert!(
                    matches!(
                        mode,
                        MaintenanceRepeatMode::EnsureState | MaintenanceRepeatMode::OncePerMatch
                    ),
                    "{:?} uses reserved repeat mode {mode:?}",
                    descriptor.kind
                );
            }
        }
    }

    #[test]
    fn destructive_actions_run_once_per_match() {
        for descriptor in action_catalog() {
            if descriptor.has_effect(MaintenanceEffectClass::DestructiveStorage) {
                assert_eq!(
                    descriptor.allowed_repeat_modes, ONCE_PER_MATCH_ONLY,
                    "{:?} deletes and must not be an ensure_state re-run",
                    descriptor.kind
                );
            }
        }
    }

    // ---- (b) wire-format contract -------------------------------------

    #[test]
    fn action_kind_wire_names_are_pinned() {
        let expected = [
            (MaintenanceActionKind::DoNothing, "do_nothing"),
            (
                MaintenanceActionKind::UnmonitorScopeKeepFiles,
                "unmonitor_scope_keep_files",
            ),
            (
                MaintenanceActionKind::DeleteTitleAndFiles,
                "delete_title_and_files",
            ),
            (
                MaintenanceActionKind::UnmonitorTitleDeleteAllFiles,
                "unmonitor_title_delete_all_files",
            ),
            (
                MaintenanceActionKind::UnmonitorShowDeleteExistingFiles,
                "unmonitor_show_delete_existing_files",
            ),
            (
                MaintenanceActionKind::UnmonitorScopeDeleteFiles,
                "unmonitor_scope_delete_files",
            ),
            (
                MaintenanceActionKind::UnmonitorSeasonDeleteFilesThenDeleteShowIfEmpty,
                "unmonitor_season_delete_files_then_delete_show_if_empty",
            ),
            (
                MaintenanceActionKind::UnmonitorSeasonThenUnmonitorShowIfEmpty,
                "unmonitor_season_then_unmonitor_show_if_empty",
            ),
            (
                MaintenanceActionKind::ChangeQualityProfileAndSearchIfChanged,
                "change_quality_profile_and_search_if_changed",
            ),
            (MaintenanceActionKind::AddTags, "add_tags"),
            (MaintenanceActionKind::RemoveTags, "remove_tags"),
        ];
        assert_eq!(expected.len(), MaintenanceActionKind::ALL.len());
        for (kind, wire) in expected {
            assert_eq!(serde_json::to_string(&kind).unwrap(), format!("\"{wire}\""));
            assert_eq!(
                serde_json::from_str::<MaintenanceActionKind>(&format!("\"{wire}\"")).unwrap(),
                kind
            );
            // A candidate row stores this string directly, so the hand-written
            // projection has to be the same name serde produces.
            assert_eq!(kind.as_wire_str(), wire);
            assert_eq!(MaintenanceActionKind::parse_wire_str(wire), Some(kind));
        }
        assert_eq!(MaintenanceActionKind::parse_wire_str("not_an_action"), None);
    }

    #[test]
    fn supporting_enum_wire_names_are_pinned() {
        for (subject, wire) in [
            (MaintenanceSubjectKind::Movie, "movie"),
            (MaintenanceSubjectKind::Show, "show"),
            (MaintenanceSubjectKind::Season, "season"),
            (MaintenanceSubjectKind::Episode, "episode"),
        ] {
            assert_eq!(
                serde_json::to_string(&subject).unwrap(),
                format!("\"{wire}\"")
            );
        }
        for (risk, wire) in [
            (MaintenanceRiskClass::None, "none"),
            (MaintenanceRiskClass::Low, "low"),
            (MaintenanceRiskClass::Medium, "medium"),
            (MaintenanceRiskClass::High, "high"),
        ] {
            assert_eq!(serde_json::to_string(&risk).unwrap(), format!("\"{wire}\""));
        }
        for (effect, wire) in [
            (MaintenanceEffectClass::Protect, "protect"),
            (MaintenanceEffectClass::Communicate, "communicate"),
            (MaintenanceEffectClass::CatalogIntent, "catalog_intent"),
            (MaintenanceEffectClass::MetadataRepair, "metadata_repair"),
            (MaintenanceEffectClass::Acquisition, "acquisition"),
            (
                MaintenanceEffectClass::FileOrganization,
                "file_organization",
            ),
            (
                MaintenanceEffectClass::DestructiveStorage,
                "destructive_storage",
            ),
        ] {
            assert_eq!(
                serde_json::to_string(&effect).unwrap(),
                format!("\"{wire}\"")
            );
        }
        for (timing, wire) in [
            (
                MaintenanceTimingMode::MembershipTracking,
                "membership_tracking",
            ),
            (MaintenanceTimingMode::AfterGrace, "after_grace"),
            (
                MaintenanceTimingMode::ZeroGraceNextHandlerPass,
                "zero_grace_next_handler_pass",
            ),
        ] {
            assert_eq!(
                serde_json::to_string(&timing).unwrap(),
                format!("\"{wire}\"")
            );
        }
        for (repeat, wire) in [
            (MaintenanceRepeatMode::EnsureState, "ensure_state"),
            (MaintenanceRepeatMode::OncePerMatch, "once_per_match"),
            (
                MaintenanceRepeatMode::PeriodicWhileMatching,
                "periodic_while_matching",
            ),
            (MaintenanceRepeatMode::ContinuousClaim, "continuous_claim"),
        ] {
            assert_eq!(
                serde_json::to_string(&repeat).unwrap(),
                format!("\"{wire}\"")
            );
        }
    }

    #[test]
    fn spec_round_trips_for_every_kind() {
        for kind in MaintenanceActionKind::ALL {
            let spec = spec_for(*kind);
            let json = serde_json::to_string(&spec).unwrap();
            let parsed: MaintenanceActionSpec = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, spec, "round-trip drift for {kind:?}");
        }
    }

    #[test]
    fn stored_spec_json_shape_is_pinned() {
        assert_eq!(
            serde_json::to_string(&MaintenanceActionSpec::new(
                MaintenanceActionKind::DeleteTitleAndFiles
            ))
            .unwrap(),
            r#"{"kind":"delete_title_and_files","schema_version":1}"#
        );
        assert_eq!(
            serde_json::to_string(&MaintenanceActionSpec::change_quality_profile("hd-1080p"))
                .unwrap(),
            r#"{"kind":"change_quality_profile_and_search_if_changed","schema_version":1,"parameters":{"change_quality_profile":{"target_quality_profile_id":"hd-1080p"}}}"#
        );
        // A parameterless payload deserializes without a `parameters` key.
        let parsed: MaintenanceActionSpec =
            serde_json::from_str(r#"{"kind":"do_nothing","schema_version":1}"#).unwrap();
        assert_eq!(
            parsed,
            MaintenanceActionSpec::new(MaintenanceActionKind::DoNothing)
        );
    }

    // ---- (c) hostile payloads -----------------------------------------

    #[test]
    fn unknown_or_misspelled_kinds_fail_to_deserialize() {
        for wire in [
            "delete_everything",
            "DO_NOTHING",
            "doNothing",
            "do-nothing",
            "delete_title",
            "protect_from_lifecycle",
            "refresh_title",
            "",
        ] {
            assert!(
                serde_json::from_str::<MaintenanceActionKind>(&format!("\"{wire}\"")).is_err(),
                "'{wire}' must not deserialize into a catalog kind"
            );
        }
    }

    #[test]
    fn unknown_fields_are_rejected() {
        assert!(
            serde_json::from_str::<MaintenanceActionSpec>(
                r#"{"kind":"do_nothing","schema_version":1,"shell_command":"rm -rf /"}"#
            )
            .is_err()
        );
        assert!(
            serde_json::from_str::<MaintenanceActionSpec>(
                r#"{"kind":"change_quality_profile_and_search_if_changed","schema_version":1,"parameters":{"change_quality_profile":{"target_quality_profile_id":"a","webhook_url":"http://evil"}}}"#
            )
            .is_err()
        );
        assert!(
            serde_json::from_str::<MaintenanceActionSpec>(r#"{"schema_version":1}"#).is_err(),
            "kind is required"
        );
    }

    // ---- (d) exhaustive subject/kind validation sweep -----------------

    #[test]
    fn validate_matches_the_rfc_subject_matrix_for_every_pair() {
        for kind in MaintenanceActionKind::ALL {
            let spec = spec_for(*kind);
            for subject in MaintenanceSubjectKind::ALL {
                let allowed = expected_subjects(*kind).contains(subject);
                let result = spec.validate(*subject);
                if allowed {
                    assert_eq!(result, Ok(()), "{kind:?} must accept {subject:?}");
                } else {
                    assert_eq!(
                        result,
                        Err(MaintenanceActionSpecError::UnsupportedSubject {
                            kind: *kind,
                            subject: *subject,
                        }),
                        "{kind:?} must reject {subject:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn validate_rejects_wrong_schema_version() {
        let mut spec = MaintenanceActionSpec::new(MaintenanceActionKind::DoNothing);
        spec.schema_version = MAINTENANCE_ACTION_SCHEMA_VERSION + 1;
        assert_eq!(
            spec.validate(MaintenanceSubjectKind::Movie),
            Err(MaintenanceActionSpecError::UnsupportedSchemaVersion {
                kind: MaintenanceActionKind::DoNothing,
                found: MAINTENANCE_ACTION_SCHEMA_VERSION + 1,
                expected: MAINTENANCE_ACTION_SCHEMA_VERSION,
            })
        );
    }

    #[test]
    fn validate_rejects_blank_and_missing_quality_profile_targets() {
        for blank in ["", "   "] {
            assert_eq!(
                MaintenanceActionSpec::change_quality_profile(blank)
                    .validate(MaintenanceSubjectKind::Movie),
                Err(MaintenanceActionSpecError::EmptyQualityProfileTarget)
            );
        }
        assert_eq!(
            MaintenanceActionSpec::new(
                MaintenanceActionKind::ChangeQualityProfileAndSearchIfChanged
            )
            .validate(MaintenanceSubjectKind::Movie),
            Err(MaintenanceActionSpecError::ParameterShapeMismatch {
                kind: MaintenanceActionKind::ChangeQualityProfileAndSearchIfChanged,
            })
        );
    }

    #[test]
    fn validate_rejects_parameters_on_parameterless_kinds() {
        let spec = MaintenanceActionSpec {
            kind: MaintenanceActionKind::DeleteTitleAndFiles,
            schema_version: MAINTENANCE_ACTION_SCHEMA_VERSION,
            parameters: MaintenanceActionParameters::ChangeQualityProfile {
                target_quality_profile_id: "hd-1080p".to_string(),
            },
        };
        assert_eq!(
            spec.validate(MaintenanceSubjectKind::Movie),
            Err(MaintenanceActionSpecError::ParameterShapeMismatch {
                kind: MaintenanceActionKind::DeleteTitleAndFiles,
            })
        );
    }

    #[test]
    fn tag_specs_pin_their_stored_shape() {
        assert_eq!(
            serde_json::to_string(&MaintenanceActionSpec::tags(
                MaintenanceActionKind::AddTags,
                vec!["needs review".to_string()]
            ))
            .unwrap(),
            r#"{"kind":"add_tags","schema_version":1,"parameters":{"tags":{"tags":["needs review"]}}}"#
        );
    }

    #[test]
    fn validate_rejects_unusable_tag_parameters() {
        use MaintenanceActionKind::{AddTags, RemoveTags};

        // A tag action with no tags does nothing at all; that is a mistake in
        // the rule, not a no-op to store.
        for kind in [AddTags, RemoveTags] {
            assert_eq!(
                MaintenanceActionSpec::tags(kind, Vec::new())
                    .validate(MaintenanceSubjectKind::Movie),
                Err(MaintenanceActionSpecError::EmptyTagList)
            );
            // The parameterless constructor is the wrong shape for a tag kind.
            assert_eq!(
                MaintenanceActionSpec::new(kind).validate(MaintenanceSubjectKind::Movie),
                Err(MaintenanceActionSpecError::ParameterShapeMismatch { kind })
            );
        }

        // Membership is by exact label, so a spelling the assignment path would
        // normalize is refused rather than silently rewritten.
        assert!(matches!(
            MaintenanceActionSpec::tags(AddTags, vec!["Needs  Review".to_string()])
                .validate(MaintenanceSubjectKind::Show),
            Err(MaintenanceActionSpecError::InvalidTagLabel { .. })
        ));
        // The reserved namespace is settings, never a user tag.
        assert!(matches!(
            MaintenanceActionSpec::tags(AddTags, vec!["scryer:monitor-type:all".to_string()])
                .validate(MaintenanceSubjectKind::Show),
            Err(MaintenanceActionSpecError::InvalidTagLabel { .. })
        ));
        assert_eq!(
            MaintenanceActionSpec::tags(RemoveTags, vec!["keep".to_string(), "keep".to_string()])
                .validate(MaintenanceSubjectKind::Movie),
            Err(MaintenanceActionSpecError::DuplicateTagLabel {
                label: "keep".to_string()
            })
        );
        let too_many: Vec<String> = (0..=crate::MAX_USER_TAGS_PER_TITLE)
            .map(|index| format!("tag-{index}"))
            .collect();
        assert_eq!(
            MaintenanceActionSpec::tags(AddTags, too_many.clone())
                .validate(MaintenanceSubjectKind::Movie),
            Err(MaintenanceActionSpecError::TooManyTags {
                found: too_many.len(),
                maximum: crate::MAX_USER_TAGS_PER_TITLE,
            })
        );
    }

    #[test]
    fn tag_labels_reads_only_the_tag_payload() {
        assert_eq!(
            MaintenanceActionSpec::tags(
                MaintenanceActionKind::RemoveTags,
                vec!["keep".to_string()]
            )
            .parameters
            .tag_labels(),
            ["keep".to_string()]
        );
        assert!(
            MaintenanceActionSpec::change_quality_profile("hd-1080p")
                .parameters
                .tag_labels()
                .is_empty()
        );
        assert!(
            MaintenanceActionSpec::new(MaintenanceActionKind::DoNothing)
                .parameters
                .tag_labels()
                .is_empty()
        );
    }

    // ---- (e) Track A2 proof: policy output cannot select an action -----

    /// INVARIANT (RFC 9.1 / Track A2): Rego returns only `match`, `no_match`,
    /// or `unknown`. It never selects an action or supplies an action
    /// parameter, and no host API turns a free-form action name into a
    /// `MaintenanceActionSpec`.
    ///
    /// The catalog offers exactly two construction paths —
    /// `MaintenanceActionSpec::new` (typed, closed `MaintenanceActionKind`) and
    /// `MaintenanceActionSpec::change_quality_profile` — plus `Deserialize`,
    /// which accepts only the nine pinned wire names. There is deliberately no
    /// `from_action_name(&str)`, no `TryFrom<&str>`, no `FromStr`, and no
    /// `serde_json::Value`-shaped parameter bag anywhere in this module, so a
    /// policy result cannot be reinterpreted as an action.
    #[test]
    fn raw_policy_output_cannot_select_an_action() {
        // Policy-shaped payloads: an invented action name, an action name
        // smuggled beside a decision, and a decision document on its own.
        for payload in [
            r#"{"kind":"delete_everything"}"#,
            r#"{"kind":"delete_everything","schema_version":1}"#,
            r#"{"match":true,"action":"delete_title_and_files"}"#,
            r#"{"result":{"kind":"delete_title_and_files","schema_version":1,"parameters":{"path":"/"}}}"#,
            r#"{"kind":"do_nothing","schema_version":1,"action":"delete_title_and_files"}"#,
            r#""delete_title_and_files""#,
            r#"{"decision":"match"}"#,
        ] {
            assert!(
                serde_json::from_str::<MaintenanceActionSpec>(payload).is_err(),
                "policy-shaped payload must not become an action spec: {payload}"
            );
        }

        // Even a well-formed spec is inert without host validation against the
        // rule's own subject; the catalog is the only authority on that pairing.
        assert!(
            MaintenanceActionSpec::new(MaintenanceActionKind::UnmonitorShowDeleteExistingFiles)
                .validate(MaintenanceSubjectKind::Movie)
                .is_err()
        );
    }
}
