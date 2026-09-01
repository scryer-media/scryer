//! Projections between the location subsystem's application types and the
//! GraphQL location surface (US2, FR-080 to FR-083).
//!
//! Nothing here derives a fact the application did not state. An unprobed
//! free-space estimate stays unknown, a plan section's sample stays a sample
//! beside its complete count, and paths are converted from their stored
//! encoding to the real filesystem spelling on the way out.

use async_graphql::ID;
use scryer_application::location::asset_listing::{
    DeduplicatedAsset, LocationOperationAssetListing, RenamedAsset, TitleAssetListing,
};
use scryer_application::location::classify::{
    DestinationRequest, SelectionClassification, TitleClassification, TitleLocationClass,
};
use scryer_application::location::consolidation_execution::RootConsolidationPreview;
use scryer_application::location::identity::{DestinationIdentityOutcome, MetadataIdentity};
use scryer_application::location::merge::map::{MergeBlockReason};
use scryer_application::location::merge::roles::RoleChangeReason;
use scryer_application::location::merge::summary::{MergePreviewSummary, PostMergeWork};
use scryer_application::location::merge::{
    DestinationIdentityMatch, MergeDisposition, MergedMediaRole,
};
use scryer_application::location::model::{
    LocationExecutionMode, LocationOperation, LocationOperationCounters, LocationOperationState,
    LocationOperationType, TitleCheckpoint, TitleCheckpointState, VerificationDepth,
};
use scryer_application::location::operations::RootMovePreview;
use scryer_application::location::preview::{
    ConfirmationRequirement, FreeSpaceEstimate, LocationPlan, PLAN_SECTION_SAMPLE_LIMIT,
    PlanConfirmation, PlanCounts, PlanFingerprint, PlanItem, PlanItemKind, PlanSection,
    VerificationStatement,
};
use scryer_application::location::root_change::{
    BlockedTitle, ClassifiedRootEntry, RootContentClass, RootContentInventory,
    RootIdentityRetention, RootRetirementContract, TitleAccounting,
};
use scryer_application::location::root_change_execution::RootChangePreview;
use scryer_application::location::transfer_effects::{
    FILES_KEEP_THEIR_NAMES, FacetConversion, SettingDisposition,
};

use crate::types::{
    CancelLocationOperationPayload, LocationAmbiguousDestinationCandidatePayload,
    LocationBlockedTitlePayload, LocationClassificationGroupPayload,
    LocationClassifiedTitlePayload, LocationConfirmationRequirementValue,
    LocationConsolidationClassificationPayload, LocationDefaultRootTransferPayload,
    LocationDestinationIdentityMatchValue, LocationDestinationInput, LocationExecutionModeInput,
    LocationExecutionModeValue, LocationFacetConversionPayload,
    LocationFacetConvertedSettingPayload, LocationFacetSettingDispositionValue,
    LocationFreeSpaceEstimatePayload, LocationMergeBlockReasonValue,
    LocationMergeBlockedRecordPayload, LocationMergeDestinationWinsPayload,
    LocationMergeDispositionValue, LocationMergeDroppedCategoryPayload,
    LocationMergeMediaRequestRepointPayload, LocationMergeMediaRoleValue,
    LocationMergePostMergeWorkValue, LocationMergePreviewPayload,
    LocationMergeReservedTagConflictPayload, LocationMergeRoleChangePayload,
    LocationMergeRoleChangeReasonValue, LocationMergeTableDispositionPayload,
    LocationOperationAssetListingPayload, LocationOperationCountersPayload,
    LocationOperationDeduplicatedAssetPayload, LocationOperationPayload,
    LocationOperationPreviewPayload, LocationOperationRenamedAssetPayload,
    LocationOperationStateValue, LocationOperationTitleAssetsPayload, LocationOperationTypeValue,
    LocationPlanConfirmationPayload, LocationPlanCountsPayload, LocationPlanItemKindValue,
    LocationPlanItemPayload, LocationPlanKindCountPayload, LocationPlanSectionPayload,
    LocationRootChangePreviewPayload, LocationRootConsolidationPreviewPayload,
    LocationRootContentBucketPayload, LocationRootContentClassValue,
    LocationRootContentEntryPayload, LocationRootContentInventoryPayload,
    LocationRootIdentityRetentionPayload, LocationRootRetirementBlockerPayload,
    LocationRootRetirementContractPayload, LocationSampledPathsPayload,
    LocationSelectionClassificationPayload, LocationTitleAccountingPayload,
    LocationTitleCheckpointPayload, LocationTitleCheckpointStateValue,
    LocationVerificationStatementPayload, Long, MediaFacetValue, ResumeLocationOperationPayload,
    StartLocationOperationPayload, TitleLocationClassValue, VerificationDepthValue,
};

/// Plan-item kinds in the order the preview presents them; the same order the
/// plan builder emits its sections in.
const PLAN_ITEM_KINDS: [PlanItemKind; 10] = [
    PlanItemKind::Move,
    PlanItemKind::Rename,
    PlanItemKind::Merge,
    PlanItemKind::Dedup,
    PlanItemKind::CatalogChange,
    PlanItemKind::RoleChange,
    PlanItemKind::NoOp,
    PlanItemKind::Blocked,
    PlanItemKind::UnmanagedContent,
    PlanItemKind::Warning,
];

/// Every class a selection is grouped into, so the preview can present all six
/// groups whether or not the selection populated them (FR-015).
const TITLE_LOCATION_CLASSES: [TitleLocationClass; 6] = [
    TitleLocationClass::CrossLibraryTransfer,
    TitleLocationClass::RootMove,
    TitleLocationClass::NoOp,
    TitleLocationClass::CatalogOnly,
    TitleLocationClass::Incompatible,
    TitleLocationClass::NeedsResolution,
];

/// Stored paths carry an internal escape form for names the platform cannot
/// spell in UTF-8; the API always hands back the real path.
fn display_path(path: &str) -> String {
    scryer_application::stored_paths::stored_path_to_display_string(path)
}

/// Byte counts are unsigned in the planner and signed on the wire; a value that
/// cannot be represented is clamped rather than wrapped into a negative size.
fn bytes(value: u64) -> Long {
    Long::from_u64_saturating(value)
}

pub fn from_location_operation_type(value: LocationOperationType) -> LocationOperationTypeValue {
    match value {
        LocationOperationType::FolderReassignment => LocationOperationTypeValue::FolderReassignment,
        LocationOperationType::RootMove => LocationOperationTypeValue::RootMove,
        LocationOperationType::RootChange => LocationOperationTypeValue::RootChange,
        LocationOperationType::RootConsolidation => LocationOperationTypeValue::RootConsolidation,
        LocationOperationType::CrossLibraryTransfer => {
            LocationOperationTypeValue::CrossLibraryTransfer
        }
        LocationOperationType::Adoption => LocationOperationTypeValue::Adoption,
    }
}

pub fn from_location_execution_mode(value: LocationExecutionMode) -> LocationExecutionModeValue {
    match value {
        LocationExecutionMode::MoveWithScryer => LocationExecutionModeValue::MoveWithScryer,
        LocationExecutionMode::FilesAlreadyThere => LocationExecutionModeValue::FilesAlreadyThere,
        LocationExecutionMode::CatalogOnly => LocationExecutionModeValue::CatalogOnly,
    }
}

pub fn from_location_operation_state(value: LocationOperationState) -> LocationOperationStateValue {
    match value {
        LocationOperationState::Queued => LocationOperationStateValue::Queued,
        LocationOperationState::Preparing => LocationOperationStateValue::Preparing,
        LocationOperationState::Moving => LocationOperationStateValue::Moving,
        LocationOperationState::Verifying => LocationOperationStateValue::Verifying,
        LocationOperationState::Reconciling => LocationOperationStateValue::Reconciling,
        LocationOperationState::CleaningUp => LocationOperationStateValue::CleaningUp,
        LocationOperationState::Completed => LocationOperationStateValue::Completed,
        LocationOperationState::CompletedWithWarnings => {
            LocationOperationStateValue::CompletedWithWarnings
        }
        LocationOperationState::Canceled => LocationOperationStateValue::Canceled,
        LocationOperationState::Failed => LocationOperationStateValue::Failed,
    }
}

fn from_title_location_class(value: TitleLocationClass) -> TitleLocationClassValue {
    match value {
        TitleLocationClass::CrossLibraryTransfer => TitleLocationClassValue::CrossLibraryTransfer,
        TitleLocationClass::RootMove => TitleLocationClassValue::RootMove,
        TitleLocationClass::NoOp => TitleLocationClassValue::NoOp,
        TitleLocationClass::CatalogOnly => TitleLocationClassValue::CatalogOnly,
        TitleLocationClass::Incompatible => TitleLocationClassValue::Incompatible,
        TitleLocationClass::NeedsResolution => TitleLocationClassValue::NeedsResolution,
    }
}

fn from_plan_item_kind(value: PlanItemKind) -> LocationPlanItemKindValue {
    match value {
        PlanItemKind::Move => LocationPlanItemKindValue::Move,
        PlanItemKind::Rename => LocationPlanItemKindValue::Rename,
        PlanItemKind::Merge => LocationPlanItemKindValue::Merge,
        PlanItemKind::Dedup => LocationPlanItemKindValue::Dedup,
        PlanItemKind::CatalogChange => LocationPlanItemKindValue::CatalogChange,
        PlanItemKind::RoleChange => LocationPlanItemKindValue::RoleChange,
        PlanItemKind::NoOp => LocationPlanItemKindValue::NoOp,
        PlanItemKind::Blocked => LocationPlanItemKindValue::Blocked,
        PlanItemKind::UnmanagedContent => LocationPlanItemKindValue::UnmanagedContent,
        PlanItemKind::Warning => LocationPlanItemKindValue::Warning,
    }
}

fn from_title_checkpoint_state(value: TitleCheckpointState) -> LocationTitleCheckpointStateValue {
    match value {
        TitleCheckpointState::Pending => LocationTitleCheckpointStateValue::Pending,
        TitleCheckpointState::Moving => LocationTitleCheckpointStateValue::Moving,
        TitleCheckpointState::Verifying => LocationTitleCheckpointStateValue::Verifying,
        TitleCheckpointState::Reconciling => LocationTitleCheckpointStateValue::Reconciling,
        TitleCheckpointState::CleaningUp => LocationTitleCheckpointStateValue::CleaningUp,
        TitleCheckpointState::Completed => LocationTitleCheckpointStateValue::Completed,
        TitleCheckpointState::CompletedWithWarnings => {
            LocationTitleCheckpointStateValue::CompletedWithWarnings
        }
        TitleCheckpointState::Skipped => LocationTitleCheckpointStateValue::Skipped,
        TitleCheckpointState::Blocked => LocationTitleCheckpointStateValue::Blocked,
        TitleCheckpointState::Failed => LocationTitleCheckpointStateValue::Failed,
    }
}

fn from_verification_depth(value: VerificationDepth) -> VerificationDepthValue {
    match value {
        VerificationDepth::Full => VerificationDepthValue::Full,
        VerificationDepth::Quick => VerificationDepthValue::Quick,
    }
}

fn from_confirmation_requirement(
    value: ConfirmationRequirement,
) -> LocationConfirmationRequirementValue {
    match value {
        ConfirmationRequirement::Simple => LocationConfirmationRequirementValue::Simple,
        ConfirmationRequirement::Typed => LocationConfirmationRequirementValue::Typed,
    }
}

/// The destination a client asked for. Both fields stay optional: the classifier
/// is what decides where a title with no explicit root ends up.
/// The execution mode a request asked for.
///
/// Omitting the field is the managed move, so a client written before adoption
/// existed keeps the behavior it already had. `CATALOG_ONLY` is not in the
/// input enum at all: it is the server's own conclusion about a fileless
/// selection (FR-076), never something a caller may claim.
pub fn location_execution_mode_into_application(
    input: Option<LocationExecutionModeInput>,
) -> LocationExecutionMode {
    match input {
        Some(LocationExecutionModeInput::FilesAlreadyThere) => {
            LocationExecutionMode::FilesAlreadyThere
        }
        Some(LocationExecutionModeInput::MoveWithScryer) | None => {
            LocationExecutionMode::MoveWithScryer
        }
    }
}

// The root-scoped workflows once had a second mapper here that refused
// `FILES_ALREADY_THERE` with an untranslatable interface sentence. Both
// planners now refuse it by name — `root_change::refusal_codes::
// MODE_NOT_SUPPORTED` and `consolidation::refusal_codes::MODE_NOT_SUPPORTED` —
// so the request travels through the shared mapper above and the refusal is
// application vocabulary the client can route and translate. `CATALOG_ONLY`
// stays unrequestable everywhere: it is the server's own conclusion about a
// root with no files on it (FR-076).

pub fn location_destination_into_application(
    input: LocationDestinationInput,
) -> DestinationRequest {
    DestinationRequest {
        library_id: input.library_id.map(|id| id.to_string()),
        root_id: input.root_id.map(|id| id.to_string()),
    }
}

fn from_plan_item(item: &PlanItem) -> LocationPlanItemPayload {
    LocationPlanItemPayload {
        kind: from_plan_item_kind(item.kind),
        title_id: item.title_id.clone().map(ID::from),
        media_file_id: item.media_file_id.clone().map(ID::from),
        source_path: item.source_path.as_deref().map(display_path),
        destination_path: item.destination_path.as_deref().map(display_path),
        size_bytes: bytes(item.size_bytes),
        same_volume: item.same_volume,
        reason_code: item.reason_code.clone(),
        detail: item.detail.clone(),
    }
}

fn from_plan_section(section: &PlanSection) -> LocationPlanSectionPayload {
    LocationPlanSectionPayload {
        kind: from_plan_item_kind(section.kind),
        items_total: Long(section.items.total),
        bytes_total: Long(section.bytes_total),
        complete: section.items.is_complete(),
        items: section.items.items.iter().map(from_plan_item).collect(),
    }
}

fn from_plan_counts(counts: &PlanCounts) -> LocationPlanCountsPayload {
    LocationPlanCountsPayload {
        items_total: Long(counts.items_total),
        titles_total: Long(counts.titles_total),
        files_total: Long(counts.files_total),
        bytes_total: Long(counts.bytes_total),
        by_kind: PLAN_ITEM_KINDS
            .iter()
            .map(|kind| LocationPlanKindCountPayload {
                kind: from_plan_item_kind(*kind),
                count: Long(counts.for_kind(*kind)),
            })
            .collect(),
    }
}

fn from_classified_title(title: &TitleClassification) -> LocationClassifiedTitlePayload {
    let identity = title.destination_identity.as_ref();
    LocationClassifiedTitlePayload {
        title_id: ID::from(title.title_id.clone()),
        class: from_title_location_class(title.class),
        source_library_id: ID::from(title.source_library_id.clone()),
        source_root_id: ID::from(title.source_root_id.clone()),
        source_folder_path: title.source_folder_path.as_deref().map(display_path),
        destination_library_id: ID::from(title.destination_library_id.clone()),
        destination_root_id: ID::from(title.destination_root_id.clone()),
        reason_code: title.reason_code.clone(),
        reason: title.reason.clone(),
        blocks_start: title.blocks_start(),
        destination_identity_match: identity
            .map(|outcome| from_destination_identity_match(outcome.match_kind)),
        merge_target_title_id: title
            .merge_target_title_id()
            .map(|id| ID::from(id.to_string())),
        // US7: detection already named the title that survives, so the merge
        // statement reads "merges into “X”" instead of quoting an id. Sourced
        // from the same outcome as the id beside it — the two can never disagree.
        merge_target_title_name: title
            .merge_target_title_name()
            .map(ToString::to_string),
        same_named_destination_title_id: title
            .same_named_destination_title_id()
            .map(|id| ID::from(id.to_string())),
        same_named_destination_title_name: identity
            .and_then(|outcome| outcome.same_name_title_name.clone()),
        ambiguous_destination_title_ids: identity
            .map(DestinationIdentityOutcome::ambiguous_title_ids)
            .unwrap_or_default()
            .into_iter()
            .map(ID::from)
            .collect(),
        // FR-055: the ids alone cannot be chosen between. The candidates carry
        // the name the user reads and the identities that put each one on the
        // list, and they are emitted only for an outcome that actually needs
        // resolving — `ambiguous_title_ids` is empty for every other one, so
        // the two lists always agree.
        ambiguous_destination_candidates: identity
            .filter(|outcome| outcome.needs_resolution())
            .map(|outcome| {
                outcome
                    .candidates
                    .iter()
                    .map(|candidate| LocationAmbiguousDestinationCandidatePayload {
                        title_id: ID::from(candidate.title_id.clone()),
                        title_name: candidate.title_name.clone(),
                        shared_identities: candidate
                            .shared_identities
                            .iter()
                            .map(MetadataIdentity::display)
                            .collect(),
                    })
                    .collect()
            })
            .unwrap_or_default(),
        facet_conversion: title
            .facet_conversion
            .as_ref()
            .map(from_facet_conversion),
    }
}

/// FR-057/FR-058: the conversion, its affected settings, and the sentence that
/// says file names are not part of it.
fn from_facet_conversion(conversion: &FacetConversion) -> LocationFacetConversionPayload {
    LocationFacetConversionPayload {
        from_facet: MediaFacetValue::from_domain(conversion.from.clone()),
        to_facet: MediaFacetValue::from_domain(conversion.to.clone()),
        settings: conversion
            .settings
            .iter()
            .map(|setting| LocationFacetConvertedSettingPayload {
                setting: setting.setting.clone(),
                label: setting.label.clone(),
                value: setting.value.clone(),
                disposition: from_setting_disposition(setting.disposition),
                detail: setting.detail.clone(),
            })
            .collect(),
        files_keep_their_names: FILES_KEEP_THEIR_NAMES.to_string(),
    }
}

fn from_setting_disposition(
    value: SettingDisposition,
) -> LocationFacetSettingDispositionValue {
    match value {
        SettingDisposition::BecomesInvalid => {
            LocationFacetSettingDispositionValue::BecomesInvalid
        }
        SettingDisposition::Resets => LocationFacetSettingDispositionValue::Resets,
        SettingDisposition::ChangesMeaning => {
            LocationFacetSettingDispositionValue::ChangesMeaning
        }
    }
}

fn from_destination_identity_match(
    value: DestinationIdentityMatch,
) -> LocationDestinationIdentityMatchValue {
    match value {
        DestinationIdentityMatch::Unique => LocationDestinationIdentityMatchValue::Unique,
        DestinationIdentityMatch::None => LocationDestinationIdentityMatchValue::None,
        DestinationIdentityMatch::Ambiguous => LocationDestinationIdentityMatchValue::Ambiguous,
        DestinationIdentityMatch::SameNameNoIdentity => {
            LocationDestinationIdentityMatchValue::SameNameNoIdentity
        }
    }
}

/// Groups the selection into all six classes, in a stable order, omitting no
/// selected title (FR-015, SC-005).
fn from_selection_classification(
    classification: &SelectionClassification,
) -> LocationSelectionClassificationPayload {
    let groups = TITLE_LOCATION_CLASSES
        .iter()
        .map(|class| {
            let titles: Vec<LocationClassifiedTitlePayload> = classification
                .titles
                .iter()
                .filter(|title| title.class == *class)
                .map(from_classified_title)
                .collect();
            LocationClassificationGroupPayload {
                class: from_title_location_class(*class),
                count: Long(titles.len() as i64),
                titles,
            }
        })
        .collect();
    LocationSelectionClassificationPayload {
        groups,
        titles_total: Long(classification.titles.len() as i64),
        blocks_start: classification.blocks_start(),
    }
}

fn from_free_space_estimate(estimate: &FreeSpaceEstimate) -> LocationFreeSpaceEstimatePayload {
    LocationFreeSpaceEstimatePayload {
        destination_required_bytes: bytes(estimate.destination_required_bytes),
        destination_total_required_bytes: bytes(estimate.destination_total_required_bytes()),
        destination_available_bytes: estimate.destination_available_bytes.map(bytes),
        recycle_required_bytes: bytes(estimate.recycle_required_bytes),
        recycle_available_bytes: estimate.recycle_available_bytes.map(bytes),
        same_volume_move: estimate.same_volume_move,
        recycle_on_other_volume: estimate.recycle_on_other_volume,
        recycle_shares_destination_volume: estimate.recycle_shares_destination_volume,
        recycling_available: estimate.recycling_available,
        probed: estimate.probed,
        sufficient: estimate.sufficient(),
    }
}

fn from_verification_statement(
    statement: &VerificationStatement,
) -> LocationVerificationStatementPayload {
    LocationVerificationStatementPayload {
        depth: from_verification_depth(statement.depth),
        files: Long(statement.files),
        bytes: Long(statement.bytes),
        applies: statement.applies(),
    }
}

fn from_plan_confirmation(confirmation: &PlanConfirmation) -> LocationPlanConfirmationPayload {
    LocationPlanConfirmationPayload {
        requirement: from_confirmation_requirement(confirmation.requirement),
        typed_phrase: confirmation.typed_phrase.clone(),
        typed_prompt: confirmation.typed_prompt.clone(),
    }
}

fn from_location_plan(
    plan: &LocationPlan,
    classification: LocationSelectionClassificationPayload,
    warnings: Vec<String>,
) -> LocationOperationPreviewPayload {
    LocationOperationPreviewPayload {
        plan_fingerprint: plan.fingerprint.0.clone(),
        operation_type: from_location_operation_type(plan.header.operation_type),
        mode: from_location_execution_mode(plan.header.mode),
        source_library_id: plan.header.source_library_id.clone().map(ID::from),
        destination_library_id: plan.header.destination_library_id.clone().map(ID::from),
        source_root_id: plan.header.source_root_id.clone().map(ID::from),
        destination_root_id: plan.header.destination_root_id.clone().map(ID::from),
        selection: plan
            .header
            .selection
            .iter()
            .cloned()
            .map(ID::from)
            .collect(),
        counts: from_plan_counts(&plan.counts),
        sections: plan.sections.iter().map(from_plan_section).collect(),
        // The plan carries per-class counts only; the grouped per-title entries
        // come from the classification the preview returns beside it.
        classification,
        free_space: from_free_space_estimate(&plan.free_space),
        verification: from_verification_statement(&plan.verification),
        confirmation: from_plan_confirmation(&plan.confirmation),
        warnings,
        blocks_start: plan.blocks_start(),
        merges: plan.merges.iter().map(from_merge_summary).collect(),
    }
}

// ── US7: the merge preview (FR-071) ─────────────────────────────────────────

fn from_merge_summary(summary: &MergePreviewSummary) -> LocationMergePreviewPayload {
    LocationMergePreviewPayload {
        source_title_id: ID::from(summary.source_title_id.clone()),
        destination_title_id: ID::from(summary.destination_title_id.clone()),
        destination_title_name: summary.destination_title_name.clone(),
        source_library_id: summary.source_library_id.clone().map(ID::from),
        destination_library_id: summary.destination_library_id.clone().map(ID::from),
        blocked: summary.is_blocked(),
        blocked_records: summary
            .blocked
            .iter()
            .map(|record| LocationMergeBlockedRecordPayload {
                table: record.table.clone(),
                reason: from_merge_block_reason(record.reason),
                source_id: ID::from(record.source_id.clone()),
                detail: record.detail.clone(),
            })
            .collect(),
        destination_wins: summary
            .destination_wins
            .iter()
            .map(|entry| LocationMergeDestinationWinsPayload {
                setting: entry.setting.clone(),
                destination_value: entry.destination_value.clone(),
                source_value: entry.source_value.clone(),
            })
            .collect(),
        dispositions: summary
            .dispositions
            .iter()
            .map(|entry| LocationMergeTableDispositionPayload {
                table: entry.table.clone(),
                disposition: from_merge_disposition(entry.disposition),
                source_row_count: Long(entry.source_row_count),
                note: entry.note.clone(),
            })
            .collect(),
        role_changes: summary
            .role_changes
            .iter()
            .map(|change| LocationMergeRoleChangePayload {
                file_id: ID::from(change.file_id.clone()),
                source_episode_id: ID::from(change.source_episode_id.clone()),
                destination_episode_id: ID::from(change.destination_episode_id.clone()),
                previous_role: from_merged_media_role(change.previous_role),
                new_role: from_merged_media_role(change.new_role),
                reason: from_role_change_reason(change.reason),
                // Phrased by the application, not here: the plan item and this
                // payload have to say the same thing about the same demotion.
                detail: change.describe(),
            })
            .collect(),
        reserved_tag_conflicts: summary
            .reserved_tag_conflicts
            .iter()
            .map(|conflict| LocationMergeReservedTagConflictPayload {
                prefix: conflict.prefix.clone(),
                setting: conflict.setting.clone(),
                destination_value: conflict.destination_value.clone(),
                source_value: conflict.source_value.clone(),
            })
            .collect(),
        free_form_tags_added: summary.free_form_tags_added.clone(),
        media_request_repoints: summary
            .media_request_repoints
            .iter()
            .map(|repoint| LocationMergeMediaRequestRepointPayload {
                request_id: ID::from(repoint.request_id.clone()),
                previous_library_id: ID::from(repoint.previous_library_id.clone()),
                destination_library_id: ID::from(repoint.destination_library_id.clone()),
            })
            .collect(),
        dropped: summary
            .dropped
            .iter()
            .map(|dropped| LocationMergeDroppedCategoryPayload {
                table: dropped.table.clone(),
                source_row_count: Long(dropped.source_row_count),
                decision: dropped.decision.clone(),
                reason: dropped.reason.clone(),
            })
            .collect(),
        post_merge_work: summary
            .post_merge_work
            .iter()
            .map(|work| from_post_merge_work(*work))
            .collect(),
        notes: summary.notes.clone(),
    }
}

fn from_merge_disposition(value: MergeDisposition) -> LocationMergeDispositionValue {
    match value {
        MergeDisposition::Union => LocationMergeDispositionValue::Union,
        MergeDisposition::Map => LocationMergeDispositionValue::Map,
        MergeDisposition::DestinationWins => LocationMergeDispositionValue::DestinationWins,
        MergeDisposition::Drop => LocationMergeDispositionValue::Drop,
    }
}

fn from_merged_media_role(value: MergedMediaRole) -> LocationMergeMediaRoleValue {
    match value {
        MergedMediaRole::Primary => LocationMergeMediaRoleValue::Primary,
        MergedMediaRole::Additional => LocationMergeMediaRoleValue::Additional,
    }
}

fn from_role_change_reason(value: RoleChangeReason) -> LocationMergeRoleChangeReasonValue {
    match value {
        RoleChangeReason::DestinationPrimaryRetained => {
            LocationMergeRoleChangeReasonValue::DestinationPrimaryRetained
        }
        RoleChangeReason::SourcePrimaryAlreadyClaimed => {
            LocationMergeRoleChangeReasonValue::SourcePrimaryAlreadyClaimed
        }
        RoleChangeReason::CollapsedSourceEpisodes => {
            LocationMergeRoleChangeReasonValue::CollapsedSourceEpisodes
        }
    }
}

fn from_post_merge_work(value: PostMergeWork) -> LocationMergePostMergeWorkValue {
    match value {
        PostMergeWork::ReindexTitleSearchTerms => {
            LocationMergePostMergeWorkValue::ReindexTitleSearchTerms
        }
        PostMergeWork::RegenerateRecommendations => {
            LocationMergePostMergeWorkValue::RegenerateRecommendations
        }
        PostMergeWork::RecomputeStatistics => LocationMergePostMergeWorkValue::RecomputeStatistics,
        PostMergeWork::DropSourceIndexerCoverage => {
            LocationMergePostMergeWorkValue::DropSourceIndexerCoverage
        }
    }
}

fn from_merge_block_reason(value: MergeBlockReason) -> LocationMergeBlockReasonValue {
    match value {
        MergeBlockReason::UnmappedEpisode => LocationMergeBlockReasonValue::UnmappedEpisode,
        MergeBlockReason::AmbiguousDestinationEpisode => {
            LocationMergeBlockReasonValue::AmbiguousDestinationEpisode
        }
        MergeBlockReason::AmbiguousSourceEpisode => {
            LocationMergeBlockReasonValue::AmbiguousSourceEpisode
        }
        MergeBlockReason::UnidentifiableEpisode => {
            LocationMergeBlockReasonValue::UnidentifiableEpisode
        }
        MergeBlockReason::UnknownEpisodeReference => {
            LocationMergeBlockReasonValue::UnknownEpisodeReference
        }
        MergeBlockReason::UnmappedCollection => LocationMergeBlockReasonValue::UnmappedCollection,
        MergeBlockReason::AmbiguousDestinationCollection => {
            LocationMergeBlockReasonValue::AmbiguousDestinationCollection
        }
        MergeBlockReason::AmbiguousSourceCollection => {
            LocationMergeBlockReasonValue::AmbiguousSourceCollection
        }
        MergeBlockReason::UnmappedSeriesMovieLink => {
            LocationMergeBlockReasonValue::UnmappedSeriesMovieLink
        }
        MergeBlockReason::AmbiguousDestinationSeriesMovieLink => {
            LocationMergeBlockReasonValue::AmbiguousDestinationSeriesMovieLink
        }
        MergeBlockReason::AmbiguousSourceSeriesMovieLink => {
            LocationMergeBlockReasonValue::AmbiguousSourceSeriesMovieLink
        }
        MergeBlockReason::ResumableOperationHoldsSource => {
            LocationMergeBlockReasonValue::ResumableOperationHoldsSource
        }
        MergeBlockReason::ActiveManualImportSelection => {
            LocationMergeBlockReasonValue::ActiveManualImportSelection
        }
    }
}

/// The complete root-move preview: the fingerprinted plan plus the grouped
/// classification the plan only counts (FR-015, FR-080, FR-081).
pub fn from_root_move_preview(preview: &RootMovePreview) -> LocationOperationPreviewPayload {
    from_location_plan(
        &preview.plan,
        from_selection_classification(&preview.classification),
        preview.warnings.clone(),
    )
}

// ── US4 and US5: the two root-scoped previews (FR-020 to FR-029) ─────────────

/// The six classification groups for a plan that has no title selection.
///
/// A root-scoped operation covers every title assigned to the root and offers
/// no way to exclude one, so there is no per-title selection to group: the plan
/// carries per-class counts and nothing else. The groups are still all six, in
/// the same order, so a client renders the same control it renders for a move;
/// the titles that need naming come from the accounting ledger instead
/// (FR-023).
fn from_classification_counts(
    counts: &scryer_application::location::classify::ClassificationCounts,
) -> LocationSelectionClassificationPayload {
    let count_for = |class: TitleLocationClass| match class {
        TitleLocationClass::CrossLibraryTransfer => counts.cross_library_transfer,
        TitleLocationClass::RootMove => counts.root_move,
        TitleLocationClass::NoOp => counts.no_op,
        TitleLocationClass::CatalogOnly => counts.catalog_only,
        TitleLocationClass::Incompatible => counts.incompatible,
        TitleLocationClass::NeedsResolution => counts.needs_resolution,
    };
    LocationSelectionClassificationPayload {
        groups: TITLE_LOCATION_CLASSES
            .iter()
            .map(|class| LocationClassificationGroupPayload {
                class: from_title_location_class(*class),
                count: Long(count_for(*class)),
                titles: Vec::new(),
            })
            .collect(),
        titles_total: Long(counts.total()),
        blocks_start: counts.blocks_start(),
    }
}

fn from_root_content_class(value: RootContentClass) -> LocationRootContentClassValue {
    match value {
        RootContentClass::Managed => LocationRootContentClassValue::Managed,
        RootContentClass::Companion => LocationRootContentClassValue::Companion,
        RootContentClass::Unknown => LocationRootContentClassValue::Unknown,
    }
}

fn from_root_content_entry(entry: &ClassifiedRootEntry) -> LocationRootContentEntryPayload {
    LocationRootContentEntryPayload {
        path: display_path(&entry.path),
        size_bytes: bytes(entry.size_bytes),
        class: from_root_content_class(entry.class),
        canonical_sidecar: entry.canonical_sidecar,
    }
}

/// One content bucket, sampled the way plan sections are: the complete count
/// and byte total always, the entries only up to the shared sample limit.
fn from_root_content_bucket(
    class: RootContentClass,
    entries: &[ClassifiedRootEntry],
) -> LocationRootContentBucketPayload {
    let bytes_total = entries
        .iter()
        .fold(0_u64, |total, entry| total.saturating_add(entry.size_bytes));
    LocationRootContentBucketPayload {
        class: from_root_content_class(class),
        total: Long(entries.len() as i64),
        bytes_total: bytes(bytes_total),
        complete: entries.len() <= PLAN_SECTION_SAMPLE_LIMIT,
        entries: entries
            .iter()
            .take(PLAN_SECTION_SAMPLE_LIMIT)
            .map(from_root_content_entry)
            .collect(),
    }
}

/// A path list the client renders, with the complete count beside it. Directory
/// lists under a large root are unbounded, so the payload states how many there
/// are rather than shipping all of them.
fn from_sampled_paths(paths: &[String]) -> LocationSampledPathsPayload {
    LocationSampledPathsPayload {
        total: Long(paths.len() as i64),
        complete: paths.len() <= PLAN_SECTION_SAMPLE_LIMIT,
        paths: paths
            .iter()
            .take(PLAN_SECTION_SAMPLE_LIMIT)
            .map(|path| display_path(path))
            .collect(),
    }
}

fn from_blocked_title(title: &BlockedTitle) -> LocationBlockedTitlePayload {
    LocationBlockedTitlePayload {
        title_id: ID::from(title.title_id.clone()),
        title_name: title.title_name.clone(),
        reason: title.reason.clone(),
        reason_code: title.reason_code.clone(),
    }
}

fn from_title_accounting(accounting: &TitleAccounting) -> LocationTitleAccountingPayload {
    LocationTitleAccountingPayload {
        assigned_total: Long(accounting.assigned_total),
        relocating: Long(accounting.relocating),
        catalog_only: Long(accounting.catalog_only),
        blocked: Long(accounting.blocked),
        accounts_for_every_title: accounting.accounts_for_every_title(),
        blocks_start: accounting.blocks_start(),
        blocked_titles: accounting
            .blocked_titles
            .iter()
            .map(from_blocked_title)
            .collect(),
    }
}

fn from_root_identity_retention(
    retention: &RootIdentityRetention,
) -> LocationRootIdentityRetentionPayload {
    LocationRootIdentityRetentionPayload {
        root_id: ID::from(retention.root_id.clone()),
        keeps_root_id: retention.keeps_root_id,
        was_library_default: retention.was_library_default,
        remains_library_default: retention.remains_library_default,
        retained_role: retention.retained_role.clone(),
        retained_title_assignments: Long(retention.retained_title_assignments),
    }
}

fn from_root_content_inventory(
    content: &RootContentInventory,
) -> LocationRootContentInventoryPayload {
    LocationRootContentInventoryPayload {
        managed: from_root_content_bucket(RootContentClass::Managed, &content.managed),
        companions: from_root_content_bucket(RootContentClass::Companion, &content.companions),
        unknown: from_root_content_bucket(RootContentClass::Unknown, &content.unknown),
        unknown_bytes: bytes(content.unknown_bytes()),
        blocks_source_removal: content.blocks_source_removal(),
        entry_count: Long(content.entry_count() as i64),
        prunable_directories: from_sampled_paths(&content.prunable_directories),
        retained_directories: from_sampled_paths(&content.retained_directories),
    }
}

fn from_root_retirement_contract(
    retirement: &RootRetirementContract,
) -> LocationRootRetirementContractPayload {
    LocationRootRetirementContractPayload {
        source_root_path: display_path(&retirement.source_root_path),
        destination_root_path: display_path(&retirement.destination_root_path),
        retire_configuration_after_recycling: retirement.retire_configuration_after_recycling,
        recycle_allowlist_paths: from_sampled_paths(&retirement.recycle_allowlist_paths),
        requires_verification_before_source_removal: retirement
            .requires_verification_before_source_removal,
        empty_directories_only: retirement.empty_directories_only,
        removable_directories: from_sampled_paths(&retirement.removable_directories),
        retained_directories: from_sampled_paths(&retirement.retained_directories),
        permits_source_removal: retirement.permits_source_removal(),
        blockers: retirement
            .blockers
            .iter()
            .map(|blocker| LocationRootRetirementBlockerPayload {
                code: blocker.code.clone(),
                detail: blocker.detail.clone(),
            })
            .collect(),
    }
}

/// The complete root-change preview (US4): the shared plan payload plus the
/// four blocks a root-scoped operation adds.
///
/// `RootChangePreview.execution` is deliberately not projected. The start path
/// rebuilds the plan from current state rather than trusting a round trip
/// (FR-081), so handing the client the instruction set would be shipping
/// something it must never send back.
pub fn from_root_change_preview(preview: &RootChangePreview) -> LocationRootChangePreviewPayload {
    LocationRootChangePreviewPayload {
        plan: from_location_plan(
            &preview.plan,
            from_classification_counts(&preview.plan.classification),
            preview.warnings.clone(),
        ),
        accounting: from_title_accounting(&preview.accounting),
        retention: from_root_identity_retention(&preview.retention),
        content: from_root_content_inventory(&preview.content),
        retirement: from_root_retirement_contract(&preview.retirement),
    }
}

/// The complete consolidation preview (US5): the shared plan payload, FR-024's
/// seven groups, FR-022's default statement, and the two blocks it shares with
/// a root change.
///
/// `RootConsolidationPreview.execution` is not projected, for the same reason a
/// root change's is not.
pub fn from_root_consolidation_preview(
    preview: &RootConsolidationPreview,
) -> LocationRootConsolidationPreviewPayload {
    let classification = &preview.classification;
    let transfer = &preview.default_transfer;
    LocationRootConsolidationPreviewPayload {
        plan: from_location_plan(
            &preview.plan,
            from_classification_counts(&preview.plan.classification),
            preview.warnings.clone(),
        ),
        accounting: from_title_accounting(&preview.accounting),
        classification: LocationConsolidationClassificationPayload {
            moving_into_unused_folders: Long(classification.moving_into_unused_folders),
            merging_with_destination_titles: Long(classification.merging_with_destination_titles),
            folder_name_collisions: Long(classification.folder_name_collisions),
            media_collisions: Long(classification.media_collisions),
            dedup_eligible_files: Long(classification.dedup_eligible_files),
            companion_collisions: Long(classification.companion_collisions),
            untracked_source_entries: Long(classification.untracked_source_entries),
            catalog_only: Long(classification.catalog_only),
            blocked: Long(classification.blocked),
        },
        default_transfer: LocationDefaultRootTransferPayload {
            source_was_default: transfer.source_was_default,
            destination_was_default: transfer.destination_was_default,
            // Both are methods on the application type, resolved here so the
            // client reads a fact rather than re-deriving the rule (FR-022).
            destination_becomes_default: transfer.destination_becomes_default(),
            transfers_the_default: transfer.transfers_the_default(),
        },
        content: from_root_content_inventory(&preview.content),
        retirement: from_root_retirement_contract(&preview.retirement),
    }
}

fn from_location_operation_counters(
    counters: &LocationOperationCounters,
) -> LocationOperationCountersPayload {
    LocationOperationCountersPayload {
        titles_total: Long(counters.titles_total),
        titles_processed: Long(counters.titles_processed),
        titles_blocked: Long(counters.titles_blocked),
        files_total: Long(counters.files_total),
        files_processed: Long(counters.files_processed),
        bytes_total: Long(counters.bytes_total),
        bytes_processed: Long(counters.bytes_processed),
        merges: Long(counters.merges),
        dedups: Long(counters.dedups),
        renames: Long(counters.renames),
        no_ops: Long(counters.no_ops),
        unresolved: Long(counters.unresolved),
    }
}

fn from_title_checkpoint(checkpoint: &TitleCheckpoint) -> LocationTitleCheckpointPayload {
    let placement = &checkpoint.placement;
    LocationTitleCheckpointPayload {
        title_id: ID::from(checkpoint.title_id.clone()),
        sequence: Long(checkpoint.sequence),
        state: from_title_checkpoint_state(checkpoint.state),
        classification: checkpoint.classification.map(from_title_location_class),
        source_library_id: placement.source_library_id.clone().map(ID::from),
        source_root_id: placement.source_root_id.clone().map(ID::from),
        source_folder_path: placement.source_folder_path.as_deref().map(display_path),
        destination_library_id: placement.destination_library_id.clone().map(ID::from),
        destination_root_id: placement.destination_root_id.clone().map(ID::from),
        destination_folder_path: placement
            .destination_folder_path
            .as_deref()
            .map(display_path),
        merged_into_title_id: placement.merged_into_title_id.clone().map(ID::from),
        merged_into_title_name: placement.merged_into_title_name.clone(),
        files_total: Long(checkpoint.files_total),
        files_verified: Long(checkpoint.files_verified),
        bytes_total: Long(checkpoint.bytes_total),
        bytes_verified: Long(checkpoint.bytes_verified),
        detail: checkpoint.detail.clone(),
        started_at: checkpoint.started_at,
        updated_at: checkpoint.updated_at,
        completed_at: checkpoint.completed_at,
    }
}

/// One operation row with the per-title checkpoints Activity expands.
pub fn from_location_operation(
    operation: &LocationOperation,
    checkpoints: &[TitleCheckpoint],
) -> LocationOperationPayload {
    LocationOperationPayload {
        id: ID::from(operation.id.clone()),
        operation_type: from_location_operation_type(operation.operation_type),
        mode: from_location_execution_mode(operation.mode),
        state: from_location_operation_state(operation.state),
        initiated_by_user_id: operation.initiated_by_user_id.clone().map(ID::from),
        source_library_id: operation.source_library_id.clone().map(ID::from),
        destination_library_id: operation.destination_library_id.clone().map(ID::from),
        source_root_id: operation.source_root_id.clone().map(ID::from),
        destination_root_id: operation.destination_root_id.clone().map(ID::from),
        plan_fingerprint: operation.plan_fingerprint.clone(),
        verification_depth: from_verification_depth(operation.verification_depth),
        verification_fallback_count: Long(operation.verification_fallback_count),
        counters: from_location_operation_counters(&operation.counters),
        detail: operation.detail.clone(),
        job_run_id: operation.job_run_id.clone().map(ID::from),
        workflow_operation_id: operation.workflow_operation_id.clone().map(ID::from),
        cancel_requested: operation.cancel_requested,
        cancel_requested_at: operation.cancel_requested_at,
        confirmed_at: operation.confirmed_at,
        started_at: operation.started_at,
        created_at: operation.created_at,
        updated_at: operation.updated_at,
        completed_at: operation.completed_at,
        title_checkpoints: checkpoints.iter().map(from_title_checkpoint).collect(),
    }
}

fn from_renamed_asset(asset: &RenamedAsset) -> LocationOperationRenamedAssetPayload {
    LocationOperationRenamedAssetPayload {
        source_path: asset.source_path.as_deref().map(display_path),
        source_name: asset.source_name.clone(),
        destination_path: display_path(&asset.destination_path),
        destination_name: asset.destination_name.clone(),
        provenance_label: asset.provenance_label.clone(),
        media_file_id: asset.media_file_id.clone().map(ID::from),
        size_bytes: bytes(asset.size_bytes),
        done: asset.done,
    }
}

fn from_deduplicated_asset(
    asset: &DeduplicatedAsset,
) -> LocationOperationDeduplicatedAssetPayload {
    LocationOperationDeduplicatedAssetPayload {
        source_path: display_path(&asset.source_path),
        source_name: asset.source_name.clone(),
        surviving_path: asset.surviving_path.as_deref().map(display_path),
        surviving_name: asset.surviving_name.clone(),
        done: asset.done,
    }
}

fn from_title_asset_listing(title: &TitleAssetListing) -> LocationOperationTitleAssetsPayload {
    LocationOperationTitleAssetsPayload {
        title_id: ID::from(title.title_id.clone()),
        title_name: title.title_name.clone(),
        sequence: Long(title.sequence),
        settled: title.settled,
        checkpoint_state: title.checkpoint_state.map(from_title_checkpoint_state),
        renames: title.renames.iter().map(from_renamed_asset).collect(),
        dedups: title.dedups.iter().map(from_deduplicated_asset).collect(),
    }
}

/// Which files an operation renames and deduplicates, per title (FR-091).
///
/// The done flags come through untouched: the application decides what settled,
/// and presenting a planned rename as a finished one is exactly the silence
/// FR-091 forbids.
pub fn from_location_operation_asset_listing(
    listing: &LocationOperationAssetListing,
) -> LocationOperationAssetListingPayload {
    LocationOperationAssetListingPayload {
        operation_id: ID::from(listing.operation_id.clone()),
        titles: listing
            .titles
            .iter()
            .map(from_title_asset_listing)
            .collect(),
        renames_total: Long(listing.renames_total),
        renames_done: Long(listing.renames_done),
        dedups_total: Long(listing.dedups_total),
        dedups_done: Long(listing.dedups_done),
    }
}

/// Acceptance of a started operation; the plan itself is not echoed back, only
/// the fingerprint the server rebuilt and accepted (FR-081).
pub fn from_started_location_operation(
    operation: &LocationOperation,
    checkpoints: &[TitleCheckpoint],
    fingerprint: &PlanFingerprint,
) -> StartLocationOperationPayload {
    StartLocationOperationPayload {
        operation: from_location_operation(operation, checkpoints),
        plan_fingerprint: fingerprint.0.clone(),
    }
}

pub fn from_canceled_location_operation(
    operation_id: &str,
    cancel_requested: bool,
) -> CancelLocationOperationPayload {
    CancelLocationOperationPayload {
        id: ID::from(operation_id.to_string()),
        cancel_requested,
    }
}

pub fn from_resumed_location_operation(
    operation_id: &str,
    resumed: bool,
    detail: Option<String>,
) -> ResumeLocationOperationPayload {
    ResumeLocationOperationPayload {
        id: ID::from(operation_id.to_string()),
        resumed,
        detail,
    }
}
