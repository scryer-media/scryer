import type {
  LifecycleRuleSetRecord,
  LifecycleRuleState,
} from "../types/lifecycle-rule-sets.ts";

/// Rows per page of the lifecycle rule table.
export const LIFECYCLE_RULE_PAGE_SIZE = 15;

export const LIFECYCLE_RULE_SORT_KEYS = [
  "name",
  "description",
  "graceDays",
  "schedule",
  "state",
  "revision",
] as const;

export type LifecycleRuleSortKey = (typeof LIFECYCLE_RULE_SORT_KEYS)[number];

export type LifecycleRuleSort = {
  key: LifecycleRuleSortKey;
  descending: boolean;
};

type SortableRule = Pick<
  LifecycleRuleSetRecord,
  | "id"
  | "name"
  | "description"
  | "graceDays"
  | "nextRunAt"
  | "state"
  | "currentRevisionNumber"
>;

/// Ascending reads from the rule doing least to the one doing most.
const STATE_ORDER: Record<LifecycleRuleState, number> = {
  DISABLED: 0,
  DRY_RUN: 1,
  ENABLED: 2,
};

const collator = new Intl.Collator(undefined, {
  numeric: true,
  sensitivity: "base",
});

/// Compare two values where a missing one always sorts last, whichever way
/// the column runs: an empty description or a rule with no planned run is
/// never the first thing a sort shows.
function compareMissingLast<T>(
  left: T | null,
  right: T | null,
  descending: boolean,
  compare: (left: T, right: T) => number,
): number {
  if (left === null || right === null) {
    if (left === right) return 0;
    return left === null ? 1 : -1;
  }
  const order = compare(left, right);
  return descending ? -order : order;
}

function text(value: string | null): string | null {
  const trimmed = value?.trim() ?? "";
  return trimmed ? trimmed : null;
}

function columnOrder(
  left: SortableRule,
  right: SortableRule,
  sort: LifecycleRuleSort,
): number {
  const direction = sort.descending ? -1 : 1;
  switch (sort.key) {
    case "name":
      return direction * collator.compare(left.name, right.name);
    case "description":
      return compareMissingLast(
        text(left.description),
        text(right.description),
        sort.descending,
        collator.compare,
      );
    case "graceDays":
      return direction * (left.graceDays - right.graceDays);
    case "schedule":
      // The next planned run: soonest first. Manual and disabled rules have
      // none and sort last.
      return compareMissingLast(
        left.nextRunAt ? Date.parse(left.nextRunAt) : null,
        right.nextRunAt ? Date.parse(right.nextRunAt) : null,
        sort.descending,
        (a, b) => a - b,
      );
    case "state":
      return direction * (STATE_ORDER[left.state] - STATE_ORDER[right.state]);
    case "revision":
      return direction * (left.currentRevisionNumber - right.currentRevisionNumber);
  }
}

/// The rules in the order the table shows them. Ties fall back to the name,
/// then the id, so the order never shifts between renders.
export function sortLifecycleRules<T extends SortableRule>(
  rules: readonly T[],
  sort: LifecycleRuleSort,
): T[] {
  return [...rules].sort(
    (left, right) =>
      columnOrder(left, right, sort) ||
      collator.compare(left.name, right.name) ||
      (left.id < right.id ? -1 : left.id > right.id ? 1 : 0),
  );
}

/// Pressing the active column reverses it; any other column starts ascending.
export function nextLifecycleRuleSort(
  current: LifecycleRuleSort,
  key: LifecycleRuleSortKey,
): LifecycleRuleSort {
  return current.key === key
    ? { key, descending: !current.descending }
    : { key, descending: false };
}

export type LifecycleRulePage<T> = {
  rows: T[];
  /// Zero-based, clamped to the pages that exist.
  page: number;
  pageCount: number;
  /// One-based positions of the first and last row shown, 0 when empty.
  start: number;
  end: number;
  total: number;
};

/// One page of rows. A page past the end (after deletions shrink the list)
/// shows the last page instead of an empty table.
export function lifecycleRulePage<T>(
  rows: readonly T[],
  page: number,
  pageSize: number = LIFECYCLE_RULE_PAGE_SIZE,
): LifecycleRulePage<T> {
  const total = rows.length;
  const pageCount = Math.max(1, Math.ceil(total / pageSize));
  const current = Math.min(Math.max(0, Math.trunc(page)), pageCount - 1);
  const offset = current * pageSize;
  const shown = rows.slice(offset, offset + pageSize);
  return {
    rows: shown,
    page: current,
    pageCount,
    start: shown.length ? offset + 1 : 0,
    end: offset + shown.length,
    total,
  };
}
