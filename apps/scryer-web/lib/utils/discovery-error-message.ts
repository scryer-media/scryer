import type { Translate } from "@/components/root/types";
import {
  graphQlErrorExtension,
  userFacingGraphQlErrorMessage,
} from "../graphql/error-message.ts";

/**
 * The server names why it refused a discovery request: a `VALIDATION_ERROR`
 * carries the condition as `extensions.reason`, beside the English sentence.
 * Each reason listed here is said in the viewer's language; any other failure
 * shows the server's sentence, or the caller's fallback.
 */
export const DISCOVERY_REFUSAL_KEYS: Readonly<Record<string, string>> = {
  DISCOVERY_NEIGHBOR_TARGET_KEY_UNSUPPORTED:
    "discovery.refusal.neighborTargetKeyUnsupported",
  DISCOVERY_NEIGHBOR_TARGET_KEY_FACET_MISMATCH:
    "discovery.refusal.neighborTargetKeyFacetMismatch",
};

/** The translation key for the reason a discovery request was refused, if known. */
export function discoveryRefusalKey(error: unknown): string | null {
  const reason = graphQlErrorExtension(error, "VALIDATION_ERROR", "reason");
  return reason !== null && Object.hasOwn(DISCOVERY_REFUSAL_KEYS, reason)
    ? DISCOVERY_REFUSAL_KEYS[reason]
    : null;
}

/**
 * What to show for a failed discovery request: the translated reason when the
 * server named one this client knows, otherwise the server's own sentence, and
 * `fallback` when the failure carries no usable sentence.
 */
export function discoveryErrorMessage(
  error: unknown,
  t: Translate,
  fallback: string,
): string {
  const key = discoveryRefusalKey(error);
  return key ? t(key) : userFacingGraphQlErrorMessage(error, fallback, t);
}
