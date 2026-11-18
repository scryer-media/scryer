import {
  ERROR_REASON_KEYS,
  refusalReasonMessage,
  type ErrorTranslate,
} from "../graphql/error-message.ts";

/**
 * Reasons a failed title in a location transfer is explained by in the
 * client's own words. Each one's copy says everything the server sentence
 * does, or names the one path it carries through a fact the panel shows.
 *
 * Left to the server sentence on purpose, because their sentence carries
 * something the copy would drop: LOCATION_DESTINATION_OCCUPIED (the occupied
 * path), LOCATION_VERIFICATION_MISMATCH (the verifier's detail),
 * LOCATION_CONFLICT_RECYCLE_UNAVAILABLE (the recycle bin's error),
 * LOCATION_VERIFICATION_FAILED, LOCATION_TRANSFERRED_FILE_*,
 * LOCATION_COULD_NOT_COMPARE, LOCATION_MERGE_UNAVAILABLE,
 * LOCATION_CONFIRMED_PLAN_NO_INSTRUCTIONS_TITLE,
 * LOCATION_SOURCE_DESTINATION_CHANGED_BEFORE_CLEANUP, the FOLDER_OWNERSHIP_*
 * reasons that name another title, every LOCATION_RECOVERY_* reason, and the
 * DISK_* reasons, whose sentence names the path they failed on.
 */
export const LOCATION_TITLE_REFUSAL_REASONS: ReadonlySet<string> = new Set([
  "LOCATION_SOURCE_DESTINATION_SAME_PATH",
  "LOCATION_SOURCE_CHANGED_DURING_FILE_PLACEMENT",
  "LOCATION_FILE_CHANGED_BEFORE_SOURCE_CLEANUP",
  "LOCATION_SOURCE_CHANGED_BEFORE_CLEANUP",
  "LOCATION_RELATED_MEDIA_FILE_COULD_NOT_TRANSFERRED",
  "LOCATION_RELATED_MEDIA_TRANSFER_INTERRUPTED",
  "LOCATION_DESTINATION_CHANGED_DURING_COMPARISON",
  "LOCATION_SOURCE_CHANGED_DURING_TRANSFER",
  "LOCATION_EXISTING_PATH_NOT_REGULAR_FILE",
  "LOCATION_FILE_CHANGED_WHILE_BEING_COMPARED",
  "LOCATION_DESTINATION_COPY_NO_LONGER_HOLDS_CONTENT_MOVE_PROVED_IDENTICAL_SOURCE",
  "LOCATION_DESTINATION_NO_SPACE",
  "LOCATION_DESTINATION_PERMISSION_DENIED",
  "LOCATION_UNPROVEN_LINK_NOT_REMOVED",
  "LOCATION_SET_ASIDE_SOURCE_NOT_PUT_BACK",
  "FOLDER_OWNERSHIP_TITLE_FOLDER_NO_EXISTING_ANCESTOR",
  "LOCATION_CONFLICT_UNSUPPORTED",
  "LOCATION_CONFLICT_OWNERSHIP_CHANGED",
  "LOCATION_CONFLICT_FOLDER_UNSAFE",
  "LOCATION_CONFLICT_PATH_MISSING",
  "LOCATION_CONFLICT_PATH_NOT_DIRECTORY",
  "LOCATION_CONFLICT_PATH_PERMISSION_DENIED",
  "LOCATION_CONFLICT_PATH_IO_FAILED",
  "LOCATION_CONFLICT_MEDIA_MISSING",
  "LOCATION_CONFLICT_PERMANENT_CONFIRMATION_REQUIRED",
  "LOCATION_CONFLICT_FILE_CHANGED",
]);

/**
 * Reasons whose copy names the path a fact carries, through a variant key
 * beside the reason's own. The base copy is used when the fact is absent.
 */
const PATH_VARIANTS: Readonly<Record<string, { key: string; fact: string }>> = {
  LOCATION_UNPROVEN_LINK_NOT_REMOVED: {
    key: "error.unprovenLinkLeftBehindNamed",
    fact: "leftBehindPath",
  },
  LOCATION_SET_ASIDE_SOURCE_NOT_PUT_BACK: {
    key: "error.setAsideSourceKeptNamed",
    fact: "leftBehindPath",
  },
};

/** Facts that hold a path, which the panel shows in its own path display. */
const PATH_FACTS: readonly string[] = [
  "leftBehindPath",
  "titleFolder",
  "missingDrive",
];

export type RefusalSegment =
  { kind: "text"; text: string } | { kind: "path"; path: string };

// Private-use characters never appear in translated copy, so a token made of
// them marks exactly where a path fact was placed.
const TOKEN_OPEN = "";
const TOKEN_CLOSE = "";
const TOKEN = /(\d+)/g;

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/**
 * A failed title's explanation in the client's words, split so that each
 * path it names can be shown in the panel's path display. Null when the title
 * has no reason, or one this table does not translate: the caller then shows
 * the server sentence.
 */
export function locationTitleRefusalMessage(
  reason: string | null | undefined,
  facts: unknown,
  translate: ErrorTranslate,
): RefusalSegment[] | null {
  if (!reason || !LOCATION_TITLE_REFUSAL_REASONS.has(reason)) return null;
  const key = ERROR_REASON_KEYS[reason];
  if (!key) return null;
  const paths: string[] = [];
  const tokenFacts: Record<string, unknown> = isRecord(facts)
    ? { ...facts }
    : {};
  for (const fact of PATH_FACTS) {
    const value = tokenFacts[fact];
    if (typeof value === "string" && value !== "") {
      tokenFacts[fact] = `${TOKEN_OPEN}${paths.length}${TOKEN_CLOSE}`;
      paths.push(value);
    }
  }
  const variant = PATH_VARIANTS[reason];
  const variantValue = variant ? tokenFacts[variant.fact] : undefined;
  const message =
    variant && typeof variantValue === "string" && variantValue !== ""
      ? translate(variant.key, { [variant.fact]: variantValue })
      : refusalReasonMessage(reason, key, tokenFacts, translate);
  const segments: RefusalSegment[] = [];
  let last = 0;
  for (const match of message.matchAll(TOKEN)) {
    const index = match.index ?? 0;
    if (index > last) {
      segments.push({ kind: "text", text: message.slice(last, index) });
    }
    const path = paths[Number(match[1])];
    if (path !== undefined) segments.push({ kind: "path", path });
    last = index + match[0].length;
  }
  if (last < message.length) {
    segments.push({ kind: "text", text: message.slice(last) });
  }
  return segments;
}
