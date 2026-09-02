/**
 * Client-side model for location operations (US2: move titles to another root).
 *
 * The GraphQL payloads are wide; everything here is the pure part of consuming
 * them — ordering, classification bookkeeping, blocked-title deselection
 * (FR-086), start eligibility (FR-016), and the polling/action predicates the
 * Activity view needs (FR-043, FR-091). React lives in the dialog and the
 * Activity panel; this file stays testable with `node --test`.
 */

import type { Translate } from "@/components/root/types";

/** GraphQL `Long` arrives as a number or a string depending on magnitude. */
export type LongValue = number | string;

export type LocationOperationType =
  | "FOLDER_REASSIGNMENT"
  | "ROOT_MOVE"
  | "ROOT_CHANGE"
  | "ROOT_CONSOLIDATION"
  | "CROSS_LIBRARY_TRANSFER"
  | "ADOPTION";

/** How the filesystem side of an operation is performed. */
export type LocationExecutionMode =
  | "MOVE_WITH_SCRYER"
  | "FILES_ALREADY_THERE"
  | "CATALOG_ONLY";

/** The one class each selected title falls into for a requested destination. */
export type TitleLocationClass =
  | "CROSS_LIBRARY_TRANSFER"
  | "ROOT_MOVE"
  | "NO_OP"
  | "CATALOG_ONLY"
  | "INCOMPATIBLE"
  | "NEEDS_RESOLUTION";

export type LocationPlanItemKind =
  | "MOVE"
  | "RENAME"
  | "MERGE"
  | "DEDUP"
  | "CATALOG_CHANGE"
  | "ROLE_CHANGE"
  | "NO_OP"
  | "BLOCKED"
  | "UNMANAGED_CONTENT"
  | "WARNING";

export type LocationOperationState =
  | "QUEUED"
  | "PREPARING"
  | "MOVING"
  | "VERIFYING"
  | "RECONCILING"
  | "CLEANING_UP"
  | "COMPLETED"
  | "COMPLETED_WITH_WARNINGS"
  | "CANCELED"
  | "FAILED";

export type LocationTitleCheckpointState =
  | "PENDING"
  | "MOVING"
  | "VERIFYING"
  | "RECONCILING"
  | "CLEANING_UP"
  | "COMPLETED"
  | "COMPLETED_WITH_WARNINGS"
  | "SKIPPED"
  | "BLOCKED"
  | "FAILED";

export type VerificationDepth = "FULL" | "QUICK";

export type LocationConfirmationRequirement = "SIMPLE" | "TYPED";

/**
 * What FR-055 destination-title detection concluded for a title crossing into
 * another library. Detection is by stable metadata identity, never by title
 * text — `SAME_NAME_NO_IDENTITY` is precisely the case where the names agree
 * and the identities do not.
 */
export type LocationDestinationIdentityMatch =
  | "UNIQUE"
  | "NONE"
  | "AMBIGUOUS"
  | "SAME_NAME_NO_IDENTITY";

export type LocationClassifiedTitle = {
  titleId: string;
  class: TitleLocationClass;
  /** Library the title lives in today (FR-012). */
  sourceLibraryId: string;
  /** Root the title lives on today (FR-012). */
  sourceRootId: string;
  /** Folder the title owns today, or null when it owns none (FR-012). */
  sourceFolderPath: string | null;
  destinationLibraryId: string;
  destinationRootId: string;
  reasonCode: string | null;
  reason: string | null;
  /**
   * FR-055 detection outcome, or null when the title stays in its own library
   * and no detection ran.
   *
   * This and the four fields below are optional rather than required: they are
   * additive on a payload the dialog also builds in tests and reads from a
   * cache, and a row that arrives without them must degrade into "no detection
   * outcome to state" rather than into a crash.
   */
  destinationIdentityMatch?: LocationDestinationIdentityMatch | null;
  /** Existing destination title this one merges into, when detection is UNIQUE. */
  mergeTargetTitleId?: string | null;
  /**
   * Name of that destination title. The dialog's own name resolver only knows
   * the selected *source* titles, so for a cross-library merge this is the only
   * thing that can name the title that survives.
   */
  mergeTargetTitleName?: string | null;
  /** Destination title sharing the name but no identity — never auto-merged. */
  sameNamedDestinationTitleId?: string | null;
  /** Name of that same-named destination title. */
  sameNamedDestinationTitleName?: string | null;
  /** Destination titles the user must choose between, for an AMBIGUOUS match. */
  ambiguousDestinationTitleIds?: string[] | null;
  /**
   * The same candidates carrying the name and the shared identities the user
   * needs in order to tell them apart (FR-055). Preferred over the bare id
   * list; the ids remain the fallback for a payload that predates it.
   */
  ambiguousDestinationCandidates?: LocationAmbiguousDestinationCandidate[] | null;
};

/** One destination title an ambiguous identity points at, as the payload sends it. */
export type LocationAmbiguousDestinationCandidate = {
  titleId: string;
  titleName: string;
  /** Identities both titles carry, as `source:external_id`. */
  sharedIdentities: string[];
};

export type LocationClassificationGroup = {
  class: TitleLocationClass;
  count: LongValue;
  titles: LocationClassifiedTitle[];
};

export type LocationSelectionClassification = {
  groups: LocationClassificationGroup[];
  titlesTotal: LongValue;
  blocksStart: boolean;
};

export type LocationPlanItem = {
  kind: LocationPlanItemKind;
  titleId: string | null;
  mediaFileId: string | null;
  sourcePath: string | null;
  destinationPath: string | null;
  sizeBytes: LongValue;
  sameVolume: boolean | null;
  reasonCode: string | null;
  detail: string | null;
};

export type LocationPlanSection = {
  kind: LocationPlanItemKind;
  itemsTotal: LongValue;
  bytesTotal: LongValue;
  complete: boolean;
  items: LocationPlanItem[];
};

export type LocationPlanKindCount = {
  kind: LocationPlanItemKind;
  count: LongValue;
};

export type LocationPlanCounts = {
  itemsTotal: LongValue;
  titlesTotal: LongValue;
  filesTotal: LongValue;
  bytesTotal: LongValue;
  byKind: LocationPlanKindCount[];
};

export type LocationFreeSpaceEstimate = {
  destinationRequiredBytes: LongValue;
  destinationTotalRequiredBytes: LongValue;
  destinationAvailableBytes: LongValue | null;
  recycleRequiredBytes: LongValue;
  recycleAvailableBytes: LongValue | null;
  sameVolumeMove: boolean;
  recycleOnOtherVolume: boolean;
  recycleSharesDestinationVolume: boolean;
  recyclingAvailable: boolean;
  probed: boolean;
  sufficient: boolean | null;
};

export type LocationVerificationStatement = {
  depth: VerificationDepth;
  files: LongValue;
  bytes: LongValue;
  applies: boolean;
};

export type LocationPlanConfirmation = {
  requirement: LocationConfirmationRequirement;
  typedPhrase: string | null;
  typedPrompt: string | null;
};

/** The role a media file holds for one logical slot after a merge (FR-068). */
export type LocationMergeMediaRole = "PRIMARY" | "ADDITIONAL";

/** Why a media file's role changed in a merge (FR-070). */
export type LocationMergeRoleChangeReason =
  | "DESTINATION_PRIMARY_RETAINED"
  | "SOURCE_PRIMARY_ALREADY_CLAIMED"
  | "COLLAPSED_SOURCE_EPISODES";

export type LocationMergeRoleChange = {
  fileId: string;
  /** Null for a movie, whose only slot is the title itself. */
  sourceEpisodeId?: string | null;
  destinationEpisodeId?: string | null;
  previousRole: LocationMergeMediaRole;
  newRole: LocationMergeMediaRole;
  reason: LocationMergeRoleChangeReason;
  detail: string;
};

export type LocationMergeBlockedRecord = {
  table: string;
  reason: string;
  sourceId: string;
  detail: string;
};

/**
 * What merging one title into an existing destination title does (FR-071).
 *
 * The destination wins everything except the merging title's media file
 * records and its history; every other row recorded against it retires with it.
 *
 * Every list is optional on the client type for the same reason the detection
 * fields are: the dialog builds this payload in tests and reads it from a
 * cache, and a summary that arrives without one section must degrade into an
 * empty section rather than a crash.
 */
export type LocationMergePreview = {
  sourceTitleId: string;
  destinationTitleId: string;
  /** Name of the surviving title, as the catalog spells it. */
  destinationTitleName?: string | null;
  sourceLibraryId?: string | null;
  destinationLibraryId?: string | null;
  blocked: boolean;
  blockedRecords?: LocationMergeBlockedRecord[] | null;
  mediaFilesRepointed?: LongValue | null;
  roleChanges?: LocationMergeRoleChange[] | null;
  historyRowsCarried?: LongValue | null;
  sourceRecordsDropped?: LongValue | null;
};

export type LocationOperationPreview = {
  planFingerprint: string;
  operationType: LocationOperationType;
  mode: LocationExecutionMode;
  sourceLibraryId: string | null;
  destinationLibraryId: string | null;
  sourceRootId: string | null;
  destinationRootId: string | null;
  selection: string[];
  counts: LocationPlanCounts;
  sections: LocationPlanSection[];
  classification: LocationSelectionClassification;
  freeSpace: LocationFreeSpaceEstimate;
  verification: LocationVerificationStatement;
  confirmation: LocationPlanConfirmation;
  warnings: string[];
  blocksStart: boolean;
  /**
   * One summary per title that merges into an existing destination title
   * (FR-071). Optional so a plan payload without the section degrades into
   * "no merges in this plan" rather than a crash.
   */
  merges?: LocationMergePreview[] | null;
};

export type LocationOperationCounters = {
  titlesTotal: LongValue;
  titlesProcessed: LongValue;
  titlesBlocked: LongValue;
  filesTotal: LongValue;
  filesProcessed: LongValue;
  bytesTotal: LongValue;
  bytesProcessed: LongValue;
  merges: LongValue;
  dedups: LongValue;
  renames: LongValue;
  noOps: LongValue;
  unresolved: LongValue;
};

export type LocationTitleCheckpoint = {
  titleId: string;
  sequence: LongValue;
  state: LocationTitleCheckpointState;
  classification: TitleLocationClass | null;
  sourceLibraryId: string | null;
  sourceRootId: string | null;
  sourceFolderPath: string | null;
  destinationLibraryId: string | null;
  destinationRootId: string | null;
  destinationFolderPath: string | null;
  mergedIntoTitleId: string | null;
  /**
   * Name of the surviving title, resolved from the catalog when the checkpoint
   * is read. Null when that title has since been deleted — optional on the
   * client type because a cached checkpoint may predate the field.
   */
  mergedIntoTitleName?: string | null;
  filesTotal: LongValue;
  filesVerified: LongValue;
  bytesTotal: LongValue;
  bytesVerified: LongValue;
  detail: string | null;
  startedAt: string | null;
  updatedAt: string;
  completedAt: string | null;
};

export type LocationOperation = {
  id: string;
  operationType: LocationOperationType;
  mode: LocationExecutionMode;
  state: LocationOperationState;
  initiatedByUserId: string | null;
  sourceLibraryId: string | null;
  destinationLibraryId: string | null;
  sourceRootId: string | null;
  destinationRootId: string | null;
  planFingerprint: string;
  verificationDepth: VerificationDepth;
  verificationFallbackCount: LongValue;
  counters: LocationOperationCounters;
  detail: string | null;
  jobRunId: string | null;
  workflowOperationId: string | null;
  cancelRequested: boolean;
  cancelRequestedAt: string | null;
  confirmedAt: string | null;
  startedAt: string | null;
  createdAt: string;
  updatedAt: string;
  completedAt: string | null;
  titleCheckpoints: LocationTitleCheckpoint[];
};

/** One file the operation lands under a different name (FR-074/075). */
export type LocationRenamedAsset = {
  sourcePath: string | null;
  sourceName: string | null;
  destinationPath: string;
  destinationName: string;
  /** Source library named inside the `(from …)` suffix, when the rename used one. */
  provenanceLabel: string | null;
  mediaFileId: string | null;
  sizeBytes: LongValue;
  /** False while the title carrying this rename has not settled. */
  done: boolean;
};

/** One source file recycled as a proven duplicate (FR-073). */
export type LocationDeduplicatedAsset = {
  sourcePath: string;
  sourceName: string;
  survivingPath: string | null;
  survivingName: string | null;
  /** False while the title carrying this dedup has not settled. */
  done: boolean;
};

/** One title's renamed and deduplicated files inside an operation. */
export type LocationTitleAssets = {
  titleId: string;
  titleName: string;
  sequence: LongValue;
  /** Whether the title finished; what turns its plan facts into history. */
  settled: boolean;
  checkpointState: LocationTitleCheckpointState | null;
  renames: LocationRenamedAsset[];
  dedups: LocationDeduplicatedAsset[];
};

/** Which files an operation renames and deduplicates, per title (FR-091). */
export type LocationOperationAssetListing = {
  operationId: string;
  titles: LocationTitleAssets[];
  renamesTotal: LongValue;
  renamesDone: LongValue;
  dedupsTotal: LongValue;
  dedupsDone: LongValue;
};

/**
 * Classes in the order the preview renders them: what will happen first, what
 * will not happen next, what stops the run last. Every class is rendered even
 * when it is empty, so no title ever appears to have been dropped (FR-015).
 */
export const CLASSIFICATION_ORDER: TitleLocationClass[] = [
  "ROOT_MOVE",
  "CROSS_LIBRARY_TRANSFER",
  "CATALOG_ONLY",
  "NO_OP",
  "NEEDS_RESOLUTION",
  "INCOMPATIBLE",
];

/** Plan-item kinds in a stable render order. */
export const PLAN_KIND_ORDER: LocationPlanItemKind[] = [
  "MOVE",
  "RENAME",
  "CATALOG_CHANGE",
  "ROLE_CHANGE",
  "MERGE",
  "DEDUP",
  "NO_OP",
  "UNMANAGED_CONTENT",
  "WARNING",
  "BLOCKED",
];

/** How often the Activity view re-reads a running operation. */
export const OPERATION_POLL_INTERVAL_MS = 2_000;

/**
 * How long an operation may go unwritten before the Activity view offers a
 * resume.
 *
 * A live runner pulses its progress every five seconds even in the middle of a
 * single multi-hour file, so this is two dozen missed pulses — comfortably past
 * any plausible slow write, and never reached by a run that is merely busy.
 * Offering resume is only ever a hint anyway: the backend refuses a resume
 * while a runner is alive, so a stale-looking-but-live operation costs the user
 * a clear refusal rather than a second runner (FR-033).
 */
export const OPERATION_STALL_THRESHOLD_MS = 120_000;

/** Coerce a GraphQL `Long` (number or string) into a finite number. */
export function toCount(value: LongValue | null | undefined): number {
  if (typeof value === "number") {
    return Number.isFinite(value) ? value : 0;
  }
  if (typeof value === "string") {
    const parsed = Number(value.trim());
    return Number.isFinite(parsed) ? parsed : 0;
  }
  return 0;
}

/** A class the operation refuses to start with (FR-016). */
export function classBlocksStart(value: TitleLocationClass): boolean {
  return value === "INCOMPATIBLE" || value === "NEEDS_RESOLUTION";
}

/** A class that moves no bytes; shown, never hidden (US2 scenario 3). */
export function classMovesFiles(value: TitleLocationClass): boolean {
  return value === "ROOT_MOVE" || value === "CROSS_LIBRARY_TRANSFER";
}

/**
 * All six classification groups in render order, including empty ones. The
 * backend already returns all six; filling the gaps here means a partial
 * payload degrades into a visible empty group rather than a missing one.
 */
export function orderedClassificationGroups(
  classification: LocationSelectionClassification | null | undefined,
): LocationClassificationGroup[] {
  const byClass = new Map<TitleLocationClass, LocationClassificationGroup>();
  for (const group of classification?.groups ?? []) {
    byClass.set(group.class, group);
  }
  return CLASSIFICATION_ORDER.map(
    (value) => byClass.get(value) ?? { class: value, count: 0, titles: [] },
  );
}

/** Plan sections in render order; unknown kinds keep their payload order last. */
export function orderedPlanSections(
  sections: LocationPlanSection[],
): LocationPlanSection[] {
  const rank = (kind: LocationPlanItemKind) => {
    const index = PLAN_KIND_ORDER.indexOf(kind);
    return index === -1 ? PLAN_KIND_ORDER.length : index;
  };
  return [...sections].sort((left, right) => rank(left.kind) - rank(right.kind));
}

/** Per-kind counts in render order, dropping kinds the plan never produced. */
export function orderedPlanKindCounts(
  counts: LocationPlanCounts | null | undefined,
): LocationPlanKindCount[] {
  const byKind = new Map<LocationPlanItemKind, number>();
  for (const entry of counts?.byKind ?? []) {
    byKind.set(entry.kind, toCount(entry.count));
  }
  return PLAN_KIND_ORDER.filter((kind) => (byKind.get(kind) ?? 0) > 0).map(
    (kind) => ({ kind, count: byKind.get(kind) ?? 0 }),
  );
}

/**
 * Every title that stops the plan from starting, in render order. These are the
 * rows the user deselects to proceed with the rest (FR-016, FR-086).
 */
export function blockingTitles(
  classification: LocationSelectionClassification | null | undefined,
): LocationClassifiedTitle[] {
  return orderedClassificationGroups(classification)
    .filter((group) => classBlocksStart(group.class))
    .flatMap((group) => group.titles);
}

/** Where one classified title lives now and where it would end up (FR-012). */
export type ClassifiedTitlePlacement = {
  /** Current folder, falling back to the current root when it owns none. */
  source: string | null;
  /** Calculated destination folder, falling back to the destination root. */
  destination: string | null;
};

/**
 * The current → destination statement FR-012 requires for *every* selected
 * title, including the no-op and catalog-only ones that contribute no plan item.
 *
 * The classification payload carries the title's own placement, so it is the
 * answer whenever it has one; the plan items are a fallback for the moving
 * titles, and the root paths are the last resort for a title with no folder.
 */
export function classifiedTitlePlacement(
  entry: LocationClassifiedTitle,
  context: {
    planFolders?: ReadonlyMap<
      string,
      { source: string | null; destination: string | null }
    >;
    rootPathById?: ReadonlyMap<string, string>;
  } = {},
): ClassifiedTitlePlacement {
  const planned = context.planFolders?.get(entry.titleId);
  const rootPath = (rootId: string) => context.rootPathById?.get(rootId) ?? null;
  return {
    source: entry.sourceFolderPath ?? planned?.source ?? rootPath(entry.sourceRootId),
    destination: planned?.destination ?? rootPath(entry.destinationRootId),
  };
}

/** True when this entry is blocked by an active download or import (FR-086). */
export function isActiveWorkBlock(entry: LocationClassifiedTitle): boolean {
  return entry.reasonCode === "active_download_or_import";
}

/**
 * Blocked because several destination titles share this title's identity: the
 * user must say which one before the transfer can start (FR-055).
 */
export function isAmbiguousDestinationBlock(
  entry: LocationClassifiedTitle,
): boolean {
  return entry.reasonCode === "ambiguous_destination_identity";
}

/**
 * The same-named destination title this transfer will *not* merge into.
 *
 * FR-055 never merges by name, so this outcome is a warning and not a block:
 * the transfer proceeds and the destination library ends up holding two titles
 * with the same name. The user has to be told that before confirming, which is
 * why the id alone is enough to warn on — the name is stated when the payload
 * carries it.
 */
export type SameNamedDestinationTitle = {
  titleId: string | null;
  name: string | null;
};

/** Whether this row must show the FR-055 same-name warning. */
export function isSameNameWarning(entry: LocationClassifiedTitle): boolean {
  return entry.destinationIdentityMatch === "SAME_NAME_NO_IDENTITY";
}

/** The same-named destination title to name in the warning, when there is one. */
export function sameNamedDestinationTitle(
  entry: LocationClassifiedTitle,
): SameNamedDestinationTitle | null {
  if (!isSameNameWarning(entry)) {
    return null;
  }
  const titleId = entry.sameNamedDestinationTitleId ?? null;
  const name = entry.sameNamedDestinationTitleName ?? null;
  if (titleId === null && name === null) {
    return null;
  }
  return { titleId, name };
}

/** One destination title the user is choosing between for an ambiguous match. */
export type AmbiguousDestinationCandidate = {
  titleId: string;
  /** Resolved display name, or null when only the identity is known. */
  name: string | null;
  /** Identities this candidate shares with the moving title, sorted as sent. */
  sharedIdentities: string[];
};

/**
 * The destination titles an ambiguous match is choosing between, in payload
 * order and de-duplicated. Empty for every other outcome, so a caller can
 * render the list unconditionally.
 *
 * The named candidates are what the user reads; the bare id list is the
 * fallback for a payload that carries only identities. `resolveName` fills a
 * name the payload did not carry, for a destination title the caller happens
 * to know about.
 *
 * There is no way to *pick* one of these yet: the backend has no
 * choose-a-candidate input, so the row's only affordance stays Deselect and
 * the identity is resolved in the destination library instead.
 */
export function ambiguousCandidates(
  entry: LocationClassifiedTitle,
  resolveName?: (titleId: string) => string | null | undefined,
): AmbiguousDestinationCandidate[] {
  if (entry.destinationIdentityMatch !== "AMBIGUOUS") {
    return [];
  }
  const seen = new Set<string>();
  const candidates: AmbiguousDestinationCandidate[] = [];
  for (const candidate of entry.ambiguousDestinationCandidates ?? []) {
    const titleId = candidate?.titleId;
    if (!titleId || seen.has(titleId)) {
      continue;
    }
    seen.add(titleId);
    candidates.push({
      titleId,
      name:
        candidate.titleName?.trim() || (resolveName?.(titleId) ?? null) || null,
      sharedIdentities: [...(candidate.sharedIdentities ?? [])],
    });
  }
  for (const titleId of entry.ambiguousDestinationTitleIds ?? []) {
    if (!titleId || seen.has(titleId)) {
      continue;
    }
    seen.add(titleId);
    candidates.push({
      titleId,
      name: resolveName?.(titleId) ?? null,
      sharedIdentities: [],
    });
  }
  return candidates;
}

/**
 * The existing destination title this one merges into, or null when the title
 * is not merging. Detection is by identity, so `UNIQUE` is the one outcome
 * that ever produces a merge (FR-055).
 */
export function mergeDestinationTitleId(
  entry: LocationClassifiedTitle,
): string | null {
  if (entry.destinationIdentityMatch !== "UNIQUE") {
    return null;
  }
  return entry.mergeTargetTitleId ?? null;
}

/**
 * The sentence a merging row leads with. A merge absorbs the moving title's
 * identity into the destination's, so it is stated as one title becoming
 * another and not as a transfer that happens to land nearby (FR-071).
 */
export type MergeStatement = {
  sourceTitleId: string;
  destinationTitleId: string;
  /** Resolved destination name, or null when the payload could not name it. */
  destinationTitleName: string | null;
  /** True when the summary says unmappable records stop this merge (FR-066). */
  blocked: boolean;
};

/**
 * The merge statement for one classified row, or null when the row is not a
 * merge. The summary is optional: a row can classify as a merge before its
 * per-merge summary is read, and the sentence is still the one to show.
 */
export function mergeStatement(
  entry: LocationClassifiedTitle,
  context: {
    merge?: LocationMergePreview | null;
    resolveTitleName?: (titleId: string) => string | null | undefined;
  } = {},
): MergeStatement | null {
  const destinationTitleId =
    mergeDestinationTitleId(entry) ?? context.merge?.destinationTitleId ?? null;
  if (!destinationTitleId) {
    return null;
  }
  return {
    sourceTitleId: entry.titleId,
    destinationTitleId,
    // The payload's own names come first: the destination title is usually in
    // another library, and the local resolver only knows the titles the user
    // selected. The resolver stays as the last fallback so a payload that
    // predates these fields still names a destination it happens to know.
    destinationTitleName:
      titleName(entry.mergeTargetTitleName) ??
      titleName(context.merge?.destinationTitleName) ??
      titleName(context.resolveTitleName?.(destinationTitleId)) ??
      null,
    blocked: context.merge?.blocked ?? false,
  };
}

/** A displayable title name, or null for absent/blank. */
function titleName(value: string | null | undefined): string | null {
  const trimmed = value?.trim();
  return trimmed ? trimmed : null;
}

/** What Activity's "merged into" checkpoint row states (US7, FR-091). */
export type CheckpointMergeTarget = {
  /** The surviving title. Always present — it is what the merge recorded. */
  titleId: string;
  /** Its name, or null when the catalog no longer has that title. */
  name: string | null;
  /** What the row shows: the name when there is one, the id otherwise. */
  label: string;
  /**
   * True when `label` is the raw id. The row renders an id in a code face and a
   * name in prose, so the two never read as the same kind of thing.
   */
  isIdFallback: boolean;
};

/**
 * The merge target of one settled checkpoint, or null when the title did not
 * merge. The name is resolved server-side at read time, so a destination
 * deleted after the merge falls back to the id rather than losing the row.
 */
export function checkpointMergeTarget(
  checkpoint: Pick<
    LocationTitleCheckpoint,
    "mergedIntoTitleId" | "mergedIntoTitleName"
  >,
): CheckpointMergeTarget | null {
  const titleId = checkpoint.mergedIntoTitleId?.trim();
  if (!titleId) {
    return null;
  }
  const name = titleName(checkpoint.mergedIntoTitleName);
  return {
    titleId,
    name,
    label: name ?? titleId,
    isIdFallback: name === null,
  };
}

/** The destination library a cross-library transfer states it lands in (FR-016). */
export type TransferStatement = {
  destinationLibraryId: string;
  /** Resolved library name, or null when the caller could not name it. */
  destinationLibraryName: string | null;
};

/**
 * The "transfers into <library>" statement for a title crossing libraries, or
 * null when the title is not crossing one. Naming the destination library is
 * the whole point: a cross-library transfer is the one class where the title's
 * library changes, and the row otherwise shows only paths.
 */
export function transferStatement(
  entry: LocationClassifiedTitle,
  resolveLibraryName?: (libraryId: string) => string | null | undefined,
): TransferStatement | null {
  if (entry.class !== "CROSS_LIBRARY_TRANSFER") {
    return null;
  }
  return {
    destinationLibraryId: entry.destinationLibraryId,
    destinationLibraryName: resolveLibraryName?.(entry.destinationLibraryId) ?? null,
  };
}

/** Every per-title merge summary in the plan, keyed by the title that merges. */
export function mergePreviewsBySourceTitle(
  preview: LocationOperationPreview | null | undefined,
): Map<string, LocationMergePreview> {
  const merges = new Map<string, LocationMergePreview>();
  for (const merge of preview?.merges ?? []) {
    if (!merge?.sourceTitleId || merges.has(merge.sourceTitleId)) {
      continue;
    }
    merges.set(merge.sourceTitleId, merge);
  }
  return merges;
}

/** One media-file role change, with the demotion called out (FR-070). */
export type MergeRoleChangeLine = {
  fileId: string;
  /** Null for a movie, whose only slot is the title itself. */
  sourceEpisodeId: string | null;
  destinationEpisodeId: string | null;
  previousRole: LocationMergeMediaRole;
  newRole: LocationMergeMediaRole;
  reason: LocationMergeRoleChangeReason;
  detail: string;
  /**
   * True when this file stops being the primary for its slot. FR-070 forbids a
   * silent demotion, so this is the flag the row renders loudly on.
   */
  demotion: boolean;
};

/**
 * Every role change the merge makes, in payload order. Demotions are not
 * filtered out or summarised into a count: FR-070 wants each one named.
 */
export function mergeRoleChangeLines(
  merge: LocationMergePreview | null | undefined,
): MergeRoleChangeLine[] {
  return (merge?.roleChanges ?? []).map((change) => ({
    fileId: change.fileId,
    sourceEpisodeId: change.sourceEpisodeId ?? null,
    destinationEpisodeId: change.destinationEpisodeId ?? null,
    previousRole: change.previousRole,
    newRole: change.newRole,
    reason: change.reason,
    detail: change.detail,
    demotion: change.previousRole === "PRIMARY" && change.newRole !== "PRIMARY",
  }));
}

/** Everything one merge summary renders, in one pass (FR-070, FR-071). */
export type MergeSummaryPresentation = {
  statement: MergeStatement;
  blocked: boolean;
  blockedRecords: LocationMergeBlockedRecord[];
  /** Media file records the surviving title takes over. */
  mediaFilesRepointed: number;
  roleChanges: MergeRoleChangeLine[];
  /** How many of those role changes take a file's primary away (FR-070). */
  demotionCount: number;
  /** History rows that follow the content onto the surviving title. */
  historyRowsCarried: number;
  /** Everything else on the merging title, which retires with it (FR-064). */
  sourceRecordsDropped: number;
  /** True when there is nothing at all beyond the statement to show. */
  empty: boolean;
};

/**
 * One pass over a merge summary. The dialog renders the result directly, so the
 * decisions about ordering, demotion detection, and "is there anything to show"
 * are all testable without a renderer.
 */
export function mergeSummaryPresentation(
  entry: LocationClassifiedTitle,
  merge: LocationMergePreview | null | undefined,
  context: {
    resolveTitleName?: (titleId: string) => string | null | undefined;
  } = {},
): MergeSummaryPresentation | null {
  const statement = mergeStatement(entry, {
    merge,
    resolveTitleName: context.resolveTitleName,
  });
  if (!statement) {
    return null;
  }
  const roleChanges = mergeRoleChangeLines(merge);
  const blockedRecords = [...(merge?.blockedRecords ?? [])];
  const mediaFilesRepointed = toCount(merge?.mediaFilesRepointed);
  const historyRowsCarried = toCount(merge?.historyRowsCarried);
  const sourceRecordsDropped = toCount(merge?.sourceRecordsDropped);
  return {
    statement,
    blocked: merge?.blocked ?? false,
    blockedRecords,
    mediaFilesRepointed,
    roleChanges,
    demotionCount: roleChanges.filter((change) => change.demotion).length,
    historyRowsCarried,
    sourceRecordsDropped,
    empty:
      roleChanges.length === 0 &&
      blockedRecords.length === 0 &&
      mediaFilesRepointed === 0 &&
      historyRowsCarried === 0 &&
      sourceRecordsDropped === 0,
  };
}

/** Translation key for a media-file role in a merge. */
export function mergeRoleLabelKey(value: LocationMergeMediaRole): string {
  return `move.mergeRole.${value}`;
}

/** Translation key for why a media file's role changed (FR-070). */
export function mergeRoleChangeReasonKey(
  value: LocationMergeRoleChangeReason,
): string {
  return `move.mergeRoleReason.${value}`;
}

/**
 * Whether the previewed plan may be confirmed. The backend refuses a blocked
 * plan — and one the destination has no room for — anyway; the dialog mirrors
 * the rules so the user sees a disabled confirm with the reason beside it
 * instead of a raw mutation error.
 *
 * `sufficient` is deliberately compared against `false` and not treated as
 * falsy: `null` means the volumes could not be measured, and an unmeasured
 * destination stays startable (FR-080).
 */
export function previewCanStart(
  preview: LocationOperationPreview | null | undefined,
): boolean {
  if (!preview) {
    return false;
  }
  if (preview.blocksStart || preview.classification.blocksStart) {
    return false;
  }
  if (preview.freeSpace?.sufficient === false) {
    return false;
  }
  return preview.selection.length > 0;
}

/** Selection minus the titles the user deselected, preserving submit order. */
export function remainingSelection(
  selection: string[],
  deselected: ReadonlySet<string>,
): string[] {
  return selection.filter((titleId) => !deselected.has(titleId));
}

/**
 * Whether a typed confirmation is still outstanding. A title-scoped root move
 * always reports SIMPLE; root-wide operations demand the phrase verbatim.
 */
export function typedConfirmationSatisfied(
  confirmation: LocationPlanConfirmation | null | undefined,
  typed: string,
): boolean {
  if (!confirmation || confirmation.requirement !== "TYPED") {
    return true;
  }
  const phrase = confirmation.typedPhrase?.trim() ?? "";
  return phrase.length > 0 && typed.trim() === phrase;
}

/**
 * A fileless selection is a catalog-only reassignment: no bytes move, so no
 * move-mode selection is offered at all (FR-076).
 */
export function offersModeSelection(
  preview: LocationOperationPreview | null | undefined,
): boolean {
  if (!preview) {
    return false;
  }
  return preview.mode !== "CATALOG_ONLY";
}

/**
 * The two modes a client may ask for. `CATALOG_ONLY` is the server's own
 * conclusion about a fileless selection (FR-076), so it is reported and never
 * requested; the input enum does not carry it.
 */
export const REQUESTABLE_MOVE_MODES = [
  "MOVE_WITH_SCRYER",
  "FILES_ALREADY_THERE",
] as const;

export type RequestableMoveMode = (typeof REQUESTABLE_MOVE_MODES)[number];

/**
 * The mode to confirm a previewed plan under.
 *
 * Read off the preview rather than off the dialog's own control, so a
 * confirmation always states the mode the plan in hand was built from. A
 * preview the server collapsed to catalog-only confirms as the managed move
 * it degrades into: the server collapses it again the same way (FR-076).
 */
export function startModeInput(
  preview: LocationOperationPreview | null | undefined,
): RequestableMoveMode {
  return preview?.mode === "FILES_ALREADY_THERE"
    ? "FILES_ALREADY_THERE"
    : "MOVE_WITH_SCRYER";
}

/** Reason codes the adoption planner stamps on its plan items (FR-050 to FR-053). */
export const ADOPTION_REASON_CODES = {
  adopted: "adopted_at_destination",
  missing: "adoption_media_missing",
  ambiguous: "adoption_media_ambiguous",
  additional: "adoption_additional_file",
  unreadable: "adoption_destination_unreadable",
  redundantSource: "adoption_redundant_source",
} as const;

/** One file line the adoption accounting states, as the plan item carries it. */
export type AdoptionFileLine = {
  titleId: string | null;
  /** Path the catalog holds, for a tracked file; null for a destination file. */
  sourcePath: string | null;
  /** Where the file is (or was looked for) at the destination. */
  destinationPath: string | null;
  sizeBytes: number;
  detail: string | null;
};

/**
 * FR-051's four-way accounting, read back out of the plan the preview already
 * carries.
 *
 * The counts come from the plan's complete per-kind totals; the lines come from
 * the sections, which the server may sample. `listingComplete` says which of the
 * two the reader is looking at, so a truncated list is never mistaken for the
 * whole refusal.
 */
export type AdoptionAccountingSummary = {
  /** Tracked files found at the destination and adopted where they lie. */
  accountedForFiles: number;
  accountedForBytes: number;
  /** Destination files no tracked media claims; surfaced, never touched. */
  additionalFiles: number;
  additionalBytes: number;
  additional: AdoptionFileLine[];
  /** Tracked files with no match at the destination (FR-052). */
  missing: AdoptionFileLine[];
  /** Tracked files whose match could not be narrowed to one file (FR-052). */
  ambiguous: AdoptionFileLine[];
  /** Destination folders that could not be scanned at all. */
  unreadable: AdoptionFileLine[];
  /** Whether every unaccounted file is listed above, or only a sample of them. */
  listingComplete: boolean;
  /** The FR-053 statement about what adoption does and does not delete. */
  sourceCleanupNotice: string | null;
  /** Whether the accounting refuses the confirmation (FR-052). */
  blocks: boolean;
};

function adoptionFileLine(item: LocationPlanItem): AdoptionFileLine {
  return {
    titleId: item.titleId,
    sourcePath: item.sourcePath,
    destinationPath: item.destinationPath,
    sizeBytes: toCount(item.sizeBytes),
    detail: item.detail,
  };
}

function planKindTotal(
  preview: LocationOperationPreview,
  kind: LocationPlanItemKind,
): number {
  const entry = (preview.counts?.byKind ?? []).find(
    (count) => count.kind === kind,
  );
  return entry ? toCount(entry.count) : 0;
}

/**
 * The adoption accounting for a preview, or null when this preview is not an
 * adoption.
 *
 * Every unaccounted file gets its own plan item plus one title-level rollup
 * carrying no source path. The rollup would double-count a file line and read
 * as a phantom row, so only the per-file items (the ones naming a tracked
 * file) become lines here.
 */
export function adoptionAccounting(
  preview: LocationOperationPreview | null | undefined,
): AdoptionAccountingSummary | null {
  if (!preview || preview.mode !== "FILES_ALREADY_THERE") {
    return null;
  }
  const missing: AdoptionFileLine[] = [];
  const ambiguous: AdoptionFileLine[] = [];
  const unreadable: AdoptionFileLine[] = [];
  const additional: AdoptionFileLine[] = [];
  let sourceCleanupNotice: string | null = null;
  let listingComplete = true;

  const sections = preview.sections ?? [];
  for (const section of sections) {
    if (section.kind === "BLOCKED" && !section.complete) {
      listingComplete = false;
    }
    for (const item of section.items) {
      switch (item.reasonCode) {
        case ADOPTION_REASON_CODES.missing:
          if (item.sourcePath) {
            missing.push(adoptionFileLine(item));
          }
          break;
        case ADOPTION_REASON_CODES.ambiguous:
          if (item.sourcePath) {
            ambiguous.push(adoptionFileLine(item));
          }
          break;
        case ADOPTION_REASON_CODES.unreadable:
          unreadable.push(adoptionFileLine(item));
          break;
        case ADOPTION_REASON_CODES.additional:
          additional.push(adoptionFileLine(item));
          break;
        case ADOPTION_REASON_CODES.redundantSource:
          sourceCleanupNotice = sourceCleanupNotice ?? item.detail;
          break;
        default:
          break;
      }
    }
  }

  const additionalSection = sections.find(
    (section) => section.kind === "UNMANAGED_CONTENT",
  );
  const adoptedSection = sections.find((section) => section.kind === "MOVE");

  return {
    accountedForFiles: planKindTotal(preview, "MOVE"),
    accountedForBytes: toCount(adoptedSection?.bytesTotal ?? 0),
    additionalFiles: planKindTotal(preview, "UNMANAGED_CONTENT"),
    additionalBytes: toCount(additionalSection?.bytesTotal ?? 0),
    additional,
    missing,
    ambiguous,
    unreadable,
    listingComplete,
    sourceCleanupNotice,
    blocks:
      missing.length > 0 || ambiguous.length > 0 || unreadable.length > 0,
  };
}

/**
 * Whether an adoption preview has anything unresolved to state. A clean
 * adoption still renders its accounting; this is what turns the panel from a
 * summary into a refusal.
 */
export function adoptionBlocks(
  accounting: AdoptionAccountingSummary | null | undefined,
): boolean {
  return accounting?.blocks === true;
}

/**
 * The reason codes an adoption refusal stamps on its `Blocked` plan items
 * (FR-052). The other two adoption codes are surfaced and never refused, so
 * they are not among them.
 */
export type AdoptionBlockingReasonCode =
  | typeof ADOPTION_REASON_CODES.missing
  | typeof ADOPTION_REASON_CODES.ambiguous
  | typeof ADOPTION_REASON_CODES.unreadable;

/**
 * Severity order for a title carrying more than one refusal: a folder that
 * could not be read at all comes before a file that was looked for and not
 * found, which comes before one that matched too many candidates.
 */
const ADOPTION_BLOCKING_REASON_ORDER: AdoptionBlockingReasonCode[] = [
  ADOPTION_REASON_CODES.unreadable,
  ADOPTION_REASON_CODES.missing,
  ADOPTION_REASON_CODES.ambiguous,
];

function adoptionBlockingReasonCode(
  item: LocationPlanItem,
): AdoptionBlockingReasonCode | null {
  const code = item.reasonCode;
  if (
    code &&
    (ADOPTION_BLOCKING_REASON_ORDER as readonly string[]).includes(code)
  ) {
    return code as AdoptionBlockingReasonCode;
  }
  return null;
}

/** One title an adoption refusal stops, as the plan items name it (FR-052). */
export type AdoptionBlockedTitle = {
  titleId: string;
  /** Every refusal reason stamped on this title, in severity order. */
  reasonCodes: AdoptionBlockingReasonCode[];
  /**
   * The reason the row leads with, or null when only the title-level rollup
   * named this title. That rollup always reads "missing" — even for a title
   * whose only problem is ambiguity — so it marks the title as refused without
   * ever being read as a reason.
   */
  primaryReasonCode: AdoptionBlockingReasonCode | null;
};

/**
 * Every title an adoption refusal blocks, in payload order.
 *
 * The refusal rides on `BLOCKED` plan items and not on the classification — the
 * title itself still classifies as a plain root move — so the plan is the only
 * place the deselect affordance can learn which titles FR-052's "resolve them
 * or deselect the title" is actually about.
 */
export function adoptionBlockedTitles(
  preview: LocationOperationPreview | null | undefined,
): AdoptionBlockedTitle[] {
  if (!preview || preview.mode !== "FILES_ALREADY_THERE") {
    return [];
  }
  const reasonsByTitle = new Map<string, Set<AdoptionBlockingReasonCode>>();
  const order: string[] = [];
  for (const section of preview.sections ?? []) {
    for (const item of section.items) {
      const code = adoptionBlockingReasonCode(item);
      if (!code || !item.titleId) {
        continue;
      }
      let reasons = reasonsByTitle.get(item.titleId);
      if (!reasons) {
        reasons = new Set<AdoptionBlockingReasonCode>();
        reasonsByTitle.set(item.titleId, reasons);
        order.push(item.titleId);
      }
      // The same rule the accounting reads by: a per-file refusal names the
      // file it is about, and the title-level rollup carries no source path.
      // Only the former says which reason applies. An unreadable destination
      // has no tracked file to name and is always its own reason.
      if (code === ADOPTION_REASON_CODES.unreadable || item.sourcePath) {
        reasons.add(code);
      }
    }
  }
  return order.map((titleId) => {
    const reasons = reasonsByTitle.get(titleId);
    const reasonCodes = ADOPTION_BLOCKING_REASON_ORDER.filter(
      (code) => reasons?.has(code) === true,
    );
    return {
      titleId,
      reasonCodes,
      primaryReasonCode: reasonCodes[0] ?? null,
    };
  });
}

/** Translation key for one adoption refusal reason, when there is one to state. */
export function adoptionBlockedReasonKey(
  code: AdoptionBlockingReasonCode | null | undefined,
): string | null {
  return code ? `move.adoptionBlockedReason.${code}` : null;
}

/**
 * One row of the dialog's deselect list: a title the plan refuses to start
 * with, whichever of the two ways it is blocked.
 */
export type BlockingTitleRow = {
  titleId: string;
  /** The classified row, when the classification is what blocks the title. */
  entry: LocationClassifiedTitle | null;
  /** The classification's own prose, when it carried any. */
  reason: string | null;
  /** The adoption refusal's reason, when an adoption is what blocks it. */
  adoptionReasonCode: AdoptionBlockingReasonCode | null;
};

/**
 * Every title the user must deselect to proceed: the classification-blocked
 * ones first, in render order, then the adoption-refused ones in payload order.
 *
 * A title blocked both ways — an adoption whose title also needs resolution —
 * gets one row carrying both reasons, so the deselect control is never rendered
 * twice for the same id (FR-016, FR-052, FR-086).
 */
export function blockingTitleRows(
  preview: LocationOperationPreview | null | undefined,
): BlockingTitleRow[] {
  const rows: BlockingTitleRow[] = [];
  const byTitle = new Map<string, BlockingTitleRow>();
  for (const entry of blockingTitles(preview?.classification)) {
    if (byTitle.has(entry.titleId)) {
      continue;
    }
    const row: BlockingTitleRow = {
      titleId: entry.titleId,
      entry,
      reason: entry.reason,
      adoptionReasonCode: null,
    };
    byTitle.set(entry.titleId, row);
    rows.push(row);
  }
  for (const blocked of adoptionBlockedTitles(preview)) {
    const existing = byTitle.get(blocked.titleId);
    if (existing) {
      existing.adoptionReasonCode = blocked.primaryReasonCode;
      continue;
    }
    const row: BlockingTitleRow = {
      titleId: blocked.titleId,
      entry: null,
      reason: null,
      adoptionReasonCode: blocked.primaryReasonCode,
    };
    byTitle.set(blocked.titleId, row);
    rows.push(row);
  }
  return rows;
}

/** Terminal operations neither progress nor resume. */
export function isTerminalOperationState(state: LocationOperationState): boolean {
  return (
    state === "COMPLETED" ||
    state === "COMPLETED_WITH_WARNINGS" ||
    state === "CANCELED" ||
    state === "FAILED"
  );
}

/** Whether the Activity view should keep polling this operation. */
export function shouldPollOperation(
  operation: LocationOperation | null | undefined,
): boolean {
  if (!operation) {
    return false;
  }
  return !isTerminalOperationState(operation.state);
}

/** Cancel stops at the next title checkpoint; asking twice does nothing. */
export function canCancelOperation(
  operation: LocationOperation | null | undefined,
): boolean {
  if (!operation) {
    return false;
  }
  return !isTerminalOperationState(operation.state) && !operation.cancelRequested;
}

/**
 * Resume is for a run the process abandoned: still non-terminal, but nothing
 * has been written for a while. A live run is left alone so a user cannot
 * spawn a second runner over the same checkpoints.
 */
export function canResumeOperation(
  operation: LocationOperation | null | undefined,
  nowMs: number,
  stallThresholdMs: number = OPERATION_STALL_THRESHOLD_MS,
): boolean {
  if (!operation || isTerminalOperationState(operation.state)) {
    return false;
  }
  const updatedAtMs = Date.parse(operation.updatedAt);
  if (!Number.isFinite(updatedAtMs)) {
    return false;
  }
  return nowMs - updatedAtMs >= stallThresholdMs;
}

/** Fraction of planned bytes the operation has processed, 0–1. */
export function operationByteProgress(
  counters: LocationOperationCounters | null | undefined,
): number {
  const total = toCount(counters?.bytesTotal);
  if (total <= 0) {
    // A catalog-only plan moves no bytes; fall back to title progress so the
    // bar is not permanently empty.
    const titlesTotal = toCount(counters?.titlesTotal);
    if (titlesTotal <= 0) {
      return 0;
    }
    return Math.min(1, toCount(counters?.titlesProcessed) / titlesTotal);
  }
  return Math.min(1, Math.max(0, toCount(counters?.bytesProcessed) / total));
}

/** A checkpoint the user must read: it failed, was blocked, or warned. */
export function checkpointNeedsAttention(
  checkpoint: LocationTitleCheckpoint,
): boolean {
  return (
    checkpoint.state === "FAILED" ||
    checkpoint.state === "BLOCKED" ||
    checkpoint.state === "COMPLETED_WITH_WARNINGS"
  );
}

/** Checkpoints in confirmed-plan order; attention-worthy rows never sort away. */
export function orderedCheckpoints(
  checkpoints: LocationTitleCheckpoint[],
): LocationTitleCheckpoint[] {
  return [...checkpoints].sort(
    (left, right) => toCount(left.sequence) - toCount(right.sequence),
  );
}

const STALE_PLAN_MARKERS = [
  "no longer matches",
  "fresh preview",
  "stale_plan",
];

/**
 * Whether a failed `startLocationOperation` means "the plan moved under you".
 * Prefer {@link recognizeStartRefusal}: this reads the prose, which is the
 * fallback for a server (or a transport) that did not carry the refusal code.
 */
export function isStalePlanMessage(message: string | null | undefined): boolean {
  if (!message) {
    return false;
  }
  const normalized = message.toLowerCase();
  return STALE_PLAN_MARKERS.some((marker) => normalized.includes(marker));
}

/** Whether a failed start means blocked titles are still selected (FR-086). */
export function isBlockedSelectionMessage(
  message: string | null | undefined,
): boolean {
  if (!message) {
    return false;
  }
  const normalized = message.toLowerCase();
  return (
    normalized.includes("blocked_items") ||
    normalized.includes("still need a decision")
  );
}

/** Whether a failed start means the destination measured too small (FR-080). */
export function isInsufficientSpaceMessage(
  message: string | null | undefined,
): boolean {
  if (!message) {
    return false;
  }
  const normalized = message.toLowerCase();
  return (
    normalized.includes("insufficient_space") ||
    normalized.includes("enough free space")
  );
}

/**
 * Why the server refused to start a confirmed plan. These are the application's
 * own refusal codes, carried on the GraphQL error as
 * `extensions.refusalCode` beside the `LOCATION_PLAN_REFUSED` error code.
 */
export type LocationRefusalCode =
  | "stale_plan"
  | "blocked_items"
  | "insufficient_space"
  | "typed_confirmation_required"
  | "typed_confirmation_mismatch";

const REFUSAL_CODES: readonly string[] = [
  "stale_plan",
  "blocked_items",
  "insufficient_space",
  "typed_confirmation_required",
  "typed_confirmation_mismatch",
];

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** The refusal code the server attached, when it attached one. */
export function startRefusalCodeFromError(
  error: unknown,
): LocationRefusalCode | null {
  if (!isRecord(error) || !Array.isArray(error.graphQLErrors)) {
    return null;
  }
  for (const graphQlError of error.graphQLErrors) {
    if (!isRecord(graphQlError) || !isRecord(graphQlError.extensions)) {
      continue;
    }
    const code = graphQlError.extensions.refusalCode;
    if (typeof code === "string" && REFUSAL_CODES.includes(code)) {
      return code as LocationRefusalCode;
    }
  }
  return null;
}

/**
 * Why a start was refused: the server's code when it sent one, and the message
 * prose otherwise. The prose path exists because a refusal that loses its
 * extensions is still a refusal the dialog must answer with a fresh preview
 * rather than a raw sentence about fingerprints.
 */
export function recognizeStartRefusal(
  error: unknown,
  message?: string | null,
): LocationRefusalCode | null {
  const code = startRefusalCodeFromError(error);
  if (code) {
    return code;
  }
  if (isStalePlanMessage(message)) {
    return "stale_plan";
  }
  if (isBlockedSelectionMessage(message)) {
    return "blocked_items";
  }
  if (isInsufficientSpaceMessage(message)) {
    return "insufficient_space";
  }
  return null;
}

/**
 * A refusal the dialog answers by re-previewing: the plan moved, or a title
 * became blocked between preview and confirm. Either way the fix is a fresh
 * plan (FR-016, FR-081). A typed-confirmation refusal is the user's to fix, and
 * so is a shortfall of free space — re-previewing the same selection onto the
 * same volume would only measure the same shortfall again (FR-080).
 */
export function refusalNeedsFreshPreview(
  code: LocationRefusalCode | null,
): boolean {
  return code === "stale_plan" || code === "blocked_items";
}

/** The message a refusal should show, when the prose the server sent is not it. */
export function refusalMessageKey(
  code: LocationRefusalCode | null,
): string | null {
  return code === "insufficient_space" ? "move.startRefusedNoSpace" : null;
}

/** One line in a title's asset section: a rename or a deduplication. */
export type LocationAssetLine = {
  kind: "RENAME" | "DEDUP";
  /** Stable id suffix, unique within its title. */
  key: string;
  /** The file as it is named on the source side. */
  from: string;
  /**
   * A rename's new name, or a dedup's surviving destination copy. Null only
   * when the stored plan could not name the other side.
   */
  to: string | null;
  /** Source library named inside a `(from …)` rename suffix (FR-074). */
  provenanceLabel: string | null;
  /** False while the title carrying this line has not settled. */
  done: boolean;
};

/** A name for a file, falling back to its path when the name is missing. */
function assetLabel(
  name: string | null | undefined,
  path: string | null | undefined,
): string | null {
  const trimmedName = name?.trim();
  if (trimmedName) {
    return trimmedName;
  }
  const trimmedPath = path?.trim();
  return trimmedPath ? trimmedPath : null;
}

/**
 * One title's renames and dedups as render-ready lines, renames first — the
 * same order the preview's plan sections use.
 *
 * A line whose source side cannot be named at all is dropped: the row would be
 * "→ X" with nothing to say what moved, which is less informative than the
 * count already shown beside it. Everything the plan can name is kept, settled
 * or not (FR-091).
 */
export function assetLines(
  title: LocationTitleAssets | null | undefined,
): LocationAssetLine[] {
  if (!title) {
    return [];
  }
  const lines: LocationAssetLine[] = [];
  title.renames.forEach((rename, index) => {
    const from = assetLabel(rename.sourceName, rename.sourcePath);
    if (!from) {
      return;
    }
    lines.push({
      kind: "RENAME",
      key: `rename-${index}`,
      from,
      to: assetLabel(rename.destinationName, rename.destinationPath),
      provenanceLabel: rename.provenanceLabel?.trim() || null,
      done: rename.done,
    });
  });
  title.dedups.forEach((dedup, index) => {
    const from = assetLabel(dedup.sourceName, dedup.sourcePath);
    if (!from) {
      return;
    }
    lines.push({
      kind: "DEDUP",
      key: `dedup-${index}`,
      from,
      to: assetLabel(dedup.survivingName, dedup.survivingPath),
      provenanceLabel: null,
      done: dedup.done,
    });
  });
  return lines;
}

/** Each title's assets, keyed by title id, for the checkpoint rows to read. */
export function assetsByTitle(
  listing: LocationOperationAssetListing | null | undefined,
): Map<string, LocationTitleAssets> {
  const byTitle = new Map<string, LocationTitleAssets>();
  for (const title of listing?.titles ?? []) {
    if (!title?.titleId || byTitle.has(title.titleId)) {
      continue;
    }
    byTitle.set(title.titleId, title);
  }
  return byTitle;
}

/** Whether any listed rename or dedup belongs to a title that has not settled. */
export function assetListingHasPlannedWork(
  listing: LocationOperationAssetListing | null | undefined,
): boolean {
  if (!listing) {
    return false;
  }
  return (
    toCount(listing.renamesDone) < toCount(listing.renamesTotal) ||
    toCount(listing.dedupsDone) < toCount(listing.dedupsTotal)
  );
}

/**
 * Whether each line should be labelled done-versus-planned.
 *
 * A finished operation whose every listed asset happened has nothing to
 * distinguish, and stamping "done" on every row of it would be noise. The
 * labels appear exactly when they carry information: while the operation is
 * still running, or when some titles settled and others did not (FR-091).
 */
export function showsAssetPlannedState(
  operation: LocationOperation | null | undefined,
  listing: LocationOperationAssetListing | null | undefined,
): boolean {
  if (!listing) {
    return false;
  }
  if (assetListingHasPlannedWork(listing)) {
    return true;
  }
  return operation ? !isTerminalOperationState(operation.state) : false;
}

/** Whether the listing has anything at all to show. */
export function assetListingIsEmpty(
  listing: LocationOperationAssetListing | null | undefined,
): boolean {
  return (listing?.titles?.length ?? 0) === 0;
}

/** Translation key for one asset line's prose. */
export function assetLineTextKey(kind: LocationAssetLine["kind"]): string {
  return kind === "RENAME" ? "move.assetRenamedAs" : "move.assetDeduplicatedAgainst";
}

/** Translation key for a classification group heading. */
export function classificationLabelKey(value: TitleLocationClass): string {
  return `move.class.${value}`;
}

/** Translation key for a plan-item kind. */
export function planKindLabelKey(value: LocationPlanItemKind): string {
  return `move.planKind.${value}`;
}

/** Translation key for an operation lifecycle state. */
export function operationStateLabelKey(value: LocationOperationState): string {
  return `move.operationState.${value}`;
}

/** Translation key for a per-title checkpoint state. */
export function checkpointStateLabelKey(
  value: LocationTitleCheckpointState,
): string {
  return `move.checkpointState.${value}`;
}

/**
 * The depth stamp Activity and the preview both show: "verified (full)" or
 * "verified (quick)", with the fallback count when full dropped to the quick
 * floor for some files (FR-043).
 */
export function verificationStampText(
  depth: VerificationDepth,
  fallbackCount: number,
  t: Translate,
): string {
  const stamp =
    depth === "FULL"
      ? t("move.verificationStampFull")
      : t("move.verificationStampQuick");
  if (fallbackCount > 0) {
    return `${stamp} · ${t("move.verificationFallbackCount", { count: fallbackCount })}`;
  }
  return stamp;
}

/**
 * Destination reachable for this selection (FR-017).
 *
 * Another library is now a reachable destination — that is the cross-library
 * transfer this story adds — so the only destinations still refused are the
 * ones no plan could be built for: nothing selected, or a selection spanning
 * several source libraries, which has no single library to transfer out of.
 */
export function destinationLibraryDisabledReasonKey(
  sourceLibraryIds: string[],
): string | null {
  if (sourceLibraryIds.length === 0) {
    return "move.destinationNoSelection";
  }
  if (sourceLibraryIds.length > 1) {
    return "move.destinationMixedSourceLibraries";
  }
  return null;
}

/** Whether picking this library would carry the selection out of its own (FR-016). */
export function isCrossLibraryDestination(
  candidateLibraryId: string,
  sourceLibraryIds: string[],
): boolean {
  if (!candidateLibraryId || sourceLibraryIds.length !== 1) {
    return false;
  }
  return sourceLibraryIds[0] !== candidateLibraryId;
}

// --------------------------------------------------------------------------
// Move wizard (title-panel entry point)
// --------------------------------------------------------------------------

/**
 * The three states of the move dialog.
 *
 * A caller that already picked a destination root (the bulk-edit path) hands
 * the dialog a plan to read; a caller that only asked to move something (the
 * title panel's "Move To…" action) has to say what kind of move it is and pick
 * a destination first, because a library with a single root can never start a
 * cross-library move from a root picker alone.
 */
export type MoveWizardStep = "kind" | "destination" | "plan";

/** What the user said they want to do, before they say where. */
export type MoveDestinationKind = "root" | "library";

/** Where the dialog opens: a caller-picked root skips the wizard entirely. */
export function initialMoveStep(
  initialRootId: string | null | undefined,
): MoveWizardStep {
  return initialRootId ? "plan" : "kind";
}

/** Whether this dialog opening runs the wizard rather than opening on a plan. */
export function movesThroughWizard(
  initialRootId: string | null | undefined,
): boolean {
  return initialMoveStep(initialRootId) === "kind";
}

/**
 * Roots worth offering for a same-library move.
 *
 * A root every selected title already sits on is not a destination — picking it
 * would plan a no-op — so it is left out, and an empty result is the honest
 * answer that this library has nowhere else to put the selection.
 */
export function eligibleSameLibraryRoots<T extends { id: string }>(
  roots: readonly T[],
  currentRootIds: readonly (string | null | undefined)[],
): T[] {
  const occupied = new Set(
    currentRootIds
      .map((rootId) => rootId?.trim() ?? "")
      .filter((rootId) => rootId.length > 0),
  );
  return roots.filter((root) => !occupied.has(root.id));
}

/** Libraries a cross-library move may land in: every one but the source. */
export function crossLibraryDestinations<T extends { id: string }>(
  libraries: readonly T[],
  sourceLibraryId: string | null,
): T[] {
  return libraries.filter((library) => library.id !== sourceLibraryId);
}

/** Picks the step's Next button enables, per step. */
export function moveWizardCanAdvance(
  step: MoveWizardStep,
  picks: {
    kind: MoveDestinationKind | null;
    libraryId: string;
    rootId: string;
  },
): boolean {
  if (step === "kind") {
    return picks.kind !== null;
  }
  if (step === "destination") {
    return picks.libraryId.length > 0 && picks.rootId.length > 0;
  }
  // The plan step confirms; it never advances.
  return false;
}

/** The step Next leads to, or the same step when there is nowhere to go. */
export function nextMoveStep(step: MoveWizardStep): MoveWizardStep {
  if (step === "kind") {
    return "destination";
  }
  if (step === "destination") {
    return "plan";
  }
  return "plan";
}

/** The step Back leads to; "kind" is the first step, so it stays put. */
export function previousMoveStep(step: MoveWizardStep): MoveWizardStep {
  if (step === "plan") {
    return "destination";
  }
  if (step === "destination") {
    return "kind";
  }
  return "kind";
}
