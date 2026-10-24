import type { Translate } from "@/components/root/types";
import {
  graphQlErrorExtension,
  userFacingGraphQlErrorMessage,
} from "../graphql/error-message.ts";

/**
 * Why the server could not show an indexer's sign-in image. Each reason
 * travels as `extensions.reason` on a `VALIDATION_ERROR`, beside the English
 * sentence, and is said here in the viewer's language. Any other failure, such
 * as the tracker refusing the connection or the plugin's own coded error,
 * shows the server's sentence.
 */
export const LOGIN_CHALLENGE_REFUSAL_KEYS: Readonly<Record<string, string>> = {
  INDEXER_LOGIN_CHALLENGE_MALFORMED: "settings.indexerLoginChallengeRefusal.malformed",
  INDEXER_LOGIN_CHALLENGE_KIND_UNSUPPORTED:
    "settings.indexerLoginChallengeRefusal.kindUnsupported",
  INDEXER_LOGIN_CHALLENGE_IMAGE_TYPE_UNSUPPORTED:
    "settings.indexerLoginChallengeRefusal.imageTypeUnsupported",
  INDEXER_LOGIN_CHALLENGE_IMAGE_TOO_LARGE:
    "settings.indexerLoginChallengeRefusal.imageTooLarge",
};

/** The translation key for why a sign-in image could not be shown, if known. */
export function loginChallengeRefusalKey(error: unknown): string | null {
  const reason = graphQlErrorExtension(error, "VALIDATION_ERROR", "reason");
  return reason !== null && Object.hasOwn(LOGIN_CHALLENGE_REFUSAL_KEYS, reason)
    ? LOGIN_CHALLENGE_REFUSAL_KEYS[reason]
    : null;
}

/**
 * What to show when a sign-in image could not be fetched: the translated
 * reason when the server named one this client knows, otherwise the server's
 * own sentence, and `fallback` when the failure carries no usable sentence.
 */
export function loginChallengeErrorMessage(
  error: unknown,
  t: Translate,
  fallback: string,
): string {
  const key = loginChallengeRefusalKey(error);
  return key ? t(key) : userFacingGraphQlErrorMessage(error, fallback);
}
