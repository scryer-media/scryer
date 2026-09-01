/**
 * Client-side model for the two root-scoped location workflows: US4 change root
 * (a root's path is replaced) and US5 consolidate root (a root is folded into
 * another root of the same library).
 *
 * FR-020 calls them one settings action with two destinations, so they share a
 * dialog and most of this file; what differs is the request and the block of
 * facts each preview adds to the shared plan. Everything here is pure and
 * testable with `node --test`; React lives in the dialog.
 */

// Relative and extension-qualified on purpose: these helpers are exercised by
// `node --test`, which resolves the import at runtime and knows nothing about
// the bundler's `@/` alias.
import { toCount } from "./location-operations.ts";
import type {
  LocationOperationPreview,
  LocationPlanItem,
  LongValue,
} from "./location-operations.ts";

/** One title that cannot enter a root-scoped operation (FR-023). */
export type LocationBlockedTitle = {
  titleId: string;
  titleName: string;
  reason: string;
  reasonCode: string | null;
};

/**
 * The every-title ledger. There is no way to exclude a title from a root-scoped
 * operation, so this is an accounting rather than a selection.
 */
export type LocationTitleAccounting = {
  assignedTotal: LongValue;
  relocating: LongValue;
  catalogOnly: LongValue;
  blocked: LongValue;
  accountsForEveryTitle: boolean;
  blocksStart: boolean;
  blockedTitles: LocationBlockedTitle[];
};

/** What the root keeps when its path changes (FR-021, FR-078). */
export type LocationRootIdentityRetention = {
  rootId: string;
  keepsRootId: boolean;
  wasLibraryDefault: boolean;
  remainsLibraryDefault: boolean;
  retainedRole: string | null;
  retainedTitleAssignments: LongValue;
};

/** FR-027's three buckets. */
export type LocationRootContentClass = "MANAGED" | "COMPANION" | "UNKNOWN";

export type LocationRootContentEntry = {
  path: string;
  sizeBytes: LongValue;
  class: LocationRootContentClass;
  canonicalSidecar: boolean;
};

export type LocationRootContentBucket = {
  class: LocationRootContentClass;
  total: LongValue;
  bytesTotal: LongValue;
  complete: boolean;
  entries: LocationRootContentEntry[];
};

/** A path list with the complete count beside the sample actually sent. */
export type LocationSampledPaths = {
  total: LongValue;
  complete: boolean;
  paths: string[];
};

export type LocationRootContentInventory = {
  managed: LocationRootContentBucket;
  companions: LocationRootContentBucket;
  unknown: LocationRootContentBucket;
  unknownBytes: LongValue;
  blocksSourceRemoval: boolean;
  entryCount: LongValue;
  prunableDirectories: LocationSampledPaths;
  retainedDirectories: LocationSampledPaths;
};

export type LocationRootRetirementBlocker = {
  code: string;
  detail: string;
};

/** What happens to the old location after every title settles (FR-028, FR-087). */
export type LocationRootRetirementContract = {
  sourceRootPath: string;
  destinationRootPath: string;
  retireConfigurationAfterRecycling: boolean;
  recycleAllowlistPaths: LocationSampledPaths;
  requiresVerificationBeforeSourceRemoval: boolean;
  emptyDirectoriesOnly: boolean;
  removableDirectories: LocationSampledPaths;
  retainedDirectories: LocationSampledPaths;
  permitsSourceRemoval: boolean;
  blockers: LocationRootRetirementBlocker[];
};

export type LocationRootChangePreview = {
  plan: LocationOperationPreview;
  accounting: LocationTitleAccounting;
  retention: LocationRootIdentityRetention;
  content: LocationRootContentInventory;
  retirement: LocationRootRetirementContract;
};

/** FR-024's seven groups, plus the two title-scoped counts beside them. */
export type LocationConsolidationClassification = {
  movingIntoUnusedFolders: LongValue;
  mergingWithDestinationTitles: LongValue;
  folderNameCollisions: LongValue;
  mediaCollisions: LongValue;
  dedupEligibleFiles: LongValue;
  companionCollisions: LongValue;
  untrackedSourceEntries: LongValue;
  catalogOnly: LongValue;
  blocked: LongValue;
};

/** Which root new content lands on afterwards (FR-022). */
export type LocationDefaultRootTransfer = {
  sourceWasDefault: boolean;
  destinationWasDefault: boolean;
  destinationBecomesDefault: boolean;
  transfersTheDefault: boolean;
};

export type LocationRootConsolidationPreview = {
  plan: LocationOperationPreview;
  accounting: LocationTitleAccounting;
  classification: LocationConsolidationClassification;
  defaultTransfer: LocationDefaultRootTransfer;
  content: LocationRootContentInventory;
  retirement: LocationRootRetirementContract;
};

/** The two destinations FR-020's single action offers. */
export type RootDestinationKind = "NEW_PATH" | "EXISTING_ROOT";

// ── Refusals ────────────────────────────────────────────────────────────────

/**
 * Every refusal the two workflows raise. The server sends these as
 * `extensions.refusalCode` beside `extensions.code: LOCATION_ROOT_REFUSED`, so
 * nothing here parses a sentence.
 */
export type LocationRootRefusalCode =
  | "root_change_path_not_absolute"
  | "root_change_paths_overlap"
  | "root_change_source_root_is_symlink"
  | "root_change_source_root_unavailable"
  | "root_change_destination_not_empty"
  | "root_change_destination_parent_missing"
  | "root_change_destination_is_configured_root"
  | "root_change_mode_not_supported"
  | "root_consolidation_path_not_absolute"
  | "root_consolidation_same_root"
  | "root_consolidation_paths_overlap"
  | "root_consolidation_destination_not_a_configured_root"
  | "root_consolidation_source_root_is_symlink"
  | "root_consolidation_source_root_unavailable"
  | "root_consolidation_destination_root_unavailable"
  | "root_consolidation_mode_not_supported";

const ROOT_REFUSAL_CODES: readonly string[] = [
  "root_change_path_not_absolute",
  "root_change_paths_overlap",
  "root_change_source_root_is_symlink",
  "root_change_source_root_unavailable",
  "root_change_destination_not_empty",
  "root_change_destination_parent_missing",
  "root_change_destination_is_configured_root",
  "root_change_mode_not_supported",
  "root_consolidation_path_not_absolute",
  "root_consolidation_same_root",
  "root_consolidation_paths_overlap",
  "root_consolidation_destination_not_a_configured_root",
  "root_consolidation_source_root_is_symlink",
  "root_consolidation_source_root_unavailable",
  "root_consolidation_destination_root_unavailable",
  "root_consolidation_mode_not_supported",
];

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** The root-scoped refusal code the server attached, when it attached one. */
export function rootRefusalCode(error: unknown): LocationRootRefusalCode | null {
  if (!isRecord(error) || !Array.isArray(error.graphQLErrors)) {
    return null;
  }
  for (const graphQlError of error.graphQLErrors) {
    if (!isRecord(graphQlError) || !isRecord(graphQlError.extensions)) {
      continue;
    }
    const code = graphQlError.extensions.refusalCode;
    if (typeof code === "string" && ROOT_REFUSAL_CODES.includes(code)) {
      return code as LocationRootRefusalCode;
    }
  }
  return null;
}

/**
 * FR-020's two halves, as one control.
 *
 * A "new path" that is already a configured root is a consolidation, and a
 * consolidation destination that is not a configured root is a root change.
 * Either refusal switches the dialog's destination branch instead of showing an
 * error the user has to interpret. Every other refusal leaves the branch alone.
 */
export function crossRouteDestination(
  code: LocationRootRefusalCode | null,
): RootDestinationKind | null {
  if (code === "root_change_destination_is_configured_root") {
    return "EXISTING_ROOT";
  }
  if (code === "root_consolidation_destination_not_a_configured_root") {
    return "NEW_PATH";
  }
  return null;
}

/** The translated sentence for a refusal, in Scryer's own words. */
export function rootRefusalMessageKey(code: LocationRootRefusalCode): string {
  return `rootChange.refusal.${code}`;
}

// ── Reason and blocker codes ────────────────────────────────────────────────

/**
 * Plan-item reason codes both root-scoped planners stamp, plus the two shared
 * with every other location workflow. A code with no key falls back to the
 * server's own sentence, which the plan item always carries.
 */
const ROOT_REASON_CODES: readonly string[] = [
  // location::root_change::plan_reasons
  "root_identity_retained",
  "catalog_only_root_change",
  "title_blocked_for_root_change",
  "unknown_root_content",
  "source_retirement_blocked",
  "file_outside_title_folder",
  "hardlinked_source",
  // location::consolidation::plan_reasons
  "roots_consolidated",
  "default_root_transferred",
  "moves_into_unused_folder",
  "merges_with_destination_title",
  "folder_name_uniqued",
  "catalog_only_consolidation",
  "destination_folder_exists",
];

/** The translation key for a reason code, or null when there is none. */
export function rootReasonKey(code: string | null | undefined): string | null {
  if (!code || !ROOT_REASON_CODES.includes(code)) {
    return null;
  }
  return `rootChange.reason.${code}`;
}

/** Retirement blockers the contract names (FR-028). */
const RETIREMENT_BLOCKER_CODES: readonly string[] = [
  "blocked_titles",
  "unexplained_source_content",
];

export function retirementBlockerKey(code: string): string | null {
  return RETIREMENT_BLOCKER_CODES.includes(code)
    ? `rootChange.retirementBlocker.${code}`
    : null;
}

// ── Presentation ────────────────────────────────────────────────────────────

/** One of FR-024's seven groups, ready to render. */
export type ConsolidationGroupLine = {
  /** Stable key, used for the element id and the translation key. */
  key: string;
  count: number;
};

/**
 * FR-024's seven groups in the order the spec states them, followed by the two
 * title-scoped counts the accounting also carries.
 *
 * Every group is returned, including the empty ones: a preview that silently
 * omitted "0 merges" would leave the user wondering whether it was measured.
 */
export const CONSOLIDATION_GROUP_KEYS = [
  "movingIntoUnusedFolders",
  "mergingWithDestinationTitles",
  "folderNameCollisions",
  "mediaCollisions",
  "dedupEligibleFiles",
  "companionCollisions",
  "untrackedSourceEntries",
] as const;

export function consolidationGroups(
  classification: LocationConsolidationClassification | null | undefined,
): ConsolidationGroupLine[] {
  if (!classification) {
    return [];
  }
  return CONSOLIDATION_GROUP_KEYS.map((key) => ({
    key,
    count: toCount(classification[key]),
  }));
}

/** One folder whose name the consolidation had to change (US5.4, FR-025). */
export type ChangedFolderName = {
  titleId: string | null;
  from: string;
  to: string;
  detail: string | null;
};

function lastSegment(path: string): string {
  const trimmed = path.replace(/[/\\]+$/, "");
  const cut = Math.max(trimmed.lastIndexOf("/"), trimmed.lastIndexOf("\\"));
  return cut >= 0 ? trimmed.slice(cut + 1) : trimmed;
}

/**
 * Every changed folder name in the plan, by name.
 *
 * US5.4 asks for the names, not a count: the rename items carry both paths, so
 * the line reads "X becomes Y" rather than "1 folder was renamed". Only folder
 * renames are listed; a file rename around a destination collision belongs to
 * the collision section, not to this one.
 */
export function changedFolderNames(
  preview: LocationOperationPreview | null | undefined,
): ChangedFolderName[] {
  const lines: ChangedFolderName[] = [];
  for (const section of preview?.sections ?? []) {
    if (section.kind !== "RENAME") {
      continue;
    }
    for (const item of section.items) {
      if (item.reasonCode !== "folder_name_uniqued") {
        continue;
      }
      if (!item.sourcePath || !item.destinationPath) {
        continue;
      }
      lines.push({
        titleId: item.titleId,
        from: lastSegment(item.sourcePath),
        to: lastSegment(item.destinationPath),
        detail: item.detail,
      });
    }
  }
  return lines;
}

/** Whether the rename section was sampled rather than sent whole. */
export function changedFolderNamesComplete(
  preview: LocationOperationPreview | null | undefined,
): boolean {
  const section = (preview?.sections ?? []).find(
    (candidate) => candidate.kind === "RENAME",
  );
  return section ? section.complete : true;
}

/**
 * The unmanaged content the plan itself named, kept separate from the inventory
 * bucket: the plan items are what the operation will leave behind, and the
 * inventory is what the scan found. They agree, and the dialog shows the
 * inventory because it carries the complete count.
 */
export function unmanagedPlanItems(
  preview: LocationOperationPreview | null | undefined,
): LocationPlanItem[] {
  const section = (preview?.sections ?? []).find(
    (candidate) => candidate.kind === "UNMANAGED_CONTENT",
  );
  return section?.items ?? [];
}

/**
 * Whether a root-scoped plan may be confirmed.
 *
 * Deliberately not `previewCanStart`: that one requires a non-empty selection,
 * and a root-scoped operation has no selection at all. What blocks it is a
 * blocked title (FR-023), a blocking plan item, or a destination the free-space
 * estimate measured as too small. Unexplained content at the source does *not*
 * block the start; it only keeps the old location standing (FR-027, FR-028).
 */
export function rootPlanCanStart(
  plan: LocationOperationPreview | null | undefined,
  accounting: LocationTitleAccounting | null | undefined,
): boolean {
  if (!plan) {
    return false;
  }
  if (plan.blocksStart || plan.classification.blocksStart) {
    return false;
  }
  if (accounting?.blocksStart) {
    return false;
  }
  // `sufficient` is null when the volumes could not be measured, and an
  // unmeasured destination stays startable (FR-080).
  return plan.freeSpace?.sufficient !== false;
}

/**
 * The identity statement a root change opens with (FR-021), as the three facts
 * the sentence is built from. Null when there is no preview to state it from.
 */
export type RootIdentityStatement = {
  keepsRootId: boolean;
  keepsDefault: boolean;
  losesDefault: boolean;
  titleAssignments: number;
};

export function rootIdentityStatement(
  retention: LocationRootIdentityRetention | null | undefined,
): RootIdentityStatement | null {
  if (!retention) {
    return null;
  }
  return {
    keepsRootId: retention.keepsRootId,
    keepsDefault: retention.wasLibraryDefault && retention.remainsLibraryDefault,
    // A root that was the default and is not one afterwards is a fact worth
    // saying out loud; nothing in a root change should do this, so if it ever
    // does the dialog says so rather than hiding it.
    losesDefault: retention.wasLibraryDefault && !retention.remainsLibraryDefault,
    titleAssignments: toCount(retention.retainedTitleAssignments),
  };
}

/**
 * Whether the ledger closes. A preview whose parts do not add up to its total
 * is one the user must not confirm blind, so the dialog says so rather than
 * rendering an accounting that quietly loses a title (SC-005).
 */
export function accountingCloses(
  accounting: LocationTitleAccounting | null | undefined,
): boolean {
  if (!accounting) {
    return false;
  }
  if (!accounting.accountsForEveryTitle) {
    return false;
  }
  return (
    toCount(accounting.relocating) +
      toCount(accounting.catalogOnly) +
      toCount(accounting.blocked) ===
    toCount(accounting.assignedTotal)
  );
}

/** The typed phrase a root-scoped plan demands. Always the shared MOVE. */
export function rootTypedPhrase(
  plan: LocationOperationPreview | null | undefined,
): string | null {
  if (!plan || plan.confirmation.requirement !== "TYPED") {
    return null;
  }
  const phrase = plan.confirmation.typedPhrase?.trim() ?? "";
  return phrase.length > 0 ? phrase : null;
}
