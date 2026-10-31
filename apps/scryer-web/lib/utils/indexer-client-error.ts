import {
  graphQlErrorExtension,
  userFacingGraphQlErrorMessage,
} from "../graphql/error-message.ts";

export const INDEXER_CLIENT_REFUSAL_KEYS: Readonly<Record<string, string>> = {
  INDEXER_CLIENTS_INTERACTIVE_SESSION_REQUIRED: "indexerClients.error.session",
  INDEXER_CLIENTS_ALL_PERMISSIONS_REQUIRED: "indexerClients.error.permissions",
  INDEXER_CLIENTS_ENABLE_REQUIRES_KEY: "indexerClients.error.enableRequiresKey",
  INDEXER_CLIENTS_KEY_INVALID: "indexerClients.error.key",
  INDEXER_CLIENTS_UNAVAILABLE: "indexerClients.error.unavailable",
  INDEXER_CLIENTS_NOT_FOUND: "indexerClients.error.missing",
  INDEXER_CLIENTS_CONFIGURATION_CHANGED: "indexerClients.error.changed",
  INDEXER_CLIENTS_NAME_INVALID: "indexerClients.error.name",
  INDEXER_CLIENTS_GRANT_INVALID: "indexerClients.error.grant",
  INDEXER_CLIENTS_REVISION_EXHAUSTED: "indexerClients.error.revision",
};

/// Refusals that mean this session may not use client management at all. The
/// page explains these in place of the client list rather than as a failure.
const ACCESS_REFUSAL_REASONS: ReadonlySet<string> = new Set([
  "INDEXER_CLIENTS_INTERACTIVE_SESSION_REQUIRED",
  "INDEXER_CLIENTS_ALL_PERMISSIONS_REQUIRED",
]);

function indexerClientRefusalReason(error: unknown): string | null {
  return (
    graphQlErrorExtension(error, "UNAUTHORIZED", "reason") ??
    graphQlErrorExtension(error, "VALIDATION_ERROR", "reason") ??
    graphQlErrorExtension(error, "NOT_FOUND", "reason") ??
    graphQlErrorExtension(error, "TEMPORARY_UNAVAILABLE", "reason")
  );
}

/// The explanation for a refusal that closes client management to this
/// session, or null when the error is an ordinary failure.
export function indexerClientAccessRefusal(
  error: unknown,
  t: (key: string) => string,
): string | null {
  const reason = indexerClientRefusalReason(error);
  return reason && ACCESS_REFUSAL_REASONS.has(reason)
    ? t(INDEXER_CLIENT_REFUSAL_KEYS[reason]!)
    : null;
}

export function indexerClientErrorMessage(
  error: unknown,
  t: (key: string) => string,
): string {
  const reason = indexerClientRefusalReason(error);
  const key =
    reason && Object.hasOwn(INDEXER_CLIENT_REFUSAL_KEYS, reason)
      ? INDEXER_CLIENT_REFUSAL_KEYS[reason]
      : null;
  return key
    ? t(key)
    : userFacingGraphQlErrorMessage(error, t("status.apiError"), t);
}
