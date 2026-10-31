import type { DownloadQueueItem, DownloadSeedingState } from "@/lib/types";

/**
 * The queue's read-only view of a torrent's seeding obligation.
 *
 * Everything here is derived from the six nullable fields the queue payload
 * carries (`seedingState`, `seedRatio`, `seedRatioGoal`, `seedTimeSeconds`,
 * `seedTimeGoalSeconds`, `isPrivate`). The rules the backend set are preserved
 * verbatim on the way to the screen:
 *
 * - `seedingState: null` means the row carries no torrent seeding information
 *   at all (usenet, or a client that reports none) and renders nothing.
 * - `seedingState: "NONE"` means "it is a torrent, and there is nothing to
 *   report yet" — still downloading, a silent client, or torrent-blackhole,
 *   which deliberately maps to `NONE` because it has no session to observe.
 *   It also renders nothing.
 * - An absent observation renders nothing for that axis. A goal is only ever
 *   shown beside an observation, and an unknown value is never shown as 0.
 */
export type SeedingProgressPresentation = {
  stateKey: Exclude<DownloadSeedingState, "NONE">;
  /** i18n key for the seeding state label. */
  labelKey: string;
  /** Token-based emphasis for the state label. */
  toneClass: string;
  /** `"0.8 / 1.5"` when a goal is resolved, `"0.8"` when not, null when unobserved. */
  ratioLabel: string | null;
  /** `"3d 4h / 7d"` when a goal is resolved, `"3d 4h"` when not, null when unobserved. */
  seedTimeLabel: string | null;
};

const SEEDING_STATE_LABEL_KEYS: Record<
  Exclude<DownloadSeedingState, "NONE">,
  string
> = {
  SEEDING: "queue.seeding.stateSeeding",
  GOAL_MET: "queue.seeding.stateGoalMet",
  HELD_PRIVATE: "queue.seeding.stateHeldPrivate",
  NEVER_REMOVE: "queue.seeding.stateNeverRemove",
};

/**
 * Existing status-token semantics only: a discharged obligation is a success,
 * an obligation still running is informational, and a deliberate hold reuses
 * the accent tone the queue already gives to "waiting on purpose"
 * (`import_pending`). None of these is a warning — a held torrent is healthy.
 */
const SEEDING_STATE_TONE_CLASSES: Record<
  Exclude<DownloadSeedingState, "NONE">,
  string
> = {
  SEEDING: "text-[var(--scry-info-text)]",
  GOAL_MET: "text-[var(--scry-success-text)]",
  HELD_PRIVATE: "text-[var(--scry-accent-text)]",
  NEVER_REMOVE: "text-[var(--scry-accent-text)]",
};

function observedNumber(value: number | null | undefined): number | null {
  if (value === null || value === undefined) {
    return null;
  }
  if (!Number.isFinite(value) || value < 0) {
    return null;
  }
  return value;
}

/**
 * Compact share-ratio text. Two decimals at most, trailing zeros dropped, so an
 * observed `0.8333` reads `0.83` and a goal typed as `1.5` in settings still
 * reads `1.5` here.
 */
export function formatSeedRatio(
  ratio: number | null | undefined,
): string | null {
  const value = observedNumber(ratio);
  return value === null ? null : String(Number.parseFloat(value.toFixed(2)));
}

/**
 * Humanized seeding duration. Seeding runs for days, so the queue's clock-style
 * `formatRemainingDuration` would read as nonsense here; this is coarse on
 * purpose and never shows a unit finer than the one above it needs.
 */
export function formatSeedDuration(
  seconds: number | null | undefined,
): string | null {
  const value = observedNumber(seconds);
  if (value === null) {
    return null;
  }
  const totalSeconds = Math.floor(value);
  const days = Math.floor(totalSeconds / 86400);
  const hours = Math.floor((totalSeconds % 86400) / 3600);
  const minutes = Math.floor((totalSeconds % 3600) / 60);

  if (days > 0) {
    return hours > 0 ? `${days}d ${hours}h` : `${days}d`;
  }
  if (hours > 0) {
    return minutes > 0 ? `${hours}h ${minutes}m` : `${hours}h`;
  }
  // A sub-minute observation is real, but "0m" would read like the
  // zero-for-unknown this module promises never to show.
  return minutes > 0 ? `${minutes}m` : "<1m";
}

function axisLabel(
  observed: string | null,
  goal: string | null,
): string | null {
  if (observed === null) {
    return null;
  }
  return goal === null ? observed : `${observed} / ${goal}`;
}

export type SeedingProgressInput = Pick<
  DownloadQueueItem,
  | "seedingState"
  | "seedRatio"
  | "seedRatioGoal"
  | "seedTimeSeconds"
  | "seedTimeGoalSeconds"
>;

/**
 * Returns null for every row that has nothing to say about seeding: usenet
 * rows, rows whose client reports nothing, and `NONE`. A state that *is*
 * reportable still returns a presentation when neither axis was observed —
 * "this torrent is held because it is private" is the whole point of the badge,
 * and it is knowable without a single number.
 */
export function deriveSeedingProgress(
  queueItem: SeedingProgressInput,
): SeedingProgressPresentation | null {
  const stateKey = queueItem.seedingState;
  if (stateKey === null || stateKey === undefined || stateKey === "NONE") {
    return null;
  }

  return {
    stateKey,
    labelKey: SEEDING_STATE_LABEL_KEYS[stateKey],
    toneClass: SEEDING_STATE_TONE_CLASSES[stateKey],
    ratioLabel: axisLabel(
      formatSeedRatio(queueItem.seedRatio),
      formatSeedRatio(queueItem.seedRatioGoal),
    ),
    seedTimeLabel: axisLabel(
      formatSeedDuration(queueItem.seedTimeSeconds),
      formatSeedDuration(queueItem.seedTimeGoalSeconds),
    ),
  };
}

/**
 * `true` only for a torrent the client positively reported as private. `null`
 * is unknown, never "public", so it renders nothing rather than an indicator
 * that says the opposite of what is known.
 */
export function isPrivateTorrentRow(
  queueItem: Pick<DownloadQueueItem, "isPrivate">,
): boolean {
  return queueItem.isPrivate === true;
}

/**
 * A tracked download that finished importing but is still discharging its
 * seeding obligation. The backend reports this as its own tracked state while
 * the display state stays `COMPLETED`, so the badge has to read the tracked
 * state to tell the two apart.
 */
export function isImportedSeedingRow(
  queueItem: Pick<DownloadQueueItem, "trackedState">,
): boolean {
  return queueItem.trackedState === "IMPORTED_SEEDING";
}
