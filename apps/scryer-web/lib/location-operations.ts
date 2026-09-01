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
  /** Destination title sharing the name but no identity — never auto-merged. */
  sameNamedDestinationTitleId?: string | null;
  /** Name of that same-named destination title. */
  sameNamedDestinationTitleName?: string | null;
  /** Destination titles the user must choose between, for an AMBIGUOUS match. */
  ambiguousDestinationTitleIds?: string[] | null;
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
 * Blocked because exactly one destination title shares this title's identity,
 * which is a merge — and the merge engine is not wired to the transfer yet.
 */
export function isMergeNotSupportedBlock(
  entry: LocationClassifiedTitle,
): boolean {
  return entry.reasonCode === "merge_not_yet_supported";
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
};

/**
 * The destination titles an ambiguous match is choosing between, in payload
 * order and de-duplicated. Empty for every other outcome, so a caller can
 * render the list unconditionally.
 *
 * The payload carries identities only; `resolveName` is how the caller supplies
 * a name for a destination title it happens to know about. Everything else
 * renders by identity, which is still what the user needs in order to resolve
 * it before starting.
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
  for (const titleId of entry.ambiguousDestinationTitleIds ?? []) {
    if (!titleId || seen.has(titleId)) {
      continue;
    }
    seen.add(titleId);
    candidates.push({ titleId, name: resolveName?.(titleId) ?? null });
  }
  return candidates;
}

/**
 * The destination title a blocked merge would have merged into, when the block
 * is `merge_not_yet_supported`. Null for every other block, so the merge prose
 * never names a title that has nothing to do with it.
 */
export function mergeBlockedTarget(
  entry: LocationClassifiedTitle,
): string | null {
  if (!isMergeNotSupportedBlock(entry)) {
    return null;
  }
  return entry.mergeTargetTitleId ?? null;
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

/** Everything the detection outcome adds to one classified row. */
export type DestinationIdentityPresentation = {
  /** Present for a title crossing libraries; null otherwise. */
  transfer: TransferStatement | null;
  /** Present for `SAME_NAME_NO_IDENTITY` with a title to name; null otherwise. */
  sameNameWarning: SameNamedDestinationTitle | null;
  /** Non-empty only for `AMBIGUOUS`. */
  ambiguous: AmbiguousDestinationCandidate[];
  /** Present only for a `merge_not_yet_supported` block that named its target. */
  mergeBlockedTargetTitleId: string | null;
};

/**
 * One pass over the FR-055 detection outcome, turning all four match kinds into
 * the render decisions the dialog needs. Keeping it here rather than in the
 * component is what makes the four outcomes testable without a renderer.
 */
export function destinationIdentityPresentation(
  entry: LocationClassifiedTitle,
  context: {
    resolveLibraryName?: (libraryId: string) => string | null | undefined;
    resolveTitleName?: (titleId: string) => string | null | undefined;
  } = {},
): DestinationIdentityPresentation {
  return {
    transfer: transferStatement(entry, context.resolveLibraryName),
    sameNameWarning: sameNamedDestinationTitle(entry),
    ambiguous: ambiguousCandidates(entry, context.resolveTitleName),
    mergeBlockedTargetTitleId: mergeBlockedTarget(entry),
  };
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
