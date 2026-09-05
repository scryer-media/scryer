//! Projections for the maintenance-rule authoring surface (RFC 137).
//!
//! The application layer owns the closed action catalog; this module only
//! renames its values onto the wire. Every string projected here (effect
//! classes, timing mode, repeat modes) uses the same snake_case wire name the
//! stored action spec serializes to, so a descriptor and a stored revision
//! never disagree about what an action is called.

use super::*;

use scryer_application::maintenance_rules::{
    MaintenanceActionDescriptor as AppActionDescriptor, MaintenanceActionKind as AppActionKind,
    MaintenanceActionParameters as AppActionParameters, MaintenanceActionSpec as AppActionSpec,
    MaintenanceEffectClass as AppEffectClass, MaintenanceRepeatMode as AppRepeatMode,
    MaintenanceRiskClass as AppRiskClass, MaintenanceSubjectKind as AppSubjectKind,
    MaintenanceTimingMode as AppTimingMode, action_catalog,
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

pub fn maintenance_rule_subject_kind_value(
    kind: scryer_domain::MaintenanceRuleSubjectKind,
) -> MaintenanceRuleSubjectKind {
    match kind {
        scryer_domain::MaintenanceRuleSubjectKind::Title => MaintenanceRuleSubjectKind::Title,
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
    let run = view.run;
    let kind = AppActionKind::parse_wire_str(&run.action_kind)?;
    Some(MaintenanceActionRun {
        id: ID::from(run.id),
        rule_set_id: ID::from(run.rule_set_id),
        candidate_id: ID::from(run.candidate_id),
        title_id: ID::from(run.title_id),
        title_name: view.title_name,
        action_kind: maintenance_action_kind_value(kind),
        match_generation: to_graphql_int(run.match_generation),
        attempt: to_graphql_int(run.attempt),
        status: run.status.as_storage_str().to_string(),
        hold_reason: run.hold_reason,
        error: run.error,
        started_at: run.started_at,
        finished_at: run.finished_at,
    })
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
    MaintenanceRuleSet {
        id: ID::from(rule_set.id.clone()),
        name: rule_set.name.clone(),
        description: rule_set.description.clone(),
        enabled: rule_set.enabled,
        evaluation_mode: maintenance_evaluation_mode_value(rule_set.evaluation_mode),
        effect_arming: maintenance_effect_arming_value(rule_set.effect_arming),
        library_ids: rule_set.library_ids.clone(),
        subject_kind: maintenance_rule_subject_kind_value(rule_set.subject_kind),
        current_revision_number: to_graphql_int(rule_set.current_revision_number),
        grace_days: to_graphql_int(detail.revision.grace_days),
        action_spec: from_maintenance_action_spec(&detail.action_spec),
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

pub fn from_maintenance_rule_set_detail(detail: AppRuleSetDetail) -> MaintenanceRuleSetDetail {
    let rule_set = from_maintenance_rule_set(&detail);
    MaintenanceRuleSetDetail {
        rule_set,
        revision: from_maintenance_rule_revision(detail.revision),
        action_spec: from_maintenance_action_spec(&detail.action_spec),
    }
}

fn from_maintenance_action_descriptor(
    descriptor: &AppActionDescriptor,
) -> MaintenanceActionDescriptor {
    MaintenanceActionDescriptor {
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
    }
}

/// The static action registry, in catalog order.
pub fn maintenance_action_descriptors() -> Vec<MaintenanceActionDescriptor> {
    action_catalog()
        .iter()
        .map(from_maintenance_action_descriptor)
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
    let candidate = view.candidate;
    MaintenanceCandidate {
        id: ID::from(candidate.id),
        rule_set_id: ID::from(candidate.rule_set_id),
        rule_name: view.rule_name,
        revision_number: to_graphql_int(candidate.revision_number),
        title_id: ID::from(candidate.title_id),
        title_name: view.title_name,
        library_id: candidate.library_id,
        facet: candidate.facet,
        state: maintenance_candidate_state_value(candidate.state),
        state_reason: candidate.state_reason,
        reason_codes: candidate.reason_codes,
        action_kind: maintenance_action_kind_value(
            AppActionKind::parse_wire_str(&candidate.action_kind)
                .unwrap_or(AppActionKind::DoNothing),
        ),
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
