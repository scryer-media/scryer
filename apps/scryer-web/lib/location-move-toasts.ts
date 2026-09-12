import {
  toCount,
  type LocationOperationState,
  type LongValue,
} from "./location-operations.ts";
import type { JobRun } from "./types";

/** The job key a location operation reports through in Activity. */
export const LOCATION_OPERATION_JOB_KEY = "LOCATION_OPERATION";

/**
 * The operation a job run reports on, read from the progress the server
 * writes with every pulse (`operationId`). Null for any other job, and for a
 * location run whose progress has not been written yet.
 */
export function locationOperationIdFromJobRun(
  run: Pick<JobRun, "jobKey" | "progressJson">,
): string | null {
  if (run.jobKey !== LOCATION_OPERATION_JOB_KEY) {
    return null;
  }
  const progress = run.progressJson;
  if (!progress || typeof progress !== "object") {
    return null;
  }
  const id = (progress as Record<string, unknown>).operationId;
  return typeof id === "string" && id.trim() !== "" ? id.trim() : null;
}

export type MoveToastVisualState =
  | "moving"
  | "success"
  | "issues"
  | "failed"
  | "canceled";

/** How the toast reads an operation state; anything not settled is moving. */
export function moveToastVisualState(
  state: LocationOperationState | null | undefined,
): MoveToastVisualState {
  switch (state) {
    case "COMPLETED":
      return "success";
    case "COMPLETED_WITH_WARNINGS":
      return "issues";
    case "FAILED":
      return "failed";
    case "CANCELED":
      return "canceled";
    default:
      return "moving";
  }
}

/** Seconds the server estimates remain, or null when it has no estimate yet. */
export function moveEtaSeconds(
  value: LongValue | null | undefined,
): number | null {
  if (value == null) {
    return null;
  }
  const seconds = toCount(value);
  return seconds > 0 ? seconds : null;
}

/** `mm:ss` under an hour, `h:mm:ss` from an hour on; never below one second. */
export function formatMoveEta(totalSeconds: number): string {
  const seconds = Math.max(1, Math.round(totalSeconds));
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  const secs = seconds % 60;
  const clock = `${String(minutes).padStart(2, "0")}:${String(secs).padStart(2, "0")}`;
  return hours > 0 ? `${hours}:${clock}` : clock;
}
