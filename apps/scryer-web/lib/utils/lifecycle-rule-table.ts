import type {
  LifecycleRuleSetRecord,
  LifecycleRuleState,
} from "../types/lifecycle-rule-sets.ts";
import {
  compareMissingLast,
  nextRuleTableSort,
  presentText,
  RULE_TABLE_PAGE_SIZE,
  ruleTableCollator,
  ruleTablePage,
  sortRuleRows,
  type RuleTablePage,
  type RuleTableSort,
} from "./rule-table.ts";

/// Rows per page of the lifecycle rule table.
export const LIFECYCLE_RULE_PAGE_SIZE = RULE_TABLE_PAGE_SIZE;

export const LIFECYCLE_RULE_SORT_KEYS = [
  "name",
  "description",
  "graceDays",
  "schedule",
  "state",
] as const;

export type LifecycleRuleSortKey = (typeof LIFECYCLE_RULE_SORT_KEYS)[number];

export type LifecycleRuleSort = RuleTableSort<LifecycleRuleSortKey>;

type SortableRule = Pick<
  LifecycleRuleSetRecord,
  "id" | "name" | "description" | "graceDays" | "nextRunAt" | "state"
>;

/// Ascending reads from the rule doing least to the one doing most.
const STATE_ORDER: Record<LifecycleRuleState, number> = {
  DISABLED: 0,
  DRY_RUN: 1,
  ENABLED: 2,
};

function columnOrder(
  left: SortableRule,
  right: SortableRule,
  sort: LifecycleRuleSort,
): number {
  const direction = sort.descending ? -1 : 1;
  switch (sort.key) {
    case "name":
      return direction * ruleTableCollator.compare(left.name, right.name);
    case "description":
      return compareMissingLast(
        presentText(left.description),
        presentText(right.description),
        sort.descending,
        ruleTableCollator.compare,
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
  }
}

/// The rules in the order the table shows them. Ties fall back to the name,
/// then the id, so the order never shifts between renders.
export function sortLifecycleRules<T extends SortableRule>(
  rules: readonly T[],
  sort: LifecycleRuleSort,
): T[] {
  return sortRuleRows(rules, sort, columnOrder);
}

/// Pressing the active column reverses it; any other column starts ascending.
export const nextLifecycleRuleSort = nextRuleTableSort<LifecycleRuleSortKey>;

export type LifecycleRulePage<T> = RuleTablePage<T>;

/// One page of rows. A page past the end (after deletions shrink the list)
/// shows the last page instead of an empty table.
export const lifecycleRulePage = ruleTablePage;
