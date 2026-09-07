import type { CombinedError } from "urql";
import { DEFAULT_LANGUAGE, t } from "../i18n/index.ts";

const GRAPHQL_PREFIX_PATTERNS: RegExp[] = [
  /^\[GraphQL\]\s*/i,
  /^validation:\s*/i,
];
const REPOSITORY_PREFIX_PATTERN = /^repository:\s*/i;
const CONFIG_STEP_UP_REQUIRED_MESSAGE =
  "Settings verification expired. Enter an authenticator code to continue.";
const INTERNAL_ERROR_CODE = "INTERNAL_ERROR";
const INTERNAL_ERROR_MESSAGE = "Internal server error";

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function firstGraphQlErrorMessage(error: CombinedError | unknown): string | null {
  if (!isRecord(error) || !Array.isArray(error.graphQLErrors)) {
    return null;
  }

  for (const graphQlError of error.graphQLErrors) {
    if (isRecord(graphQlError) && typeof graphQlError.message === "string") {
      const message = graphQlError.message.trim();
      if (message) {
        return message;
      }
    }
  }

  return null;
}

export function hasGraphQlErrorCode(error: CombinedError | unknown, code: string): boolean {
  if (!isRecord(error) || !Array.isArray(error.graphQLErrors)) {
    return false;
  }

  return error.graphQLErrors.some((graphQlError) => {
    if (!isRecord(graphQlError) || !isRecord(graphQlError.extensions)) {
      return false;
    }
    return graphQlError.extensions.code === code;
  });
}

function internalGraphQlErrorMessage(error: CombinedError | unknown): string | null {
  if (!isRecord(error) || !Array.isArray(error.graphQLErrors)) {
    return null;
  }

  for (const graphQlError of error.graphQLErrors) {
    if (!isRecord(graphQlError) || !isRecord(graphQlError.extensions)) {
      continue;
    }
    if (graphQlError.extensions.code !== INTERNAL_ERROR_CODE) {
      continue;
    }

    const errorId =
      typeof graphQlError.extensions.errorId === "string"
        ? graphQlError.extensions.errorId.trim()
        : "";
    return errorId ? `${INTERNAL_ERROR_MESSAGE}. Reference ID: ${errorId}` : INTERNAL_ERROR_MESSAGE;
  }

  return null;
}

export function isMfaStepUpRequiredError(error: unknown): boolean {
  return hasGraphQlErrorCode(error, "MFA_STEP_UP_REQUIRED");
}

function fallbackErrorMessage(error: unknown): string | null {
  if (error instanceof Error) {
    const message = error.message.trim();
    return message || null;
  }

  if (isRecord(error) && typeof error.message === "string") {
    const message = error.message.trim();
    return message || null;
  }

  return null;
}

export function normalizeGraphQlErrorMessage(message: string): string {
  let normalized = message.trim();
  let changed = true;

  while (normalized && changed) {
    changed = false;
    for (const pattern of GRAPHQL_PREFIX_PATTERNS) {
      const next = normalized.replace(pattern, "").trimStart();
      if (next !== normalized) {
        normalized = next;
        changed = true;
      }
    }
  }

  normalized = normalized.trim();
  if (/^(?:unauthorized:\s*)?invalid release candidate token:\s*ExpiredSignature$/i.test(normalized)) {
    return t("status.searchResultsExpired", DEFAULT_LANGUAGE);
  }
  if (REPOSITORY_PREFIX_PATTERN.test(normalized)) {
    return INTERNAL_ERROR_MESSAGE;
  }

  if (
    normalized.toLowerCase() ===
    "mfa verification is required before changing system configuration"
  ) {
    return CONFIG_STEP_UP_REQUIRED_MESSAGE;
  }

  return normalized;
}

export function userFacingGraphQlErrorMessage(error: unknown, fallback: string): string {
  const internalMessage = internalGraphQlErrorMessage(error);
  if (internalMessage) {
    return internalMessage;
  }

  const rawMessage =
    firstGraphQlErrorMessage(error) ?? fallbackErrorMessage(error) ?? fallback.trim();
  const normalized = normalizeGraphQlErrorMessage(rawMessage);
  return normalized || fallback.trim();
}
