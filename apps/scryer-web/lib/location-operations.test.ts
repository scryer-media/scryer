import assert from "node:assert/strict";
import test from "node:test";

import type { Translate } from "@/components/root/types";
import {
  ambiguousCandidates,
  blockingTitles,
  canCancelOperation,
  canResumeOperation,
  CLASSIFICATION_ORDER,
  checkpointNeedsAttention,
  classBlocksStart,
  classifiedTitlePlacement,
  classMovesFiles,
  destinationIdentityPresentation,
  isActiveWorkBlock,
  isAmbiguousDestinationBlock,
  isBlockedSelectionMessage,
  isCrossLibraryDestination,
  isMergeNotSupportedBlock,
  isSameNameWarning,
  mergeBlockedTarget,
  isInsufficientSpaceMessage,
  isStalePlanMessage,
  isTerminalOperationState,
  offersModeSelection,
  operationByteProgress,
  orderedCheckpoints,
  orderedClassificationGroups,
  orderedPlanKindCounts,
  orderedPlanSections,
  previewCanStart,
  recognizeStartRefusal,
  refusalMessageKey,
  refusalNeedsFreshPreview,
  remainingSelection,
  sameNamedDestinationTitle,
  shouldPollOperation,
  startRefusalCodeFromError,
  toCount,
  transferStatement,
  typedConfirmationSatisfied,
  verificationStampText,
  destinationLibraryDisabledReasonKey,
  type LocationClassificationGroup,
  type LocationClassifiedTitle,
  type LocationOperation,
  type LocationOperationCounters,
  type LocationOperationPreview,
  type LocationSelectionClassification,
  type LocationTitleCheckpoint,
} from "./location-operations.ts";

const translate: Translate = (key, values) =>
  values ? `${key}:${JSON.stringify(values)}` : key;

function classification(
  groups: Partial<LocationClassificationGroup>[],
  blocksStart = false,
): LocationSelectionClassification {
  return {
    groups: groups.map((group) => ({
      class: group.class ?? "ROOT_MOVE",
      count: group.count ?? group.titles?.length ?? 0,
      titles: group.titles ?? [],
    })),
    titlesTotal: groups.reduce(
      (total, group) => total + (group.titles?.length ?? 0),
      0,
    ),
    blocksStart,
  };
}

test("Long values arrive as numbers or strings", () => {
  assert.equal(toCount(42), 42);
  assert.equal(toCount("42"), 42);
  assert.equal(toCount(" 9007199254740991 "), 9007199254740991);
  assert.equal(toCount(null), 0);
  assert.equal(toCount(undefined), 0);
  assert.equal(toCount("not a number"), 0);
  assert.equal(toCount(Number.NaN), 0);
});

test("all six classification groups render, empty ones included", () => {
  const groups = orderedClassificationGroups(
    classification([
      {
        class: "NO_OP",
        titles: [
          {
            titleId: "b",
            class: "NO_OP",
            sourceLibraryId: "lib",
            sourceRootId: "root-b",
            sourceFolderPath: "/data/b/Settled",
            destinationLibraryId: "lib",
            destinationRootId: "root-b",
            reasonCode: "already_at_destination",
            reason: null,
          },
        ],
      },
    ]),
  );
  // Six groups, in render order — a class the payload never mentioned is a
  // visible empty group, never a missing one (US2 scenario 3, FR-015).
  assert.deepEqual(
    groups.map((group) => group.class),
    CLASSIFICATION_ORDER,
  );
  assert.equal(groups.length, 6);
  const noOp = groups.find((group) => group.class === "NO_OP");
  assert.equal(noOp?.titles.length, 1);
  const catalogOnly = groups.find((group) => group.class === "CATALOG_ONLY");
  assert.deepEqual(catalogOnly, {
    class: "CATALOG_ONLY",
    count: 0,
    titles: [],
  });
});

test("a null classification still renders six groups", () => {
  assert.equal(orderedClassificationGroups(null).length, 6);
  assert.equal(orderedClassificationGroups(undefined).length, 6);
});

test("only incompatible and needs-resolution classes block the start", () => {
  assert.equal(classBlocksStart("INCOMPATIBLE"), true);
  assert.equal(classBlocksStart("NEEDS_RESOLUTION"), true);
  assert.equal(classBlocksStart("NO_OP"), false);
  assert.equal(classBlocksStart("CATALOG_ONLY"), false);
  assert.equal(classBlocksStart("ROOT_MOVE"), false);
  assert.equal(classBlocksStart("CROSS_LIBRARY_TRANSFER"), false);
});

test("only transfers and root moves move bytes", () => {
  assert.equal(classMovesFiles("ROOT_MOVE"), true);
  assert.equal(classMovesFiles("CROSS_LIBRARY_TRANSFER"), true);
  assert.equal(classMovesFiles("CATALOG_ONLY"), false);
  assert.equal(classMovesFiles("NO_OP"), false);
});

test("blocked titles collect from every blocking class, in order", () => {
  const blocked = blockingTitles(
    classification(
      [
        {
          class: "INCOMPATIBLE",
          titles: [
            {
              titleId: "c",
              class: "INCOMPATIBLE",
              sourceLibraryId: "lib",
              sourceRootId: "root-a",
              sourceFolderPath: "/data/a/C",
              destinationLibraryId: "lib",
              destinationRootId: "root",
              reasonCode: "incompatible_facet",
              reason: "Movies cannot go in a series library.",
            },
          ],
        },
        {
          class: "NEEDS_RESOLUTION",
          titles: [
            {
              titleId: "a",
              class: "NEEDS_RESOLUTION",
              sourceLibraryId: "lib",
              sourceRootId: "root-a",
              sourceFolderPath: "/data/a/A",
              destinationLibraryId: "lib",
              destinationRootId: "root",
              reasonCode: "active_download_or_import",
              reason: "An import is running for this title.",
            },
          ],
        },
      ],
      true,
    ),
  );
  // NEEDS_RESOLUTION sorts before INCOMPATIBLE in render order.
  assert.deepEqual(
    blocked.map((entry) => entry.titleId),
    ["a", "c"],
  );
  assert.equal(isActiveWorkBlock(blocked[0]), true);
  assert.equal(isActiveWorkBlock(blocked[1]), false);
});

function preview(
  overrides: Partial<LocationOperationPreview> = {},
): LocationOperationPreview {
  return {
    planFingerprint: "fp-1",
    operationType: "ROOT_MOVE",
    mode: "MOVE_WITH_SCRYER",
    sourceLibraryId: "lib",
    destinationLibraryId: "lib",
    sourceRootId: "root-a",
    destinationRootId: "root-b",
    selection: ["a"],
    counts: {
      itemsTotal: 1,
      titlesTotal: 1,
      filesTotal: 3,
      bytesTotal: 300,
      byKind: [
        { kind: "MOVE", count: 3 },
        { kind: "MERGE", count: 0 },
        { kind: "RENAME", count: 1 },
      ],
    },
    sections: [],
    classification: classification([]),
    freeSpace: {
      destinationRequiredBytes: 300,
      destinationTotalRequiredBytes: 300,
      destinationAvailableBytes: 9000,
      recycleRequiredBytes: 0,
      recycleAvailableBytes: null,
      sameVolumeMove: false,
      recycleOnOtherVolume: false,
      recycleSharesDestinationVolume: true,
      recyclingAvailable: true,
      probed: true,
      sufficient: true,
    },
    verification: { depth: "FULL", files: 3, bytes: 300, applies: true },
    confirmation: { requirement: "SIMPLE", typedPhrase: null, typedPrompt: null },
    warnings: [],
    blocksStart: false,
    ...overrides,
  };
}

test("a plan with blocked items is not startable until they are deselected", () => {
  assert.equal(previewCanStart(preview()), true);
  // The backend refuses either flag; the dialog mirrors both so the confirm
  // button is disabled instead of the user meeting a raw mutation error.
  assert.equal(previewCanStart(preview({ blocksStart: true })), false);
  assert.equal(
    previewCanStart(preview({ classification: classification([], true) })),
    false,
  );
  assert.equal(previewCanStart(preview({ selection: [] })), false);
  assert.equal(previewCanStart(null), false);
});

test("a destination measured too small is not startable, but an unmeasured one is", () => {
  const withSpace = (sufficient: boolean | null) =>
    preview({
      freeSpace: { ...preview().freeSpace, sufficient, probed: sufficient !== null },
    });

  // FR-080: the backend refuses a measured shortfall, so the confirm is
  // disabled rather than the user meeting a raw mutation error.
  assert.equal(previewCanStart(withSpace(false)), false);
  assert.equal(previewCanStart(withSpace(true)), true);
  // `null` is "we could not measure it", which stays startable — refusing on
  // unknown would block every move onto a volume Scryer cannot stat.
  assert.equal(previewCanStart(withSpace(null)), true);
});

test("deselecting a title keeps the rest of the selection in submit order", () => {
  assert.deepEqual(
    remainingSelection(["a", "b", "c"], new Set(["b"])),
    ["a", "c"],
  );
  assert.deepEqual(remainingSelection(["a", "b"], new Set()), ["a", "b"]);
  assert.deepEqual(remainingSelection(["a"], new Set(["a"])), []);
});

test("typed confirmation is only outstanding when the plan demands it", () => {
  assert.equal(typedConfirmationSatisfied(null, ""), true);
  assert.equal(
    typedConfirmationSatisfied(
      { requirement: "SIMPLE", typedPhrase: null, typedPrompt: null },
      "",
    ),
    true,
  );
  const typed = {
    requirement: "TYPED" as const,
    typedPhrase: "retire /data/old",
    typedPrompt: null,
  };
  assert.equal(typedConfirmationSatisfied(typed, "retire /data/old"), true);
  assert.equal(typedConfirmationSatisfied(typed, " retire /data/old "), true);
  assert.equal(typedConfirmationSatisfied(typed, "retire"), false);
  assert.equal(typedConfirmationSatisfied(typed, ""), false);
});

test("a fileless plan never offers a move mode (FR-076)", () => {
  assert.equal(offersModeSelection(preview()), true);
  assert.equal(offersModeSelection(preview({ mode: "CATALOG_ONLY" })), false);
  assert.equal(offersModeSelection(null), false);
});

test("plan-kind counts render in order and drop kinds the plan never produced", () => {
  assert.deepEqual(orderedPlanKindCounts(preview().counts), [
    { kind: "MOVE", count: 3 },
    { kind: "RENAME", count: 1 },
  ]);
  assert.deepEqual(orderedPlanKindCounts(null), []);
});

test("plan sections sort into render order", () => {
  const sections = orderedPlanSections([
    { kind: "WARNING", itemsTotal: 1, bytesTotal: 0, complete: true, items: [] },
    { kind: "MOVE", itemsTotal: 2, bytesTotal: 20, complete: true, items: [] },
    { kind: "NO_OP", itemsTotal: 1, bytesTotal: 0, complete: true, items: [] },
  ]);
  assert.deepEqual(
    sections.map((section) => section.kind),
    ["MOVE", "NO_OP", "WARNING"],
  );
});

function counters(
  overrides: Partial<LocationOperationCounters> = {},
): LocationOperationCounters {
  return {
    titlesTotal: 4,
    titlesProcessed: 1,
    titlesBlocked: 0,
    filesTotal: 10,
    filesProcessed: 5,
    bytesTotal: 1000,
    bytesProcessed: 250,
    merges: 0,
    dedups: 0,
    renames: 0,
    noOps: 0,
    unresolved: 0,
    ...overrides,
  };
}

test("byte progress falls back to title progress for a catalog-only plan", () => {
  assert.equal(operationByteProgress(counters()), 0.25);
  assert.equal(
    operationByteProgress(counters({ bytesTotal: 0, bytesProcessed: 0 })),
    0.25,
  );
  assert.equal(
    operationByteProgress(
      counters({ bytesTotal: 0, bytesProcessed: 0, titlesTotal: 0 }),
    ),
    0,
  );
  // A runner that over-counts never pushes the bar past full.
  assert.equal(
    operationByteProgress(counters({ bytesProcessed: 5000 })),
    1,
  );
  assert.equal(operationByteProgress(null), 0);
});

function operation(
  overrides: Partial<LocationOperation> = {},
): LocationOperation {
  return {
    id: "op-1",
    operationType: "ROOT_MOVE",
    mode: "MOVE_WITH_SCRYER",
    state: "MOVING",
    initiatedByUserId: "user-1",
    sourceLibraryId: "lib",
    destinationLibraryId: "lib",
    sourceRootId: "root-a",
    destinationRootId: "root-b",
    planFingerprint: "fp-1",
    verificationDepth: "FULL",
    verificationFallbackCount: 0,
    counters: counters(),
    detail: null,
    jobRunId: null,
    workflowOperationId: null,
    cancelRequested: false,
    cancelRequestedAt: null,
    confirmedAt: "2026-08-31T00:00:00Z",
    startedAt: "2026-08-31T00:00:01Z",
    createdAt: "2026-08-31T00:00:00Z",
    updatedAt: "2026-08-31T00:00:05Z",
    completedAt: null,
    titleCheckpoints: [],
    ...overrides,
  };
}

test("terminal states stop the poll and every action", () => {
  for (const state of [
    "COMPLETED",
    "COMPLETED_WITH_WARNINGS",
    "CANCELED",
    "FAILED",
  ] as const) {
    assert.equal(isTerminalOperationState(state), true, state);
    assert.equal(shouldPollOperation(operation({ state })), false, state);
    assert.equal(canCancelOperation(operation({ state })), false, state);
  }
  for (const state of [
    "QUEUED",
    "PREPARING",
    "MOVING",
    "VERIFYING",
    "RECONCILING",
    "CLEANING_UP",
  ] as const) {
    assert.equal(isTerminalOperationState(state), false, state);
    assert.equal(shouldPollOperation(operation({ state })), true, state);
  }
  assert.equal(shouldPollOperation(null), false);
});

test("cancel is offered once, then the runner drains", () => {
  assert.equal(canCancelOperation(operation()), true);
  assert.equal(canCancelOperation(operation({ cancelRequested: true })), false);
  assert.equal(canCancelOperation(null), false);
});

test("resume is offered only for a run nothing has written to in a while", () => {
  const updatedAt = "2026-08-31T00:00:00.000Z";
  const updatedAtMs = Date.parse(updatedAt);
  // A live run is left alone: a second runner over the same checkpoints is the
  // failure mode resume exists to avoid.
  assert.equal(
    canResumeOperation(operation({ updatedAt }), updatedAtMs + 5_000),
    false,
  );
  assert.equal(
    canResumeOperation(operation({ updatedAt }), updatedAtMs + 120_000),
    true,
  );
  // A terminal operation never resumes, however long ago it settled.
  assert.equal(
    canResumeOperation(
      operation({ updatedAt, state: "COMPLETED" }),
      updatedAtMs + 120_000,
    ),
    false,
  );
  assert.equal(
    canResumeOperation(operation({ updatedAt: "not a date" }), Date.now()),
    false,
  );
  assert.equal(canResumeOperation(null, Date.now()), false);
});

function checkpoint(
  overrides: Partial<LocationTitleCheckpoint> = {},
): LocationTitleCheckpoint {
  return {
    titleId: "a",
    sequence: 0,
    state: "PENDING",
    classification: "ROOT_MOVE",
    sourceLibraryId: "lib",
    sourceRootId: "root-a",
    sourceFolderPath: "/data/a/Arrival",
    destinationLibraryId: "lib",
    destinationRootId: "root-b",
    destinationFolderPath: "/data/b/Arrival",
    mergedIntoTitleId: null,
    filesTotal: 2,
    filesVerified: 0,
    bytesTotal: 200,
    bytesVerified: 0,
    detail: null,
    startedAt: null,
    updatedAt: "2026-08-31T00:00:00Z",
    completedAt: null,
    ...overrides,
  };
}

test("checkpoints render in confirmed-plan order", () => {
  const ordered = orderedCheckpoints([
    checkpoint({ titleId: "c", sequence: "2" }),
    checkpoint({ titleId: "a", sequence: 0 }),
    checkpoint({ titleId: "b", sequence: 1 }),
  ]);
  assert.deepEqual(
    ordered.map((entry) => entry.titleId),
    ["a", "b", "c"],
  );
});

test("blocked, failed, and warned checkpoints are the ones to read", () => {
  assert.equal(checkpointNeedsAttention(checkpoint({ state: "BLOCKED" })), true);
  assert.equal(checkpointNeedsAttention(checkpoint({ state: "FAILED" })), true);
  assert.equal(
    checkpointNeedsAttention(checkpoint({ state: "COMPLETED_WITH_WARNINGS" })),
    true,
  );
  assert.equal(
    checkpointNeedsAttention(checkpoint({ state: "COMPLETED" })),
    false,
  );
  assert.equal(checkpointNeedsAttention(checkpoint({ state: "SKIPPED" })), false);
});

test("the verification stamp names the depth and any quick-check fallback", () => {
  assert.equal(
    verificationStampText("FULL", 0, translate),
    "move.verificationStampFull",
  );
  assert.equal(
    verificationStampText("QUICK", 0, translate),
    "move.verificationStampQuick",
  );
  assert.equal(
    verificationStampText("FULL", 3, translate),
    'move.verificationStampFull · move.verificationFallbackCount:{"count":3}',
  );
});

test("a refused confirmation is recognised as a stale plan, not a raw failure", () => {
  assert.equal(
    isStalePlanMessage(
      "the preview no longer matches what is on disk or in the catalog; review a fresh preview before confirming",
    ),
    true,
  );
  assert.equal(isStalePlanMessage("stale_plan"), true);
  assert.equal(isStalePlanMessage("something else went wrong"), false);
  assert.equal(isStalePlanMessage(null), false);
});

test("a refused confirmation over blocked titles is recognised too", () => {
  assert.equal(
    isBlockedSelectionMessage(
      "some selected titles still need a decision; resolve or remove them before starting",
    ),
    true,
  );
  assert.equal(isBlockedSelectionMessage("blocked_items"), true);
  assert.equal(isBlockedSelectionMessage("disk full"), false);
  assert.equal(isBlockedSelectionMessage(undefined), false);
});

test("the server's refusal code is preferred over the message prose", () => {
  const refused = (refusalCode: string) => ({
    graphQLErrors: [
      {
        message: "validation: something the client should not have to read",
        extensions: { code: "LOCATION_PLAN_REFUSED", refusalCode },
      },
    ],
  });

  assert.equal(recognizeStartRefusal(refused("stale_plan"), null), "stale_plan");
  assert.equal(
    recognizeStartRefusal(refused("blocked_items"), null),
    "blocked_items",
  );
  assert.equal(startRefusalCodeFromError(refused("stale_plan")), "stale_plan");
  // An unknown code is not silently promoted into a refusal.
  assert.equal(startRefusalCodeFromError(refused("something_new")), null);

  // The prose stays the fallback for an error that carries no extensions.
  assert.equal(
    recognizeStartRefusal(
      { graphQLErrors: [{ message: "validation: boom" }] },
      "the preview no longer matches what is on disk or in the catalog",
    ),
    "stale_plan",
  );
  assert.equal(
    recognizeStartRefusal(new Error("boom"), "blocked_items"),
    "blocked_items",
  );
  assert.equal(recognizeStartRefusal(new Error("boom"), "disk full"), null);

  // Only the two re-previewable refusals send the dialog back for a fresh plan.
  assert.equal(refusalNeedsFreshPreview("stale_plan"), true);
  assert.equal(refusalNeedsFreshPreview("blocked_items"), true);
  assert.equal(refusalNeedsFreshPreview("typed_confirmation_mismatch"), false);
  assert.equal(refusalNeedsFreshPreview(null), false);
});

test("a refusal for free space is recognized and is not re-previewable", () => {
  const refused = {
    graphQLErrors: [
      {
        message: "validation: the destination does not have enough free space",
        extensions: {
          code: "LOCATION_PLAN_REFUSED",
          refusalCode: "insufficient_space",
        },
      },
    ],
  };

  assert.equal(
    startRefusalCodeFromError(refused),
    "insufficient_space",
    "the new code is recognized rather than dropped as unknown",
  );
  assert.equal(recognizeStartRefusal(refused, null), "insufficient_space");
  // The prose fallback, for a refusal that lost its extensions in transit.
  assert.equal(
    recognizeStartRefusal(new Error("boom"), "insufficient_space"),
    "insufficient_space",
  );
  assert.equal(
    recognizeStartRefusal(
      new Error("boom"),
      "The destination does not have enough free space for this move.",
    ),
    "insufficient_space",
  );
  assert.equal(isInsufficientSpaceMessage("the plan went stale"), false);

  // Re-previewing the same selection onto the same volume would only measure
  // the same shortfall again, so the dialog says so instead.
  assert.equal(refusalNeedsFreshPreview("insufficient_space"), false);
  assert.equal(
    refusalMessageKey("insufficient_space"),
    "move.startRefusedNoSpace",
  );
  assert.equal(refusalMessageKey("stale_plan"), null);
});

test("every classified title states where it lives now (FR-012)", () => {
  const rootPathById = new Map([
    ["root-a", "/data/a"],
    ["root-b", "/data/b"],
  ]);
  const planFolders = new Map([
    ["moving", { source: "/data/a/stale-name", destination: "/data/b/Moving (2024)" }],
  ]);
  const entry = (
    titleId: string,
    overrides: Partial<LocationClassifiedTitle> = {},
  ): LocationClassifiedTitle => ({
    titleId,
    class: "ROOT_MOVE",
    sourceLibraryId: "lib",
    sourceRootId: "root-a",
    sourceFolderPath: `/data/a/${titleId}`,
    destinationLibraryId: "lib",
    destinationRootId: "root-b",
    reasonCode: null,
    reason: null,
    ...overrides,
  });

  // The classification's own folder wins over the plan item's source path, so
  // the row states where the title is, not where a sampled item started.
  assert.deepEqual(
    classifiedTitlePlacement(entry("moving"), { planFolders, rootPathById }),
    { source: "/data/a/moving", destination: "/data/b/Moving (2024)" },
  );

  // A no-op contributes no plan item and still states current → destination.
  assert.deepEqual(
    classifiedTitlePlacement(
      entry("settled", {
        class: "NO_OP",
        sourceRootId: "root-b",
        sourceFolderPath: "/data/b/Settled",
      }),
      { planFolders, rootPathById },
    ),
    { source: "/data/b/Settled", destination: "/data/b" },
  );

  // A fileless catalog-only title owns no folder, so its root stands in for it
  // rather than the row going blank.
  assert.deepEqual(
    classifiedTitlePlacement(
      entry("fileless", { class: "CATALOG_ONLY", sourceFolderPath: null }),
      { planFolders, rootPathById },
    ),
    { source: "/data/a", destination: "/data/b" },
  );

  // With nothing but the payload, the fields that exist are still reported.
  assert.deepEqual(classifiedTitlePlacement(entry("bare")), {
    source: "/data/a/bare",
    destination: null,
  });
});

test("a disabled destination names why it cannot accept the selection", () => {
  assert.equal(destinationLibraryDisabledReasonKey(["lib"]), null);
  assert.equal(
    destinationLibraryDisabledReasonKey([]),
    "move.destinationNoSelection",
  );
  assert.equal(
    destinationLibraryDisabledReasonKey(["lib", "other"]),
    "move.destinationMixedSourceLibraries",
  );
  // Another library is a reachable destination now: it is the cross-library
  // transfer, not a refusal (US6, FR-016).
  assert.equal(destinationLibraryDisabledReasonKey(["lib"]), null);
});

test("a destination in another library is recognised as a transfer", () => {
  assert.equal(isCrossLibraryDestination("other", ["lib"]), true);
  assert.equal(isCrossLibraryDestination("lib", ["lib"]), false);
  // Nothing picked, nothing selected, or a selection spanning libraries has no
  // single source library to transfer out of.
  assert.equal(isCrossLibraryDestination("", ["lib"]), false);
  assert.equal(isCrossLibraryDestination("other", []), false);
  assert.equal(isCrossLibraryDestination("other", ["lib", "second"]), false);
});

/** A classified row carrying one FR-055 detection outcome. */
function crossLibraryEntry(
  overrides: Partial<LocationClassifiedTitle> = {},
): LocationClassifiedTitle {
  return {
    titleId: "moving",
    class: "CROSS_LIBRARY_TRANSFER",
    sourceLibraryId: "lib-movies",
    sourceRootId: "root-a",
    sourceFolderPath: "/data/a/Moving (2024)",
    destinationLibraryId: "lib-4k",
    destinationRootId: "root-b",
    reasonCode: null,
    reason: null,
    ...overrides,
  };
}

test("no destination match transfers, naming the library it lands in", () => {
  const entry = crossLibraryEntry({ destinationIdentityMatch: "NONE" });
  const presentation = destinationIdentityPresentation(entry, {
    resolveLibraryName: (id) => (id === "lib-4k" ? "4K Movies" : null),
  });

  assert.deepEqual(presentation.transfer, {
    destinationLibraryId: "lib-4k",
    destinationLibraryName: "4K Movies",
  });
  // NONE is the plain transfer: nothing to warn about, nothing to resolve.
  assert.equal(presentation.sameNameWarning, null);
  assert.deepEqual(presentation.ambiguous, []);
  assert.equal(presentation.mergeBlockedTargetTitleId, null);

  // An unnamable library still states the transfer; the caller falls back to
  // the identity rather than dropping the sentence.
  assert.deepEqual(transferStatement(entry), {
    destinationLibraryId: "lib-4k",
    destinationLibraryName: null,
  });
  // A title staying in its own library makes no transfer statement at all.
  assert.equal(
    transferStatement(crossLibraryEntry({ class: "ROOT_MOVE" })),
    null,
  );
});

test("a same-named destination title warns instead of merging (FR-055)", () => {
  const entry = crossLibraryEntry({
    destinationIdentityMatch: "SAME_NAME_NO_IDENTITY",
    sameNamedDestinationTitleId: "other-title",
    sameNamedDestinationTitleName: "Moving",
  });

  assert.equal(isSameNameWarning(entry), true);
  assert.deepEqual(sameNamedDestinationTitle(entry), {
    titleId: "other-title",
    name: "Moving",
  });
  // The warning is not a block: the transfer still happens, and it still
  // states the library it lands in.
  const presentation = destinationIdentityPresentation(entry);
  assert.notEqual(presentation.transfer, null);
  assert.deepEqual(presentation.ambiguous, []);
  assert.equal(presentation.mergeBlockedTargetTitleId, null);

  // The match kind is what makes it a warning; a row that lost the name still
  // warns, because the user must see the second same-named title coming.
  const nameless = crossLibraryEntry({
    destinationIdentityMatch: "SAME_NAME_NO_IDENTITY",
  });
  assert.equal(isSameNameWarning(nameless), true);
  assert.equal(sameNamedDestinationTitle(nameless), null);

  // Every other match kind is silent here.
  assert.equal(
    isSameNameWarning(crossLibraryEntry({ destinationIdentityMatch: "NONE" })),
    false,
  );
  assert.equal(isSameNameWarning(crossLibraryEntry()), false);
});

test("an ambiguous identity lists the candidates it must be resolved against", () => {
  const entry = crossLibraryEntry({
    class: "NEEDS_RESOLUTION",
    destinationIdentityMatch: "AMBIGUOUS",
    reasonCode: "ambiguous_destination_identity",
    reason: "Several destination titles share this identity.",
    // A repeated identity is one candidate, not two.
    ambiguousDestinationTitleIds: ["cand-a", "cand-b", "cand-a"],
  });

  assert.equal(isAmbiguousDestinationBlock(entry), true);
  assert.deepEqual(
    ambiguousCandidates(entry, (id) => (id === "cand-a" ? "Moving (2024)" : null)),
    [
      { titleId: "cand-a", name: "Moving (2024)" },
      // The payload carries identities only, so an unnamable candidate is
      // still listed by identity rather than dropped.
      { titleId: "cand-b", name: null },
    ],
  );

  // Candidates belong to the ambiguous outcome alone.
  assert.deepEqual(
    ambiguousCandidates(
      crossLibraryEntry({ ambiguousDestinationTitleIds: ["cand-a"] }),
    ),
    [],
  );
  assert.deepEqual(
    ambiguousCandidates(
      crossLibraryEntry({ destinationIdentityMatch: "AMBIGUOUS" }),
    ),
    [],
  );

  const presentation = destinationIdentityPresentation(entry);
  assert.equal(presentation.ambiguous.length, 2);
  // A blocked row is not a transfer statement: it never starts.
  assert.equal(presentation.transfer, null);
  assert.equal(presentation.sameNameWarning, null);
  assert.equal(presentation.mergeBlockedTargetTitleId, null);
});

test("a unique identity blocks as a merge and names the target (FR-055)", () => {
  const entry = crossLibraryEntry({
    class: "NEEDS_RESOLUTION",
    destinationIdentityMatch: "UNIQUE",
    reasonCode: "merge_not_yet_supported",
    reason: "The destination already has this title.",
    mergeTargetTitleId: "destination-title",
  });

  assert.equal(isMergeNotSupportedBlock(entry), true);
  assert.equal(mergeBlockedTarget(entry), "destination-title");
  assert.deepEqual(ambiguousCandidates(entry), []);

  // The merge target is only named for the merge block; a title that carries
  // one while blocked for another reason must not have it read out as a merge.
  assert.equal(
    mergeBlockedTarget(
      crossLibraryEntry({
        class: "NEEDS_RESOLUTION",
        destinationIdentityMatch: "UNIQUE",
        reasonCode: "active_download_or_import",
        mergeTargetTitleId: "destination-title",
      }),
    ),
    null,
  );
  assert.equal(isMergeNotSupportedBlock(crossLibraryEntry()), false);
  assert.equal(isAmbiguousDestinationBlock(entry), false);

  const presentation = destinationIdentityPresentation(entry);
  assert.equal(presentation.mergeBlockedTargetTitleId, "destination-title");
  assert.equal(presentation.transfer, null);
});

test("blocked identity outcomes still hold the plan back (FR-016)", () => {
  const ambiguous = crossLibraryEntry({
    titleId: "ambiguous",
    class: "NEEDS_RESOLUTION",
    destinationIdentityMatch: "AMBIGUOUS",
    reasonCode: "ambiguous_destination_identity",
    ambiguousDestinationTitleIds: ["cand-a"],
  });
  const merge = crossLibraryEntry({
    titleId: "merge",
    class: "NEEDS_RESOLUTION",
    destinationIdentityMatch: "UNIQUE",
    reasonCode: "merge_not_yet_supported",
    mergeTargetTitleId: "destination-title",
  });
  const transferring = crossLibraryEntry({ destinationIdentityMatch: "NONE" });

  // Both new reason codes ride the NEEDS_RESOLUTION class, so they reach the
  // existing blocked list with its Deselect — and the transferring title does
  // not.
  assert.deepEqual(
    blockingTitles(
      classification([
        { class: "NEEDS_RESOLUTION", titles: [ambiguous, merge] },
        { class: "CROSS_LIBRARY_TRANSFER", titles: [transferring] },
      ]),
    ).map((entry) => entry.titleId),
    ["ambiguous", "merge"],
  );
  // Neither is the active-work block, which has its own prose.
  assert.equal(isActiveWorkBlock(ambiguous), false);
  assert.equal(isActiveWorkBlock(merge), false);
});
