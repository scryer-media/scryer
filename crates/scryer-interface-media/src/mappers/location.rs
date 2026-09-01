//! Projections between the location subsystem's application types and the
//! GraphQL location surface (US2, FR-080 to FR-083).
//!
//! Nothing here derives a fact the application did not state. An unprobed
//! free-space estimate stays unknown, a plan section's sample stays a sample
//! beside its complete count, and paths are converted from their stored
//! encoding to the real filesystem spelling on the way out.

use async_graphql::ID;
use scryer_application::location::classify::{
    DestinationRequest, SelectionClassification, TitleClassification, TitleLocationClass,
};
use scryer_application::location::model::{
    LocationExecutionMode, LocationOperation, LocationOperationCounters, LocationOperationState,
    LocationOperationType, TitleCheckpoint, TitleCheckpointState, VerificationDepth,
};
use scryer_application::location::operations::RootMovePreview;
use scryer_application::location::preview::{
    ConfirmationRequirement, FreeSpaceEstimate, LocationPlan, PlanConfirmation, PlanCounts,
    PlanFingerprint, PlanItem, PlanItemKind, PlanSection, VerificationStatement,
};

use crate::types::{
    CancelLocationOperationPayload, LocationClassificationGroupPayload,
    LocationClassifiedTitlePayload, LocationConfirmationRequirementValue, LocationDestinationInput,
    LocationExecutionModeValue, LocationFreeSpaceEstimatePayload, LocationOperationCountersPayload,
    LocationOperationPayload, LocationOperationPreviewPayload, LocationOperationStateValue,
    LocationOperationTypeValue, LocationPlanConfirmationPayload, LocationPlanCountsPayload,
    LocationPlanItemKindValue, LocationPlanItemPayload, LocationPlanKindCountPayload,
    LocationPlanSectionPayload, LocationSelectionClassificationPayload,
    LocationTitleCheckpointPayload, LocationTitleCheckpointStateValue,
    LocationVerificationStatementPayload, Long, ResumeLocationOperationPayload,
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
