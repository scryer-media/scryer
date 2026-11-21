import { lifecycleCountdown } from "./lifecycle-rule-sets.ts";

/**
 * The upcoming lifecycle actions panel: what one placement shows, read from
 * the settings saved with it, and how a candidate's timing reads. Settings
 * come back from the server as whatever an older or newer build saved, so
 * reading them never throws: unknown keys are ignored and a malformed value
 * falls back to its default.
 */

/**
 * The action groups a placement can narrow to, in the order the server applies
 * the actions. Each is one rule head; a group also covers the season and
 * episode forms of that head.
 */
export const LIFECYCLE_UPCOMING_ACTION_GROUPS = [
  "unmonitor",
  "delete_files",
  "delete_title",
  "search",
  "set_quality_profile",
  "add_tags",
  "remove_tags",
  "monitor",
  "move_to_library",
  "move_to_root",
  "notify",
  "fetch_subtitles",
] as const;

export type LifecycleUpcomingActionGroup =
  (typeof LIFECYCLE_UPCOMING_ACTION_GROUPS)[number];

/** The API heads each group stands for, title head first. */
const GROUP_HEADS: Readonly<
  Record<LifecycleUpcomingActionGroup, readonly string[]>
> = {
  unmonitor: ["UNMONITOR", "UNMONITOR_SEASON", "UNMONITOR_EPISODE"],
  delete_files: ["DELETE_FILES", "DELETE_SEASON_FILES", "DELETE_EPISODE_FILES"],
  delete_title: ["DELETE_TITLE"],
  search: ["SEARCH"],
  set_quality_profile: ["SET_QUALITY_PROFILE"],
  add_tags: ["ADD_TAGS"],
  remove_tags: ["REMOVE_TAGS"],
  monitor: ["MONITOR", "MONITOR_SEASON", "MONITOR_EPISODE"],
  move_to_library: ["MOVE_TO_LIBRARY"],
  move_to_root: ["MOVE_TO_ROOT"],
  notify: ["NOTIFY", "NOTIFY_SEASON", "NOTIFY_EPISODE"],
  fetch_subtitles: [
    "FETCH_SUBTITLES",
    "FETCH_SEASON_SUBTITLES",
    "FETCH_EPISODE_SUBTITLES",
  ],
};

export const LIFECYCLE_UPCOMING_NAME_MAX_LENGTH = 60;

export type LifecycleUpcomingPanelSettings = {
  /** Empty shows every action. */
  actions: LifecycleUpcomingActionGroup[];
  /** Empty uses the panel's own title. */
  name: string;
};

function isActionGroup(value: unknown): value is LifecycleUpcomingActionGroup {
  return (
    typeof value === "string" &&
    (LIFECYCLE_UPCOMING_ACTION_GROUPS as readonly string[]).includes(value)
  );
}

function panelName(value: unknown): string {
  if (typeof value !== "string") {
    return "";
  }
  return value.trim().slice(0, LIFECYCLE_UPCOMING_NAME_MAX_LENGTH).trim();
}

/** A placement's settings from whatever was saved with it. */
export function lifecycleUpcomingPanelSettings(
  saved: unknown,
): LifecycleUpcomingPanelSettings {
  const source =
    typeof saved === "object" && saved !== null && !Array.isArray(saved)
      ? (saved as Record<string, unknown>)
      : {};
  const chosen = Array.isArray(source.actions)
    ? new Set(source.actions.filter(isActionGroup))
    : new Set<LifecycleUpcomingActionGroup>();
  return {
    actions: LIFECYCLE_UPCOMING_ACTION_GROUPS.filter((group) =>
      chosen.has(group),
    ),
    name: panelName(source.name),
  };
}

/**
 * What a placement saves: null for the defaults, so a placement nobody
 * configured stores nothing.
 */
export function lifecycleUpcomingSettingsValue(
  settings: LifecycleUpcomingPanelSettings,
): Record<string, unknown> | null {
  const normalized = lifecycleUpcomingPanelSettings(settings);
  if (normalized.actions.length === 0 && normalized.name === "") {
    return null;
  }
  return { actions: normalized.actions, name: normalized.name };
}

/** The group's title head, whose label names the group. */
export function lifecycleUpcomingGroupHead(
  group: LifecycleUpcomingActionGroup,
): string {
  return GROUP_HEADS[group][0];
}

/**
 * The `actions` argument for the candidates query, or undefined to send none
 * when every action is shown.
 */
export function lifecycleUpcomingActionHeads(
  actions: readonly LifecycleUpcomingActionGroup[],
): string[] | undefined {
  if (actions.length === 0) {
    return undefined;
  }
  return actions.flatMap((group) => GROUP_HEADS[group]);
}

export type LifecycleUpcomingTiming =
  | { due: true }
  /** A null key means the due time did not parse; show it as sent. */
  | { due: false; labelKey: string | null; values: { count: number } };

/**
 * When a candidate's action runs: now, for one already due or waiting on the
 * handler, or for a due time already past; otherwise how long until it is.
 */
export function lifecycleUpcomingTiming(
  candidate: { state: string; dueAt: string },
  now: number = Date.now(),
): LifecycleUpcomingTiming {
  if (candidate.state === "DUE" || candidate.state === "PENDING_ACTION") {
    return { due: true };
  }
  const countdown = lifecycleCountdown(candidate.dueAt, now);
  if (!countdown) {
    return { due: false, labelKey: null, values: { count: 0 } };
  }
  if (
    countdown.overdue ||
    countdown.labelKey === "settings.lifecycleCountdownDueNow"
  ) {
    return { due: true };
  }
  return { due: false, labelKey: countdown.labelKey, values: countdown.values };
}
