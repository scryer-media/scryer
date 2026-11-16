//! Projections for the lifecycle-rule authoring surface.
//!
//! A rule's source declares its own actions; this module renames the declared
//! and decided actions onto the wire, along with the step sequence each
//! candidate's decided actions build.

use super::*;

use scryer_application::lifecycle_rules::{
    LifecycleActionKind as AppActionKind, LifecycleActionSequence as AppActionSequence,
    LifecycleActionStepKind as AppActionStepKind,
    LifecycleActionStepParameters as AppActionStepParameters,
    LifecycleDeclaredActions as AppDeclaredActions, LifecycleSearchCondition as AppSearchCondition,
    decided_actions_from_json, decided_heads_in_execution_order, derived_effect_arming,
    lifecycle_rule_state,
};
use scryer_application::lifecycle_rules::{
    LifecyclePreviewResult as AppPreviewResult, LifecycleRuleSetDetail as AppRuleSetDetail,
};
use scryer_rules::lifecycle_actions::{
    LifecycleActionHead as AppActionHead, LifecycleActions as AppDecidedActions,
};

/// Revision numbers and grace periods are stored as `i64` but are small by
/// construction; saturating keeps a corrupt row from panicking a read.
fn to_graphql_int(value: i64) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

pub fn lifecycle_rule_state_value(state: scryer_domain::LifecycleRuleState) -> LifecycleRuleState {
    match state {
        scryer_domain::LifecycleRuleState::Disabled => LifecycleRuleState::Disabled,
        scryer_domain::LifecycleRuleState::DryRun => LifecycleRuleState::DryRun,
        scryer_domain::LifecycleRuleState::Enabled => LifecycleRuleState::Enabled,
    }
}

pub fn lifecycle_rule_state_into_application(
    state: LifecycleRuleState,
) -> scryer_domain::LifecycleRuleState {
    match state {
        LifecycleRuleState::Disabled => scryer_domain::LifecycleRuleState::Disabled,
        LifecycleRuleState::DryRun => scryer_domain::LifecycleRuleState::DryRun,
        LifecycleRuleState::Enabled => scryer_domain::LifecycleRuleState::Enabled,
    }
}

pub fn lifecycle_rule_subject_kind_into_application(
    kind: Option<LifecycleRuleSubjectKind>,
) -> scryer_domain::LifecycleRuleSubjectKind {
    match kind.unwrap_or(LifecycleRuleSubjectKind::Title) {
        LifecycleRuleSubjectKind::Title => scryer_domain::LifecycleRuleSubjectKind::Title,
        LifecycleRuleSubjectKind::Season => scryer_domain::LifecycleRuleSubjectKind::Season,
        LifecycleRuleSubjectKind::Episode => scryer_domain::LifecycleRuleSubjectKind::Episode,
    }
}

pub fn lifecycle_rule_subject_kind_value(
    kind: scryer_domain::LifecycleRuleSubjectKind,
) -> LifecycleRuleSubjectKind {
    match kind {
        scryer_domain::LifecycleRuleSubjectKind::Title => LifecycleRuleSubjectKind::Title,
        scryer_domain::LifecycleRuleSubjectKind::Season => LifecycleRuleSubjectKind::Season,
        scryer_domain::LifecycleRuleSubjectKind::Episode => LifecycleRuleSubjectKind::Episode,
    }
}

pub fn lifecycle_effect_arming_value(
    arming: scryer_domain::LifecycleEffectArming,
) -> LifecycleEffectArming {
    match arming {
        scryer_domain::LifecycleEffectArming::None => LifecycleEffectArming::None,
        scryer_domain::LifecycleEffectArming::Reversible => LifecycleEffectArming::Reversible,
        scryer_domain::LifecycleEffectArming::Destructive => LifecycleEffectArming::Destructive,
    }
}

/// One action-handler attempt. An action kind this build cannot parse renders
/// nothing rather than a wrong kind, but by construction the executor only
/// stores catalog wire names, so the fallback is unreachable in practice.
pub fn from_lifecycle_action_run(
    view: scryer_application::lifecycle_rules::LifecycleActionRunView,
) -> Option<LifecycleActionRun> {
    let scryer_application::lifecycle_rules::LifecycleActionRunView {
        subject_label,
        run,
        title_name,
        action_sequence,
        sequence_steps,
    } = view;
    let action_kind =
        if run.action_kind == scryer_application::lifecycle_rules::ACTION_SEQUENCE_KIND {
            LifecycleActionKind::ActionSequence
        } else {
            lifecycle_action_kind_value(AppActionKind::parse_wire_str(&run.action_kind)?)
        };
    let action_sequence = action_sequence.as_ref().map(from_lifecycle_action_sequence);
    Some(LifecycleActionRun {
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
            .map(from_lifecycle_action_step_progress)
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

fn from_lifecycle_action_step_progress(
    progress: &scryer_application::lifecycle_rules::action_execution::LifecycleActionStepProgressView,
) -> LifecycleActionStepProgress {
    LifecycleActionStepProgress {
        step: LifecycleActionStep {
            id: ID::from(progress.step.id.clone()),
            kind: lifecycle_action_step_kind_value(progress.step.kind),
            parameters: from_lifecycle_action_step_parameters(&progress.step.parameters),
        },
        run: progress.run.as_ref().map(|run| LifecycleActionStepRun {
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
            .map(|receipt| LifecycleActionJobReceipt {
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

pub fn lifecycle_action_kind_value(kind: AppActionKind) -> LifecycleActionKind {
    match kind {
        AppActionKind::DoNothing => LifecycleActionKind::DoNothing,
        AppActionKind::UnmonitorScopeKeepFiles => LifecycleActionKind::UnmonitorScopeKeepFiles,
        AppActionKind::DeleteTitleAndFiles => LifecycleActionKind::DeleteTitleAndFiles,
        AppActionKind::UnmonitorTitleDeleteAllFiles => {
            LifecycleActionKind::UnmonitorTitleDeleteAllFiles
        }
        AppActionKind::UnmonitorShowDeleteExistingFiles => {
            LifecycleActionKind::UnmonitorShowDeleteExistingFiles
        }
        AppActionKind::UnmonitorScopeDeleteFiles => LifecycleActionKind::UnmonitorScopeDeleteFiles,
        AppActionKind::UnmonitorSeasonDeleteFilesThenDeleteShowIfEmpty => {
            LifecycleActionKind::UnmonitorSeasonDeleteFilesThenDeleteShowIfEmpty
        }
        AppActionKind::UnmonitorSeasonThenUnmonitorShowIfEmpty => {
            LifecycleActionKind::UnmonitorSeasonThenUnmonitorShowIfEmpty
        }
        AppActionKind::ChangeQualityProfileAndSearchIfChanged => {
            LifecycleActionKind::ChangeQualityProfileAndSearchIfChanged
        }
        AppActionKind::AddTags => LifecycleActionKind::AddTags,
        AppActionKind::RemoveTags => LifecycleActionKind::RemoveTags,
    }
}

fn lifecycle_outcome_value(outcome: scryer_rules::lifecycle::LifecycleOutcome) -> LifecycleOutcome {
    match outcome {
        scryer_rules::lifecycle::LifecycleOutcome::Match => LifecycleOutcome::Match,
        scryer_rules::lifecycle::LifecycleOutcome::NoMatch => LifecycleOutcome::NoMatch,
        scryer_rules::lifecycle::LifecycleOutcome::Unknown => LifecycleOutcome::Unknown,
    }
}

/// Projected from the detail, not from the rule-set row alone: the action and
/// grace period a list view has to show live on the revision in force, and
/// making the client fetch each row's detail to render them would turn one
/// list into N+1 requests.
/// `next_run_at` comes from the in-memory scheduler, not from the row.
pub fn from_lifecycle_rule_set(
    detail: &AppRuleSetDetail,
    next_run_at: Option<chrono::DateTime<chrono::Utc>>,
) -> LifecycleRuleSet {
    let rule_set = &detail.rule_set;
    LifecycleRuleSet {
        id: ID::from(rule_set.id.clone()),
        name: rule_set.name.clone(),
        description: rule_set.description.clone(),
        state: lifecycle_rule_state_value(lifecycle_rule_state(detail)),
        enabled_arming: lifecycle_effect_arming_value(derived_effect_arming(
            &detail.declared_actions,
        )),
        edited_since_enabled: rule_set.destructive_rearm_required,
        system_owned: rule_set.system_owned,
        library_ids: rule_set.library_ids.clone(),
        current_revision_number: to_graphql_int(rule_set.current_revision_number),
        grace_days: to_graphql_int(detail.revision.grace_days),
        declared_actions: from_lifecycle_declared_actions(&detail.declared_actions),
        load_refusal: detail.refusal().map(str::to_string),
        schedule: rule_set.schedule.clone().into(),
        max_titles_per_run: rule_set
            .max_titles_per_run
            .map(|max| i32::try_from(max).unwrap_or(i32::MAX)),
        next_run_at,
        created_at: rule_set.created_at,
        updated_at: rule_set.updated_at,
    }
}

/// The stored source always carries the system-assigned package declaration and
/// the rego.v1 import; both are stripped here for the same reason the release
/// rule editor strips them, so what the editor shows is what the author wrote.
pub fn from_lifecycle_rule_revision(
    revision: scryer_domain::LifecycleRuleRevision,
) -> LifecycleRuleRevision {
    LifecycleRuleRevision {
        id: ID::from(revision.id),
        rule_set_id: ID::from(revision.rule_set_id),
        revision_number: to_graphql_int(revision.revision_number),
        rego_source: scryer_rules::strip_editor_source(&revision.rego_source),
        grace_days: to_graphql_int(revision.grace_days),
        matcher_content_hash: revision.matcher_content_hash,
        created_by: revision.created_by.map(ID::from),
        created_at: revision.created_at,
    }
}

fn lifecycle_search_condition_value(condition: AppSearchCondition) -> LifecycleSearchCondition {
    match condition {
        AppSearchCondition::Unconditional => LifecycleSearchCondition::Unconditional,
        AppSearchCondition::PreviousProfileChanged => {
            LifecycleSearchCondition::PreviousProfileChanged
        }
    }
}

pub fn from_lifecycle_action_sequence(sequence: &AppActionSequence) -> LifecycleActionSequence {
    LifecycleActionSequence {
        schema_version: to_graphql_int(i64::from(sequence.schema_version)),
        steps: sequence
            .steps
            .iter()
            .map(|step| LifecycleActionStep {
                id: ID::from(step.id.clone()),
                kind: lifecycle_action_step_kind_value(step.kind),
                parameters: from_lifecycle_action_step_parameters(&step.parameters),
            })
            .collect(),
    }
}

fn from_lifecycle_action_step_parameters(
    parameters: &AppActionStepParameters,
) -> LifecycleActionStepParameters {
    match parameters {
        AppActionStepParameters::None => LifecycleActionStepParameters::default(),
        AppActionStepParameters::Unmonitor {
            include_descendants,
        } => LifecycleActionStepParameters {
            include_descendants: Some(*include_descendants),
            ..LifecycleActionStepParameters::default()
        },
        AppActionStepParameters::ChangeQualityProfile {
            target_quality_profile_id,
        } => LifecycleActionStepParameters {
            target_quality_profile_id: Some(target_quality_profile_id.clone()),
            ..LifecycleActionStepParameters::default()
        },
        AppActionStepParameters::Search { condition } => LifecycleActionStepParameters {
            search_condition: Some(lifecycle_search_condition_value(*condition)),
            ..LifecycleActionStepParameters::default()
        },
        AppActionStepParameters::Tags { tags } => LifecycleActionStepParameters {
            tags: tags.clone(),
            ..LifecycleActionStepParameters::default()
        },
        AppActionStepParameters::Library { library } => LifecycleActionStepParameters {
            target_library: Some(library.clone()),
            ..LifecycleActionStepParameters::default()
        },
        AppActionStepParameters::Root { root } => LifecycleActionStepParameters {
            target_root: Some(root.clone()),
            ..LifecycleActionStepParameters::default()
        },
    }
}

fn lifecycle_action_step_kind_value(kind: AppActionStepKind) -> LifecycleActionStepKind {
    match kind {
        AppActionStepKind::Unmonitor => LifecycleActionStepKind::Unmonitor,
        AppActionStepKind::DeleteFiles => LifecycleActionStepKind::DeleteFiles,
        AppActionStepKind::ChangeQualityProfile => LifecycleActionStepKind::ChangeQualityProfile,
        AppActionStepKind::Search => LifecycleActionStepKind::Search,
        AppActionStepKind::AddTags => LifecycleActionStepKind::AddTags,
        AppActionStepKind::RemoveTags => LifecycleActionStepKind::RemoveTags,
        AppActionStepKind::DeleteTitleAndFiles => LifecycleActionStepKind::DeleteTitleAndFiles,
        AppActionStepKind::Monitor => LifecycleActionStepKind::Monitor,
        AppActionStepKind::FetchSubtitles => LifecycleActionStepKind::FetchSubtitles,
        AppActionStepKind::MoveToLibrary => LifecycleActionStepKind::MoveToLibrary,
        AppActionStepKind::MoveToRoot => LifecycleActionStepKind::MoveToRoot,
    }
}

pub fn lifecycle_action_head_value(head: AppActionHead) -> LifecycleActionHead {
    match head {
        AppActionHead::Unmonitor => LifecycleActionHead::Unmonitor,
        AppActionHead::DeleteFiles => LifecycleActionHead::DeleteFiles,
        AppActionHead::DeleteTitle => LifecycleActionHead::DeleteTitle,
        AppActionHead::Search => LifecycleActionHead::Search,
        AppActionHead::SetQualityProfile => LifecycleActionHead::SetQualityProfile,
        AppActionHead::AddTags => LifecycleActionHead::AddTags,
        AppActionHead::RemoveTags => LifecycleActionHead::RemoveTags,
        AppActionHead::Monitor => LifecycleActionHead::Monitor,
        AppActionHead::FetchSubtitles => LifecycleActionHead::FetchSubtitles,
        AppActionHead::MoveToLibrary => LifecycleActionHead::MoveToLibrary,
        AppActionHead::MoveToRoot => LifecycleActionHead::MoveToRoot,
        AppActionHead::Notify => LifecycleActionHead::Notify,
        AppActionHead::MonitorSeason => LifecycleActionHead::MonitorSeason,
        AppActionHead::UnmonitorSeason => LifecycleActionHead::UnmonitorSeason,
        AppActionHead::DeleteSeasonFiles => LifecycleActionHead::DeleteSeasonFiles,
        AppActionHead::FetchSeasonSubtitles => LifecycleActionHead::FetchSeasonSubtitles,
        AppActionHead::NotifySeason => LifecycleActionHead::NotifySeason,
        AppActionHead::MonitorEpisode => LifecycleActionHead::MonitorEpisode,
        AppActionHead::UnmonitorEpisode => LifecycleActionHead::UnmonitorEpisode,
        AppActionHead::DeleteEpisodeFiles => LifecycleActionHead::DeleteEpisodeFiles,
        AppActionHead::FetchEpisodeSubtitles => LifecycleActionHead::FetchEpisodeSubtitles,
        AppActionHead::NotifyEpisode => LifecycleActionHead::NotifyEpisode,
    }
}

pub fn from_lifecycle_declared_actions(declared: &AppDeclaredActions) -> LifecycleDeclaredActions {
    LifecycleDeclaredActions {
        heads: declared
            .heads
            .iter()
            .map(|head| lifecycle_action_head_value(*head))
            .collect(),
        destructive: declared.destructive,
        quality_profiles: declared.quality_profiles.clone(),
        add_tags: declared.add_tags.clone(),
        remove_tags: declared.remove_tags.clone(),
        libraries: declared.libraries.clone(),
        roots: declared.roots.clone(),
        notifications: declared.notifications.clone(),
    }
}

pub fn from_lifecycle_decided_actions(actions: &AppDecidedActions) -> LifecycleDecidedActions {
    LifecycleDecidedActions {
        heads: decided_heads_in_execution_order(actions)
            .into_iter()
            .map(lifecycle_action_head_value)
            .collect(),
        quality_profile: actions.set_quality_profile.clone(),
        add_tags: actions.add_tags.clone(),
        remove_tags: actions.remove_tags.clone(),
        library: actions.move_to_library.clone(),
        root: actions.move_to_root.clone(),
        notification: actions.notify.clone(),
    }
}

pub fn from_lifecycle_rule_set_detail(
    detail: AppRuleSetDetail,
    next_run_at: Option<chrono::DateTime<chrono::Utc>>,
) -> LifecycleRuleSetDetail {
    let rule_set = from_lifecycle_rule_set(&detail, next_run_at);
    let builder_model =
        scryer_application::stored_builder_model(detail.revision.builder_model_json.as_deref())
            .map(async_graphql::Json);
    LifecycleRuleSetDetail {
        declared_actions: rule_set.declared_actions.clone(),
        rule_set,
        revision: from_lifecycle_rule_revision(detail.revision),
        builder_model,
    }
}

pub fn from_lifecycle_preview_result(result: AppPreviewResult) -> LifecyclePreviewPayload {
    LifecyclePreviewPayload {
        rule_set_id: result.rule_set_id,
        matcher_content_hash: result.matcher_content_hash,
        evaluated_at: result.evaluated_at,
        titles: result
            .titles
            .into_iter()
            .map(|title| LifecyclePreviewTitle {
                excluded: title.excluded,
                due_at: title.due_at,
                due_at_is_estimate: title.due_at_is_estimate,
                subject_kind: title.subject_kind.as_storage_str().to_string(),
                subject_id: ID::from(title.subject_id),
                subject_label: title.subject_label,
                file_count: to_graphql_int(title.file_count),
                total_size_bytes: title.total_size_bytes,
                title_id: ID::from(title.title_id),
                title_name: title.title_name,
                facet: title.facet.as_str().to_string(),
                library_id: title.library_id,
                outcome: title.outcome.map(lifecycle_outcome_value),
                reason_codes: title.reason_codes,
                error: title.error,
                error_code: title.error_code.map(str::to_string),
                actions: title.actions.as_ref().map(from_lifecycle_decided_actions),
                facts: title.facts.map(async_graphql::Json),
            })
            .collect(),
    }
}

// ── Scheduled evaluation ────────────────────────────────────────────────────

pub fn lifecycle_candidate_state_value(
    state: scryer_domain::LifecycleCandidateState,
) -> LifecycleCandidateState {
    match state {
        scryer_domain::LifecycleCandidateState::Observing => LifecycleCandidateState::Observing,
        scryer_domain::LifecycleCandidateState::PendingAction => {
            LifecycleCandidateState::PendingAction
        }
        scryer_domain::LifecycleCandidateState::Due => LifecycleCandidateState::Due,
        scryer_domain::LifecycleCandidateState::Executing => LifecycleCandidateState::Executing,
        scryer_domain::LifecycleCandidateState::Succeeded => LifecycleCandidateState::Succeeded,
        scryer_domain::LifecycleCandidateState::Failed => LifecycleCandidateState::Failed,
        scryer_domain::LifecycleCandidateState::Canceled => LifecycleCandidateState::Canceled,
        scryer_domain::LifecycleCandidateState::Excluded => LifecycleCandidateState::Excluded,
        scryer_domain::LifecycleCandidateState::Blocked => LifecycleCandidateState::Blocked,
    }
}

pub fn lifecycle_candidate_state_into_application(
    state: LifecycleCandidateState,
) -> scryer_domain::LifecycleCandidateState {
    match state {
        LifecycleCandidateState::Observing => scryer_domain::LifecycleCandidateState::Observing,
        LifecycleCandidateState::PendingAction => {
            scryer_domain::LifecycleCandidateState::PendingAction
        }
        LifecycleCandidateState::Due => scryer_domain::LifecycleCandidateState::Due,
        LifecycleCandidateState::Executing => scryer_domain::LifecycleCandidateState::Executing,
        LifecycleCandidateState::Succeeded => scryer_domain::LifecycleCandidateState::Succeeded,
        LifecycleCandidateState::Failed => scryer_domain::LifecycleCandidateState::Failed,
        LifecycleCandidateState::Canceled => scryer_domain::LifecycleCandidateState::Canceled,
        LifecycleCandidateState::Excluded => scryer_domain::LifecycleCandidateState::Excluded,
        LifecycleCandidateState::Blocked => scryer_domain::LifecycleCandidateState::Blocked,
    }
}

/// A candidate stores its action by the catalog's wire name. A name this build
/// does not know was written by a newer one; it projects as `DO_NOTHING`, the
/// only kind that authorizes nothing, rather than guessing at a destructive
/// action whose semantics this build does not have.
pub fn from_lifecycle_candidate(
    view: scryer_application::lifecycle_rules::LifecycleCandidateView,
) -> LifecycleCandidate {
    let scryer_application::lifecycle_rules::LifecycleCandidateView {
        subject_label,
        file_count,
        total_size_bytes,
        candidate,
        rule_name,
        title_name,
        action_sequence,
        sequence_steps,
    } = view;
    let action_sequence = action_sequence.as_ref().map(from_lifecycle_action_sequence);
    let actions = decided_actions_from_json(candidate.decided_actions_json.as_deref())
        .as_ref()
        .map(from_lifecycle_decided_actions);
    LifecycleCandidate {
        file_count: file_count.map(to_graphql_int),
        total_size_bytes,
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
        state: lifecycle_candidate_state_value(candidate.state),
        state_reason: candidate.state_reason,
        reason_codes: candidate.reason_codes,
        action_kind: if candidate.action_kind
            == scryer_application::lifecycle_rules::ACTION_SEQUENCE_KIND
        {
            LifecycleActionKind::ActionSequence
        } else {
            lifecycle_action_kind_value(
                AppActionKind::parse_wire_str(&candidate.action_kind)
                    .unwrap_or(AppActionKind::DoNothing),
            )
        },
        actions,
        action_sequence,
        sequence_steps: sequence_steps
            .iter()
            .map(from_lifecycle_action_step_progress)
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

pub fn from_lifecycle_evaluation_run(
    run: scryer_domain::LifecycleEvaluationRun,
) -> LifecycleEvaluationRun {
    LifecycleEvaluationRun {
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

pub fn from_lifecycle_instance_gates(
    gates: scryer_application::lifecycle_rules::LifecycleGates,
) -> LifecycleInstanceGates {
    LifecycleInstanceGates {
        enabled: gates.enabled(),
        actions_paused: gates.actions_paused(),
    }
}

pub fn from_lifecycle_exclusion(
    view: scryer_application::lifecycle_rules::LifecycleExclusionView,
) -> LifecycleExclusion {
    let exclusion = view.exclusion;
    LifecycleExclusion {
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

pub fn from_lifecycle_evaluation_trigger(
    trigger: scryer_application::lifecycle_rules::LifecycleEvaluationTrigger,
) -> LifecycleEvaluationTriggerPayload {
    LifecycleEvaluationTriggerPayload {
        started: trigger.started,
        message: trigger.message,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use scryer_application::lifecycle_rules::LifecycleActionStep as AppActionStep;
    use scryer_application::lifecycle_rules::action_execution::{
        LifecycleActionRunView, LifecycleActionStepProgressView,
    };
    use scryer_application::lifecycle_rules::evaluation::LifecycleCandidateView;
    use scryer_domain::{
        LifecycleActionJobReceipt, LifecycleActionJobReceiptState, LifecycleActionRun,
        LifecycleActionRunStatus, LifecycleActionStepKey, LifecycleActionStepRun,
        LifecycleActionStepState, LifecycleCandidate, LifecycleCandidateState,
    };

    #[test]
    fn sequence_projection_preserves_every_step() {
        let definition = AppActionSequence {
            schema_version: 2,
            steps: vec![AppActionStep {
                id: "search".into(),
                kind: AppActionStepKind::Search,
                parameters: AppActionStepParameters::Search {
                    condition: AppSearchCondition::PreviousProfileChanged,
                },
            }],
        };

        let sequence = from_lifecycle_action_sequence(&definition);
        assert_eq!(sequence.steps[0].kind, LifecycleActionStepKind::Search);
        assert_eq!(
            sequence.steps[0].parameters.search_condition,
            Some(LifecycleSearchCondition::PreviousProfileChanged)
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
        let search_key = LifecycleActionStepKey {
            candidate_id: "candidate-1".into(),
            match_generation: 3,
            revision_number: 2,
            step_id: search_step.id.clone(),
        };
        let search_run = LifecycleActionStepRun {
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
            state: LifecycleActionStepState::Succeeded,
            attempt: 1,
            lease_id: None,
            lease_expires_at: None,
            hold_reason: None,
            error: None,
            created_at: now,
            updated_at: now,
            finished_at: Some(now),
        };
        let receipt = LifecycleActionJobReceipt {
            schema_version: 1,
            key: search_key,
            dispatch_attempt: 1,
            logical_request_key: "maintenance-sequence:candidate-1:3:2:search".into(),
            request_hash: "request-hash".into(),
            job_run_id: Some("job-1".into()),
            state: LifecycleActionJobReceiptState::Accepted,
            reconciliation_evidence_json: "{}".into(),
            created_at: now,
            updated_at: now,
        };
        let view = LifecycleActionRunView {
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
            action_sequence: Some(AppActionSequence {
                schema_version: 2,
                steps: vec![profile_step, search_step],
            }),
            sequence_steps: vec![LifecycleActionStepProgressView {
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

        let projected = from_lifecycle_action_run(view).expect("known action kind projects");
        assert_eq!(projected.action_kind, LifecycleActionKind::ActionSequence);
        let sequence = projected.action_sequence.expect("sequence");
        assert_eq!(sequence.steps.len(), 2);
        assert_eq!(
            sequence
                .steps
                .iter()
                .map(|step| step.kind)
                .collect::<Vec<_>>(),
            vec![
                LifecycleActionStepKind::ChangeQualityProfile,
                LifecycleActionStepKind::Search,
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
    fn candidate_projection_preserves_empty_and_absent_action_sequences() {
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
            state: LifecycleCandidateState::Blocked,
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
        let sequence_view = LifecycleCandidateView {
            subject_label: "Example".into(),
            file_count: Some(1),
            total_size_bytes: Some(42),
            candidate: candidate.clone(),
            rule_name: "Rule".into(),
            title_name: "Example".into(),
            action_sequence: Some(AppActionSequence {
                schema_version: 2,
                steps: Vec::new(),
            }),
            sequence_steps: Vec::new(),
        };
        let projected = from_lifecycle_candidate(sequence_view);
        assert_eq!(projected.action_kind, LifecycleActionKind::ActionSequence);
        assert!(projected.action_sequence.is_some());
        assert!(projected.sequence_steps.is_empty());

        let legacy_view = LifecycleCandidateView {
            candidate: LifecycleCandidate {
                action_kind: AppActionKind::DoNothing.as_wire_str().into(),
                ..candidate
            },
            subject_label: "Example".into(),
            file_count: None,
            total_size_bytes: None,
            rule_name: "Rule".into(),
            title_name: "Example".into(),
            action_sequence: None,
            sequence_steps: Vec::new(),
        };
        let legacy = from_lifecycle_candidate(legacy_view);
        assert_eq!(legacy.action_kind, LifecycleActionKind::DoNothing);
        assert!(legacy.action_sequence.is_none());
        assert!(legacy.sequence_steps.is_empty());
    }

    #[test]
    fn decided_actions_project_in_execution_order_and_declared_ones_in_source_order() {
        let decided = from_lifecycle_decided_actions(&AppDecidedActions {
            delete_files: true,
            add_tags: vec!["stale".into()],
            set_quality_profile: Some("Compact 1080p".into()),
            search: true,
            ..AppDecidedActions::default()
        });
        assert_eq!(
            decided.heads,
            vec![
                LifecycleActionHead::AddTags,
                LifecycleActionHead::SetQualityProfile,
                LifecycleActionHead::Search,
                LifecycleActionHead::Unmonitor,
                LifecycleActionHead::DeleteFiles,
            ]
        );
        assert_eq!(decided.quality_profile.as_deref(), Some("Compact 1080p"));
        assert_eq!(decided.add_tags, vec!["stale".to_string()]);

        let scan = scryer_rules::lifecycle_actions::scan_lifecycle_action_heads(
            "package rules\nimport rego.v1\n\nsearch if {\n\ttrue\n}\n\ndelete_title if {\n\tfalse\n}\n",
            "rule-1",
        )
        .expect("source parses");
        let declared = from_lifecycle_declared_actions(&AppDeclaredActions::from_scan(&scan));
        assert_eq!(
            declared.heads,
            vec![
                LifecycleActionHead::Search,
                LifecycleActionHead::DeleteTitle
            ]
        );
        assert!(declared.destructive);
    }
}
