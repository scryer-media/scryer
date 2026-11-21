/// Sorting and paging shared by the rule tables. Each rule list arrives whole,
/// so its table sorts and pages it in the page.

/// Rows per page of a rule table.
export const RULE_TABLE_PAGE_SIZE = 15;

export type RuleTableSort<Key extends string> = {
  key: Key;
  descending: boolean;
};

export const ruleTableCollator = new Intl.Collator(undefined, {
  numeric: true,
  sensitivity: "base",
});

/// Compare two values where a missing one always sorts last, whichever way
/// the column runs: an empty description or a rule with no planned run is
/// never the first thing a sort shows.
export function compareMissingLast<T>(
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

/// Trimmed text, or null when nothing is left.
export function presentText(value: string | null | undefined): string | null {
  const trimmed = value?.trim() ?? "";
  return trimmed ? trimmed : null;
}

/// The rows in the order the table shows them. Ties fall back to the name,
/// then the id, so the order never shifts between renders.
export function sortRuleRows<
  Row extends { id: string; name: string },
  Key extends string,
>(
  rows: readonly Row[],
  sort: RuleTableSort<Key>,
  columnOrder: (left: Row, right: Row, sort: RuleTableSort<Key>) => number,
): Row[] {
  return [...rows].sort(
    (left, right) =>
      columnOrder(left, right, sort) ||
      ruleTableCollator.compare(left.name, right.name) ||
      (left.id < right.id ? -1 : left.id > right.id ? 1 : 0),
  );
}

/// Pressing the active column reverses it; any other column starts ascending.
export function nextRuleTableSort<Key extends string>(
  current: RuleTableSort<Key>,
  key: Key,
): RuleTableSort<Key> {
  return current.key === key
    ? { key, descending: !current.descending }
    : { key, descending: false };
}

export type RuleTablePage<T> = {
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
export function ruleTablePage<T>(
  rows: readonly T[],
  page: number,
  pageSize: number = RULE_TABLE_PAGE_SIZE,
): RuleTablePage<T> {
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

// ---- Scoring rules ----

export const SCORING_RULE_SORT_KEYS = [
  "name",
  "description",
  "appliedFacets",
  "enabled",
] as const;

export type ScoringRuleSortKey = (typeof SCORING_RULE_SORT_KEYS)[number];

type SortableScoringRule = {
  id: string;
  name: string;
  description: string | null;
  appliedFacets: readonly string[];
  enabled: boolean;
};

/// Scoring rules in table order. A rule with no facets applies everywhere
/// and sorts with the facet-less rules last, as a missing value does.
export function sortScoringRules<T extends SortableScoringRule>(
  rules: readonly T[],
  sort: RuleTableSort<ScoringRuleSortKey>,
): T[] {
  return sortRuleRows(rules, sort, (left, right, { key, descending }) => {
    const direction = descending ? -1 : 1;
    switch (key) {
      case "name":
        return direction * ruleTableCollator.compare(left.name, right.name);
      case "description":
        return compareMissingLast(
          presentText(left.description),
          presentText(right.description),
          descending,
          ruleTableCollator.compare,
        );
      case "appliedFacets":
        return compareMissingLast(
          presentText([...left.appliedFacets].sort().join(", ")),
          presentText([...right.appliedFacets].sort().join(", ")),
          descending,
          ruleTableCollator.compare,
        );
      case "enabled":
        // Ascending reads from the rule doing least to the one doing most.
        return direction * (Number(left.enabled) - Number(right.enabled));
    }
  });
}

// ---- Request rules ----

export const REQUEST_RULE_SORT_KEYS = [
  "name",
  "description",
  "enabled",
  "libraries",
  "decisions",
] as const;

export type RequestRuleSortKey = (typeof REQUEST_RULE_SORT_KEYS)[number];

type SortableRequestRule = {
  id: string;
  name: string;
  description: string | null;
  enabled: boolean;
  decisionCount: number;
};

/// Request rules in table order. `libraryLabel` is the text the Libraries
/// column shows for a rule, so the column sorts by what it reads.
export function sortRequestRules<T extends SortableRequestRule>(
  rules: readonly T[],
  sort: RuleTableSort<RequestRuleSortKey>,
  libraryLabel: (rule: T) => string,
): T[] {
  return sortRuleRows(rules, sort, (left, right, { key, descending }) => {
    const direction = descending ? -1 : 1;
    switch (key) {
      case "name":
        return direction * ruleTableCollator.compare(left.name, right.name);
      case "description":
        return compareMissingLast(
          presentText(left.description),
          presentText(right.description),
          descending,
          ruleTableCollator.compare,
        );
      case "enabled":
        return direction * (Number(left.enabled) - Number(right.enabled));
      case "libraries":
        return (
          direction *
          ruleTableCollator.compare(libraryLabel(left), libraryLabel(right))
        );
      case "decisions":
        return direction * (left.decisionCount - right.decisionCount);
    }
  });
}
