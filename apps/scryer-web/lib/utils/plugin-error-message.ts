import type { Translate } from "@/components/root/types";
import {
  graphQlErrorExtension,
  userFacingGraphQlErrorMessage,
} from "../graphql/error-message.ts";

/**
 * A failed plugin call reaches the client as a `PLUGIN_FAILED` error whose
 * `extensions.reason` names why it failed, beside the server's English
 * sentence. The plugin's own sentence, when it said something specific, rides
 * as `extensions.detail`. Each reason listed here is said in the viewer's
 * language, with the plugin's sentence after it.
 *
 * Reasons are open: a plugin that reports a new code `foo_bar` reaches the
 * client as `PLUGIN_FOO_BAR` with no SDK or server change, so translating it
 * is one entry here plus its string in every locale. Until that entry exists
 * the plugin's own sentence is shown, or a translated "the plugin reported a
 * problem" naming the code when it gave none.
 */
export const PLUGIN_FAILURE_KEYS: Readonly<Record<string, string>> = {
  PLUGIN_INVALID_CONFIG: "plugins.failure.invalidConfig",
  PLUGIN_AUTH_FAILED: "plugins.failure.authFailed",
  PLUGIN_RATE_LIMITED: "plugins.failure.rateLimited",
  PLUGIN_UPSTREAM_UNAVAILABLE: "plugins.failure.upstreamUnavailable",
  PLUGIN_UNSUPPORTED: "plugins.failure.unsupported",
  PLUGIN_TEMPORARY_FAILURE: "plugins.failure.temporary",
  PLUGIN_PERMANENT_FAILURE: "plugins.failure.permanent",
  PLUGIN_HOST_FAILURE: "plugins.failure.hostFailure",
  PLUGIN_TIMED_OUT: "plugins.failure.timedOut",
  PLUGIN_RESOURCE_LIMIT: "plugins.failure.resourceLimit",
  PLUGIN_INVALID_USERNAME_PASSWORD: "plugins.failure.invalidUsernamePassword",
  PLUGIN_INVALID_API_KEY: "plugins.failure.invalidApiKey",
  PLUGIN_INVALID_PASSKEY: "plugins.failure.invalidPasskey",
  PLUGIN_INVALID_COOKIE: "plugins.failure.invalidCookie",
  PLUGIN_CAPTCHA_REQUIRED: "plugins.failure.captchaRequired",
  PLUGIN_TWO_FACTOR_REQUIRED: "plugins.failure.twoFactorRequired",
  PLUGIN_ACCOUNT_BANNED: "plugins.failure.accountBanned",
  PLUGIN_ACCOUNT_DISABLED: "plugins.failure.accountDisabled",
  PLUGIN_ENDPOINT_NOT_FOUND: "plugins.failure.endpointNotFound",
  PLUGIN_TLS_ERROR: "plugins.failure.tlsError",
  PLUGIN_INVALID_RESPONSE: "plugins.failure.invalidResponse",
  PLUGIN_QUOTA_EXCEEDED: "plugins.failure.quotaExceeded",
  PLUGIN_ITEM_NOT_FOUND: "plugins.failure.itemNotFound",
  PLUGIN_DOWNLOAD_REJECTED: "plugins.failure.downloadRejected",
  PLUGIN_PATH_NOT_WRITABLE: "plugins.failure.pathNotWritable",
};

const PLUGIN_REASON_PREFIX = "PLUGIN_";

const PLUGIN_FAILED = "PLUGIN_FAILED";

/** The translation key for the reason a plugin call failed, if known. */
export function pluginFailureKey(error: unknown): string | null {
  const reason = graphQlErrorExtension(error, PLUGIN_FAILED, "reason");
  return reason !== null && Object.hasOwn(PLUGIN_FAILURE_KEYS, reason)
    ? PLUGIN_FAILURE_KEYS[reason]
    : null;
}

/**
 * A plugin failure reason this client has no translation for, if the error is
 * one: a reason the plugin named with a code newer than this client.
 */
export function unknownPluginFailureReason(error: unknown): string | null {
  const reason = graphQlErrorExtension(error, PLUGIN_FAILED, "reason");
  if (
    reason === null ||
    !reason.startsWith(PLUGIN_REASON_PREFIX) ||
    reason.length === PLUGIN_REASON_PREFIX.length ||
    Object.hasOwn(PLUGIN_FAILURE_KEYS, reason)
  ) {
    return null;
  }
  return reason;
}

/** `PLUGIN_FOO_BAR` as words a person can read: `foo bar`. */
export function humanisePluginReason(reason: string): string {
  return reason
    .slice(PLUGIN_REASON_PREFIX.length)
    .split("_")
    .filter(Boolean)
    .join(" ")
    .toLowerCase();
}

/** The plugin's own sentence about a failed call, when it gave one. */
export function pluginFailureDetail(error: unknown): string | null {
  const detail = graphQlErrorExtension(error, PLUGIN_FAILED, "detail")?.trim();
  return detail ? detail : null;
}

/**
 * What to show for a failed plugin call: the translated reason, followed by
 * the plugin's own sentence when it gave one, when the server named a reason
 * this client knows. A plugin reason this client does not know shows the
 * plugin's own sentence alone, or a translated "the plugin reported a problem"
 * naming the code when it gave none. Any other error shows the server's own
 * sentence, and `fallback` when it carries no usable sentence.
 */
export function pluginErrorMessage(error: unknown, t: Translate, fallback: string): string {
  const key = pluginFailureKey(error);
  const detail = pluginFailureDetail(error);
  if (key) {
    const reason = t(key);
    return detail ? t("plugins.failure.withDetail", { reason, detail }) : reason;
  }
  const unknownReason = unknownPluginFailureReason(error);
  if (unknownReason) {
    return detail ?? t("plugins.failure.unknown", { code: humanisePluginReason(unknownReason) });
  }
  return userFacingGraphQlErrorMessage(error, fallback);
}
