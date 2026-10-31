import type { Translate } from "@/components/root/types";
import { graphQlErrorExtension } from "../graphql/error-message.ts";
import { pluginErrorMessage } from "./plugin-error-message.ts";

/**
 * Why an HTTP-family release cannot be grabbed or downloaded. The server sends
 * these as `extensions.reason` on a `VALIDATION_ERROR`; each is said in the
 * viewer's language, and a reason this client does not know shows the
 * server's sentence.
 */
export const HTTP_RELEASE_REFUSAL_KEYS: Readonly<Record<string, string>> = {
  HTTP_RELEASE_DETAILS_UNAVAILABLE: "httpRelease.refusal.detailsUnavailable",
  HTTP_RELEASE_LIVE: "httpRelease.refusal.live",
  HTTP_RELEASE_UPCOMING: "httpRelease.refusal.upcoming",
  HTTP_RELEASE_REMOVED: "httpRelease.refusal.removed",
  HTTP_RELEASE_UNSUPPORTED_TRANSPORT:
    "httpRelease.refusal.unsupportedTransport",
  HTTP_RELEASE_SEPARATE_TRACKS: "httpRelease.refusal.separateTracks",
  HTTP_RELEASE_NO_MEDIA_RESOURCE: "httpRelease.refusal.noMediaResource",
  HTTP_RELEASE_NO_SUBTITLE_RESOURCE: "httpRelease.refusal.noSubtitleResource",
  HTTP_RELEASE_NO_AUDIO_RESOURCE: "httpRelease.refusal.noAudioResource",
  HTTP_RELEASE_UNSUPPORTED_AUTH: "httpRelease.refusal.unsupportedAuth",
  HTTP_RELEASE_UNKNOWN_CONTAINER: "httpRelease.refusal.unknownContainer",
  HTTP_RELEASE_DESTINATION_EXISTS: "httpRelease.refusal.destinationExists",
  HTTP_RELEASE_PUBLICATION_FAILED: "httpRelease.refusal.publicationFailed",
  HTTP_RELEASE_RECOVERY_UNVERIFIED: "httpRelease.refusal.recoveryUnverified",
};

/** The translation key for an HTTP release refusal, if the error is one this client knows. */
export function httpReleaseRefusalKey(error: unknown): string | null {
  const reason = graphQlErrorExtension(error, "VALIDATION_ERROR", "reason");
  return reason !== null && Object.hasOwn(HTTP_RELEASE_REFUSAL_KEYS, reason)
    ? HTTP_RELEASE_REFUSAL_KEYS[reason]
    : null;
}

/**
 * What to show when queueing a release fails: a known HTTP release refusal in
 * the viewer's language, otherwise whatever a plugin failure or the server's
 * own sentence says.
 */
export function grabErrorMessage(
  error: unknown,
  t: Translate,
  fallback: string,
): string {
  const key = httpReleaseRefusalKey(error);
  return key ? t(key) : pluginErrorMessage(error, t, fallback);
}
