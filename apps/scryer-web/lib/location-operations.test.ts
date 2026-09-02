import assert from "node:assert/strict";
import test from "node:test";

import type { Translate } from "@/components/root/types";
import {
  ADOPTION_REASON_CODES,
  adoptionAccounting,
  adoptionBlockedReasonKey,
  adoptionBlockedTitles,
  adoptionBlocks,
  ambiguousCandidates,
  assetLineTextKey,
  assetLines,
  assetListingHasPlannedWork,
  assetListingIsEmpty,
  assetsByTitle,
  blockingTitleRows,
  blockingTitles,
  canCancelOperation,
  canResumeOperation,
  CLASSIFICATION_ORDER,
  checkpointMergeTarget,
  checkpointNeedsAttention,
  classBlocksStart,
  classifiedTitlePlacement,
  classMovesFiles,
  isActiveWorkBlock,
  isAmbiguousDestinationBlock,
  isBlockedSelectionMessage,
  isCrossLibraryDestination,
  isSameNameWarning,
  mergeDestinationTitleId,
  mergePreviewsBySourceTitle,
  mergeRoleChangeLines,
  mergeStatement,
  mergeSummaryPresentation,
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
  REQUESTABLE_MOVE_MODES,
  sameNamedDestinationTitle,
  shouldPollOperation,
  startModeInput,
  showsAssetPlannedState,
  startRefusalCodeFromError,
  toCount,
  transferStatement,
  typedConfirmationSatisfied,
  verificationStampText,
  destinationLibraryDisabledReasonKey,
  type LocationClassificationGroup,
  type LocationClassifiedTitle,
  type LocationMergePreview,
  type LocationOperation,
  type LocationOperationCounters,
  type LocationOperationPreview,
  type LocationPlanCounts,
  type LocationPlanItem,
  type LocationPlanSection,
  type LocationSelectionClassification,
  type LocationOperationAssetListing,
  type LocationTitleAssets,
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

test("only the two requestable modes are offered; catalog-only is derived", () => {
  assert.deepEqual(
    [...REQUESTABLE_MOVE_MODES],
    ["MOVE_WITH_SCRYER", "FILES_ALREADY_THERE"],
  );
});

test("the confirmed mode is read off the previewed plan, never off the control", () => {
  assert.equal(startModeInput(preview()), "MOVE_WITH_SCRYER");
  assert.equal(
    startModeInput(preview({ mode: "FILES_ALREADY_THERE" })),
    "FILES_ALREADY_THERE",
  );
  // A plan the server collapsed to catalog-only confirms as the managed move
  // it degrades into: CATALOG_ONLY is not a mode a client may ask for.
  assert.equal(startModeInput(preview({ mode: "CATALOG_ONLY" })), "MOVE_WITH_SCRYER");
  assert.equal(startModeInput(null), "MOVE_WITH_SCRYER");
});

function adoptionItem(
  overrides: Partial<LocationPlanItem> = {},
): LocationPlanItem {
  return {
    kind: "BLOCKED",
    titleId: "a",
    mediaFileId: null,
    sourcePath: null,
    destinationPath: null,
    sizeBytes: 0,
    sameVolume: null,
    reasonCode: null,
    detail: null,
    ...overrides,
  };
}

function adoptionPreview(
  sections: LocationPlanSection[],
  byKind: LocationPlanCounts["byKind"],
): LocationOperationPreview {
  return preview({
    mode: "FILES_ALREADY_THERE",
    operationType: "ADOPTION",
    sections,
    counts: {
      itemsTotal: 0,
      titlesTotal: 1,
      filesTotal: 0,
      bytesTotal: 0,
      byKind,
    },
  });
}

test("a managed-move preview has no adoption accounting to state", () => {
  assert.equal(adoptionAccounting(preview()), null);
  assert.equal(adoptionAccounting(preview({ mode: "CATALOG_ONLY" })), null);
  assert.equal(adoptionAccounting(null), null);
  assert.equal(adoptionBlocks(null), false);
});

test("a clean adoption counts what it found and states the FR-053 cleanup rule", () => {
  const accounting = adoptionAccounting(
    adoptionPreview(
      [
        {
          kind: "MOVE",
          itemsTotal: 2,
          bytesTotal: 400,
          complete: true,
          items: [
            adoptionItem({
              kind: "MOVE",
              sourcePath: "/old/A/one.mkv",
              destinationPath: "/new/A/one.mkv",
              sizeBytes: 200,
              reasonCode: ADOPTION_REASON_CODES.adopted,
            }),
            adoptionItem({
              kind: "MOVE",
              sourcePath: "/old/A/two.mkv",
              destinationPath: "/new/A/two.mkv",
              sizeBytes: 200,
              reasonCode: ADOPTION_REASON_CODES.adopted,
            }),
          ],
        },
        {
          kind: "UNMANAGED_CONTENT",
          itemsTotal: 1,
          bytesTotal: 30,
          complete: true,
          items: [
            adoptionItem({
              kind: "UNMANAGED_CONTENT",
              destinationPath: "/new/A/extra.nfo",
              sizeBytes: 30,
              reasonCode: ADOPTION_REASON_CODES.additional,
            }),
          ],
        },
        {
          kind: "WARNING",
          itemsTotal: 1,
          bytesTotal: 0,
          complete: true,
          items: [
            adoptionItem({
              kind: "WARNING",
              reasonCode: ADOPTION_REASON_CODES.redundantSource,
              detail: "adoption does not delete anything at the old location",
            }),
          ],
        },
      ],
      [
        { kind: "MOVE", count: 2 },
        { kind: "UNMANAGED_CONTENT", count: 1 },
      ],
    ),
  );
  assert.ok(accounting);
  assert.equal(accounting.accountedForFiles, 2);
  assert.equal(accounting.accountedForBytes, 400);
  assert.equal(accounting.additionalFiles, 1);
  assert.equal(accounting.additionalBytes, 30);
  assert.equal(accounting.additional.length, 1);
  assert.deepEqual(accounting.missing, []);
  assert.deepEqual(accounting.ambiguous, []);
  // FR-051: an additional file is surfaced and never blocks.
  assert.equal(accounting.blocks, false);
  assert.equal(adoptionBlocks(accounting), false);
  assert.equal(
    accounting.sourceCleanupNotice,
    "adoption does not delete anything at the old location",
  );
});

test("unaccounted media names its files and refuses the confirmation (FR-052)", () => {
  const accounting = adoptionAccounting(
    adoptionPreview(
      [
        {
          kind: "BLOCKED",
          itemsTotal: 3,
          bytesTotal: 0,
          complete: true,
          items: [
            adoptionItem({
              sourcePath: "/old/A/one.mkv",
              destinationPath: "/new/A",
              reasonCode: ADOPTION_REASON_CODES.missing,
              detail: "nothing at the destination matches this file",
            }),
            adoptionItem({
              sourcePath: "/old/A/two.mkv",
              destinationPath: "/new/A",
              reasonCode: ADOPTION_REASON_CODES.ambiguous,
              detail: "two destination files are equally plausible",
            }),
            // The title-level rollup carries no source path; counting it would
            // invent a file the user cannot go and look at.
            adoptionItem({
              reasonCode: ADOPTION_REASON_CODES.missing,
              detail: "1 tracked file is missing and 1 is ambiguous",
            }),
          ],
        },
      ],
      [{ kind: "BLOCKED", count: 3 }],
    ),
  );
  assert.ok(accounting);
  assert.equal(accounting.missing.length, 1);
  assert.equal(accounting.missing[0]?.sourcePath, "/old/A/one.mkv");
  assert.equal(accounting.ambiguous.length, 1);
  assert.equal(accounting.ambiguous[0]?.sourcePath, "/old/A/two.mkv");
  assert.equal(accounting.blocks, true);
  assert.equal(adoptionBlocks(accounting), true);
  assert.equal(accounting.listingComplete, true);
});

test("an unreadable destination blocks, and a sampled block list says so", () => {
  const accounting = adoptionAccounting(
    adoptionPreview(
      [
        {
          kind: "BLOCKED",
          itemsTotal: 40,
          bytesTotal: 0,
          complete: false,
          items: [
            adoptionItem({
              destinationPath: "/new/A",
              reasonCode: ADOPTION_REASON_CODES.unreadable,
              detail: "/new/A could not be scanned",
            }),
          ],
        },
      ],
      [{ kind: "BLOCKED", count: 40 }],
    ),
  );
  assert.ok(accounting);
  assert.equal(accounting.unreadable.length, 1);
  assert.equal(accounting.blocks, true);
  assert.equal(accounting.listingComplete, false);
});

test("an adoption refusal names the titles its copy tells the user to deselect", () => {
  // FR-052's refusal rides on BLOCKED plan items; the titles themselves stay
  // classified ROOT_MOVE, so the plan is the only place the deselect list can
  // learn which titles "resolve them or deselect the title" is about.
  const blocked = adoptionBlockedTitles(
    adoptionPreview(
      [
        {
          kind: "BLOCKED",
          itemsTotal: 5,
          bytesTotal: 0,
          complete: true,
          items: [
            adoptionItem({
              titleId: "a",
              sourcePath: "/old/A/one.mkv",
              destinationPath: "/new/A",
              reasonCode: ADOPTION_REASON_CODES.missing,
            }),
            adoptionItem({
              titleId: "a",
              reasonCode: ADOPTION_REASON_CODES.missing,
              detail: "1 tracked file is missing",
            }),
            adoptionItem({
              titleId: "b",
              sourcePath: "/old/B/one.mkv",
              destinationPath: "/new/B",
              reasonCode: ADOPTION_REASON_CODES.ambiguous,
            }),
            // The rollup always reads "missing", even for a title whose only
            // problem is ambiguity — it must never become the stated reason.
            adoptionItem({
              titleId: "b",
              reasonCode: ADOPTION_REASON_CODES.missing,
              detail: "1 tracked file is ambiguous",
            }),
            adoptionItem({
              titleId: "c",
              destinationPath: "/new/C",
              reasonCode: ADOPTION_REASON_CODES.unreadable,
            }),
          ],
        },
        {
          // Surfaced, never refused: an additional file is not a blocked title.
          kind: "UNMANAGED_CONTENT",
          itemsTotal: 1,
          bytesTotal: 10,
          complete: true,
          items: [
            adoptionItem({
              kind: "UNMANAGED_CONTENT",
              titleId: "d",
              destinationPath: "/new/D/extra.nfo",
              reasonCode: ADOPTION_REASON_CODES.additional,
            }),
          ],
        },
      ],
      [{ kind: "BLOCKED", count: 5 }],
    ),
  );
  assert.deepEqual(
    blocked.map((entry) => entry.titleId),
    ["a", "b", "c"],
  );
  assert.deepEqual(blocked[0].reasonCodes, [ADOPTION_REASON_CODES.missing]);
  assert.equal(blocked[1].primaryReasonCode, ADOPTION_REASON_CODES.ambiguous);
  assert.equal(blocked[2].primaryReasonCode, ADOPTION_REASON_CODES.unreadable);
});

test("a rollup-only refusal still offers a deselect, with no reason invented", () => {
  // A sampled BLOCKED section can drop every per-file item and keep the
  // title-level rollup, which names no file and always reads "missing".
  const blocked = adoptionBlockedTitles(
    adoptionPreview(
      [
        {
          kind: "BLOCKED",
          itemsTotal: 40,
          bytesTotal: 0,
          complete: false,
          items: [
            adoptionItem({
              titleId: "a",
              reasonCode: ADOPTION_REASON_CODES.missing,
              detail: "20 tracked files are missing and 19 are ambiguous",
            }),
          ],
        },
      ],
      [{ kind: "BLOCKED", count: 40 }],
    ),
  );
  assert.deepEqual(
    blocked.map((entry) => entry.titleId),
    ["a"],
  );
  assert.deepEqual(blocked[0].reasonCodes, []);
  assert.equal(blocked[0].primaryReasonCode, null);
  assert.equal(adoptionBlockedReasonKey(null), null);
});

test("only an adoption preview has adoption-blocked titles", () => {
  assert.deepEqual(adoptionBlockedTitles(preview()), []);
  assert.deepEqual(adoptionBlockedTitles(preview({ mode: "CATALOG_ONLY" })), []);
  assert.deepEqual(adoptionBlockedTitles(null), []);
});

test("each adoption refusal reason has its own translation key", () => {
  assert.equal(
    adoptionBlockedReasonKey(ADOPTION_REASON_CODES.missing),
    "move.adoptionBlockedReason.adoption_media_missing",
  );
  assert.equal(
    adoptionBlockedReasonKey(ADOPTION_REASON_CODES.ambiguous),
    "move.adoptionBlockedReason.adoption_media_ambiguous",
  );
  assert.equal(
    adoptionBlockedReasonKey(ADOPTION_REASON_CODES.unreadable),
    "move.adoptionBlockedReason.adoption_destination_unreadable",
  );
});

test("a title blocked both ways gets one deselect row, not two", () => {
  const needsResolution: LocationClassifiedTitle = {
    titleId: "a",
    class: "NEEDS_RESOLUTION",
    sourceLibraryId: "lib",
    sourceRootId: "root-a",
    sourceFolderPath: "/old/A",
    destinationLibraryId: "lib",
    destinationRootId: "root-b",
    reasonCode: "active_download_or_import",
    reason: "An import is running for this title.",
  };
  const rows = blockingTitleRows(
    preview({
      mode: "FILES_ALREADY_THERE",
      operationType: "ADOPTION",
      selection: ["a", "b"],
      blocksStart: true,
      classification: classification(
        [{ class: "NEEDS_RESOLUTION", titles: [needsResolution] }],
        true,
      ),
      sections: [
        {
          kind: "BLOCKED",
          itemsTotal: 2,
          bytesTotal: 0,
          complete: true,
          items: [
            adoptionItem({
              titleId: "a",
              sourcePath: "/old/A/one.mkv",
              reasonCode: ADOPTION_REASON_CODES.missing,
            }),
            adoptionItem({
              titleId: "b",
              sourcePath: "/old/B/one.mkv",
              reasonCode: ADOPTION_REASON_CODES.ambiguous,
            }),
          ],
        },
      ],
    }),
  );
  // One row per title id: the classification-blocked one keeps its classified
  // entry and its prose, and carries the adoption reason too.
  assert.deepEqual(
    rows.map((row) => row.titleId),
    ["a", "b"],
  );
  assert.equal(rows[0].entry, needsResolution);
  assert.equal(rows[0].reason, "An import is running for this title.");
  assert.equal(rows[0].adoptionReasonCode, ADOPTION_REASON_CODES.missing);
  // The adoption-only row has no classified entry to detail, so the dialog
  // renders the reason and nothing that assumes a classification block.
  assert.equal(rows[1].entry, null);
  assert.equal(rows[1].reason, null);
  assert.equal(rows[1].adoptionReasonCode, ADOPTION_REASON_CODES.ambiguous);
});

test("deselecting the last adoption-refused title yields a confirmable preview", () => {
  const refused = adoptionPreview(
    [
      {
        kind: "BLOCKED",
        itemsTotal: 1,
        bytesTotal: 0,
        complete: true,
        items: [
          adoptionItem({
            titleId: "b",
            sourcePath: "/old/B/one.mkv",
            reasonCode: ADOPTION_REASON_CODES.missing,
          }),
        ],
      },
    ],
    [{ kind: "BLOCKED", count: 1 }],
  );
  refused.selection = ["a", "b"];
  // The backend blocks a plan carrying any BLOCKED item, so the confirm is
  // disabled and the only affordance is the deselect these rows now render.
  refused.blocksStart = true;
  assert.equal(previewCanStart(refused), false);
  assert.deepEqual(
    blockingTitleRows(refused).map((row) => row.titleId),
    ["b"],
  );

  // Deselecting it re-previews the remaining selection; that plan carries no
  // blocked item, so it has no deselect rows left and is confirmable.
  const remaining = remainingSelection(refused.selection, new Set(["b"]));
  assert.deepEqual(remaining, ["a"]);
  const rePreviewed = adoptionPreview([], [{ kind: "MOVE", count: 3 }]);
  rePreviewed.selection = remaining;
  assert.deepEqual(blockingTitleRows(rePreviewed), []);
  assert.equal(previewCanStart(rePreviewed), true);
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

test("a merged checkpoint names the surviving title, falling back to its id", () => {
  // Not a merge at all.
  assert.equal(checkpointMergeTarget(checkpoint()), null);
  assert.equal(
    checkpointMergeTarget(checkpoint({ mergedIntoTitleId: "   " })),
    null,
  );

  // The ordinary case: the catalog still has the surviving title, so the row
  // reads as prose rather than as an identifier.
  assert.deepEqual(
    checkpointMergeTarget(
      checkpoint({
        mergedIntoTitleId: "title-99",
        mergedIntoTitleName: "Arrival (Director's Cut)",
      }),
    ),
    {
      titleId: "title-99",
      name: "Arrival (Director's Cut)",
      label: "Arrival (Director's Cut)",
      isIdFallback: false,
    },
  );

  // The surviving title was deleted after the merge, so the server could not
  // resolve a name. The row still states where the title went.
  assert.deepEqual(checkpointMergeTarget(checkpoint({ mergedIntoTitleId: "title-99" })), {
    titleId: "title-99",
    name: null,
    label: "title-99",
    isIdFallback: true,
  });

  // A blank name is not a name — it must not render as empty quotation marks.
  assert.deepEqual(
    checkpointMergeTarget(
      checkpoint({ mergedIntoTitleId: "title-99", mergedIntoTitleName: "  " }),
    ),
    {
      titleId: "title-99",
      name: null,
      label: "title-99",
      isIdFallback: true,
    },
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
  assert.deepEqual(
    transferStatement(entry, (id) => (id === "lib-4k" ? "4K Movies" : null)),
    { destinationLibraryId: "lib-4k", destinationLibraryName: "4K Movies" },
  );
  // NONE is the plain transfer: nothing to warn about, nothing to resolve.
  assert.equal(sameNamedDestinationTitle(entry), null);
  assert.deepEqual(ambiguousCandidates(entry), []);
  assert.equal(mergeStatement(entry), null);

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
  assert.notEqual(transferStatement(entry), null);
  assert.deepEqual(ambiguousCandidates(entry), []);
  assert.equal(mergeStatement(entry), null);

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

test("an ambiguous identity names the candidates and what they share", () => {
  const entry = crossLibraryEntry({
    class: "NEEDS_RESOLUTION",
    destinationIdentityMatch: "AMBIGUOUS",
    reasonCode: "ambiguous_destination_identity",
    reason: "Several destination titles share this identity.",
    // A repeated identity is one candidate, not two — in either payload.
    ambiguousDestinationTitleIds: ["cand-a", "cand-b", "cand-a"],
    ambiguousDestinationCandidates: [
      {
        titleId: "cand-a",
        titleName: "Moving (2024)",
        sharedIdentities: ["tmdb:1", "imdb:tt1"],
      },
      { titleId: "cand-a", titleName: "Moving (2024)", sharedIdentities: [] },
    ],
  });

  assert.equal(isAmbiguousDestinationBlock(entry), true);
  // The named candidates come first, then the ids the named list did not
  // cover — so a payload carrying both never loses a candidate.
  assert.deepEqual(
    ambiguousCandidates(entry, (id) => (id === "cand-b" ? "Moving" : null)),
    [
      {
        titleId: "cand-a",
        name: "Moving (2024)",
        sharedIdentities: ["tmdb:1", "imdb:tt1"],
      },
      { titleId: "cand-b", name: "Moving", sharedIdentities: [] },
    ],
  );

  // An id-only payload still lists every candidate, resolved where it can be.
  assert.deepEqual(
    ambiguousCandidates(
      crossLibraryEntry({
        destinationIdentityMatch: "AMBIGUOUS",
        ambiguousDestinationTitleIds: ["cand-a", "cand-b"],
      }),
      (id) => (id === "cand-a" ? "Moving (2024)" : null),
    ),
    [
      { titleId: "cand-a", name: "Moving (2024)", sharedIdentities: [] },
      // The payload carries identities only, so an unnamable candidate is
      // still listed by identity rather than dropped.
      { titleId: "cand-b", name: null, sharedIdentities: [] },
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

  // A blocked row is not a transfer statement: it never starts.
  assert.equal(transferStatement({ ...entry, class: "NEEDS_RESOLUTION" }), null);
  assert.equal(sameNamedDestinationTitle(entry), null);
  assert.equal(mergeStatement(entry), null);
});

/** A row that merges into an existing destination title (FR-055 UNIQUE). */
function mergingEntry(
  overrides: Partial<LocationClassifiedTitle> = {},
): LocationClassifiedTitle {
  return crossLibraryEntry({
    destinationIdentityMatch: "UNIQUE",
    mergeTargetTitleId: "destination-title",
    ...overrides,
  });
}

function mergePreview(
  overrides: Partial<LocationMergePreview> = {},
): LocationMergePreview {
  return {
    sourceTitleId: "moving",
    destinationTitleId: "destination-title",
    sourceLibraryId: "lib-movies",
    destinationLibraryId: "lib-4k",
    blocked: false,
    ...overrides,
  };
}

test("a unique identity merges and says so, naming the surviving title", () => {
  const entry = mergingEntry();

  assert.equal(mergeDestinationTitleId(entry), "destination-title");
  assert.deepEqual(ambiguousCandidates(entry), []);
  assert.equal(isAmbiguousDestinationBlock(entry), false);
  // A merge is no longer a block: the row transfers, and the destination
  // title it lands in is the one it becomes part of.
  assert.equal(entry.class, "CROSS_LIBRARY_TRANSFER");

  assert.deepEqual(
    mergeStatement(entry, {
      resolveTitleName: (id) =>
        id === "destination-title" ? "Moving (2024)" : null,
    }),
    {
      sourceTitleId: "moving",
      destinationTitleId: "destination-title",
      destinationTitleName: "Moving (2024)",
      blocked: false,
    },
  );

  // A destination title the caller cannot name still states the merge; the
  // renderer falls back to the identity rather than dropping the sentence.
  assert.equal(mergeStatement(entry)?.destinationTitleName, null);

  // Detection is by identity, so no other match kind ever merges — even one
  // carrying a stale merge target.
  assert.equal(
    mergeDestinationTitleId(
      crossLibraryEntry({
        destinationIdentityMatch: "SAME_NAME_NO_IDENTITY",
        mergeTargetTitleId: "destination-title",
      }),
    ),
    null,
  );
  assert.equal(mergeStatement(crossLibraryEntry()), null);

  // A summary that says the merge is blocked carries that into the statement,
  // so the row can say the merge cannot run (FR-066).
  assert.equal(
    mergeStatement(entry, { merge: mergePreview({ blocked: true }) })?.blocked,
    true,
  );

  assert.equal(mergeStatement(entry)?.destinationTitleId, "destination-title");
  // It is still a cross-library transfer, so it still names the library.
  assert.notEqual(transferStatement(entry), null);
});

test("the merge statement takes the surviving title's name from the payload", () => {
  // The local resolver only knows the titles the user selected, and the title
  // that survives a cross-library merge is not one of them. So a name on the
  // payload always beats one the resolver guesses at.
  const named = mergingEntry({ mergeTargetTitleName: "Amélie (Restored)" });
  assert.equal(
    mergeStatement(named, {
      resolveTitleName: () => "the source title's own name",
    })?.destinationTitleName,
    "Amélie (Restored)",
  );

  // The classified row is the first source; the merge summary is the second,
  // for a row that classified before its summary arrived and vice versa.
  assert.equal(
    mergeStatement(mergingEntry(), {
      merge: mergePreview({ destinationTitleName: "Amélie (Restored)" }),
    })?.destinationTitleName,
    "Amélie (Restored)",
  );
  assert.equal(
    mergeStatement(named, {
      merge: mergePreview({ destinationTitleName: "a stale summary name" }),
    })?.destinationTitleName,
    "Amélie (Restored)",
  );

  // A blank name is not a name: it falls through rather than rendering an
  // empty pair of quotation marks.
  assert.equal(
    mergeStatement(mergingEntry({ mergeTargetTitleName: "   " }), {
      merge: mergePreview({ destinationTitleName: "" }),
      resolveTitleName: () => "Resolved Locally",
    })?.destinationTitleName,
    "Resolved Locally",
  );

  // With no name anywhere the statement still stands; naming it by id is the
  // renderer's fallback, not a name this helper invents.
  assert.equal(mergeStatement(mergingEntry())?.destinationTitleName, null);
});

test("a merge summary carries the payload's destination name into its statement", () => {
  const summary = mergeSummaryPresentation(
    mergingEntry({ mergeTargetTitleName: "Amélie (Restored)" }),
    mergePreview({ destinationTitleName: "Amélie (Restored)" }),
  );
  assert.equal(summary?.statement.destinationTitleName, "Amélie (Restored)");
  assert.equal(summary?.statement.destinationTitleId, "destination-title");
});

test("merge summaries are indexed by the title that merges away", () => {
  const plan = preview({
    merges: [
      mergePreview({ sourceTitleId: "moving" }),
      mergePreview({ sourceTitleId: "second", destinationTitleId: "other" }),
      // A duplicate never displaces the first summary for that title.
      mergePreview({ sourceTitleId: "moving", destinationTitleId: "ignored" }),
    ],
  });

  const merges = mergePreviewsBySourceTitle(plan);
  assert.equal(merges.size, 2);
  assert.equal(merges.get("moving")?.destinationTitleId, "destination-title");
  assert.equal(merges.get("second")?.destinationTitleId, "other");
  // A plan with no merge section at all reads as "no merges", not a crash.
  assert.equal(mergePreviewsBySourceTitle(preview()).size, 0);
  assert.equal(mergePreviewsBySourceTitle(null).size, 0);
});

test("every media-role change is named, and demotions are flagged (FR-070)", () => {
  const merge = mergePreview({
    roleChanges: [
      {
        fileId: "file-demoted",
        sourceEpisodeId: "src-1",
        destinationEpisodeId: "dst-1",
        previousRole: "PRIMARY",
        newRole: "ADDITIONAL",
        reason: "DESTINATION_PRIMARY_RETAINED",
        detail: "The destination already had a primary for S01E01.",
      },
      {
        fileId: "file-kept",
        sourceEpisodeId: "src-2",
        destinationEpisodeId: "dst-2",
        previousRole: "ADDITIONAL",
        newRole: "ADDITIONAL",
        reason: "COLLAPSED_SOURCE_EPISODES",
        detail: "Two source episodes collapsed onto S01E02.",
      },
      // A movie has one slot and it is the title, so it carries no episode ids.
      {
        fileId: "file-movie",
        previousRole: "PRIMARY",
        newRole: "ADDITIONAL",
        reason: "DESTINATION_PRIMARY_RETAINED",
        detail: "The destination already has a primary file.",
      },
    ],
  });

  const lines = mergeRoleChangeLines(merge);
  // No line is summarised away: FR-070 wants each change readable.
  assert.deepEqual(
    lines.map((line) => [line.fileId, line.demotion]),
    [
      ["file-demoted", true],
      ["file-kept", false],
      ["file-movie", true],
    ],
  );
  assert.equal(
    lines[0].detail,
    "The destination already had a primary for S01E01.",
  );
  // The movie line reads with nulls rather than empty strings.
  assert.equal(lines[2].sourceEpisodeId, null);
  assert.equal(lines[2].destinationEpisodeId, null);
  assert.deepEqual(mergeRoleChangeLines(mergePreview()), []);
});

test("a merge summary is one pass, and an empty one says so (FR-071)", () => {
  const entry = mergingEntry();
  const summary = mergeSummaryPresentation(
    entry,
    mergePreview({
      blocked: true,
      blockedRecords: [
        {
          table: "file_episode_map",
          reason: "UNMAPPED_EPISODE",
          sourceId: "ep-9",
          detail: "S01E09 has no destination episode.",
        },
      ],
      // A `Long` that arrived as a string still counts.
      mediaFilesRepointed: "8",
      roleChanges: [
        {
          fileId: "file-demoted",
          sourceEpisodeId: "src-1",
          destinationEpisodeId: "dst-1",
          previousRole: "PRIMARY",
          newRole: "ADDITIONAL",
          reason: "SOURCE_PRIMARY_ALREADY_CLAIMED",
          detail: "Another moving file already claimed primary.",
        },
      ],
      historyRowsCarried: 40,
      sourceRecordsDropped: 12,
    }),
    { resolveTitleName: () => "Moving (2024)" },
  );

  assert.ok(summary);
  assert.equal(summary.statement.destinationTitleName, "Moving (2024)");
  assert.equal(summary.blocked, true);
  assert.equal(summary.blockedRecords.length, 1);
  assert.equal(summary.mediaFilesRepointed, 8);
  assert.equal(summary.historyRowsCarried, 40);
  assert.equal(summary.sourceRecordsDropped, 12);
  assert.equal(summary.roleChanges.length, 1);
  // The demotion count is what the heading warns with; FR-070 forbids a
  // silent one, so it is counted rather than inferred by the renderer.
  assert.equal(summary.demotionCount, 1);
  assert.equal(summary.empty, false);

  // A merge with nothing beyond the statement still states the statement, and
  // says outright that nothing else carries over.
  const bare = mergeSummaryPresentation(entry, mergePreview());
  assert.equal(bare?.empty, true);
  assert.equal(bare?.statement.destinationTitleId, "destination-title");

  // A merging row whose summary has not arrived is still a merge.
  const summaryless = mergeSummaryPresentation(entry, null);
  assert.equal(summaryless?.empty, true);
  assert.equal(summaryless?.blocked, false);

  // A row that is not merging produces no summary at all.
  assert.equal(mergeSummaryPresentation(crossLibraryEntry(), null), null);
});

test("only an unresolved identity still holds the plan back (FR-016)", () => {
  const ambiguous = crossLibraryEntry({
    titleId: "ambiguous",
    class: "NEEDS_RESOLUTION",
    destinationIdentityMatch: "AMBIGUOUS",
    reasonCode: "ambiguous_destination_identity",
    ambiguousDestinationTitleIds: ["cand-a"],
  });
  const merging = mergingEntry({ titleId: "merge" });
  const transferring = crossLibraryEntry({ destinationIdentityMatch: "NONE" });

  // The ambiguous row rides the NEEDS_RESOLUTION class and reaches the blocked
  // list with its Deselect. The merging row does not: a unique match is a
  // startable transfer that merges.
  assert.deepEqual(
    blockingTitles(
      classification([
        { class: "NEEDS_RESOLUTION", titles: [ambiguous] },
        { class: "CROSS_LIBRARY_TRANSFER", titles: [merging, transferring] },
      ]),
    ).map((entry) => entry.titleId),
    ["ambiguous"],
  );
  // It is not the active-work block, which has its own prose.
  assert.equal(isActiveWorkBlock(ambiguous), false);
});

function titleAssets(
  overrides: Partial<LocationTitleAssets> = {},
): LocationTitleAssets {
  return {
    titleId: "a",
    titleName: "Arrival",
    sequence: 1,
    settled: true,
    checkpointState: "COMPLETED_WITH_WARNINGS",
    renames: [
      {
        sourcePath: "/data/a/Arrival/Arrival.mkv",
        sourceName: "Arrival.mkv",
        destinationPath: "/data/b/Arrival/Arrival (from Movies 4K).mkv",
        destinationName: "Arrival (from Movies 4K).mkv",
        provenanceLabel: "Movies 4K",
        mediaFileId: "media-1",
        sizeBytes: 4096,
        done: true,
      },
    ],
    dedups: [
      {
        sourcePath: "/data/a/Arrival/Arrival.nfo",
        sourceName: "Arrival.nfo",
        survivingPath: "/data/b/Arrival/Arrival.nfo",
        survivingName: "Arrival.nfo",
        done: true,
      },
    ],
    ...overrides,
  };
}

function assetListing(
  overrides: Partial<LocationOperationAssetListing> = {},
): LocationOperationAssetListing {
  return {
    operationId: "op-1",
    titles: [titleAssets()],
    renamesTotal: 1,
    renamesDone: 1,
    dedupsTotal: 1,
    dedupsDone: 1,
    ...overrides,
  };
}

test("a title's assets read out as renames first, then dedups (FR-091)", () => {
  const lines = assetLines(titleAssets());

  assert.deepEqual(
    lines.map((line) => [line.kind, line.key, line.from, line.to]),
    [
      ["RENAME", "rename-0", "Arrival.mkv", "Arrival (from Movies 4K).mkv"],
      ["DEDUP", "dedup-0", "Arrival.nfo", "Arrival.nfo"],
    ],
  );
  // The collision suffix names the library the file came from (FR-074); a
  // dedup has no such provenance to state.
  assert.equal(lines[0].provenanceLabel, "Movies 4K");
  assert.equal(lines[1].provenanceLabel, null);
  assert.equal(assetLineTextKey("RENAME"), "move.assetRenamedAs");
  assert.equal(
    assetLineTextKey("DEDUP"),
    "move.assetDeduplicatedAgainst",
  );
  assert.deepEqual(assetLines(null), []);
  assert.deepEqual(assetLines(titleAssets({ renames: [], dedups: [] })), []);
});

test("an unsettled title's assets stay planned, never history", () => {
  const pending = titleAssets({
    settled: false,
    checkpointState: "PENDING",
    renames: [
      { ...titleAssets().renames[0], done: false },
    ],
    dedups: [{ ...titleAssets().dedups[0], done: false }],
  });

  assert.deepEqual(
    assetLines(pending).map((line) => line.done),
    [false, false],
  );

  // The listing knows there is something outstanding, which is what turns the
  // per-row done/planned labels on.
  const partial = assetListing({
    titles: [titleAssets(), pending],
    renamesTotal: 2,
    renamesDone: 1,
    dedupsTotal: 2,
    dedupsDone: 1,
  });
  assert.equal(assetListingHasPlannedWork(partial), true);
  assert.equal(
    showsAssetPlannedState(operation({ state: "CANCELED" }), partial),
    true,
    "a canceled operation with unsettled titles must not read as history",
  );
});

test("a finished operation whose assets all landed hides the done labels", () => {
  const settled = assetListing();
  assert.equal(assetListingHasPlannedWork(settled), false);
  assert.equal(
    showsAssetPlannedState(operation({ state: "COMPLETED" }), settled),
    false,
    "stamping done on every row of a finished operation is noise",
  );
  // While the operation is still running, the same listing is a snapshot, so
  // the labels stay on.
  assert.equal(
    showsAssetPlannedState(operation({ state: "MOVING" }), settled),
    true,
  );
  assert.equal(showsAssetPlannedState(operation(), null), false);
});

test("assets are looked up by title, and a plan with no collisions lists none", () => {
  const byTitle = assetsByTitle(
    assetListing({
      titles: [titleAssets(), titleAssets({ titleId: "b" }), titleAssets()],
    }),
  );
  assert.deepEqual([...byTitle.keys()], ["a", "b"]);
  assert.equal(byTitle.get("a")?.titleName, "Arrival");
  assert.equal(byTitle.get("missing"), undefined);
  assert.equal(assetsByTitle(null).size, 0);

  const empty = assetListing({
    titles: [],
    renamesTotal: 0,
    renamesDone: 0,
    dedupsTotal: 0,
    dedupsDone: 0,
  });
  assert.equal(assetListingIsEmpty(empty), true);
  assert.equal(assetListingIsEmpty(assetListing()), false);
  assert.equal(assetListingIsEmpty(null), true);
});

test("a file the stored plan cannot fully name still says what it can", () => {
  const partiallyNamed = titleAssets({
    renames: [
      {
        ...titleAssets().renames[0],
        sourceName: null,
        sourcePath: "/data/a/Arrival/Arrival.mkv",
        provenanceLabel: null,
      },
      { ...titleAssets().renames[0], sourceName: null, sourcePath: null },
    ],
    dedups: [
      {
        ...titleAssets().dedups[0],
        survivingName: null,
        survivingPath: null,
      },
    ],
  });

  const lines = assetLines(partiallyNamed);
  // The nameless source falls back to its path; the row with no source side at
  // all is dropped rather than rendered as an arrow pointing at nothing.
  assert.deepEqual(
    lines.map((line) => [line.kind, line.from, line.to]),
    [
      [
        "RENAME",
        "/data/a/Arrival/Arrival.mkv",
        "Arrival (from Movies 4K).mkv",
      ],
      ["DEDUP", "Arrival.nfo", null],
    ],
  );
});
