import assert from "node:assert/strict";
import test from "node:test";

import en from "../i18n/locales/en.ts";
import {
  MAINTENANCE_EXECUTION_REASON_KEYS,
  maintenanceExecutionReasonText,
} from "./maintenance-execution-reason.ts";

const dictionary = en as Record<string, string>;
const t = (key: string) => dictionary[key] ?? key;

test("every translated execution reason has an English string", () => {
  for (const [code, key] of Object.entries(MAINTENANCE_EXECUTION_REASON_KEYS)) {
    assert.equal(typeof dictionary[key], "string", `${code} → ${key}`);
  }
});

test("a watch re-check hold is said in translation", () => {
  assert.equal(
    maintenanceExecutionReasonText(t, "watch_recheck_newer_play"),
    dictionary["settings.maintenanceExecutionReason.watchRecheckNewerPlay"],
  );
});

test("a code the client does not translate is shown as written", () => {
  assert.equal(
    maintenanceExecutionReasonText(t, "some_future_reason"),
    "some_future_reason",
  );
});

// Mirrors `ALL` in `execution_reason`,
// crates/scryer-application/src/maintenance_rules/action_execution.rs.
test("the client translates exactly the server's execution reasons", () => {
  assert.deepEqual(Object.keys(MAINTENANCE_EXECUTION_REASON_KEYS).sort(), [
    "acquisition_unknown",
    "action_failed",
    "action_not_declared",
    "action_not_fully_supported",
    "action_succeeded",
    "actions_changed_at_execution",
    "actions_conflict",
    "actions_not_runnable",
    "active_acquisition",
    "already_satisfied",
    "ambiguous_episode_file_coverage",
    "lease_reclaimed",
    "location_operation_hold",
    "monitoring_conflict",
    "move_library_facet_mismatch",
    "move_library_unresolved",
    "move_refused",
    "move_root_not_in_library",
    "no_match_at_execution",
    "pass_action_step_budget",
    "pass_high_risk_step_budget",
    "playback_hold",
    "playback_unknown",
    "quality_profile_unresolved",
    "retry_pending",
    "rule_not_eligible",
    "storage_deletion_in_progress",
    "storage_root_action_not_allowed",
    "tag_not_defined",
    "tag_patch_conflict",
    "title_missing",
    "unknown_at_execution",
    "watch_recheck_newer_play",
    "watch_recheck_unavailable",
    "watch_recheck_uncredited_play",
    "watch_recheck_unmapped",
  ]);
});
