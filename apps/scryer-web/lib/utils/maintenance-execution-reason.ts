import type { Translate } from "@/components/root/types";

/**
 * The executor writes a stable code as a held action's `holdReason` and its
 * candidate's `stateReason`. The codes listed here are said in the viewer's
 * language; any other code is shown as the server wrote it.
 */
export const MAINTENANCE_EXECUTION_REASON_KEYS: Readonly<
  Record<string, string>
> = {
  rule_not_eligible: "settings.maintenanceExecutionReason.ruleNotEligible",
  no_match_at_execution:
    "settings.maintenanceExecutionReason.noMatchAtExecution",
  unknown_at_execution:
    "settings.maintenanceExecutionReason.unknownAtExecution",
  playback_hold: "settings.maintenanceExecutionReason.playbackHold",
  playback_unknown: "settings.maintenanceExecutionReason.playbackUnknown",
  watch_recheck_newer_play:
    "settings.maintenanceExecutionReason.watchRecheckNewerPlay",
  watch_recheck_uncredited_play:
    "settings.maintenanceExecutionReason.watchRecheckUncreditedPlay",
  watch_recheck_unavailable:
    "settings.maintenanceExecutionReason.watchRecheckUnavailable",
  watch_recheck_unmapped:
    "settings.maintenanceExecutionReason.watchRecheckUnmapped",
  active_acquisition: "settings.maintenanceExecutionReason.activeAcquisition",
  acquisition_unknown: "settings.maintenanceExecutionReason.acquisitionUnknown",
  storage_deletion_in_progress:
    "settings.maintenanceExecutionReason.storageDeletionInProgress",
  action_not_fully_supported:
    "settings.maintenanceExecutionReason.actionNotFullySupported",
  title_missing: "settings.maintenanceExecutionReason.titleMissing",
  location_operation_hold:
    "settings.maintenanceExecutionReason.locationOperationHold",
  pass_action_step_budget:
    "settings.maintenanceExecutionReason.passActionStepBudget",
  pass_high_risk_step_budget:
    "settings.maintenanceExecutionReason.passHighRiskStepBudget",
  action_succeeded: "settings.maintenanceExecutionReason.actionSucceeded",
  already_satisfied: "settings.maintenanceExecutionReason.alreadySatisfied",
  retry_pending: "settings.maintenanceExecutionReason.retryPending",
  action_failed: "settings.maintenanceExecutionReason.actionFailed",
  lease_reclaimed: "settings.maintenanceExecutionReason.leaseReclaimed",
  tag_not_defined: "settings.maintenanceExecutionReason.tagNotDefined",
  tag_patch_conflict: "settings.maintenanceExecutionReason.tagPatchConflict",
  monitoring_conflict: "settings.maintenanceExecutionReason.monitoringConflict",
  ambiguous_episode_file_coverage:
    "settings.maintenanceExecutionReason.ambiguousEpisodeFileCoverage",
  quality_profile_unresolved:
    "settings.maintenanceExecutionReason.qualityProfileUnresolved",
  actions_not_runnable:
    "settings.maintenanceExecutionReason.actionsNotRunnable",
  actions_changed_at_execution:
    "settings.maintenanceExecutionReason.actionsChangedAtExecution",
  action_not_declared: "settings.maintenanceExecutionReason.actionNotDeclared",
  storage_root_action_not_allowed:
    "settings.maintenanceExecutionReason.storageRootActionNotAllowed",
  actions_conflict: "settings.maintenanceExecutionReason.actionsConflict",
  move_library_unresolved:
    "settings.maintenanceExecutionReason.moveLibraryUnresolved",
  move_library_facet_mismatch:
    "settings.maintenanceExecutionReason.moveLibraryFacetMismatch",
  move_root_not_in_library:
    "settings.maintenanceExecutionReason.moveRootNotInLibrary",
  move_refused: "settings.maintenanceExecutionReason.moveRefused",
};

/** A hold or state reason for display: translated when known, else as given. */
export function maintenanceExecutionReasonText(
  t: Translate,
  code: string,
): string {
  if (Object.hasOwn(MAINTENANCE_EXECUTION_REASON_KEYS, code)) {
    return t(MAINTENANCE_EXECUTION_REASON_KEYS[code]);
  }
  return code;
}
