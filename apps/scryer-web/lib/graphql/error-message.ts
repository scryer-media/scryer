import { PUBLIC_ERROR_REASON_CODES } from "./public-error-reason-codes.ts";
import { PUBLIC_ERROR_REASON_KEYS } from "./public-error-reason-keys.ts";
import type { CombinedError } from "urql";
import { DEFAULT_LANGUAGE, t } from "../i18n/index.ts";

const GRAPHQL_PREFIX_PATTERNS: RegExp[] = [
  /^\[GraphQL\]\s*/i,
  /^validation:\s*/i,
];
type ErrorTranslate = (
  key: string,
  values?: Record<string, string | number | boolean | null | undefined>,
) => string;
const defaultTranslate: ErrorTranslate = (key, values) =>
  t(key, DEFAULT_LANGUAGE, values);

const INTERNAL_ERROR_CODE = "INTERNAL_ERROR";

/** Public reasons name conditions; their sentences never determine behavior. */
export const ERROR_REASON_KEYS: Readonly<Record<string, string>> = {
  ...PUBLIC_ERROR_REASON_KEYS,
  INITIAL_AUTH_REQUIRED: "initialAuth.required",
  INITIAL_AUTH_ALREADY_COMPLETED: "initialAuth.alreadyCompleted",
  INITIAL_AUTH_PASSWORD_REPLACEMENT_REQUIRED: "initialAuth.replaceRequired",
  INITIAL_AUTH_CODE_INVALID: "initialAuth.invalidCode",
  INITIAL_AUTH_CODE_RATE_LIMITED: "initialAuth.rateLimited",
  INITIAL_AUTH_MODE_CONFLICT: "initialAuth.modeConflict",
  INITIAL_AUTH_ACKNOWLEDGEMENT_REQUIRED: "initialAuth.acknowledgeRequired",
  INTERFACE_ACTION_DEFINITION_CONFLICT:
    "error.reason.INTERFACE_ACTION_DEFINITION_CONFLICT",
  INTERFACE_ACTION_DEFINITION_REQUIRED:
    "error.reason.INTERFACE_ACTION_DEFINITION_REQUIRED",
  INTERFACE_ACTION_SEQUENCE_INPUT_REQUIRED:
    "error.reason.INTERFACE_ACTION_SEQUENCE_INPUT_REQUIRED",
  INTERFACE_ACTION_SCHEMA_VERSION_INVALID:
    "error.reason.INTERFACE_ACTION_SCHEMA_VERSION_INVALID",
  INTERFACE_ACTION_UNMONITOR_PARAMETERS_INVALID:
    "error.reason.INTERFACE_ACTION_UNMONITOR_PARAMETERS_INVALID",
  INTERFACE_ACTION_UNMONITOR_DESCENDANTS_REQUIRED:
    "error.reason.INTERFACE_ACTION_UNMONITOR_DESCENDANTS_REQUIRED",
  INTERFACE_ACTION_QUALITY_PARAMETERS_INVALID:
    "error.reason.INTERFACE_ACTION_QUALITY_PARAMETERS_INVALID",
  INTERFACE_ACTION_QUALITY_PROFILE_REQUIRED:
    "error.reason.INTERFACE_ACTION_QUALITY_PROFILE_REQUIRED",
  INTERFACE_ACTION_SEARCH_PARAMETERS_INVALID:
    "error.reason.INTERFACE_ACTION_SEARCH_PARAMETERS_INVALID",
  INTERFACE_ACTION_SEARCH_CONDITION_REQUIRED:
    "error.reason.INTERFACE_ACTION_SEARCH_CONDITION_REQUIRED",
  INTERFACE_ACTION_TAG_PARAMETERS_INVALID:
    "error.reason.INTERFACE_ACTION_TAG_PARAMETERS_INVALID",
  INTERFACE_ACTION_TAGS_REQUIRED: "error.reason.INTERFACE_ACTION_TAGS_REQUIRED",
  INTERFACE_ACTION_PARAMETERS_UNEXPECTED:
    "error.reason.INTERFACE_ACTION_PARAMETERS_UNEXPECTED",
  INTERFACE_REQUEST_SAMPLE_LEASE_CONFLICT:
    "error.reason.INTERFACE_REQUEST_SAMPLE_LEASE_CONFLICT",

  INTERFACE_MINIMUM_AVAILABILITY_INVALID:
    "error.reason.INTERFACE_MINIMUM_AVAILABILITY_INVALID",
  INTERFACE_MINIMUM_AVAILABILITY_FACET_UNSUPPORTED:
    "error.reason.INTERFACE_MINIMUM_AVAILABILITY_FACET_UNSUPPORTED",
  INTERFACE_SEASON_FOLDERS_FACET_UNSUPPORTED:
    "error.reason.INTERFACE_SEASON_FOLDERS_FACET_UNSUPPORTED",
  INTERFACE_RELEASE_NUMBERING_FACET_UNSUPPORTED:
    "error.reason.INTERFACE_RELEASE_NUMBERING_FACET_UNSUPPORTED",
  INTERFACE_MONITOR_SELECTION_FACET_UNSUPPORTED:
    "error.reason.INTERFACE_MONITOR_SELECTION_FACET_UNSUPPORTED",
  INTERFACE_MONITOR_SELECTION_REQUIRED:
    "error.reason.INTERFACE_MONITOR_SELECTION_REQUIRED",
  INTERFACE_MONITOR_SELECTION_MODE_INVALID:
    "error.reason.INTERFACE_MONITOR_SELECTION_MODE_INVALID",
  INTERFACE_METADATA_LANGUAGE_UNSUPPORTED:
    "error.reason.INTERFACE_METADATA_LANGUAGE_UNSUPPORTED",
  INTERFACE_SEARCH_OPTIONS_LIMIT_EXCEEDED:
    "error.reason.INTERFACE_SEARCH_OPTIONS_LIMIT_EXCEEDED",
  INTERFACE_SEARCH_ALIAS_INVALID: "error.reason.INTERFACE_SEARCH_ALIAS_INVALID",
  INTERFACE_ROOT_FOLDER_NOT_FOUND:
    "error.reason.INTERFACE_ROOT_FOLDER_NOT_FOUND",
  INTERFACE_ROOT_FOLDER_LIBRARY_MISMATCH:
    "error.reason.INTERFACE_ROOT_FOLDER_LIBRARY_MISMATCH",
  INTERFACE_TITLE_LIBRARY_UNAVAILABLE:
    "error.reason.INTERFACE_TITLE_LIBRARY_UNAVAILABLE",
  INTERFACE_SEARCH_LANGUAGE_UNSUPPORTED:
    "error.reason.INTERFACE_SEARCH_LANGUAGE_UNSUPPORTED",
  INTERFACE_CONFIG_JSON_INVALID: "error.reason.INTERFACE_CONFIG_JSON_INVALID",
  INTERFACE_NOTIFICATION_JSON_INVALID:
    "error.reason.INTERFACE_NOTIFICATION_JSON_INVALID",
  INTERFACE_LOGIN_FORM_REQUIRED: "error.reason.INTERFACE_LOGIN_FORM_REQUIRED",
  INTERFACE_OAUTH_SESSION_REQUIRED:
    "error.reason.INTERFACE_OAUTH_SESSION_REQUIRED",
  INTERFACE_BROWSE_DIRECTORY_MISSING:
    "error.reason.INTERFACE_BROWSE_DIRECTORY_MISSING",
  INTERFACE_BROWSE_DIRECTORY_UNREADABLE:
    "error.reason.INTERFACE_BROWSE_DIRECTORY_UNREADABLE",
  INTERFACE_BROWSE_DIRECTORY_UNAVAILABLE:
    "error.reason.INTERFACE_BROWSE_DIRECTORY_UNAVAILABLE",
  INTERFACE_RULE_ACTION_INVALID: "error.reason.INTERFACE_RULE_ACTION_INVALID",
  INTERFACE_REQUEST_RULE_SAMPLE_INVALID:
    "error.reason.INTERFACE_REQUEST_RULE_SAMPLE_INVALID",

  INTERFACE_ACQUISITION_SCOPE_INVALID:
    "error.reason.INTERFACE_ACQUISITION_SCOPE_INVALID",
  INTERFACE_BROWSE_PATH_NOT_ABSOLUTE:
    "error.reason.INTERFACE_BROWSE_PATH_NOT_ABSOLUTE",
  INTERFACE_BROWSE_PATH_NOT_DIRECTORY:
    "error.reason.INTERFACE_BROWSE_PATH_NOT_DIRECTORY",
  INTERFACE_CATALOG_FILTER_BLANK: "error.reason.INTERFACE_CATALOG_FILTER_BLANK",
  INTERFACE_CATALOG_RATING_INVALID:
    "error.reason.INTERFACE_CATALOG_RATING_INVALID",
  INTERFACE_CATALOG_TAG_FILTER_INVALID:
    "error.reason.INTERFACE_CATALOG_TAG_FILTER_INVALID",
  INTERFACE_CATALOG_YEARS_INVALID:
    "error.reason.INTERFACE_CATALOG_YEARS_INVALID",
  INTERFACE_CONFIG_KEY_REQUIRED: "error.reason.INTERFACE_CONFIG_KEY_REQUIRED",
  INTERFACE_CONFIG_NUMBER_INVALID:
    "error.reason.INTERFACE_CONFIG_NUMBER_INVALID",
  INTERFACE_CONFIG_VALUE_AMBIGUOUS:
    "error.reason.INTERFACE_CONFIG_VALUE_AMBIGUOUS",
  INTERFACE_CONFIG_VALUE_OUT_OF_RANGE:
    "error.reason.INTERFACE_CONFIG_VALUE_OUT_OF_RANGE",
  INTERFACE_CUSTOM_JOB_ID_REQUIRED:
    "error.reason.INTERFACE_CUSTOM_JOB_ID_REQUIRED",
  INTERFACE_CUSTOM_JOB_ID_UNEXPECTED:
    "error.reason.INTERFACE_CUSTOM_JOB_ID_UNEXPECTED",
  INTERFACE_DISCOVERY_FACET_UNSUPPORTED:
    "error.reason.INTERFACE_DISCOVERY_FACET_UNSUPPORTED",
  INTERFACE_DISCOVERY_FILTER_BLANK:
    "error.reason.INTERFACE_DISCOVERY_FILTER_BLANK",
  INTERFACE_DISCOVERY_LIMIT_INVALID:
    "error.reason.INTERFACE_DISCOVERY_LIMIT_INVALID",
  INTERFACE_DISCOVERY_RATING_INVALID:
    "error.reason.INTERFACE_DISCOVERY_RATING_INVALID",
  INTERFACE_DISCOVERY_SURFACE_UNSUPPORTED:
    "error.reason.INTERFACE_DISCOVERY_SURFACE_UNSUPPORTED",
  INTERFACE_DISCOVERY_YEARS_INVALID:
    "error.reason.INTERFACE_DISCOVERY_YEARS_INVALID",
  INTERFACE_DOWNLOAD_CLIENT_NOT_FOUND:
    "error.reason.INTERFACE_DOWNLOAD_CLIENT_NOT_FOUND",
  INTERFACE_DOWNLOAD_CLIENT_PROVIDER_MISMATCH:
    "error.reason.INTERFACE_DOWNLOAD_CLIENT_PROVIDER_MISMATCH",
  INTERFACE_DOWNLOAD_ID_INVALID: "error.reason.INTERFACE_DOWNLOAD_ID_INVALID",
  INTERFACE_HISTORY_EVENT_TYPE_INVALID:
    "error.reason.INTERFACE_HISTORY_EVENT_TYPE_INVALID",
  INTERFACE_IDEMPOTENCY_KEY_DUPLICATE:
    "error.reason.INTERFACE_IDEMPOTENCY_KEY_DUPLICATE",
  INTERFACE_IDEMPOTENCY_KEY_REQUIRED:
    "error.reason.INTERFACE_IDEMPOTENCY_KEY_REQUIRED",
  INTERFACE_IMPORT_LIBRARY_FACET_MISMATCH:
    "error.reason.INTERFACE_IMPORT_LIBRARY_FACET_MISMATCH",
  INTERFACE_IMPORT_MAPPING_DUPLICATE:
    "error.reason.INTERFACE_IMPORT_MAPPING_DUPLICATE",
  INTERFACE_IMPORT_MAPPING_FACET_MISMATCH:
    "error.reason.INTERFACE_IMPORT_MAPPING_FACET_MISMATCH",
  INTERFACE_IMPORT_MAPPING_INCOMPLETE:
    "error.reason.INTERFACE_IMPORT_MAPPING_INCOMPLETE",
  INTERFACE_IMPORT_MAPPING_REQUIRED:
    "error.reason.INTERFACE_IMPORT_MAPPING_REQUIRED",
  INTERFACE_IMPORT_MAPPING_ROOT_UNKNOWN:
    "error.reason.INTERFACE_IMPORT_MAPPING_ROOT_UNKNOWN",
  INTERFACE_IMPORT_MAPPING_SOURCE_INCOMPLETE:
    "error.reason.INTERFACE_IMPORT_MAPPING_SOURCE_INCOMPLETE",
  INTERFACE_IMPORT_MAPPING_SOURCE_MISMATCH:
    "error.reason.INTERFACE_IMPORT_MAPPING_SOURCE_MISMATCH",
  INTERFACE_IMPORT_MAPPING_SOURCE_UNSELECTED:
    "error.reason.INTERFACE_IMPORT_MAPPING_SOURCE_UNSELECTED",
  INTERFACE_IMPORT_PROWLARR_SOURCE_CONFLICT:
    "error.reason.INTERFACE_IMPORT_PROWLARR_SOURCE_CONFLICT",
  INTERFACE_IMPORT_ROOT_LIBRARY_MISMATCH:
    "error.reason.INTERFACE_IMPORT_ROOT_LIBRARY_MISMATCH",
  INTERFACE_IMPORT_SOURCE_NOT_READY:
    "error.reason.INTERFACE_IMPORT_SOURCE_NOT_READY",
  INTERFACE_IMPORT_SOURCE_REQUIRED:
    "error.reason.INTERFACE_IMPORT_SOURCE_REQUIRED",
  INTERFACE_IMPORT_URL_REQUIRED: "error.reason.INTERFACE_IMPORT_URL_REQUIRED",
  INTERFACE_INDEXER_ERROR_LIMIT_INVALID:
    "error.reason.INTERFACE_INDEXER_ERROR_LIMIT_INVALID",
  INTERFACE_INTERACTIVE_SESSION_REQUIRED:
    "error.reason.INTERFACE_INTERACTIVE_SESSION_REQUIRED",
  INTERFACE_LIST_EPISODE_POLICY_INVALID:
    "error.reason.INTERFACE_LIST_EPISODE_POLICY_INVALID",
  INTERFACE_LIST_EXCLUSIONS_INVALID:
    "error.reason.INTERFACE_LIST_EXCLUSIONS_INVALID",
  INTERFACE_LIST_EXCLUSION_TARGET_REQUIRED:
    "error.reason.INTERFACE_LIST_EXCLUSION_TARGET_REQUIRED",
  INTERFACE_LIST_FACET_REQUIRED: "error.reason.INTERFACE_LIST_FACET_REQUIRED",
  INTERFACE_LIST_LANGUAGE_UNSUPPORTED:
    "error.reason.INTERFACE_LIST_LANGUAGE_UNSUPPORTED",
  INTERFACE_LIST_NOT_PUBLIC: "error.reason.INTERFACE_LIST_NOT_PUBLIC",
  INTERFACE_LIST_RATING_LIMIT_EXCEEDED:
    "error.reason.INTERFACE_LIST_RATING_LIMIT_EXCEEDED",
  INTERFACE_LIST_RATING_OUT_OF_RANGE:
    "error.reason.INTERFACE_LIST_RATING_OUT_OF_RANGE",
  INTERFACE_LIST_RATING_SCALE_REQUIRED:
    "error.reason.INTERFACE_LIST_RATING_SCALE_REQUIRED",
  INTERFACE_LIST_RATING_SOURCE_UNSUPPORTED:
    "error.reason.INTERFACE_LIST_RATING_SOURCE_UNSUPPORTED",
  INTERFACE_LIST_RATING_VALUE_REQUIRED:
    "error.reason.INTERFACE_LIST_RATING_VALUE_REQUIRED",
  INTERFACE_LOCATION_DESTINATION_INVALID:
    "error.reason.INTERFACE_LOCATION_DESTINATION_INVALID",
  INTERFACE_LOCATION_TARGET_INVALID:
    "error.reason.INTERFACE_LOCATION_TARGET_INVALID",
  INTERFACE_LOGIN_MFA_CONFLICT: "error.reason.INTERFACE_LOGIN_MFA_CONFLICT",
  INTERFACE_LOGIN_MFA_REQUIRED: "error.reason.INTERFACE_LOGIN_MFA_REQUIRED",
  INTERFACE_MAINTENANCE_RULE_NOT_FOUND:
    "error.reason.INTERFACE_MAINTENANCE_RULE_NOT_FOUND",
  INTERFACE_METADATA_DATE_INVALID:
    "error.reason.INTERFACE_METADATA_DATE_INVALID",
  INTERFACE_METADATA_ID_INVALID: "error.reason.INTERFACE_METADATA_ID_INVALID",
  INTERFACE_NOTIFICATION_CHANNEL_NOT_FOUND:
    "error.reason.INTERFACE_NOTIFICATION_CHANNEL_NOT_FOUND",
  INTERFACE_PROXY_PROTOCOL_UNSUPPORTED:
    "error.reason.INTERFACE_PROXY_PROTOCOL_UNSUPPORTED",
  INTERFACE_PROXY_PROVIDER_UNSUPPORTED:
    "error.reason.INTERFACE_PROXY_PROVIDER_UNSUPPORTED",
  INTERFACE_PROXY_TIMEOUT_INVALID:
    "error.reason.INTERFACE_PROXY_TIMEOUT_INVALID",
  INTERFACE_QUALITY_PROFILE_ID_REQUIRED:
    "error.reason.INTERFACE_QUALITY_PROFILE_ID_REQUIRED",
  INTERFACE_QUALITY_PROFILE_NAME_REQUIRED:
    "error.reason.INTERFACE_QUALITY_PROFILE_NAME_REQUIRED",
  INTERFACE_QUALITY_PROFILE_TIERS_REQUIRED:
    "error.reason.INTERFACE_QUALITY_PROFILE_TIERS_REQUIRED",
  INTERFACE_QUALITY_PROFILE_VALUE_INVALID:
    "error.reason.INTERFACE_QUALITY_PROFILE_VALUE_INVALID",
  INTERFACE_RELEASE_EXTRA_INVALID:
    "error.reason.INTERFACE_RELEASE_EXTRA_INVALID",
  INTERFACE_RELEASE_SEARCH_EPISODE_INCOMPLETE:
    "error.reason.INTERFACE_RELEASE_SEARCH_EPISODE_INCOMPLETE",
  INTERFACE_RELEASE_SEARCH_MOVIE_SCOPE_INVALID:
    "error.reason.INTERFACE_RELEASE_SEARCH_MOVIE_SCOPE_INVALID",
  INTERFACE_RELEASE_SEARCH_TITLE_REQUIRED:
    "error.reason.INTERFACE_RELEASE_SEARCH_TITLE_REQUIRED",
  INTERFACE_REQUEST_LEASE_CONFLICT:
    "error.reason.INTERFACE_REQUEST_LEASE_CONFLICT",
  INTERFACE_RESERVED_TAG: "error.reason.INTERFACE_RESERVED_TAG",
  INTERFACE_RESTORE_SETUP_REQUIRED:
    "error.reason.INTERFACE_RESTORE_SETUP_REQUIRED",
  INTERFACE_RESTORE_UNAVAILABLE: "error.reason.INTERFACE_RESTORE_UNAVAILABLE",
  INTERFACE_RESTORE_UPLOAD_ID_INVALID:
    "error.reason.INTERFACE_RESTORE_UPLOAD_ID_INVALID",
  INTERFACE_RESTORE_UPLOAD_INVALID:
    "error.reason.INTERFACE_RESTORE_UPLOAD_INVALID",
  INTERFACE_RESTORE_UPLOAD_NOT_FOUND:
    "error.reason.INTERFACE_RESTORE_UPLOAD_NOT_FOUND",
  INTERFACE_ROOT_FOLDER_REQUIRED: "error.reason.INTERFACE_ROOT_FOLDER_REQUIRED",
  INTERFACE_RULE_EXCLUSION_SUBJECT_REQUIRED:
    "error.reason.INTERFACE_RULE_EXCLUSION_SUBJECT_REQUIRED",
  INTERFACE_RULE_PREVIEW_TARGET_CONFLICT:
    "error.reason.INTERFACE_RULE_PREVIEW_TARGET_CONFLICT",
  INTERFACE_RULE_PREVIEW_TARGET_REQUIRED:
    "error.reason.INTERFACE_RULE_PREVIEW_TARGET_REQUIRED",
  INTERFACE_RULE_SCOPE_CONFLICT: "error.reason.INTERFACE_RULE_SCOPE_CONFLICT",
  INTERFACE_RULE_SCOPE_REQUIRED: "error.reason.INTERFACE_RULE_SCOPE_REQUIRED",
  INTERFACE_RULE_SOURCE_REQUIRED: "error.reason.INTERFACE_RULE_SOURCE_REQUIRED",
  INTERFACE_RULE_SUBJECT_SELECTION_INVALID:
    "error.reason.INTERFACE_RULE_SUBJECT_SELECTION_INVALID",
  INTERFACE_SCRIPT_ACKNOWLEDGEMENT_REQUIRED:
    "error.reason.INTERFACE_SCRIPT_ACKNOWLEDGEMENT_REQUIRED",
  INTERFACE_SCRIPT_NOT_FOUND: "error.reason.INTERFACE_SCRIPT_NOT_FOUND",
  INTERFACE_SCRIPT_PATH_NOT_ABSOLUTE:
    "error.reason.INTERFACE_SCRIPT_PATH_NOT_ABSOLUTE",
  INTERFACE_SCRIPT_PATH_REQUIRED: "error.reason.INTERFACE_SCRIPT_PATH_REQUIRED",
  INTERFACE_SCRIPT_SCHEDULE_INCOMPLETE:
    "error.reason.INTERFACE_SCRIPT_SCHEDULE_INCOMPLETE",
  INTERFACE_SCRIPT_TRIGGER_IMMUTABLE:
    "error.reason.INTERFACE_SCRIPT_TRIGGER_IMMUTABLE",
  INTERFACE_SCRIPT_TYPE_INVALID: "error.reason.INTERFACE_SCRIPT_TYPE_INVALID",
  INTERFACE_SUBTITLE_DELETE_CONFIRMATION_REQUIRED:
    "error.reason.INTERFACE_SUBTITLE_DELETE_CONFIRMATION_REQUIRED",
  INTERFACE_TAG_NOT_FOUND: "error.reason.INTERFACE_TAG_NOT_FOUND",
  INTERFACE_TIMESTAMP_INVALID: "error.reason.INTERFACE_TIMESTAMP_INVALID",
  INTERFACE_UPGRADE_INELIGIBLE: "error.reason.INTERFACE_UPGRADE_INELIGIBLE",

  LIBRARY_SCAN_ALREADY_RUNNING: "error.libraryScanAlreadyRunning",
  EXTERNAL_IMPORT_SESSION_GONE: "error.externalImportSessionGone",
  TAG_ALREADY_EXISTS: "error.tagAlreadyExists",
  RULE_TARGETS_PERSON: "error.ruleTargetsPerson",
  RULE_PACK_UP_TO_DATE: "error.rulePackUpToDate",
  TITLE_NOT_FOUND: "title.notFound",
  RELEASE_TOKEN_EXPIRED: "status.searchResultsExpired",
  MAINTENANCE_ARM_COUNT_MISMATCH: "error.maintenanceArmCountMismatch",
};
const ERROR_CODE_KEYS: Readonly<Record<string, string>> = {
  MFA_STEP_UP_REQUIRED: "error.mfaStepUpRequired",
  INTERNAL_ERROR: "error.internal",
  REAUTHENTICATION_REQUIRED: "error.reauthenticationRequired",
  RATE_LIMITED: "error.rateLimited",
  AUTHENTICATION_REQUIRED: "error.authenticationRequired",
};

function reasonMatchesErrorCode(
  reason: unknown,
  code: unknown,
): reason is string {
  return (
    typeof reason === "string" &&
    typeof code === "string" &&
    Boolean(PUBLIC_ERROR_REASON_CODES.get(reason)?.includes(code))
  );
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function firstGraphQlErrorMessage(
  error: CombinedError | unknown,
): string | null {
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

export function hasGraphQlErrorCode(
  error: CombinedError | unknown,
  code: string,
): boolean {
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

/** A string extension of the first GraphQL error carrying `code`. */
export function graphQlErrorExtension(
  error: CombinedError | unknown,
  code: string,
  key: string,
): string | null {
  if (!isRecord(error) || !Array.isArray(error.graphQLErrors)) {
    return null;
  }
  for (const graphQlError of error.graphQLErrors) {
    if (!isRecord(graphQlError) || !isRecord(graphQlError.extensions)) {
      continue;
    }
    const value = graphQlError.extensions[key];
    if (graphQlError.extensions.code === code && typeof value === "string") {
      return value;
    }
  }
  return null;
}

/** Whether an error carries the expected coarse code and stable reason together. */
export function hasGraphQlErrorReason(
  error: unknown,
  reason: string,
  code: string,
): boolean {
  return (
    isRecord(error) &&
    Array.isArray(error.graphQLErrors) &&
    error.graphQLErrors.some(
      (entry) =>
        isRecord(entry) &&
        isRecord(entry.extensions) &&
        entry.extensions.reason === reason &&
        entry.extensions.code === code,
    )
  );
}

export function graphQlErrorNumberExtension(
  error: unknown,
  code: string,
  reason: string,
  key: string,
): number | null {
  if (!isRecord(error) || !Array.isArray(error.graphQLErrors)) return null;
  for (const entry of error.graphQLErrors) {
    if (
      !isRecord(entry) ||
      !isRecord(entry.extensions) ||
      entry.extensions.reason !== reason ||
      entry.extensions.code !== code
    )
      continue;
    const value = entry.extensions[key];
    if (typeof value === "number" && Number.isSafeInteger(value) && value >= 0)
      return value;
  }
  return null;
}

export function isLostWarmupSessionError(error: unknown): boolean {
  return hasGraphQlErrorReason(
    error,
    "EXTERNAL_IMPORT_SESSION_GONE",
    "NOT_FOUND",
  );
}

function internalGraphQlErrorMessage(
  error: CombinedError | unknown,
  translate: ErrorTranslate,
): string | null {
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
    return errorId
      ? translate("error.internalReference", { id: errorId })
      : translate("error.internal");
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
  return normalized;
}

export function userFacingGraphQlErrorMessage(
  error: unknown,
  fallback: string,
  translate: ErrorTranslate = defaultTranslate,
): string {
  const internalMessage = internalGraphQlErrorMessage(error, translate);
  if (internalMessage) {
    return internalMessage;
  }

  if (isRecord(error) && error.networkError)
    return translate("error.disconnected");
  if (isRecord(error) && Array.isArray(error.graphQLErrors)) {
    for (const entry of error.graphQLErrors) {
      if (!isRecord(entry) || !isRecord(entry.extensions)) continue;
      const { reason, code } = entry.extensions;
      const key = reasonMatchesErrorCode(reason, code)
        ? ERROR_REASON_KEYS[reason]
        : undefined;
      const codeKey =
        typeof code === "string" ? ERROR_CODE_KEYS[code] : undefined;
      if (key && reason === "SECURITY_PASSWORD_OUT_OF_RANGE") {
        const value = entry.extensions.minLength;
        const minLength =
          typeof value === "number" ||
          (typeof value === "string" && /^[0-9]+$/.test(value))
            ? Number(value)
            : NaN;
        if (Number.isSafeInteger(minLength) && minLength > 0) {
          return translate(key, { minLength });
        }
        return translate("error.passwordLengthRequirement");
      }
      if (key || codeKey) return translate(key ?? codeKey!);
    }
  }
  const rawMessage =
    firstGraphQlErrorMessage(error) ??
    fallbackErrorMessage(error) ??
    fallback.trim();
  const normalized = normalizeGraphQlErrorMessage(rawMessage);
  return normalized || fallback.trim();
}
