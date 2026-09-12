import assert from "node:assert/strict";
import test from "node:test";

import type {
  LocationOperationPreview,
  LocationPlanSection,
} from "./location-operations.ts";
import {
  accountingCloses,
  changedFolderNames,
  changedFolderNamesComplete,
  consolidationGroups,
  CONSOLIDATION_GROUP_KEYS,
  retirementBlockerKey,
  rootIdentityStatement,
  rootPlanCanStart,
  rootReasonKey,
  rootRefusalCode,
  rootRefusalMessageKey,
  rootTypedPhrase,
  unmanagedPlanItems,
  type LocationConsolidationClassification,
  type LocationTitleAccounting,
} from "./root-location-operations.ts";

function graphQlError(extensions: Record<string, unknown>) {
  return { graphQLErrors: [{ message: "nope", extensions }] };
}

function plan(
  overrides: Partial<LocationOperationPreview> = {},
): LocationOperationPreview {
  return {
    planFingerprint: "fp-root-1",
    operationType: "ROOT_CHANGE",
    mode: "MOVE_WITH_SCRYER",
    sourceLibraryId: "lib",
    destinationLibraryId: "lib",
    sourceRootId: "root-a",
    destinationRootId: "root-a",
    // A root-scoped plan carries no selection at all: it takes every title
    // assigned to the root and offers no way to express a subset.
    selection: [],
    counts: {
      itemsTotal: 2,
      titlesTotal: 2,
      filesTotal: 2,
      bytesTotal: 200,
      byKind: [{ kind: "MOVE", count: 2 }],
    },
    sections: [],
    classification: { groups: [], titlesTotal: 2, blocksStart: false },
    freeSpace: {
      destinationRequiredBytes: 200,
      destinationTotalRequiredBytes: 200,
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
    verification: { depth: "FULL", files: 2, bytes: 200, applies: true },
    confirmation: {
      requirement: "TYPED",
      typedPhrase: "MOVE",
      typedPrompt: "Type MOVE to confirm this root-wide operation.",
    },
    warnings: [],
    blocksStart: false,
    ...overrides,
  };
}

function accounting(
  overrides: Partial<LocationTitleAccounting> = {},
): LocationTitleAccounting {
  return {
    assignedTotal: 3,
    relocating: 2,
    catalogOnly: 1,
    blocked: 0,
    accountsForEveryTitle: true,
    blocksStart: false,
    blockedTitles: [],
    ...overrides,
  };
}

function renameSection(
  items: LocationPlanSection["items"],
  complete = true,
): LocationPlanSection {
  return {
    kind: "RENAME",
    itemsTotal: items.length,
    bytesTotal: 0,
    complete,
    items,
  };
}

// ── FR-020: one control, two destinations ───────────────────────────────────

test("a refusal code is read off the extensions, never off the sentence", () => {
  assert.equal(
    rootRefusalCode(
      graphQlError({
        code: "LOCATION_ROOT_REFUSED",
        refusalCode: "root_change_destination_is_configured_root",
      }),
    ),
    "root_change_destination_is_configured_root",
  );
  // A code from another family is not ours to route on.
  assert.equal(
    rootRefusalCode(graphQlError({ refusalCode: "stale_plan" })),
    null,
  );
  assert.equal(rootRefusalCode(graphQlError({})), null);
  assert.equal(rootRefusalCode(new Error("network")), null);
  assert.equal(rootRefusalCode(null), null);
});

test("every refusal code has its own message key", () => {
  assert.equal(
    rootRefusalMessageKey("root_consolidation_mode_not_supported"),
    "rootChange.refusal.root_consolidation_mode_not_supported",
  );
});

// ── FR-023: the ledger closes, or the dialog says so ────────────────────────

test("the every-title ledger has to close before anything may be confirmed", () => {
  assert.equal(accountingCloses(accounting()), true);
  // The server's own verdict, and the arithmetic, are both checked: either one
  // failing means a title went missing between the root and the ledger.
  assert.equal(
    accountingCloses(accounting({ accountsForEveryTitle: false })),
    false,
  );
  assert.equal(accountingCloses(accounting({ relocating: 1 })), false);
  assert.equal(accountingCloses(null), false);
});

test("a blocked title stops the start, and unexplained content does not", () => {
  assert.equal(rootPlanCanStart(plan(), accounting()), true);
  assert.equal(
    rootPlanCanStart(plan(), accounting({ blocked: 1, blocksStart: true })),
    false,
  );
  assert.equal(rootPlanCanStart(plan({ blocksStart: true }), accounting()), false);
  assert.equal(
    rootPlanCanStart(
      plan({ classification: { groups: [], titlesTotal: 2, blocksStart: true } }),
      accounting(),
    ),
    false,
  );
  assert.equal(rootPlanCanStart(null, accounting()), false);
});

test("a measured shortfall blocks the start and an unmeasured destination does not", () => {
  const withSpace = (sufficient: boolean | null) =>
    plan({ freeSpace: { ...plan().freeSpace, sufficient } });
  assert.equal(rootPlanCanStart(withSpace(false), accounting()), false);
  assert.equal(rootPlanCanStart(withSpace(true), accounting()), true);
  // FR-080: null is "could not be measured", which stays startable.
  assert.equal(rootPlanCanStart(withSpace(null), accounting()), true);
});

// ── FR-021: what the root keeps ─────────────────────────────────────────────

test("the identity statement says what the root keeps, including the default", () => {
  const statement = rootIdentityStatement({
    rootId: "root-a",
    keepsRootId: true,
    wasLibraryDefault: true,
    remainsLibraryDefault: true,
    retainedRole: null,
    retainedTitleAssignments: "7",
  });
  assert.deepEqual(statement, {
    keepsRootId: true,
    keepsDefault: true,
    losesDefault: false,
    titleAssignments: 7,
  });

  // A root that was the default and is not one afterwards is stated rather
  // than hidden; nothing in a root change should do this.
  const demoted = rootIdentityStatement({
    rootId: "root-a",
    keepsRootId: true,
    wasLibraryDefault: true,
    remainsLibraryDefault: false,
    retainedRole: null,
    retainedTitleAssignments: 0,
  });
  assert.equal(demoted?.losesDefault, true);
  assert.equal(demoted?.keepsDefault, false);
  assert.equal(rootIdentityStatement(null), null);
});

// ── FR-024: the seven groups are the consolidation preview ──────────────────

test("all seven FR-024 groups are rendered, including the empty ones", () => {
  const classification: LocationConsolidationClassification = {
    movingIntoUnusedFolders: 4,
    mergingWithDestinationTitles: 1,
    folderNameCollisions: 2,
    mediaCollisions: "3",
    dedupEligibleFiles: 0,
    companionCollisions: 0,
    untrackedSourceEntries: 5,
    catalogOnly: 1,
    blocked: 0,
  };
  const groups = consolidationGroups(classification);
  assert.deepEqual(
    groups.map((group) => group.key),
    [...CONSOLIDATION_GROUP_KEYS],
  );
  assert.equal(groups.length, 7);
  assert.deepEqual(
    groups.map((group) => group.count),
    [4, 1, 2, 3, 0, 0, 5],
  );
  assert.deepEqual(consolidationGroups(null), []);
});

// ── US5.4: every changed folder name, by name ───────────────────────────────

test("every uniqued folder is listed by the name it had and the name it gets", () => {
  const preview = plan({
    operationType: "ROOT_CONSOLIDATION",
    sections: [
      renameSection([
        {
          kind: "RENAME",
          titleId: "title-1",
          mediaFileId: null,
          sourcePath: "/old/Blade Runner (1982)",
          destinationPath: "/new/Blade Runner (1982) (from old-disk)",
          sizeBytes: 0,
          sameVolume: null,
          reasonCode: "folder_name_uniqued",
          detail: "the destination already owns that folder name",
        },
        // A file rename around a destination collision belongs to the
        // collision counts, not to the changed-folder list.
        {
          kind: "RENAME",
          titleId: "title-1",
          mediaFileId: "file-1",
          sourcePath: "/old/Blade Runner (1982)/br.mkv",
          destinationPath: "/new/Blade Runner (1982) (from old-disk)/br (2).mkv",
          sizeBytes: 100,
          sameVolume: null,
          reasonCode: "collision_renamed",
          detail: null,
        },
      ]),
    ],
  });

  const lines = changedFolderNames(preview);
  assert.equal(lines.length, 1);
  assert.deepEqual(lines[0], {
    titleId: "title-1",
    from: "Blade Runner (1982)",
    to: "Blade Runner (1982) (from old-disk)",
    detail: "the destination already owns that folder name",
  });
  assert.equal(changedFolderNamesComplete(preview), true);
  // A plan with no rename section renames nothing, and says so as an empty
  // list rather than as an unknown.
  assert.deepEqual(changedFolderNames(plan()), []);
  assert.equal(changedFolderNamesComplete(plan()), true);
});

test("a sampled rename section says it was sampled", () => {
  const preview = plan({
    sections: [
      {
        ...renameSection([], false),
        itemsTotal: 400,
      },
    ],
  });
  assert.equal(changedFolderNamesComplete(preview), false);
});

test("unmanaged plan items come from the plan's own section", () => {
  const preview = plan({
    sections: [
      {
        kind: "UNMANAGED_CONTENT",
        itemsTotal: 1,
        bytesTotal: 12,
        complete: true,
        items: [
          {
            kind: "UNMANAGED_CONTENT",
            titleId: null,
            mediaFileId: null,
            sourcePath: "/old/stranger.txt",
            destinationPath: null,
            sizeBytes: 12,
            sameVolume: null,
            reasonCode: "unknown_root_content",
            detail: "nothing accounts for this file",
          },
        ],
      },
    ],
  });
  assert.equal(unmanagedPlanItems(preview).length, 1);
  assert.deepEqual(unmanagedPlanItems(plan()), []);
});

// ── FR-029 and the reason vocabulary ────────────────────────────────────────

test("the typed phrase comes off the plan, never out of the client", () => {
  assert.equal(rootTypedPhrase(plan()), "MOVE");
  assert.equal(
    rootTypedPhrase(
      plan({
        confirmation: {
          requirement: "SIMPLE",
          typedPhrase: null,
          typedPrompt: null,
        },
      }),
    ),
    null,
  );
  // A TYPED requirement with no phrase is not a phrase of our invention.
  assert.equal(
    rootTypedPhrase(
      plan({
        confirmation: {
          requirement: "TYPED",
          typedPhrase: "   ",
          typedPrompt: null,
        },
      }),
    ),
    null,
  );
  assert.equal(rootTypedPhrase(null), null);
});

test("both planners' reason codes translate, and anything else falls back", () => {
  assert.equal(
    rootReasonKey("root_identity_retained"),
    "rootChange.reason.root_identity_retained",
  );
  assert.equal(
    rootReasonKey("folder_name_uniqued"),
    "rootChange.reason.folder_name_uniqued",
  );
  // Not ours: the plan item's own sentence is what gets shown.
  assert.equal(rootReasonKey("adopted_at_destination"), null);
  assert.equal(rootReasonKey(null), null);
});

test("the two retirement blockers translate", () => {
  assert.equal(
    retirementBlockerKey("blocked_titles"),
    "rootChange.retirementBlocker.blocked_titles",
  );
  assert.equal(
    retirementBlockerKey("unexplained_source_content"),
    "rootChange.retirementBlocker.unexplained_source_content",
  );
  assert.equal(retirementBlockerKey("something_else"), null);
});
