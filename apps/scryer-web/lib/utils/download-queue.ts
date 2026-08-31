import type {
  DownloadActivityStatus,
  DownloadClientFilterOption,
  DownloadImportStatus,
  DownloadQueueItem,
} from "@/lib/types";

export type DownloadQueueDisplayStateInput = Pick<
  DownloadQueueItem,
  | "state"
  | "attentionReason"
  | "importStatus"
  | "importErrorMessage"
  | "deleteStatus"
  | "deleteErrorMessage"
  | "trackedState"
  | "trackedStatusMessages"
>;

type QueueItemValueKey = Exclude<
  keyof DownloadQueueItem,
  "trackedStatusMessages" | "queueScope"
>;

const QUEUE_ITEM_VALUE_KEYS: readonly QueueItemValueKey[] = [
  "id",
  "titleId",
  "episodeId",
  "titleName",
  "facet",
  "isScryerOrigin",
  "sourceProvider",
  "clientId",
  "clientName",
  "clientType",
  "state",
  "displayState",
  "progressPercent",
  "importTransferPhase",
  "importTransferBytes",
  "importTransferTotalBytes",
  "importTransferStartedAt",
  "importTransferUpdatedAt",
  "sizeBytes",
  "remainingSeconds",
  "queuedAt",
  "lastUpdatedAt",
  "attentionRequired",
  "attentionReason",
  "downloadClientItemId",
  "downloadId",
  "importStatus",
  "importErrorCode",
  "importErrorMessage",
  "importedAt",
  "deleteStatus",
  "deleteErrorMessage",
  "trackedState",
  "trackedStatus",
  "trackedMatchType",
  "seedingState",
  "seedRatio",
  "seedRatioGoal",
  "seedTimeSeconds",
  "seedTimeGoalSeconds",
  "isPrivate",
];

export function sameDownloadQueueItem(
  current: DownloadQueueItem,
  next: DownloadQueueItem,
): boolean {
  const currentStatusMessages = current.trackedStatusMessages ?? [];
  const nextStatusMessages = next.trackedStatusMessages ?? [];
  return (
    current === next ||
    (QUEUE_ITEM_VALUE_KEYS.every((key) => current[key] === next[key]) &&
      currentStatusMessages.length === nextStatusMessages.length &&
      currentStatusMessages.every(
        (message, index) => message === nextStatusMessages[index],
      ) &&
      JSON.stringify(current.queueScope) === JSON.stringify(next.queueScope))
  );
}

export function reconcileDownloadQueueItems(
  current: DownloadQueueItem[],
  next: readonly DownloadQueueItem[],
): DownloadQueueItem[] {
  if (current.length === 0) {
    return [...next];
  }

  const currentByIdentity = new Map(
    current.map((item) => [downloadQueueItemIdentityKey(item), item]),
  );
  let changed = current.length !== next.length;
  const reconciled = next.map((item, index) => {
    const currentItem = currentByIdentity.get(downloadQueueItemIdentityKey(item));
    if (currentItem && sameDownloadQueueItem(currentItem, item)) {
      if (current[index] !== currentItem) {
        changed = true;
      }
      return currentItem;
    }

    changed = true;
    return item;
  });

  return changed ? reconciled : current;
}

export function downloadQueueItemIdentityKey(
  item: Pick<DownloadQueueItem, "id" | "clientId" | "clientType" | "downloadClientItemId">,
): string {
  if (!item.clientType.trim() && !item.downloadClientItemId.trim()) {
    return item.id;
  }

  if (item.clientId.trim()) {
    return `${item.clientId}::${item.downloadClientItemId}`;
  }

  return `${item.clientType}::${item.downloadClientItemId}`;
}

function parseQueueSortTimestamp(value: string | null | undefined): number {
  const parsed = Date.parse(value ?? "");
  return Number.isFinite(parsed) ? parsed : 0;
}

function queueStateSortRank(state: string | null | undefined): number {
  switch (normalizeQueueState(state)) {
    case "downloading":
    case "verifying":
    case "repairing":
    case "extracting":
      return 0;
    case "queued":
      return 1;
    case "paused":
      return 2;
    case "import_pending":
    case "importpending":
    case "completed":
      return 3;
    // Both states want the operator's attention, so they sort together at the
    // end; only their handling differs.
    case "warning":
    case "failed":
      return 4;
    default:
      return 5;
  }
}

export function normalizeQueueState(state: string | null | undefined): string {
  return (state ?? "").trim().toLowerCase();
}

export function isActiveQueueState(state: string | null | undefined): boolean {
  const normalized = normalizeQueueState(state);
  return (
    normalized === "downloading" ||
    normalized === "queued" ||
    normalized === "paused" ||
    normalized === "verifying" ||
    normalized === "repairing" ||
    normalized === "extracting"
  );
}

export function isHistoryQueueState(state: string | null | undefined): boolean {
  const normalized = normalizeQueueState(state);
  return (
    normalized === "completed" ||
    normalized === "failed" ||
    normalized === "import_pending" ||
    normalized === "importpending"
  );
}

export function isTransientQueueDisplayState(
  displayState: string | null | undefined,
): boolean {
  switch (normalizeQueueState(displayState)) {
    case "queued":
    case "downloading":
    case "paused":
    case "post_processing":
    case "importing":
    case "import_pending":
      return true;
    default:
      return false;
  }
}

export function compareDownloadQueueItems(
  left: DownloadQueueItem,
  right: DownloadQueueItem,
): number {
  const leftRank = queueStateSortRank(left.state);
  const rightRank = queueStateSortRank(right.state);
  if (leftRank !== rightRank) {
    return leftRank - rightRank;
  }

  const leftState = normalizeQueueState(left.state);
  if (
    leftState === "downloading" ||
    leftState === "verifying" ||
    leftState === "repairing" ||
    leftState === "extracting"
  ) {
    return (
      right.progressPercent - left.progressPercent ||
      left.id.localeCompare(right.id)
    );
  }

  if (leftState === "queued" || leftState === "paused") {
    return (
      parseQueueSortTimestamp(left.queuedAt) - parseQueueSortTimestamp(right.queuedAt) ||
      left.id.localeCompare(right.id)
    );
  }

  return (
    parseQueueSortTimestamp(right.lastUpdatedAt) -
      parseQueueSortTimestamp(left.lastUpdatedAt) ||
    left.id.localeCompare(right.id)
  );
}

export function sortDownloadQueueItems(
  items: DownloadQueueItem[],
): DownloadQueueItem[] {
  return [...items].sort(compareDownloadQueueItems);
}

function queueItemRecencyValue(
  item: Pick<DownloadQueueItem, "lastUpdatedAt" | "queuedAt">,
): number {
  return parseQueueSortTimestamp(item.lastUpdatedAt) ||
    parseQueueSortTimestamp(item.queuedAt);
}

function transientQueueStateRank(displayState: string | null | undefined): number {
  // `warning` ranks with the terminal states: a client that starts reporting a
  // problem is the fresher, more specific observation, exactly as `failed` was
  // before it had its own state — without a rank here the merge keeps the stale
  // downloading row and the badge never catches up. `warning` deliberately
  // stays out of `isTransientQueueDisplayState`: a warned row that disappears
  // from the authoritative list is gone, and must not be resurrected.
  switch (normalizeQueueState(displayState)) {
    case "completed":
    case "failed":
    case "remove_failed":
    case "import_failed":
    case "import_blocked":
    case "warning":
      return 6;
    case "import_pending":
      return 5;
    case "importing":
      return 4;
    case "post_processing":
      return 3;
    case "downloading":
      return 2;
    case "queued":
      return 1;
    case "paused":
      return 0;
    default:
      return -1;
  }
}

function fillMissingQueueItemFields(
  primary: DownloadQueueItem,
  secondary: DownloadQueueItem,
): DownloadQueueItem {
  const primaryHasImportOverlay = primary.importStatus !== null;
  return {
    ...primary,
    titleId: primary.titleId ?? secondary.titleId,
    episodeId: primary.episodeId ?? secondary.episodeId,
    titleName: primary.titleName.trim().length > 0
      ? primary.titleName
      : secondary.titleName,
    facet: primary.facet ?? secondary.facet,
    isScryerOrigin: primary.isScryerOrigin || secondary.isScryerOrigin,
    clientId: primary.clientId.trim().length > 0 ? primary.clientId : secondary.clientId,
    clientName: primary.clientName.trim().length > 0
      ? primary.clientName
      : secondary.clientName,
    clientType: primary.clientType.trim().length > 0
      ? primary.clientType
      : secondary.clientType,
    sizeBytes: primary.sizeBytes ?? secondary.sizeBytes,
    remainingSeconds: primary.remainingSeconds ?? secondary.remainingSeconds,
    queuedAt: primary.queuedAt ?? secondary.queuedAt,
    lastUpdatedAt: primary.lastUpdatedAt ?? secondary.lastUpdatedAt,
    attentionRequired: primary.attentionRequired || secondary.attentionRequired,
    attentionReason: primary.attentionReason ?? secondary.attentionReason,
    importStatus: primary.importStatus ?? secondary.importStatus,
    importTransferPhase: primaryHasImportOverlay
      ? primary.importTransferPhase
      : primary.importTransferPhase ?? secondary.importTransferPhase,
    importTransferBytes: primaryHasImportOverlay
      ? primary.importTransferBytes
      : primary.importTransferBytes ?? secondary.importTransferBytes,
    importTransferTotalBytes:
      primaryHasImportOverlay
        ? primary.importTransferTotalBytes
        : primary.importTransferTotalBytes ?? secondary.importTransferTotalBytes,
    importTransferStartedAt:
      primaryHasImportOverlay
        ? primary.importTransferStartedAt
        : primary.importTransferStartedAt ?? secondary.importTransferStartedAt,
    importTransferUpdatedAt:
      primaryHasImportOverlay
        ? primary.importTransferUpdatedAt
        : primary.importTransferUpdatedAt ?? secondary.importTransferUpdatedAt,
    importErrorCode: primary.importErrorCode ?? secondary.importErrorCode,
    importErrorMessage: primary.importErrorMessage ?? secondary.importErrorMessage,
    importedAt: primary.importedAt ?? secondary.importedAt,
    deleteStatus: primary.deleteStatus ?? secondary.deleteStatus,
    deleteErrorMessage: primary.deleteErrorMessage ?? secondary.deleteErrorMessage,
    trackedState: primary.trackedState ?? secondary.trackedState,
    trackedStatus: primary.trackedStatus ?? secondary.trackedStatus,
    trackedStatusMessages:
      primary.trackedStatusMessages.length > 0
        ? primary.trackedStatusMessages
        : secondary.trackedStatusMessages,
    trackedMatchType: primary.trackedMatchType ?? secondary.trackedMatchType,
    // A present observation always wins and an absent one inherits the last
    // known value, matching how the tracker retains seeding observations
    // server-side: a row that blinks to "unknown" between two sources would
    // make the badge and the numbers flicker on every poll.
    seedingState: primary.seedingState ?? secondary.seedingState,
    seedRatio: primary.seedRatio ?? secondary.seedRatio,
    seedRatioGoal: primary.seedRatioGoal ?? secondary.seedRatioGoal,
    seedTimeSeconds: primary.seedTimeSeconds ?? secondary.seedTimeSeconds,
    seedTimeGoalSeconds:
      primary.seedTimeGoalSeconds ?? secondary.seedTimeGoalSeconds,
    isPrivate: primary.isPrivate ?? secondary.isPrivate,
    queueScope: primary.queueScope ?? secondary.queueScope,
  };
}

function shouldPreferPreviousTransientItem(
  authoritative: DownloadQueueItem,
  previous: DownloadQueueItem,
): boolean {
  if (!isTransientQueueDisplayState(previous.displayState)) {
    return false;
  }

  const authoritativeRank = transientQueueStateRank(authoritative.displayState);
  const previousRank = transientQueueStateRank(previous.displayState);

  if (authoritativeRank > previousRank) {
    return false;
  }

  if (previousRank > authoritativeRank) {
    return true;
  }

  const authoritativeRecency = queueItemRecencyValue(authoritative);
  const previousRecency = queueItemRecencyValue(previous);
  if (previousRecency > authoritativeRecency) {
    return true;
  }
  if (authoritativeRecency > previousRecency) {
    return false;
  }

  if (previous.progressPercent > authoritative.progressPercent) {
    return true;
  }
  if (authoritative.progressPercent > previous.progressPercent) {
    return false;
  }

  return transientQueueStateRank(previous.displayState) >
    transientQueueStateRank(authoritative.displayState);
}

export function mergeLiveQueueItems(
  liveItems: DownloadQueueItem[],
  previousItems: DownloadQueueItem[],
): DownloadQueueItem[] {
  const previousById = new Map(
    previousItems.map((item) => [downloadQueueItemIdentityKey(item), item]),
  );
  const merged = liveItems.map((item) => {
    const previous = previousById.get(downloadQueueItemIdentityKey(item));
    return previous ? fillMissingQueueItemFields(item, previous) : item;
  });

  return sortDownloadQueueItems(merged);
}

export function mergeAuthoritativeQueueItems(
  authoritativeItems: DownloadQueueItem[],
  previousItems: DownloadQueueItem[],
): DownloadQueueItem[] {
  const previousById = new Map(
    previousItems.map((item) => [downloadQueueItemIdentityKey(item), item]),
  );
  const authoritativeById = new Map(
    authoritativeItems.map((item) => [downloadQueueItemIdentityKey(item), item]),
  );
  const merged = authoritativeItems.map((item) => {
    const previous = previousById.get(downloadQueueItemIdentityKey(item));
    if (!previous) {
      return item;
    }
    if (shouldPreferPreviousTransientItem(item, previous)) {
      return fillMissingQueueItemFields(previous, item);
    }
    return item;
  });

  for (const item of previousItems) {
    if (
      isTransientQueueDisplayState(item.displayState) &&
      !authoritativeById.has(downloadQueueItemIdentityKey(item))
    ) {
      merged.push(item);
    }
  }

  return sortDownloadQueueItems(merged);
}

export function buildQueueStatusDetail(
  queueItem: DownloadQueueDisplayStateInput,
): string {
  const messages = [
    ...(queueItem.trackedStatusMessages ?? []),
    queueItem.attentionReason,
    queueItem.deleteErrorMessage,
    queueItem.importErrorMessage,
  ]
    .map((value) => value?.trim())
    .filter((value): value is string => Boolean(value));

  return Array.from(new Set(messages)).join("\n");
}

export function isPostProcessingReason(reason: string | null | undefined): boolean {
  if (!reason) return false;
  const normalized = reason.toUpperCase();
  return (
    normalized.includes("PP_QUEUED") ||
    normalized.includes("POSTPROCESSING") ||
    normalized.includes("UNPACKING") ||
    normalized.includes("REPAIRING") ||
    normalized.includes("VERIFYING") ||
    normalized.includes("RENAMING") ||
    normalized.includes("MOVING") ||
    normalized.includes("EXECUTING_SCRIPT")
  );
}

export function deriveDownloadQueueDisplayState(
  queueItem: DownloadQueueDisplayStateInput,
): string {
  const stateKey = normalizeQueueState(queueItem.state);
  const trackedStateKey = normalizeQueueState(queueItem.trackedState);
  const failureReason = buildQueueStatusDetail(queueItem);
  const importStatusKey = normalizeQueueState(queueItem.importStatus);
  const deleteStatusKey = normalizeQueueState(queueItem.deleteStatus);

  if (deleteStatusKey === "queued" || deleteStatusKey === "running") {
    return "removing";
  }

  if (deleteStatusKey === "failed") {
    return "remove_failed";
  }

  if (stateKey === "failed") {
    return "failed";
  }

  if (
    importStatusKey === "pending" ||
    importStatusKey === "running" ||
    importStatusKey === "processing"
  ) {
    return "importing";
  }

  if (
    (importStatusKey === "failed" || importStatusKey === "skipped") &&
    (trackedStateKey === "import_blocked" ||
      stateKey === "completed" ||
      stateKey === "import_pending")
  ) {
    return "import_failed";
  }

  if (trackedStateKey === "import_blocked" || trackedStateKey === "import_pending") {
    return trackedStateKey;
  }

  const canDeriveBlockedState =
    trackedStateKey.length === 0 &&
    failureReason.length > 0 &&
    (stateKey === "completed" || stateKey === "import_pending") &&
    (importStatusKey === "skipped" || importStatusKey === "failed");
  if (canDeriveBlockedState) {
    return "import_blocked";
  }

  if (
    stateKey === "extracting" ||
    stateKey === "verifying" ||
    stateKey === "repairing"
  ) {
    return "post_processing";
  }

  if (
    stateKey === "downloading" &&
    isPostProcessingReason(queueItem.attentionReason)
  ) {
    return "post_processing";
  }

  return stateKey;
}

export function isManualImportRequiredQueueItem(
  queueItem: DownloadQueueDisplayStateInput,
): boolean {
  const state = deriveDownloadQueueDisplayState(queueItem);
  return state === "import_blocked" || state === "import_failed";
}

export const IMPORT_ATTENTION_STATUSES: DownloadImportStatus[] = [
  "PENDING",
  "BLOCKED",
  "FAILED",
];

export function downloadQueueClientFilterKey(
  item: Pick<DownloadQueueItem, "id" | "clientId" | "clientType">,
): string {
  const clientId = item.clientId.trim();
  if (clientId.length > 0) {
    return clientId;
  }

  const clientType = item.clientType.trim();
  if (clientType.length > 0) {
    return clientType.toLowerCase();
  }

  return item.id;
}

export function collectDownloadClientFilterOptions(
  items: DownloadQueueItem[],
): DownloadClientFilterOption[] {
  const seen = new Set<string>();
  const clients: DownloadClientFilterOption[] = [];

  for (const item of items) {
    const clientId = downloadQueueClientFilterKey(item);
    if (seen.has(clientId)) {
      continue;
    }
    seen.add(clientId);

    const clientName = item.clientName.trim();
    const clientType = item.clientType.trim();
    clients.push({
      clientId,
      clientName: clientName.length > 0 ? clientName : clientType,
      clientType,
    });
  }

  return clients.sort((left, right) => {
    return (
      left.clientName.localeCompare(right.clientName, undefined, { sensitivity: "base" }) ||
      left.clientType.localeCompare(right.clientType, undefined, { sensitivity: "base" }) ||
      left.clientId.localeCompare(right.clientId, undefined, { sensitivity: "base" })
    );
  });
}

export function matchesImportStatuses(
  item: Pick<DownloadQueueItem, "displayState">,
  statuses: DownloadImportStatus[],
): boolean {
  if (statuses.length === 0) {
    return false;
  }

  switch (item.displayState) {
    case "IMPORTING":
      return statuses.includes("IMPORTING");
    case "IMPORT_PENDING":
      return statuses.includes("PENDING");
    case "IMPORT_BLOCKED":
      return statuses.includes("BLOCKED");
    case "IMPORT_FAILED":
      return statuses.includes("FAILED");
    default:
      return false;
  }
}

export function matchesActivityStatuses(
  item: Pick<DownloadQueueItem, "displayState">,
  statuses: DownloadActivityStatus[],
): boolean {
  if (statuses.length === 0) {
    return false;
  }

  switch (item.displayState) {
    case "DOWNLOADING":
      return statuses.includes("DOWNLOADING");
    case "QUEUED":
      return statuses.includes("QUEUED");
    case "PAUSED":
      return statuses.includes("PAUSED");
    case "POST_PROCESSING":
      return statuses.includes("POST_PROCESSING");
    // A warned download is still live in the client, so it belongs with the
    // activity it is part of rather than with the failed history.
    case "WARNING":
      return statuses.includes("WARNING");
    default:
      return false;
  }
}
