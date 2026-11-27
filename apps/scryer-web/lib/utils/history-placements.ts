import type {
  ImportFilePlacement,
  ImportLinkFallbackCause,
  ImportPlacement,
} from "../types/title-history.ts";

/** Labels for each way an import can place a file. */
export const IMPORT_PLACEMENT_KEYS = {
  HARD_LINK: "history.placementHardLink",
  COPY: "history.placementCopy",
  SYMLINK: "history.placementSymlink",
  RENAMED: "history.placementRenamed",
  ALREADY_PRESENT: "history.placementAlreadyPresent",
} satisfies Record<ImportPlacement, string>;

/** Labels for each reason a tried hard link was not used. */
export const IMPORT_LINK_FALLBACK_KEYS = {
  FAILED: "history.linkFallbackFailed",
  NOT_POSSIBLE: "history.linkFallbackNotPossible",
  UNVERIFIABLE: "history.linkFallbackUnverifiable",
} satisfies Record<ImportLinkFallbackCause, string>;

/** The label key for a placement, or null for a code this client does not know. */
export function importPlacementLabelKey(placement: string): string | null {
  return Object.hasOwn(IMPORT_PLACEMENT_KEYS, placement)
    ? IMPORT_PLACEMENT_KEYS[placement as ImportPlacement]
    : null;
}

/** The label key for a fallback cause, or null for a code this client does not know. */
export function importLinkFallbackLabelKey(cause: string): string | null {
  return Object.hasOwn(IMPORT_LINK_FALLBACK_KEYS, cause)
    ? IMPORT_LINK_FALLBACK_KEYS[cause as ImportLinkFallbackCause]
    : null;
}

/**
 * Files an import copied because their hard link failed. A destination that
 * cannot hold a link is the volume's nature, so only a failed link is singled
 * out.
 */
export function failedLinkPlacements(
  placements: readonly ImportFilePlacement[] | null | undefined,
): ImportFilePlacement[] {
  return (placements ?? []).filter(
    (placement) => placement.linkFallback?.cause === "FAILED",
  );
}
