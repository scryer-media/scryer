import type { Translate } from "@/components/root/types";
import {
  graphQlErrorExtension,
  userFacingGraphQlErrorMessage,
} from "../graphql/error-message.ts";

/**
 * The server names why a backup restore stopped or was refused: the condition
 * travels as `extensions.reason` on a `VALIDATION_ERROR`, and as `reason` on a
 * failed or interrupted `restoreStatus`. Each reason listed here is said in the
 * viewer's language; any other failure shows the server's sentence.
 */
export const RESTORE_REFUSAL_KEYS: Readonly<Record<string, string>> = {
  RESTORE_DEADLINE_EXCEEDED: "setup.restoreRefusal.deadlineExceeded",
  RESTORE_MAINTENANCE_ACTIVE: "setup.restoreRefusal.maintenanceActive",
  RESTORE_INTERRUPTED: "setup.restoreRefusal.interrupted",
  RESTORE_ATTEMPT_NOT_FOUND: "setup.restoreRefusal.attemptNotFound",
  RESTORE_RECOVERY_REQUIRED: "setup.restoreRefusal.recoveryRequired",
  RESTORE_OPERATION_INVALID: "setup.restoreRefusal.operationInvalid",
  RESTORE_RESTART_FAILED: "setup.restoreRefusal.restartFailed",
};

/** A failed restore attempt as `restoreStatus` reports it. */
export type RestoreJobFailure = {
  reason: string | null;
  message: string | null;
};

/**
 * A failed or interrupted restore attempt in the same shape as a refused
 * request, so one translation path explains both.
 */
export function restoreJobError(
  job: RestoreJobFailure,
  fallback: string,
): Error {
  const error = new Error(job.message ?? fallback) as Error & {
    graphQLErrors: unknown[];
  };
  error.graphQLErrors = [
    {
      message: job.message ?? fallback,
      extensions: job.reason
        ? { code: "VALIDATION_ERROR", reason: job.reason }
        : {},
    },
  ];
  return error;
}

/** The translation key for the reason a restore stopped, if known. */
export function restoreRefusalKey(error: unknown): string | null {
  const reason = graphQlErrorExtension(error, "VALIDATION_ERROR", "reason");
  return reason !== null && Object.hasOwn(RESTORE_REFUSAL_KEYS, reason)
    ? RESTORE_REFUSAL_KEYS[reason]
    : null;
}

/**
 * What to show for a failed restore: the translated reason when the server
 * named one this client knows, otherwise the shared GraphQL error translation
 * with the server's own sentence as its fallback, and `fallback` when the
 * failure carries no usable sentence.
 */
export function restoreErrorMessage(
  error: unknown,
  t: Translate,
  fallback: string,
): string {
  const key = restoreRefusalKey(error);
  return key ? t(key) : userFacingGraphQlErrorMessage(error, fallback, t);
}
