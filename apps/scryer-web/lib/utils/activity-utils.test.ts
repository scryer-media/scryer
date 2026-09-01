import assert from "node:assert/strict";
import test, { after, before } from "node:test";
import { fileURLToPath } from "node:url";
import type { DownloadQueueItem } from "../types/download-queue.ts";
import { createServer, type ViteDevServer } from "vite";

const WEB_ROOT = fileURLToPath(new URL("../..", import.meta.url));

type QueueRowResult = {
  failureReason: string;
  hasStatusDetails: boolean;
  hasExpandableDetails: boolean;
  displayStateKey: string;
  statusBadgeKey: string;
  statusLabel: string;
};

type DeriveQueueRowPresentation = (
  item: DownloadQueueItem,
  translate: (key: string) => string,
) => QueueRowResult;

let server: ViteDevServer;
let deriveQueueRowPresentation: DeriveQueueRowPresentation;

before(async () => {
  server = await createServer({
    root: WEB_ROOT,
    server: { middlewareMode: true },
    appType: "custom",
    logLevel: "silent",
  });
  const module = await server.ssrLoadModule("/lib/utils/activity-utils.ts");
  deriveQueueRowPresentation =
    module.deriveQueueRowPresentation as DeriveQueueRowPresentation;
});

after(async () => {
  await server.close();
});

const translations: Record<string, string> = {
  "queue.state.completed": "Completed",
  "queue.state.warning": "Warning",
  "queue.state.importedSeeding": "Imported · Seeding",
  "queue.blockReasonFallbackUnassigned":
    "Automatic import could not identify a library title. Assign a title to continue.",
  "queue.blockReasonFallbackEpisodic":
    "Automatic import could not determine a unique season and episode mapping. Open Manual Import and assign the correct season and episode.",
  "queue.blockReasonFallbackReview":
    "Automatic import needs operator review. Open Manual Import and confirm the file mapping to continue.",
};

const translate = (key: string) => translations[key] ?? key;

function blockedItem(overrides: Partial<DownloadQueueItem> = {}): DownloadQueueItem {
  return {
    id: "queue-1",
    titleId: null,
    episodeId: null,
    titleName:
      "[Erai-raws].Yuki-sama.Kagami.no.Toki.Desu-09.[1080p][Multiple.Subtitle][AA7AC7E5]",
    facet: "ANIME",
    isScryerOrigin: true,
    sourceProvider: null,
    clientId: "client-1",
    clientName: "Weaver",
    clientType: "weaver",
    state: "COMPLETED",
    displayState: "IMPORT_BLOCKED",
    progressPercent: 100,
    importTransferPhase: null,
    importTransferBytes: null,
    importTransferTotalBytes: null,
    importTransferStartedAt: null,
    importTransferUpdatedAt: null,
    sizeBytes: null,
    remainingSeconds: null,
    queuedAt: null,
    lastUpdatedAt: null,
    attentionRequired: true,
    attentionReason: null,
    downloadClientItemId: "download-1",
    downloadId: "scryer-download:queue-1",
    importStatus: null,
    importErrorCode: null,
    importErrorMessage: null,
    importedAt: null,
    deleteStatus: null,
    deleteErrorMessage: null,
    trackedState: "IMPORT_BLOCKED",
    trackedStatus: "WARNING",
    trackedStatusMessages: [],
    trackedMatchType: "UNMATCHED",
    seedingState: null,
    seedRatio: null,
    seedRatioGoal: null,
    seedTimeSeconds: null,
    seedTimeGoalSeconds: null,
    isPrivate: null,
    queueScope: null,
    ...overrides,
  };
}

test("blocked queue rows explain that an unassigned title needs assignment", () => {
  const row = deriveQueueRowPresentation(blockedItem(), translate);

  assert.equal(
    row.failureReason,
    translations["queue.blockReasonFallbackUnassigned"],
  );
  assert.equal(row.hasStatusDetails, true);
  assert.notEqual(row.failureReason, "—");
});

test("blocked episodic rows direct the operator to season and episode mapping", () => {
  const row = deriveQueueRowPresentation(
    blockedItem({ titleId: "title-1", trackedMatchType: "SUBMISSION" }),
    translate,
  );

  assert.equal(
    row.failureReason,
    translations["queue.blockReasonFallbackEpisodic"],
  );
});

test("blocked movie rows direct the operator to review the file mapping", () => {
  const row = deriveQueueRowPresentation(
    blockedItem({ titleId: "title-1", facet: "MOVIE" }),
    translate,
  );

  assert.equal(
    row.failureReason,
    translations["queue.blockReasonFallbackReview"],
  );
});

test("an imported torrent that is still seeding gets its own badge", () => {
  const row = deriveQueueRowPresentation(
    blockedItem({
      displayState: "IMPORTED_SEEDING",
      trackedState: "IMPORTED_SEEDING",
      trackedStatus: "OK",
      trackedMatchType: "SUBMISSION",
      titleId: "title-1",
      attentionRequired: false,
      seedingState: "SEEDING",
      seedRatio: 0.8,
      seedRatioGoal: 1.5,
    }),
    translate,
  );

  assert.equal(row.displayStateKey, "IMPORTED_SEEDING");
  assert.equal(row.statusBadgeKey, "IMPORTED_SEEDING");
  assert.equal(row.statusLabel, translations["queue.state.importedSeeding"]);
});

test("a finished download without a seeding hold keeps the completed badge", () => {
  const row = deriveQueueRowPresentation(
    blockedItem({
      displayState: "COMPLETED",
      trackedState: "IMPORTED",
      trackedStatus: "OK",
      trackedMatchType: "SUBMISSION",
      titleId: "title-1",
      attentionRequired: false,
    }),
    translate,
  );

  assert.equal(row.statusBadgeKey, "COMPLETED");
  assert.equal(row.statusLabel, translations["queue.state.completed"]);
});

test("a warned download reads as a warning and shows the client's message", () => {
  // qBittorrent's `error` / `missingFiles` reach the queue as WARNING with the
  // client's own message, and must not be dressed up as a failed grab.
  const row = deriveQueueRowPresentation(
    blockedItem({
      state: "WARNING",
      displayState: "WARNING",
      trackedState: "DOWNLOADING",
      trackedStatus: "WARNING",
      trackedMatchType: "SUBMISSION",
      titleId: "title-1",
      progressPercent: 42,
      attentionRequired: true,
      attentionReason: "files are missing from the save path",
    }),
    translate,
  );

  assert.equal(row.statusBadgeKey, "WARNING");
  assert.equal(row.statusLabel, translations["queue.state.warning"]);
  assert.equal(row.failureReason, "files are missing from the save path");
  assert.equal(row.hasStatusDetails, true);
  assert.equal(row.hasExpandableDetails, true);
});

test("a warning on a still-seeding import wins over the seeding badge", () => {
  // The IMPORTED_SEEDING badge only exists because those rows display as
  // COMPLETED. When the client is reporting a live problem the display state is
  // already the specific one, and hiding it behind "Imported · Seeding" is how
  // a stuck torrent looks healthy.
  const row = deriveQueueRowPresentation(
    blockedItem({
      state: "WARNING",
      displayState: "WARNING",
      trackedState: "IMPORTED_SEEDING",
      trackedStatus: "OK",
      trackedMatchType: "SUBMISSION",
      titleId: "title-1",
      attentionRequired: true,
      attentionReason: "files are missing from the save path",
      seedingState: "SEEDING",
    }),
    translate,
  );

  assert.equal(row.statusBadgeKey, "WARNING");
  assert.equal(row.statusLabel, translations["queue.state.warning"]);
  assert.equal(row.failureReason, "files are missing from the save path");
  assert.equal(row.hasExpandableDetails, true);
});

test("backend import-block detail takes precedence over frontend fallback copy", () => {
  const backendReason =
    "Automatic import could not choose a season for episode 9 because the downloaded filename is obfuscated.";
  const row = deriveQueueRowPresentation(
    blockedItem({
      titleId: "title-1",
      trackedStatusMessages: [backendReason],
    }),
    translate,
  );

  assert.equal(row.failureReason, backendReason);
});
