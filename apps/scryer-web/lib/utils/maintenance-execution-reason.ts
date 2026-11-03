import type { Translate } from "@/components/root/types";

/**
 * The executor writes a stable code as a held action's `holdReason` and its
 * candidate's `stateReason`. The codes listed here are said in the viewer's
 * language; any other code is shown as the server wrote it.
 */
export const MAINTENANCE_EXECUTION_REASON_KEYS: Readonly<
  Record<string, string>
> = {
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
