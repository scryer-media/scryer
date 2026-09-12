//! Projections for the maintenance-rule authoring surface (RFC 137).
//!
//! The application layer owns the closed action catalog; this module only
//! renames its values onto the wire. Every string projected here (effect
//! classes, timing mode, repeat modes) uses the same snake_case wire name the
//! stored action spec serializes to, so a descriptor and a stored revision
//! never disagree about what an action is called.

use super::*;

use scryer_application::maintenance_rules::{
    MaintenanceActionCompletionPolicy as AppActionCompletionPolicy,
    MaintenanceActionDefinition as AppActionDefinition,
    MaintenanceActionDescriptor as AppActionDescriptor, MaintenanceActionKind as AppActionKind,
    MaintenanceActionParameters as AppActionParameters,
    MaintenanceActionSequence as AppActionSequence, MaintenanceActionSpec as AppActionSpec,
    MaintenanceActionStep as AppActionStep,
    MaintenanceActionStepDescriptor as AppActionStepDescriptor,
    MaintenanceActionStepKind as AppActionStepKind,
    MaintenanceActionStepParameters as AppActionStepParameters,
    MaintenanceActionStepRequirement as AppActionStepRequirement,
    MaintenanceEffectClass as AppEffectClass, MaintenanceRepeatMode as AppRepeatMode,
    MaintenanceRiskClass as AppRiskClass, MaintenanceSearchCondition as AppSearchCondition,
    MaintenanceSubjectKind as AppSubjectKind, MaintenanceTimingMode as AppTimingMode,
    action_catalog, action_sequence_catalog,
};
use scryer_application::maintenance_rules::{
    MaintenancePreviewResult as AppPreviewResult, MaintenanceRuleSetDetail as AppRuleSetDetail,
};

/// Revision numbers and grace periods are stored as `i64` but are small by
/// construction; saturating keeps a corrupt row from panicking a read.
fn to_graphql_int(value: i64) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

pub fn maintenance_evaluation_mode_value(
    mode: scryer_domain::MaintenanceEvaluationMode,
) -> MaintenanceEvaluationMode {
    match mode {
        scryer_domain::MaintenanceEvaluationMode::Disabled => MaintenanceEvaluationMode::Disabled,
        scryer_domain::MaintenanceEvaluationMode::Shadow => MaintenanceEvaluationMode::Shadow,
        scryer_domain::MaintenanceEvaluationMode::Observe => MaintenanceEvaluationMode::Observe,
    }
}

pub fn maintenance_evaluation_mode_into_application(
    mode: MaintenanceEvaluationMode,
) -> scryer_domain::MaintenanceEvaluationMode {
    match mode {
        MaintenanceEvaluationMode::Disabled => scryer_domain::MaintenanceEvaluationMode::Disabled,
        MaintenanceEvaluationMode::Shadow => scryer_domain::MaintenanceEvaluationMode::Shadow,
        MaintenanceEvaluationMode::Observe => scryer_domain::MaintenanceEvaluationMode::Observe,
    }
}

pub fn maintenance_rule_subject_kind_into_application(
    kind: Option<MaintenanceRuleSubjectKind>,
) -> scryer_domain::MaintenanceRuleSubjectKind {
    match kind.unwrap_or(MaintenanceRuleSubjectKind::Title) {
        MaintenanceRuleSubjectKind::Title => scryer_domain::MaintenanceRuleSubjectKind::Title,
        MaintenanceRuleSubjectKind::Season => scryer_domain::MaintenanceRuleSubjectKind::Season,
        MaintenanceRuleSubjectKind::Episode => scryer_domain::MaintenanceRuleSubjectKind::Episode,
    }
}

pub fn maintenance_rule_subject_kind_value(
    kind: scryer_domain::MaintenanceRuleSubjectKind,
) -> MaintenanceRuleSubjectKind {
    match kind {
        scryer_domain::MaintenanceRuleSubjectKind::Title => MaintenanceRuleSubjectKind::Title,
        scryer_domain::MaintenanceRuleSubjectKind::Season => MaintenanceRuleSubjectKind::Season,
        scryer_domain::MaintenanceRuleSubjectKind::Episode => MaintenanceRuleSubjectKind::Episode,
    }
}

pub fn maintenance_effect_arming_value(
    arming: scryer_domain::MaintenanceEffectArming,
) -> MaintenanceEffectArming {
    match arming {
        scryer_domain::MaintenanceEffectArming::None => MaintenanceEffectArming::None,
        scryer_domain::MaintenanceEffectArming::Reversible => MaintenanceEffectArming::Reversible,
        scryer_domain::MaintenanceEffectArming::Destructive => MaintenanceEffectArming::Destructive,
    }
}

pub fn maintenance_effect_arming_into_application(
    arming: MaintenanceEffectArming,
) -> scryer_domain::MaintenanceEffectArming {
    match arming {
        MaintenanceEffectArming::None => scryer_domain::MaintenanceEffectArming::None,
        MaintenanceEffectArming::Reversible => scryer_domain::MaintenanceEffectArming::Reversible,
        MaintenanceEffectArming::Destructive => scryer_domain::MaintenanceEffectArming::Destructive,
    }
}

/// One action-handler attempt. An action kind this build cannot parse renders
/// nothing rather than a wrong kind, but by construction the executor only
/// stores catalog wire names, so the fallback is unreachable in practice.
pub fn from_maintenance_action_run(
    view: scryer_application::maintenance_rules::MaintenanceActionRunView,
) -> Option<MaintenanceActionRun> {
    let scryer_application::maintenance_rules::MaintenanceActionRunView {
        subject_label,
        run,
        title_name,
        action_definition,
        sequence_steps,
    } = view;
    let action_kind =
        if run.action_kind == scryer_application::maintenance_rules::ACTION_SEQUENCE_KIND {
            MaintenanceActionKind::ActionSequence
        } else {
            maintenance_action_kind_value(AppActionKind::parse_wire_str(&run.action_kind)?)
        };
    let action_sequence = action_definition.as_ref().and_then(|definition| {
        let (_, sequence) = from_maintenance_action_definition(definition);
        sequence
    });
    Some(MaintenanceActionRun {
        subject_label,
        subject_kind: run.subject_kind,
        subject_id: ID::from(run.subject_id),
        detail: run.detail,
        id: ID::from(run.id),
        rule_set_id: ID::from(run.rule_set_id),
        candidate_id: ID::from(run.candidate_id),
        title_id: ID::from(run.title_id),
        title_name,
        action_kind,
        action_sequence,
        sequence_steps: sequence_steps
            .iter()
            .map(from_maintenance_action_step_progress)
            .collect(),
        match_generation: to_graphql_int(run.match_generation),
        attempt: to_graphql_int(run.attempt),
        status: run.status.as_storage_str().to_string(),
        hold_reason: run.hold_reason,
        error: run.error,
        started_at: run.started_at,
        finished_at: run.finished_at,
    })
}

fn from_maintenance_action_step_progress(
    progress: &scryer_application::maintenance_rules::action_execution::MaintenanceActionStepProgressView,
) -> MaintenanceActionStepProgress {
    MaintenanceActionStepProgress {
        step: MaintenanceActionStep {
            id: ID::from(progress.step.id.clone()),
            kind: maintenance_action_step_kind_value(progress.step.kind),
            parameters: from_maintenance_action_step_parameters(&progress.step.parameters),
        },
        run: progress.run.as_ref().map(|run| MaintenanceActionStepRun {
            step_id: ID::from(run.key.step_id.clone()),
            step_kind: run.step_kind.clone(),
            state: run.state.as_storage_str().to_string(),
            attempt: to_graphql_int(run.attempt),
            hold_reason: run.hold_reason.clone(),
            error: run.error.clone(),
            created_at: run.created_at.to_owned(),
            updated_at: run.updated_at.to_owned(),
            finished_at: run.finished_at.to_owned(),
        }),
        receipts: progress
            .receipts
            .iter()
            .map(|receipt| MaintenanceActionJobReceipt {
                dispatch_attempt: to_graphql_int(receipt.dispatch_attempt),
                logical_request_key: receipt.logical_request_key.clone(),
                job_run_id: receipt.job_run_id.clone().map(ID::from),
                state: receipt.state.as_storage_str().to_string(),
                created_at: receipt.created_at.to_owned(),
                updated_at: receipt.updated_at.to_owned(),
            })
            .collect(),
    }
}

pub fn maintenance_action_kind_value(kind: AppActionKind) -> MaintenanceActionKind {
    match kind {
        AppActionKind::DoNothing => MaintenanceActionKind::DoNothing,
        AppActionKind::UnmonitorScopeKeepFiles => MaintenanceActionKind::UnmonitorScopeKeepFiles,
        AppActionKind::DeleteTitleAndFiles => MaintenanceActionKind::DeleteTitleAndFiles,
        AppActionKind::UnmonitorTitleDeleteAllFiles => {
            MaintenanceActionKind::UnmonitorTitleDeleteAllFiles
        }
        AppActionKind::UnmonitorShowDeleteExistingFiles => {
            MaintenanceActionKind::UnmonitorShowDeleteExistingFiles
        }
        AppActionKind::UnmonitorScopeDeleteFiles => {
            MaintenanceActionKind::UnmonitorScopeDeleteFiles
        }
        AppActionKind::UnmonitorSeasonDeleteFilesThenDeleteShowIfEmpty => {
            MaintenanceActionKind::UnmonitorSeasonDeleteFilesThenDeleteShowIfEmpty
        }
        AppActionKind::UnmonitorSeasonThenUnmonitorShowIfEmpty => {
            MaintenanceActionKind::UnmonitorSeasonThenUnmonitorShowIfEmpty
        }
        AppActionKind::ChangeQualityProfileAndSearchIfChanged => {
            MaintenanceActionKind::ChangeQualityProfileAndSearchIfChanged
        }
        AppActionKind::AddTags => MaintenanceActionKind::AddTags,
        AppActionKind::RemoveTags => MaintenanceActionKind::RemoveTags,
    }
}

pub fn maintenance_action_kind_into_application(kind: MaintenanceActionKind) -> AppActionKind {
    match kind {
        MaintenanceActionKind::ActionSequence => {
            unreachable!("ACTION_SEQUENCE is an output-only legacy projection")
        }
        MaintenanceActionKind::DoNothing => AppActionKind::DoNothing,
        MaintenanceActionKind::UnmonitorScopeKeepFiles => AppActionKind::UnmonitorScopeKeepFiles,
        MaintenanceActionKind::DeleteTitleAndFiles => AppActionKind::DeleteTitleAndFiles,
        MaintenanceActionKind::UnmonitorTitleDeleteAllFiles => {
            AppActionKind::UnmonitorTitleDeleteAllFiles
        }
        MaintenanceActionKind::UnmonitorShowDeleteExistingFiles => {
            AppActionKind::UnmonitorShowDeleteExistingFiles
        }
        MaintenanceActionKind::UnmonitorScopeDeleteFiles => {
            AppActionKind::UnmonitorScopeDeleteFiles
        }
        MaintenanceActionKind::UnmonitorSeasonDeleteFilesThenDeleteShowIfEmpty => {
            AppActionKind::UnmonitorSeasonDeleteFilesThenDeleteShowIfEmpty
        }
        MaintenanceActionKind::UnmonitorSeasonThenUnmonitorShowIfEmpty => {
            AppActionKind::UnmonitorSeasonThenUnmonitorShowIfEmpty
        }
        MaintenanceActionKind::ChangeQualityProfileAndSearchIfChanged => {
            AppActionKind::ChangeQualityProfileAndSearchIfChanged
        }
        MaintenanceActionKind::AddTags => AppActionKind::AddTags,
        MaintenanceActionKind::RemoveTags => AppActionKind::RemoveTags,
    }
}

fn maintenance_action_subject_value(subject: AppSubjectKind) -> MaintenanceActionSubject {
    match subject {
        AppSubjectKind::Movie => MaintenanceActionSubject::Movie,
        AppSubjectKind::Show => MaintenanceActionSubject::Show,
        AppSubjectKind::Season => MaintenanceActionSubject::Season,
        AppSubjectKind::Episode => MaintenanceActionSubject::Episode,
    }
}

fn maintenance_risk_class_value(risk: AppRiskClass) -> MaintenanceRiskClass {
    match risk {
        AppRiskClass::None => MaintenanceRiskClass::None,
        AppRiskClass::Low => MaintenanceRiskClass::Low,
        AppRiskClass::Medium => MaintenanceRiskClass::Medium,
        AppRiskClass::High => MaintenanceRiskClass::High,
    }
}

fn maintenance_effect_class_name(effect: AppEffectClass) -> &'static str {
    match effect {
        AppEffectClass::Protect => "protect",
        AppEffectClass::Communicate => "communicate",
        AppEffectClass::CatalogIntent => "catalog_intent",
        AppEffectClass::MetadataRepair => "metadata_repair",
        AppEffectClass::Acquisition => "acquisition",
        AppEffectClass::FileOrganization => "file_organization",
        AppEffectClass::DestructiveStorage => "destructive_storage",
    }
}

fn maintenance_timing_mode_name(timing: AppTimingMode) -> &'static str {
    match timing {
        AppTimingMode::MembershipTracking => "membership_tracking",
        AppTimingMode::AfterGrace => "after_grace",
        AppTimingMode::ZeroGraceNextHandlerPass => "zero_grace_next_handler_pass",
    }
}

fn maintenance_repeat_mode_name(repeat: AppRepeatMode) -> &'static str {
    match repeat {
        AppRepeatMode::EnsureState => "ensure_state",
        AppRepeatMode::OncePerMatch => "once_per_match",
        AppRepeatMode::PeriodicWhileMatching => "periodic_while_matching",
        AppRepeatMode::ContinuousClaim => "continuous_claim",
    }
}

fn maintenance_outcome_value(
    outcome: scryer_rules::maintenance::MaintenanceOutcome,
) -> MaintenanceOutcome {
    match outcome {
        scryer_rules::maintenance::MaintenanceOutcome::Match => MaintenanceOutcome::Match,
        scryer_rules::maintenance::MaintenanceOutcome::NoMatch => MaintenanceOutcome::NoMatch,
        scryer_rules::maintenance::MaintenanceOutcome::Unknown => MaintenanceOutcome::Unknown,
    }
}

/// Projected from the detail, not from the rule-set row alone: the action and
/// grace period a list view has to show live on the revision in force, and
/// making the client fetch each row's detail to render them would turn one
/// list into N+1 requests.
pub fn from_maintenance_rule_set(detail: &AppRuleSetDetail) -> MaintenanceRuleSet {
    let rule_set = &detail.rule_set;
    let (action_spec, action_sequence) =
        from_maintenance_action_definition(&detail.action_definition);
    MaintenanceRuleSet {
        id: ID::from(rule_set.id.clone()),
        name: rule_set.name.clone(),
        description: rule_set.description.clone(),
        enabled: rule_set.enabled,
        evaluation_mode: maintenance_evaluation_mode_value(rule_set.evaluation_mode),
        effect_arming: maintenance_effect_arming_value(rule_set.effect_arming),
        destructive_rearm_required: rule_set.destructive_rearm_required,
        library_ids: rule_set.library_ids.clone(),
        subject_kind: maintenance_rule_subject_kind_value(rule_set.subject_kind),
        current_revision_number: to_graphql_int(rule_set.current_revision_number),
        grace_days: to_graphql_int(detail.revision.grace_days),
        action_spec,
        action_sequence,
        created_at: rule_set.created_at,
        updated_at: rule_set.updated_at,
    }
}

/// The stored source always carries the system-assigned package declaration and
/// the rego.v1 import; both are stripped here for the same reason the release
/// rule editor strips them, so what the editor shows is what the author wrote.
pub fn from_maintenance_rule_revision(
    revision: scryer_domain::MaintenanceRuleRevision,
) -> MaintenanceRuleRevision {
    MaintenanceRuleRevision {
        id: ID::from(revision.id),
        rule_set_id: ID::from(revision.rule_set_id),
        revision_number: to_graphql_int(revision.revision_number),
        rego_source: scryer_rules::strip_editor_source(&revision.rego_source),
        grace_days: to_graphql_int(revision.grace_days),
        storage_root_id: revision.storage_root_id,
        matcher_content_hash: revision.matcher_content_hash,
        created_by: revision.created_by.map(ID::from),
        created_at: revision.created_at,
    }
}

pub fn from_maintenance_action_spec(spec: &AppActionSpec) -> MaintenanceActionSpec {
    MaintenanceActionSpec {
        kind: maintenance_action_kind_value(spec.kind),
        schema_version: to_graphql_int(i64::from(spec.schema_version)),
        target_quality_profile_id: match &spec.parameters {
            AppActionParameters::ChangeQualityProfile {
                target_quality_profile_id,
            } => Some(target_quality_profile_id.clone()),
            AppActionParameters::None | AppActionParameters::Tags { .. } => None,
        },
        tags: spec.parameters.tag_labels().to_vec(),
    }
}

fn maintenance_search_condition_value(condition: AppSearchCondition) -> MaintenanceSearchCondition {
    match condition {
        AppSearchCondition::Unconditional => MaintenanceSearchCondition::Unconditional,
        AppSearchCondition::PreviousProfileChanged => {
            MaintenanceSearchCondition::PreviousProfileChanged
        }
    }
}

fn maintenance_search_condition_into_application(
    condition: MaintenanceSearchCondition,
) -> AppSearchCondition {
    match condition {
        MaintenanceSearchCondition::Unconditional => AppSearchCondition::Unconditional,
        MaintenanceSearchCondition::PreviousProfileChanged => {
            AppSearchCondition::PreviousProfileChanged
        }
    }
}

pub fn from_maintenance_action_sequence(sequence: &AppActionSequence) -> MaintenanceActionSequence {
    MaintenanceActionSequence {
        schema_version: to_graphql_int(i64::from(sequence.schema_version)),
        steps: sequence
            .steps
            .iter()
            .map(|step| MaintenanceActionStep {
                id: ID::from(step.id.clone()),
                kind: maintenance_action_step_kind_value(step.kind),
                parameters: from_maintenance_action_step_parameters(&step.parameters),
            })
            .collect(),
    }
}

fn from_maintenance_action_step_parameters(
    parameters: &AppActionStepParameters,
) -> MaintenanceActionStepParameters {
    match parameters {
        AppActionStepParameters::None => MaintenanceActionStepParameters {
            include_descendants: None,
            target_quality_profile_id: None,
            search_condition: None,
            tags: Vec::new(),
        },
        AppActionStepParameters::Unmonitor {
            include_descendants,
        } => MaintenanceActionStepParameters {
            include_descendants: Some(*include_descendants),
            target_quality_profile_id: None,
            search_condition: None,
            tags: Vec::new(),
        },
        AppActionStepParameters::ChangeQualityProfile {
            target_quality_profile_id,
        } => MaintenanceActionStepParameters {
            include_descendants: None,
            target_quality_profile_id: Some(target_quality_profile_id.clone()),
            search_condition: None,
            tags: Vec::new(),
        },
        AppActionStepParameters::Search { condition } => MaintenanceActionStepParameters {
            include_descendants: None,
            target_quality_profile_id: None,
            search_condition: Some(maintenance_search_condition_value(*condition)),
            tags: Vec::new(),
        },
        AppActionStepParameters::Tags { tags } => MaintenanceActionStepParameters {
            include_descendants: None,
            target_quality_profile_id: None,
            search_condition: None,
            tags: tags.clone(),
        },
    }
}

fn maintenance_action_step_kind_value(kind: AppActionStepKind) -> MaintenanceActionStepKind {
    match kind {
        AppActionStepKind::Unmonitor => MaintenanceActionStepKind::Unmonitor,
        AppActionStepKind::DeleteFiles => MaintenanceActionStepKind::DeleteFiles,
        AppActionStepKind::ChangeQualityProfile => MaintenanceActionStepKind::ChangeQualityProfile,
        AppActionStepKind::Search => MaintenanceActionStepKind::Search,
        AppActionStepKind::AddTags => MaintenanceActionStepKind::AddTags,
        AppActionStepKind::RemoveTags => MaintenanceActionStepKind::RemoveTags,
        AppActionStepKind::DeleteTitleAndFiles => MaintenanceActionStepKind::DeleteTitleAndFiles,
    }
}

fn maintenance_action_step_kind_into_application(
    kind: MaintenanceActionStepKind,
) -> AppActionStepKind {
    match kind {
        MaintenanceActionStepKind::Unmonitor => AppActionStepKind::Unmonitor,
        MaintenanceActionStepKind::DeleteFiles => AppActionStepKind::DeleteFiles,
        MaintenanceActionStepKind::ChangeQualityProfile => AppActionStepKind::ChangeQualityProfile,
        MaintenanceActionStepKind::Search => AppActionStepKind::Search,
        MaintenanceActionStepKind::AddTags => AppActionStepKind::AddTags,
        MaintenanceActionStepKind::RemoveTags => AppActionStepKind::RemoveTags,
        MaintenanceActionStepKind::DeleteTitleAndFiles => AppActionStepKind::DeleteTitleAndFiles,
    }
}

fn from_maintenance_action_definition(
    definition: &AppActionDefinition,
) -> (MaintenanceActionSpec, Option<MaintenanceActionSequence>) {
    match definition {
        AppActionDefinition::Legacy(spec) => (from_maintenance_action_spec(spec), None),
        AppActionDefinition::Sequence(sequence) => (
            MaintenanceActionSpec {
                kind: MaintenanceActionKind::ActionSequence,
                schema_version: to_graphql_int(i64::from(sequence.schema_version)),
                target_quality_profile_id: None,
                tags: Vec::new(),
            },
            Some(from_maintenance_action_sequence(sequence)),
        ),
    }
}

pub fn from_maintenance_rule_set_detail(detail: AppRuleSetDetail) -> MaintenanceRuleSetDetail {
    let (action_spec, action_sequence) =
        from_maintenance_action_definition(&detail.action_definition);
    let rule_set = from_maintenance_rule_set(&detail);
    MaintenanceRuleSetDetail {
        rule_set,
        revision: from_maintenance_rule_revision(detail.revision),
        action_spec,
        action_sequence,
    }
}

fn from_maintenance_action_descriptor(
    descriptor: &AppActionDescriptor,
) -> MaintenanceActionDescriptor {
    MaintenanceActionDescriptor {
        supported_rule_scopes: [
            scryer_domain::MaintenanceRuleSubjectKind::Title,
            scryer_domain::MaintenanceRuleSubjectKind::Season,
            scryer_domain::MaintenanceRuleSubjectKind::Episode,
        ]
        .into_iter()
        .filter(|scope| {
            scryer_application::maintenance_rules::action_execution::supported_scope_actions(*scope)
                .contains(&descriptor.kind)
        })
        .map(maintenance_rule_subject_kind_value)
        .collect(),
        kind: maintenance_action_kind_value(descriptor.kind),
        supported_subjects: descriptor
            .supported_subjects
            .iter()
            .map(|subject| maintenance_action_subject_value(*subject))
            .collect(),
        risk_class: maintenance_risk_class_value(descriptor.risk_class),
        effect_classes: descriptor
            .effect_classes
            .iter()
            .map(|effect| maintenance_effect_class_name(*effect).to_string())
            .collect(),
        timing_mode: maintenance_timing_mode_name(descriptor.timing_mode).to_string(),
        allowed_repeat_modes: descriptor
            .allowed_repeat_modes
            .iter()
            .map(|repeat| maintenance_repeat_mode_name(*repeat).to_string())
            .collect(),
        requires_target_quality_profile: descriptor.kind
            == AppActionKind::ChangeQualityProfileAndSearchIfChanged,
        requires_tags: matches!(
            descriptor.kind,
            AppActionKind::AddTags | AppActionKind::RemoveTags
        ),
        supports_storage_scope:
            scryer_application::maintenance_rules::action_execution::supports_storage_scope(
                descriptor.kind,
            ),
    }
}

/// The static action registry, in catalog order.
pub fn maintenance_action_descriptors() -> Vec<MaintenanceActionDescriptor> {
    action_catalog()
        .iter()
        .map(from_maintenance_action_descriptor)
        .collect()
}

fn maintenance_action_step_requirement_name(requirement: AppActionStepRequirement) -> &'static str {
    match requirement {
        AppActionStepRequirement::PriorCompleteCoveringUnmonitor => {
            "prior_complete_covering_unmonitor"
        }
    }
}

fn maintenance_action_completion_policy_name(policy: AppActionCompletionPolicy) -> &'static str {
    match policy {
        AppActionCompletionPolicy::Accepted => "accepted",
        AppActionCompletionPolicy::Completed => "completed",
    }
}

fn from_maintenance_action_step_descriptor(
    descriptor: &AppActionStepDescriptor,
) -> MaintenanceActionStepDescriptor {
    MaintenanceActionStepDescriptor {
        id: descriptor.id.to_string(),
        kind: maintenance_action_step_kind_value(descriptor.kind),
        label: descriptor.label.to_string(),
        supported_subjects: descriptor
            .supported_subjects
            .iter()
            .map(|subject| maintenance_action_subject_value(*subject))
            .collect(),
        parameter_schema: descriptor.parameter_schema.to_string(),
        effect_classes: descriptor
            .effect_classes
            .iter()
            .map(|effect| maintenance_effect_class_name(*effect).to_string())
            .collect(),
        risk_class: maintenance_risk_class_value(descriptor.risk_class),
        requires: descriptor
            .requires
            .iter()
            .map(|requirement| maintenance_action_step_requirement_name(*requirement).to_string())
            .collect(),
        terminal: descriptor.terminal,
        completion_policy: maintenance_action_completion_policy_name(descriptor.completion_policy)
            .to_string(),
        storage_root_allowed: descriptor.storage_root_allowed,
    }
}

/// The sequence catalog is the source of authoring availability and parameter
/// forms for schema-2 actions.
pub fn maintenance_action_step_descriptors() -> Vec<MaintenanceActionStepDescriptor> {
    action_sequence_catalog()
        .iter()
        .map(from_maintenance_action_step_descriptor)
        .collect()
}

pub fn from_maintenance_preview_result(result: AppPreviewResult) -> MaintenancePreviewPayload {
    MaintenancePreviewPayload {
        rule_set_id: result.rule_set_id,
        matcher_content_hash: result.matcher_content_hash,
        evaluated_at: result.evaluated_at,
        titles: result
            .titles
            .into_iter()
            .map(|title| MaintenancePreviewTitle {
                excluded: title.excluded,
                due_at: title.due_at,
                due_at_is_estimate: title.due_at_is_estimate,
                subject_kind: title.subject_kind.as_storage_str().to_string(),
                subject_id: ID::from(title.subject_id),
                subject_label: title.subject_label,
                file_count: to_graphql_int(title.file_count),
                total_size_bytes: title.total_size_bytes,
                storage_root_file_count: title.storage_root_file_count.map(to_graphql_int),
                storage_root_total_size_bytes: title.storage_root_total_size_bytes,
                title_id: ID::from(title.title_id),
                title_name: title.title_name,
                facet: title.facet.as_str().to_string(),
                library_id: title.library_id,
                outcome: title.outcome.map(maintenance_outcome_value),
                reason_codes: title.reason_codes,
                error: title.error,
            })
            .collect(),
    }
}

// ── Scheduled evaluation (RFC 137 tracks C1/C2) ─────────────────────────────

pub fn maintenance_candidate_state_value(
    state: scryer_domain::MaintenanceCandidateState,
) -> MaintenanceCandidateState {
    match state {
        scryer_domain::MaintenanceCandidateState::Observing => MaintenanceCandidateState::Observing,
        scryer_domain::MaintenanceCandidateState::PendingAction => {
            MaintenanceCandidateState::PendingAction
        }
        scryer_domain::MaintenanceCandidateState::Due => MaintenanceCandidateState::Due,
        scryer_domain::MaintenanceCandidateState::Executing => MaintenanceCandidateState::Executing,
        scryer_domain::MaintenanceCandidateState::Succeeded => MaintenanceCandidateState::Succeeded,
        scryer_domain::MaintenanceCandidateState::Failed => MaintenanceCandidateState::Failed,
        scryer_domain::MaintenanceCandidateState::Canceled => MaintenanceCandidateState::Canceled,
        scryer_domain::MaintenanceCandidateState::Excluded => MaintenanceCandidateState::Excluded,
        scryer_domain::MaintenanceCandidateState::Blocked => MaintenanceCandidateState::Blocked,
    }
}

pub fn maintenance_candidate_state_into_application(
    state: MaintenanceCandidateState,
) -> scryer_domain::MaintenanceCandidateState {
    match state {
        MaintenanceCandidateState::Observing => scryer_domain::MaintenanceCandidateState::Observing,
        MaintenanceCandidateState::PendingAction => {
            scryer_domain::MaintenanceCandidateState::PendingAction
        }
        MaintenanceCandidateState::Due => scryer_domain::MaintenanceCandidateState::Due,
        MaintenanceCandidateState::Executing => scryer_domain::MaintenanceCandidateState::Executing,
        MaintenanceCandidateState::Succeeded => scryer_domain::MaintenanceCandidateState::Succeeded,
        MaintenanceCandidateState::Failed => scryer_domain::MaintenanceCandidateState::Failed,
        MaintenanceCandidateState::Canceled => scryer_domain::MaintenanceCandidateState::Canceled,
        MaintenanceCandidateState::Excluded => scryer_domain::MaintenanceCandidateState::Excluded,
        MaintenanceCandidateState::Blocked => scryer_domain::MaintenanceCandidateState::Blocked,
    }
}

/// A candidate stores its action by the catalog's wire name. A name this build
/// does not know was written by a newer one; it projects as `DO_NOTHING`, the
/// only kind that authorizes nothing, rather than guessing at a destructive
/// action whose semantics this build does not have.
pub fn from_maintenance_candidate(
    view: scryer_application::maintenance_rules::MaintenanceCandidateView,
) -> MaintenanceCandidate {
    let scryer_application::maintenance_rules::MaintenanceCandidateView {
        subject_label,
        file_count,
        total_size_bytes,
        storage_root_file_count,
        storage_root_total_size_bytes,
        candidate,
        rule_name,
        title_name,
        action_definition,
        sequence_steps,
    } = view;
    let action_sequence = action_definition.as_ref().and_then(|definition| {
        let (_, sequence) = from_maintenance_action_definition(definition);
        sequence
    });
    MaintenanceCandidate {
        file_count: file_count.map(to_graphql_int),
        total_size_bytes,
        storage_root_file_count: storage_root_file_count.map(to_graphql_int),
        storage_root_total_size_bytes,
        subject_label,
        subject_kind: candidate.subject_kind,
        subject_id: ID::from(candidate.subject_id),
        id: ID::from(candidate.id),
        rule_set_id: ID::from(candidate.rule_set_id),
        rule_name,
        revision_number: to_graphql_int(candidate.revision_number),
        title_id: ID::from(candidate.title_id),
        title_name,
        library_id: candidate.library_id,
        facet: candidate.facet,
        state: maintenance_candidate_state_value(candidate.state),
        state_reason: candidate.state_reason,
        reason_codes: candidate.reason_codes,
        action_kind: if candidate.action_kind
            == scryer_application::maintenance_rules::ACTION_SEQUENCE_KIND
        {
            MaintenanceActionKind::ActionSequence
        } else {
            maintenance_action_kind_value(
                AppActionKind::parse_wire_str(&candidate.action_kind)
                    .unwrap_or(AppActionKind::DoNothing),
            )
        },
        action_sequence,
        sequence_steps: sequence_steps
            .iter()
            .map(from_maintenance_action_step_progress)
            .collect(),
        grace_days: to_graphql_int(candidate.grace_days),
        match_generation: to_graphql_int(candidate.match_generation),
        first_matched_at: candidate.first_matched_at,
        last_matched_at: candidate.last_matched_at,
        due_at: candidate.due_at,
        held_since: candidate.held_since,
        updated_at: candidate.updated_at,
    }
}

pub fn from_maintenance_evaluation_run(
    run: scryer_domain::MaintenanceEvaluationRun,
) -> MaintenanceEvaluationRun {
    MaintenanceEvaluationRun {
        id: ID::from(run.id),
        rule_set_id: ID::from(run.rule_set_id),
        revision_number: to_graphql_int(run.revision_number),
        status: run.status.as_storage_str().to_string(),
        started_at: run.started_at,
        finished_at: run.finished_at,
        evaluated_count: to_graphql_int(run.evaluated_count),
        matched_count: to_graphql_int(run.matched_count),
        no_match_count: to_graphql_int(run.no_match_count),
        unknown_count: to_graphql_int(run.unknown_count),
        error_count: to_graphql_int(run.error_count),
        duration_ms: run.duration_ms.map(to_graphql_int),
        error: run.error,
    }
}

pub fn from_maintenance_instance_gates(
    gates: scryer_application::maintenance_rules::MaintenanceGates,
) -> MaintenanceInstanceGates {
    MaintenanceInstanceGates {
        evaluation_enabled: gates.evaluation_enabled,
        result_display_enabled: gates.result_display_enabled,
        presentation_effects_enabled: gates.presentation_effects_enabled,
        reversible_effects_enabled: gates.reversible_effects_enabled,
        destructive_effects_enabled: gates.destructive_effects_enabled,
    }
}

pub fn from_maintenance_exclusion(
    view: scryer_application::maintenance_rules::MaintenanceExclusionView,
) -> MaintenanceExclusion {
    let exclusion = view.exclusion;
    MaintenanceExclusion {
        subject_label: view.subject_label,
        subject_kind: exclusion.subject_kind.as_storage_str().to_string(),
        subject_id: ID::from(exclusion.subject_id),
        id: ID::from(exclusion.id),
        rule_set_id: exclusion.rule_set_id.map(ID::from),
        title_id: ID::from(exclusion.title_id),
        title_name: view.title_name,
        reason: exclusion.reason,
        created_by: exclusion.created_by.map(ID::from),
        created_at: exclusion.created_at,
    }
}

pub fn from_maintenance_evaluation_trigger(
    trigger: scryer_application::maintenance_rules::MaintenanceEvaluationTrigger,
) -> MaintenanceEvaluationTriggerPayload {
    MaintenanceEvaluationTriggerPayload {
        started: trigger.started,
        message: trigger.message,
    }
}

/// Shape mapping only: the kind and its one optional parameter are handed to
/// the application layer as written, and every illegal pairing (a
/// quality-profile action with no target above all) is rejected by the service
/// validation that also guards stored revisions. A target supplied for a
/// parameterless kind is dropped, because the catalog has nowhere to put it.
pub fn maintenance_action_spec_from_input(input: MaintenanceActionInput) -> AppActionSpec {
    let kind = maintenance_action_kind_into_application(input.kind);
    match kind {
        AppActionKind::ChangeQualityProfileAndSearchIfChanged => {
            match input.target_quality_profile_id {
                Some(target) => AppActionSpec::change_quality_profile(target),
                None => AppActionSpec::new(kind),
            }
        }
        AppActionKind::AddTags | AppActionKind::RemoveTags => {
            AppActionSpec::tags(kind, input.tags.unwrap_or_default())
        }
        kind => AppActionSpec::new(kind),
    }
}

/// Resolve the two mutually exclusive authoring shapes without ever converting
/// a legacy action into a sequence during a read. GraphQL calls this before it
/// reaches the service, and the service receives one typed definition only.
pub fn maintenance_action_definition_from_inputs(
    action: Option<MaintenanceActionInput>,
    action_sequence: Option<MaintenanceActionSequenceInput>,
) -> Result<AppActionDefinition, String> {
    match (action, action_sequence) {
        (Some(_), Some(_)) => Err("supply either 'action' or 'actionSequence', not both".into()),
        (None, None) => Err("a maintenance rule requires 'action' or 'actionSequence'".into()),
        (Some(action), None) => {
            if action.kind == MaintenanceActionKind::ActionSequence {
                return Err("ACTION_SEQUENCE is output-only; use 'actionSequence'".into());
            }
            Ok(AppActionDefinition::Legacy(
                maintenance_action_spec_from_input(action),
            ))
        }
        (None, Some(sequence)) => {
            let schema_version = u32::try_from(sequence.schema_version)
                .map_err(|_| "actionSequence.schemaVersion must be zero or greater".to_string())?;
            let steps = sequence
                .steps
                .into_iter()
                .map(maintenance_action_step_from_input)
                .collect::<Result<Vec<_>, _>>()?;
            Ok(AppActionDefinition::Sequence(AppActionSequence {
                schema_version,
                steps,
            }))
        }
    }
}

fn maintenance_action_step_from_input(
    input: MaintenanceActionStepInput,
) -> Result<AppActionStep, String> {
    let kind = maintenance_action_step_kind_into_application(input.kind);
    let parameters = maintenance_action_step_parameters_from_input(kind, input.parameters)?;
    Ok(AppActionStep {
        id: input.id.to_string(),
        kind,
        parameters,
    })
}

fn maintenance_action_step_parameters_from_input(
    kind: AppActionStepKind,
    input: MaintenanceActionStepParametersInput,
) -> Result<AppActionStepParameters, String> {
    let MaintenanceActionStepParametersInput {
        include_descendants,
        target_quality_profile_id,
        search_condition,
        tags,
    } = input;
    let allows_only =
        |allows_descendants: bool, allows_target: bool, allows_search: bool, allows_tags: bool| {
            (allows_descendants || include_descendants.is_none())
                && (allows_target || target_quality_profile_id.is_none())
                && (allows_search || search_condition.is_none())
                && (allows_tags || tags.is_none())
        };
    match kind {
        AppActionStepKind::Unmonitor => {
            if !allows_only(true, false, false, false) {
                return Err("unmonitor accepts only includeDescendants".into());
            }
            Ok(AppActionStepParameters::Unmonitor {
                include_descendants: include_descendants
                    .ok_or_else(|| "unmonitor requires includeDescendants".to_string())?,
            })
        }
        AppActionStepKind::ChangeQualityProfile => {
            if !allows_only(false, true, false, false) {
                return Err("change-quality-profile accepts only targetQualityProfileId".into());
            }
            Ok(AppActionStepParameters::ChangeQualityProfile {
                target_quality_profile_id: target_quality_profile_id.ok_or_else(|| {
                    "change-quality-profile requires targetQualityProfileId".to_string()
                })?,
            })
        }
        AppActionStepKind::Search => {
            if !allows_only(false, false, true, false) {
                return Err("search accepts only searchCondition".into());
            }
            Ok(AppActionStepParameters::Search {
                condition: maintenance_search_condition_into_application(
                    search_condition
                        .ok_or_else(|| "search requires searchCondition".to_string())?,
                ),
            })
        }
        AppActionStepKind::AddTags | AppActionStepKind::RemoveTags => {
            if !allows_only(false, false, false, true) {
                return Err("a tag step accepts only tags".into());
            }
            Ok(AppActionStepParameters::Tags {
                tags: tags.ok_or_else(|| "a tag step requires tags".to_string())?,
            })
        }
        AppActionStepKind::DeleteFiles | AppActionStepKind::DeleteTitleAndFiles => {
            if !allows_only(false, false, false, false) {
                return Err("this step does not accept parameters".into());
            }
            Ok(AppActionStepParameters::None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use scryer_application::maintenance_rules::action_execution::{
        MaintenanceActionRunView, MaintenanceActionStepProgressView,
    };
    use scryer_application::maintenance_rules::evaluation::MaintenanceCandidateView;
    use scryer_domain::{
        LifecycleActionRun, LifecycleActionRunStatus, LifecycleCandidate,
        MaintenanceActionJobReceipt, MaintenanceActionJobReceiptState, MaintenanceActionStepKey,
        MaintenanceActionStepRun, MaintenanceActionStepState, MaintenanceCandidateState,
    };

    /// The descriptor strings are hand-written above so the schema does not
    /// depend on serde's rename rules; this proves the two never drift.
    fn serde_name<T: serde::Serialize>(value: &T) -> String {
        serde_json::to_value(value)
            .expect("catalog enums serialize")
            .as_str()
            .expect("catalog enums serialize as strings")
            .to_string()
    }

    #[test]
    fn descriptor_strings_match_the_stored_action_wire_names() {
        for descriptor in action_catalog() {
            assert_eq!(
                maintenance_timing_mode_name(descriptor.timing_mode),
                serde_name(&descriptor.timing_mode)
            );
            for effect in descriptor.effect_classes {
                assert_eq!(
                    maintenance_effect_class_name(*effect),
                    serde_name(effect),
                    "effect drift for {:?}",
                    descriptor.kind
                );
            }
            for repeat in descriptor.allowed_repeat_modes {
                assert_eq!(
                    maintenance_repeat_mode_name(*repeat),
                    serde_name(repeat),
                    "repeat mode drift for {:?}",
                    descriptor.kind
                );
            }
        }
    }

    #[test]
    fn every_catalog_action_is_projected_and_only_one_needs_a_profile() {
        let descriptors = maintenance_action_descriptors();
        assert_eq!(descriptors.len(), action_catalog().len());
        let requiring: Vec<_> = descriptors
            .iter()
            .filter(|descriptor| descriptor.requires_target_quality_profile)
            .map(|descriptor| descriptor.kind)
            .collect();
        assert_eq!(
            requiring,
            vec![MaintenanceActionKind::ChangeQualityProfileAndSearchIfChanged]
        );
        let storage_scoped: Vec<_> = descriptors
            .iter()
            .filter(|descriptor| descriptor.supports_storage_scope)
            .map(|descriptor| descriptor.kind)
            .collect();
        assert_eq!(
            storage_scoped,
            vec![
                MaintenanceActionKind::DoNothing,
                MaintenanceActionKind::UnmonitorScopeKeepFiles,
                MaintenanceActionKind::UnmonitorTitleDeleteAllFiles,
                MaintenanceActionKind::UnmonitorScopeDeleteFiles,
            ]
        );
        for descriptor in &descriptors {
            assert!(!descriptor.supported_subjects.is_empty());
            assert!(!descriptor.effect_classes.is_empty());
            assert!(!descriptor.allowed_repeat_modes.is_empty());
        }
    }

    #[test]
    fn action_input_maps_the_quality_profile_target_and_leaves_it_off_other_kinds() {
        let quality = maintenance_action_spec_from_input(MaintenanceActionInput {
            kind: MaintenanceActionKind::ChangeQualityProfileAndSearchIfChanged,
            target_quality_profile_id: Some("hd-1080p".to_string()),
            tags: None,
        });
        assert_eq!(
            quality,
            AppActionSpec::change_quality_profile("hd-1080p"),
            "the target must reach the stored spec"
        );

        // No resolver-side gate: the missing target has to fail in the same
        // validation that guards a stored revision.
        let missing_target = maintenance_action_spec_from_input(MaintenanceActionInput {
            kind: MaintenanceActionKind::ChangeQualityProfileAndSearchIfChanged,
            target_quality_profile_id: None,
            tags: None,
        });
        assert!(
            missing_target.validate(AppSubjectKind::Movie).is_err(),
            "a quality-profile action without a target must be rejected downstream"
        );

        let parameterless = maintenance_action_spec_from_input(MaintenanceActionInput {
            kind: MaintenanceActionKind::DeleteTitleAndFiles,
            target_quality_profile_id: Some("hd-1080p".to_string()),
            tags: Some(vec!["keep".to_string()]),
        });
        assert_eq!(
            parameterless,
            AppActionSpec::new(AppActionKind::DeleteTitleAndFiles),
            "parameters the kind cannot hold are dropped rather than smuggled through"
        );
    }

    #[test]
    fn action_input_maps_tags_only_onto_the_tag_kinds() {
        for kind in [
            MaintenanceActionKind::AddTags,
            MaintenanceActionKind::RemoveTags,
        ] {
            let spec = maintenance_action_spec_from_input(MaintenanceActionInput {
                kind,
                target_quality_profile_id: Some("hd-1080p".to_string()),
                tags: Some(vec!["needs review".to_string()]),
            });
            assert_eq!(
                spec,
                AppActionSpec::tags(
                    maintenance_action_kind_into_application(kind),
                    vec!["needs review".to_string()]
                )
            );
            assert!(spec.validate(AppSubjectKind::Show).is_ok());
        }

        // No resolver-side gate here either: an empty tag list has to fail in
        // the same validation that guards a stored revision.
        let missing = maintenance_action_spec_from_input(MaintenanceActionInput {
            kind: MaintenanceActionKind::AddTags,
            target_quality_profile_id: None,
            tags: None,
        });
        assert!(missing.validate(AppSubjectKind::Movie).is_err());
    }

    #[test]
    fn action_spec_round_trips_through_the_wire_projection() {
        let projected =
            from_maintenance_action_spec(&AppActionSpec::change_quality_profile("hd-1080p"));
        assert_eq!(
            projected.kind,
            MaintenanceActionKind::ChangeQualityProfileAndSearchIfChanged
        );
        assert_eq!(
            projected.target_quality_profile_id.as_deref(),
            Some("hd-1080p")
        );
        assert_eq!(
            from_maintenance_action_spec(&AppActionSpec::new(AppActionKind::DoNothing))
                .target_quality_profile_id,
            None
        );
    }

    fn step_parameters() -> MaintenanceActionStepParametersInput {
        MaintenanceActionStepParametersInput {
            include_descendants: None,
            target_quality_profile_id: None,
            search_condition: None,
            tags: None,
        }
    }

    #[test]
    fn action_definition_input_requires_exactly_one_shape() {
        let action = MaintenanceActionInput {
            kind: MaintenanceActionKind::DoNothing,
            target_quality_profile_id: None,
            tags: None,
        };
        let sequence = MaintenanceActionSequenceInput {
            schema_version: 2,
            steps: Vec::new(),
        };

        assert!(
            maintenance_action_definition_from_inputs(None, None)
                .expect_err("missing action definition must fail")
                .contains("requires")
        );
        assert!(
            maintenance_action_definition_from_inputs(Some(action), Some(sequence))
                .expect_err("both action shapes must fail")
                .contains("either")
        );
    }

    #[test]
    fn sequence_input_keeps_typed_parameters_and_rejects_stale_ones() {
        let mut unmonitor_parameters = step_parameters();
        unmonitor_parameters.include_descendants = Some(true);
        let mut search_parameters = step_parameters();
        search_parameters.search_condition = Some(MaintenanceSearchCondition::Unconditional);
        let definition = maintenance_action_definition_from_inputs(
            None,
            Some(MaintenanceActionSequenceInput {
                schema_version: 2,
                steps: vec![
                    MaintenanceActionStepInput {
                        id: ID::from("unmonitor"),
                        kind: MaintenanceActionStepKind::Unmonitor,
                        parameters: unmonitor_parameters,
                    },
                    MaintenanceActionStepInput {
                        id: ID::from("search"),
                        kind: MaintenanceActionStepKind::Search,
                        parameters: search_parameters,
                    },
                ],
            }),
        )
        .expect("typed sequence input maps");
        let AppActionDefinition::Sequence(sequence) = definition else {
            panic!("expected sequence definition");
        };
        assert_eq!(sequence.schema_version, 2);
        assert!(matches!(
            sequence.steps[0].parameters,
            AppActionStepParameters::Unmonitor {
                include_descendants: true
            }
        ));
        assert!(matches!(
            sequence.steps[1].parameters,
            AppActionStepParameters::Search {
                condition: AppSearchCondition::Unconditional
            }
        ));

        let mut invalid_parameters = step_parameters();
        invalid_parameters.include_descendants = Some(true);
        invalid_parameters.tags = Some(vec!["keep".into()]);
        assert!(
            maintenance_action_definition_from_inputs(
                None,
                Some(MaintenanceActionSequenceInput {
                    schema_version: 2,
                    steps: vec![MaintenanceActionStepInput {
                        id: ID::from("invalid"),
                        kind: MaintenanceActionStepKind::Unmonitor,
                        parameters: invalid_parameters,
                    }],
                }),
            )
            .expect_err("a step must reject fields from another parameter schema")
            .contains("only")
        );
    }

    #[test]
    fn sequence_projection_uses_the_sentinel_and_preserves_every_step() {
        let definition = AppActionDefinition::Sequence(AppActionSequence {
            schema_version: 2,
            steps: vec![AppActionStep {
                id: "search".into(),
                kind: AppActionStepKind::Search,
                parameters: AppActionStepParameters::Search {
                    condition: AppSearchCondition::PreviousProfileChanged,
                },
            }],
        });

        let (legacy_projection, sequence) = from_maintenance_action_definition(&definition);
        assert_eq!(
            legacy_projection.kind,
            MaintenanceActionKind::ActionSequence
        );
        let sequence = sequence.expect("sequence payload is never replaced by a legacy action");
        assert_eq!(sequence.steps[0].kind, MaintenanceActionStepKind::Search);
        assert_eq!(
            sequence.steps[0].parameters.search_condition,
            Some(MaintenanceSearchCondition::PreviousProfileChanged)
        );
    }

    #[test]
    fn sequence_action_history_projects_ordered_steps_and_accepted_search_receipt() {
        let now = Utc::now();
        let profile_step = AppActionStep {
            id: "profile".into(),
            kind: AppActionStepKind::ChangeQualityProfile,
            parameters: AppActionStepParameters::ChangeQualityProfile {
                target_quality_profile_id: "quality-hd".into(),
            },
        };
        let search_step = AppActionStep {
            id: "search".into(),
            kind: AppActionStepKind::Search,
            parameters: AppActionStepParameters::Search {
                condition: AppSearchCondition::PreviousProfileChanged,
            },
        };
        let search_key = MaintenanceActionStepKey {
            candidate_id: "candidate-1".into(),
            match_generation: 3,
            revision_number: 2,
            step_id: search_step.id.clone(),
        };
        let search_run = MaintenanceActionStepRun {
            key: search_key.clone(),
            rule_set_id: "rule-1".into(),
            title_id: "title-1".into(),
            subject_kind: "title".into(),
            subject_id: "title-1".into(),
            sequence_content_hash: "hash".into(),
            step_kind: "search".into(),
            intent_json: "{}".into(),
            before_state_json: "{}".into(),
            target_identity_json: "{}".into(),
            provenance_json: "{}".into(),
            state: MaintenanceActionStepState::Succeeded,
            attempt: 1,
            lease_id: None,
            lease_expires_at: None,
            hold_reason: None,
            error: None,
            created_at: now,
            updated_at: now,
            finished_at: Some(now),
        };
        let receipt = MaintenanceActionJobReceipt {
            schema_version: 1,
            key: search_key,
            dispatch_attempt: 1,
            logical_request_key: "maintenance-sequence:candidate-1:3:2:search".into(),
            request_hash: "request-hash".into(),
            job_run_id: Some("job-1".into()),
            state: MaintenanceActionJobReceiptState::Accepted,
            reconciliation_evidence_json: "{}".into(),
            created_at: now,
            updated_at: now,
        };
        let view = MaintenanceActionRunView {
            subject_label: "Example".into(),
            run: LifecycleActionRun {
                id: "action-run-1".into(),
                candidate_id: "candidate-1".into(),
                rule_set_id: "rule-1".into(),
                revision_number: 2,
                title_id: "title-1".into(),
                subject_kind: "title".into(),
                subject_id: "title-1".into(),
                action_kind: "action_sequence".into(),
                match_generation: 3,
                idempotency_key: "key".into(),
                attempt: 1,
                status: LifecycleActionRunStatus::Succeeded,
                hold_reason: None,
                error: None,
                detail: "{}".into(),
                started_at: now,
                finished_at: Some(now),
                created_at: now,
            },
            title_name: "Example".into(),
            action_definition: Some(AppActionDefinition::Sequence(AppActionSequence {
                schema_version: 2,
                steps: vec![profile_step, search_step],
            })),
            sequence_steps: vec![MaintenanceActionStepProgressView {
                step: AppActionStep {
                    id: "search".into(),
                    kind: AppActionStepKind::Search,
                    parameters: AppActionStepParameters::Search {
                        condition: AppSearchCondition::PreviousProfileChanged,
                    },
                },
                run: Some(search_run),
                receipts: vec![receipt],
            }],
        };

        let projected = from_maintenance_action_run(view).expect("known action kind projects");
        assert_eq!(projected.action_kind, MaintenanceActionKind::ActionSequence);
        let sequence = projected.action_sequence.expect("sequence");
        assert_eq!(sequence.steps.len(), 2);
        assert_eq!(
            sequence
                .steps
                .iter()
                .map(|step| step.kind)
                .collect::<Vec<_>>(),
            vec![
                MaintenanceActionStepKind::ChangeQualityProfile,
                MaintenanceActionStepKind::Search,
            ]
        );
        assert_eq!(projected.sequence_steps.len(), 1);
        assert_eq!(
            projected.sequence_steps[0]
                .run
                .as_ref()
                .map(|run| run.state.as_str()),
            Some("succeeded")
        );
        assert_eq!(projected.sequence_steps[0].receipts[0].state, "accepted");
        assert_eq!(
            projected.sequence_steps[0].receipts[0]
                .job_run_id
                .as_ref()
                .map(|id| id.as_str()),
            Some("job-1")
        );
    }

    #[test]
    fn candidate_projection_preserves_empty_and_legacy_action_definitions() {
        let now = Utc::now();
        let candidate = LifecycleCandidate {
            id: "candidate-1".into(),
            rule_set_id: "rule-1".into(),
            revision_number: 2,
            matcher_content_hash: "hash".into(),
            title_id: "title-1".into(),
            library_id: "library-1".into(),
            facet: "movie".into(),
            subject_kind: "title".into(),
            subject_id: "title-1".into(),
            match_generation: 1,
            state: MaintenanceCandidateState::Blocked,
            state_reason: "waiting".into(),
            reason_codes: Vec::new(),
            action_kind: "action_sequence".into(),
            grace_days: 0,
            first_matched_at: now,
            last_matched_at: now,
            due_at: now,
            last_evaluated_at: now,
            held_since: Some(now),
            action_attempts: 0,
            created_at: now,
            updated_at: now,
        };
        let sequence_view = MaintenanceCandidateView {
            subject_label: "Example".into(),
            file_count: Some(1),
            total_size_bytes: Some(42),
            storage_root_file_count: Some(1),
            storage_root_total_size_bytes: Some(42),
            candidate: candidate.clone(),
            rule_name: "Rule".into(),
            title_name: "Example".into(),
            action_definition: Some(AppActionDefinition::Sequence(AppActionSequence {
                schema_version: 2,
                steps: Vec::new(),
            })),
            sequence_steps: Vec::new(),
        };
        let projected = from_maintenance_candidate(sequence_view);
        assert_eq!(projected.action_kind, MaintenanceActionKind::ActionSequence);
        assert_eq!(projected.storage_root_file_count, Some(1));
        assert_eq!(projected.storage_root_total_size_bytes, Some(42));
        assert!(projected.action_sequence.is_some());
        assert!(projected.sequence_steps.is_empty());

        let legacy_view = MaintenanceCandidateView {
            candidate: LifecycleCandidate {
                action_kind: AppActionKind::DoNothing.as_wire_str().into(),
                ..candidate
            },
            subject_label: "Example".into(),
            file_count: None,
            total_size_bytes: None,
            storage_root_file_count: None,
            storage_root_total_size_bytes: None,
            rule_name: "Rule".into(),
            title_name: "Example".into(),
            action_definition: Some(AppActionDefinition::Legacy(AppActionSpec::new(
                AppActionKind::DoNothing,
            ))),
            sequence_steps: Vec::new(),
        };
        let legacy = from_maintenance_candidate(legacy_view);
        assert_eq!(legacy.action_kind, MaintenanceActionKind::DoNothing);
        assert_eq!(legacy.storage_root_file_count, None);
        assert_eq!(legacy.storage_root_total_size_bytes, None);
        assert!(legacy.action_sequence.is_none());
        assert!(legacy.sequence_steps.is_empty());
    }
}
