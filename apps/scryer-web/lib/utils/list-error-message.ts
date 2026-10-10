import type { Translate } from "@/components/root/types";
import {
  graphQlErrorExtension,
  userFacingGraphQlErrorMessage,
} from "../graphql/error-message.ts";

/**
 * The server names why it refused a lists request: a `VALIDATION_ERROR`
 * carries the condition as `extensions.reason`, beside the English sentence.
 * Each reason listed here is said in the viewer's language; any other failure,
 * with or without a reason, shows the server's sentence as before.
 *
 * Some reasons are left out on purpose. Their sentence names the option, title
 * kind or library that was wrong (LIST_PARAM_REQUIRED, LIST_PARAM_INVALID,
 * LIST_KIND_NOT_IN_LIST, LIST_ROUTE_KIND_NOT_KEPT, LIST_ROUTE_KIND_DUPLICATED,
 * LIST_ROUTE_LIBRARY_KIND_MISMATCH, LIST_PROVIDER_SETTING_UNKNOWN), which a
 * fixed translation would drop, or they describe a request this client never
 * makes (LIST_PARAM_UNKNOWN, LIST_ACCOUNT_LINK_NOT_POLLED).
 */
export const LIST_REFUSAL_KEYS: Readonly<Record<string, string>> = {
  LIST_ALREADY_FOLLOWED: "lists.refusal.alreadyFollowed",
  LIST_SOURCE_REQUIRED: "lists.refusal.sourceRequired",
  LIST_LINK_NOT_RECOGNIZED: "lists.refusal.linkNotRecognized",
  LIST_PROVIDER_NOT_INSTALLED: "lists.refusal.providerNotInstalled",
  LIST_PROVIDER_UNAVAILABLE: "lists.refusal.providerUnavailable",
  LIST_SOURCE_NOT_OFFERED: "lists.refusal.sourceNotOffered",
  LIST_CHART_UNAVAILABLE: "lists.refusal.chartUnavailable",
  LIST_MEMBER_ACCOUNT_ONLY: "lists.refusal.memberAccountOnly",
  LIST_MODE_NOT_ALLOWED: "lists.refusal.modeNotAllowed",
  LIST_KINDS_REQUIRED: "lists.refusal.kindsRequired",
  // Both leave a title kind with nowhere to go, so they read the same.
  LIST_ROUTE_MISSING_FOR_KIND: "lists.refusal.routeLibraryRequired",
  LIST_ROUTE_LIBRARY_REQUIRED: "lists.refusal.routeLibraryRequired",
  LIST_ROUTE_LIBRARY_NOT_FOUND: "lists.refusal.routeLibraryNotFound",
  LIST_SYNC_CAP_INVALID: "lists.refusal.syncCapInvalid",
  LIST_DISABLED: "lists.refusal.disabled",
  LIST_EXCLUSION_IDS_REQUIRED: "lists.refusal.exclusionIdsRequired",
  LIST_EXCLUSION_TITLE_REQUIRED: "lists.refusal.exclusionTitleRequired",
  LIST_ACCOUNT_REQUIRED: "lists.refusal.accountRequired",
  LIST_ACCOUNT_RECONNECT_REQUIRED: "lists.refusal.accountReconnectRequired",
  LIST_ACCOUNT_PROVIDER_MISMATCH: "lists.refusal.accountProviderMismatch",
  LIST_ACCOUNT_IDENTITY_MISSING: "lists.refusal.accountIdentityMissing",
  LIST_ACCOUNT_IDENTITY_UNVERIFIED: "lists.refusal.accountIdentityUnverified",
  LIST_ACCOUNT_LINKING_UNSUPPORTED: "lists.refusal.accountLinkingUnsupported",
  LIST_ACCOUNT_LINK_ORIGIN_INVALID: "lists.refusal.accountLinkOriginInvalid",
  LIST_ACCOUNT_LINKS_TOO_MANY: "lists.refusal.accountLinksTooMany",
  LIST_ACCOUNT_LINK_RESULT_INVALID: "lists.refusal.accountLinkResultInvalid",
  LIST_ACCOUNT_LINK_ISSUER_INVALID: "lists.refusal.accountLinkIssuerInvalid",
  LIST_ACCOUNT_AUTH_UNAVAILABLE: "lists.refusal.accountAuthUnavailable",
  LIST_PROVIDER_APP_UNSUPPORTED: "lists.refusal.providerAppUnsupported",
  LIST_PROVIDER_APP_REDIRECT_INVALID: "lists.refusal.providerAppRedirectInvalid",
  LIST_PROVIDER_APP_INCOMPLETE: "lists.refusal.providerAppIncomplete",
};

/** The translation key for the reason a lists request was refused, if known. */
export function listRefusalKey(error: unknown): string | null {
  const reason = graphQlErrorExtension(error, "VALIDATION_ERROR", "reason");
  return reason !== null && Object.hasOwn(LIST_REFUSAL_KEYS, reason)
    ? LIST_REFUSAL_KEYS[reason]
    : null;
}

/**
 * What to show for a failed lists request: the translated reason when the
 * server named one this client knows, otherwise the server's own sentence, and
 * `fallback` when the failure carries no usable sentence.
 */
export function listErrorMessage(error: unknown, t: Translate, fallback: string): string {
  const key = listRefusalKey(error);
  return key ? t(key) : userFacingGraphQlErrorMessage(error, fallback);
}
