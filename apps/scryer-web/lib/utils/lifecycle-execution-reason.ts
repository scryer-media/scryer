import type { Translate } from "@/components/root/types";

/**
 * The executor writes a stable code as a held action's `holdReason` and its
 * candidate's `stateReason`. The codes listed here are said in the viewer's
 * language; any other code is shown as the server wrote it.
 */
export const LIFECYCLE_EXECUTION_REASON_KEYS: Readonly<Record<string, string>> =
  {
    rule_not_eligible: "settings.lifecycleExecutionReason.ruleNotEligible",
    no_match_at_execution:
      "settings.lifecycleExecutionReason.noMatchAtExecution",
    unknown_at_execution:
      "settings.lifecycleExecutionReason.unknownAtExecution",
    playback_hold: "settings.lifecycleExecutionReason.playbackHold",
    playback_unknown: "settings.lifecycleExecutionReason.playbackUnknown",
    watch_recheck_newer_play:
      "settings.lifecycleExecutionReason.watchRecheckNewerPlay",
    watch_recheck_newer_favorite:
      "settings.lifecycleExecutionReason.watchRecheckNewerFavorite",
    watch_recheck_uncredited_play:
      "settings.lifecycleExecutionReason.watchRecheckUncreditedPlay",
    watch_recheck_unavailable:
      "settings.lifecycleExecutionReason.watchRecheckUnavailable",
    watch_recheck_unmapped:
      "settings.lifecycleExecutionReason.watchRecheckUnmapped",
    active_acquisition: "settings.lifecycleExecutionReason.activeAcquisition",
    acquisition_unknown: "settings.lifecycleExecutionReason.acquisitionUnknown",
    storage_deletion_in_progress:
      "settings.lifecycleExecutionReason.storageDeletionInProgress",
    action_not_fully_supported:
      "settings.lifecycleExecutionReason.actionNotFullySupported",
    title_missing: "settings.lifecycleExecutionReason.titleMissing",
    location_operation_hold:
      "settings.lifecycleExecutionReason.locationOperationHold",
    pass_action_step_budget:
      "settings.lifecycleExecutionReason.passActionStepBudget",
    pass_high_risk_step_budget:
      "settings.lifecycleExecutionReason.passHighRiskStepBudget",
    action_succeeded: "settings.lifecycleExecutionReason.actionSucceeded",
    already_satisfied: "settings.lifecycleExecutionReason.alreadySatisfied",
    retry_pending: "settings.lifecycleExecutionReason.retryPending",
    action_failed: "settings.lifecycleExecutionReason.actionFailed",
    lease_reclaimed: "settings.lifecycleExecutionReason.leaseReclaimed",
    tag_not_defined: "settings.lifecycleExecutionReason.tagNotDefined",
    tag_patch_conflict: "settings.lifecycleExecutionReason.tagPatchConflict",
    monitoring_conflict: "settings.lifecycleExecutionReason.monitoringConflict",
    ambiguous_episode_file_coverage:
      "settings.lifecycleExecutionReason.ambiguousEpisodeFileCoverage",
    quality_profile_unresolved:
      "settings.lifecycleExecutionReason.qualityProfileUnresolved",
    actions_not_runnable:
      "settings.lifecycleExecutionReason.actionsNotRunnable",
    actions_changed_at_execution:
      "settings.lifecycleExecutionReason.actionsChangedAtExecution",
    action_not_declared: "settings.lifecycleExecutionReason.actionNotDeclared",
    actions_conflict: "settings.lifecycleExecutionReason.actionsConflict",
    move_library_unresolved:
      "settings.lifecycleExecutionReason.moveLibraryUnresolved",
    move_library_facet_mismatch:
      "settings.lifecycleExecutionReason.moveLibraryFacetMismatch",
    move_root_not_in_library:
      "settings.lifecycleExecutionReason.moveRootNotInLibrary",
    move_refused: "settings.lifecycleExecutionReason.moveRefused",
  };

/** A hold or state reason for display: translated when known, else as given. */
export function lifecycleExecutionReasonText(
  t: Translate,
  code: string,
): string {
  if (Object.hasOwn(LIFECYCLE_EXECUTION_REASON_KEYS, code)) {
    return t(LIFECYCLE_EXECUTION_REASON_KEYS[code]);
  }
  return code;
}

/**
 * The executor records the stable code behind a hold or failure as a step's
 * or run's `causeCode` and a candidate's `stateCauseCode`: the refusal code a
 * move gave, or the code of the failure a step hit. The codes listed here are
 * said in the viewer's language; any other code is shown as the server wrote
 * it.
 */
export const LIFECYCLE_EXECUTION_CAUSE_KEYS: Readonly<Record<string, string>> =
  {
    stale_plan: "settings.lifecycleExecutionCause.staleMovePlan",
    blocked_items: "settings.lifecycleExecutionCause.blockedItems",
    insufficient_space: "settings.lifecycleExecutionCause.insufficientSpace",
  };

/** A hold or failure cause for display: translated when known, else as given. */
export function lifecycleExecutionCauseText(
  t: Translate,
  code: string,
): string {
  if (Object.hasOwn(LIFECYCLE_EXECUTION_CAUSE_KEYS, code)) {
    return t(LIFECYCLE_EXECUTION_CAUSE_KEYS[code]);
  }
  return code;
}
