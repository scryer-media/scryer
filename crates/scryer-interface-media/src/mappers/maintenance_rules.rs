//! Projections for the maintenance-rule authoring surface.
//!
//! A rule's source declares its own actions; this module renames the declared
//! and decided actions onto the wire. Historical candidates and runs keep the
//! v1 action kind or v2 sequence they were written with.

use super::*;

use scryer_application::maintenance_rules::{
    MaintenanceActionDefinition as AppActionDefinition, MaintenanceActionKind as AppActionKind,
    MaintenanceActionSequence as AppActionSequence,
    MaintenanceActionStepKind as AppActionStepKind,
    MaintenanceActionStepParameters as AppActionStepParameters,
    MaintenanceDeclaredActions as AppDeclaredActions,
    MaintenanceSearchCondition as AppSearchCondition, decided_actions_from_json,
    decided_heads_in_execution_order, derived_effect_arming, maintenance_rule_state,
};
use scryer_rules::maintenance_actions::{
    MaintenanceActionHead as AppActionHead, MaintenanceActions as AppDecidedActions,
};
use scryer_application::maintenance_rules::{
    MaintenancePreviewResult as AppPreviewResult, MaintenanceRuleSetDetail as AppRuleSetDetail,
};

/// Revision numbers and grace periods are stored as `i64` but are small by
/// construction; saturating keeps a corrupt row from panicking a read.
fn to_graphql_int(value: i64) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

pub fn maintenance_rule_state_value(
    state: scryer_domain::MaintenanceRuleState,
) -> MaintenanceRuleState {
    match state {
        scryer_domain::MaintenanceRuleState::Disabled => MaintenanceRuleState::Disabled,
        scryer_domain::MaintenanceRuleState::DryRun => MaintenanceRuleState::DryRun,
        scryer_domain::MaintenanceRuleState::Enabled => MaintenanceRuleState::Enabled,
    }
}

pub fn maintenance_rule_state_into_application(
    state: MaintenanceRuleState,
) -> scryer_domain::MaintenanceRuleState {
    match state {
        MaintenanceRuleState::Disabled => scryer_domain::MaintenanceRuleState::Disabled,
        MaintenanceRuleState::DryRun => scryer_domain::MaintenanceRuleState::DryRun,
        MaintenanceRuleState::Enabled => scryer_domain::MaintenanceRuleState::Enabled,
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
    let action_sequence = action_definition
        .as_ref()
        .and_then(from_maintenance_action_definition);
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
    MaintenanceRuleSet {
        id: ID::from(rule_set.id.clone()),
        name: rule_set.name.clone(),
        description: rule_set.description.clone(),
        state: maintenance_rule_state_value(maintenance_rule_state(detail)),
        enabled_arming: maintenance_effect_arming_value(derived_effect_arming(
            &detail.action_definition,
        )),
        edited_since_enabled: rule_set.destructive_rearm_required,
        system_owned: rule_set.system_owned,
        library_ids: rule_set.library_ids.clone(),
        subject_kind: maintenance_rule_subject_kind_value(rule_set.subject_kind),
        current_revision_number: to_graphql_int(rule_set.current_revision_number),
        grace_days: to_graphql_int(detail.revision.grace_days),
        declared_actions: from_maintenance_declared_definition(&detail.action_definition),
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

fn maintenance_search_condition_value(condition: AppSearchCondition) -> MaintenanceSearchCondition {
    match condition {
        AppSearchCondition::Unconditional => MaintenanceSearchCondition::Unconditional,
        AppSearchCondition::PreviousProfileChanged => {
            MaintenanceSearchCondition::PreviousProfileChanged
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

/// The v2 sequence a historical candidate or run was written with. A v1
/// action and a declared-actions revision have none.
fn from_maintenance_action_definition(
    definition: &AppActionDefinition,
) -> Option<MaintenanceActionSequence> {
    match definition {
        AppActionDefinition::Sequence(sequence) => Some(from_maintenance_action_sequence(sequence)),
        AppActionDefinition::Legacy(_) | AppActionDefinition::Declared(_) => None,
    }
}

pub fn maintenance_action_head_value(head: AppActionHead) -> MaintenanceActionHead {
    match head {
        AppActionHead::Unmonitor => MaintenanceActionHead::Unmonitor,
        AppActionHead::DeleteFiles => MaintenanceActionHead::DeleteFiles,
        AppActionHead::DeleteTitle => MaintenanceActionHead::DeleteTitle,
        AppActionHead::Search => MaintenanceActionHead::Search,
        AppActionHead::SetQualityProfile => MaintenanceActionHead::SetQualityProfile,
        AppActionHead::AddTags => MaintenanceActionHead::AddTags,
        AppActionHead::RemoveTags => MaintenanceActionHead::RemoveTags,
    }
}

pub fn from_maintenance_declared_actions(declared: &AppDeclaredActions) -> MaintenanceDeclaredActions {
    MaintenanceDeclaredActions {
        heads: declared
            .heads
            .iter()
            .map(|head| maintenance_action_head_value(*head))
            .collect(),
        destructive: declared.destructive,
        quality_profiles: declared.quality_profiles.clone(),
        add_tags: declared.add_tags.clone(),
        remove_tags: declared.remove_tags.clone(),
        libraries: Vec::new(),
        roots: Vec::new(),
        notifications: Vec::new(),
    }
}

/// Every revision in force has been converted to declared actions by the
/// startup migration. A revision that could not be converted keeps its old
/// definition and is disabled; it declares nothing here.
fn from_maintenance_declared_definition(
    definition: &AppActionDefinition,
) -> MaintenanceDeclaredActions {
    match definition {
        AppActionDefinition::Declared(declared) => from_maintenance_declared_actions(declared),
        AppActionDefinition::Legacy(_) | AppActionDefinition::Sequence(_) => {
            MaintenanceDeclaredActions {
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
    }
}

pub fn from_maintenance_decided_actions(actions: &AppDecidedActions) -> MaintenanceDecidedActions {
    MaintenanceDecidedActions {
        heads: decided_heads_in_execution_order(actions)
            .into_iter()
            .map(maintenance_action_head_value)
            .collect(),
        quality_profile: actions.set_quality_profile.clone(),
        add_tags: actions.add_tags.clone(),
        remove_tags: actions.remove_tags.clone(),
        library: None,
        root: None,
        notification: None,
    }
}

pub fn from_maintenance_rule_set_detail(detail: AppRuleSetDetail) -> MaintenanceRuleSetDetail {
    let rule_set = from_maintenance_rule_set(&detail);
    MaintenanceRuleSetDetail {
        declared_actions: rule_set.declared_actions.clone(),
        rule_set,
        revision: from_maintenance_rule_revision(detail.revision),
    }
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
                actions: title.actions.as_ref().map(from_maintenance_decided_actions),
                facts: title.facts.map(async_graphql::Json),
            })
            .collect(),
    }
}

// ── Scheduled evaluation ────────────────────────────────────────────────────

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
    let action_sequence = action_definition
        .as_ref()
        .and_then(from_maintenance_action_definition);
    let actions = decided_actions_from_json(candidate.decided_actions_json.as_deref())
        .as_ref()
        .map(from_maintenance_decided_actions);
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
        actions,
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
        enabled: gates.enabled(),
        actions_paused: gates.actions_paused(),
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

    #[test]
    fn sequence_projection_preserves_every_step() {
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

        let sequence = from_maintenance_action_definition(&definition)
            .expect("sequence payload is never replaced by a legacy action");
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
            decided_actions_json: None,
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
            action_definition: Some(AppActionDefinition::Legacy(
                scryer_application::maintenance_rules::MaintenanceActionSpec::new(
                    AppActionKind::DoNothing,
                ),
            )),
            sequence_steps: Vec::new(),
        };
        let legacy = from_maintenance_candidate(legacy_view);
        assert_eq!(legacy.action_kind, MaintenanceActionKind::DoNothing);
        assert_eq!(legacy.storage_root_file_count, None);
        assert_eq!(legacy.storage_root_total_size_bytes, None);
        assert!(legacy.action_sequence.is_none());
        assert!(legacy.sequence_steps.is_empty());
    }

    #[test]
    fn decided_actions_project_in_execution_order_and_declared_ones_in_source_order() {
        let decided = from_maintenance_decided_actions(&AppDecidedActions {
            delete_files: true,
            add_tags: vec!["stale".into()],
            set_quality_profile: Some("Compact 1080p".into()),
            search: true,
            ..AppDecidedActions::default()
        });
        assert_eq!(
            decided.heads,
            vec![
                MaintenanceActionHead::AddTags,
                MaintenanceActionHead::SetQualityProfile,
                MaintenanceActionHead::Search,
                MaintenanceActionHead::Unmonitor,
                MaintenanceActionHead::DeleteFiles,
            ]
        );
        assert_eq!(decided.quality_profile.as_deref(), Some("Compact 1080p"));
        assert_eq!(decided.add_tags, vec!["stale".to_string()]);

        let scan = scryer_rules::maintenance_actions::scan_maintenance_action_heads(
            "package rules\nimport rego.v1\n\nsearch if {\n\ttrue\n}\n\ndelete_title if {\n\tfalse\n}\n",
            "rule-1",
        )
        .expect("source parses");
        let declared = from_maintenance_declared_actions(&AppDeclaredActions::from_scan(&scan));
        assert_eq!(
            declared.heads,
            vec![MaintenanceActionHead::Search, MaintenanceActionHead::DeleteTitle]
        );
        assert!(declared.destructive);
    }
}
