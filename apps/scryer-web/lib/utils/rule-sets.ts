import type { RuleSetDraft, RuleSetRecord } from "@/lib/types/rule-sets";
import { formatRegoCopy } from "./format-rego-copy.ts";

export function copyRuleSetDraft(record: RuleSetRecord): RuleSetDraft {
  return {
    name: `Copy of ${record.name}`,
    description: record.description,
    regoSource: formatRegoCopy(record.regoSource),
    enabled: record.enabled,
    priority: record.priority,
    appliedFacets: [...record.appliedFacets],
  };
}

export function createRuleSetInput(draft: RuleSetDraft) {
  return {
    name: draft.name.trim(),
    description: draft.description.trim() || undefined,
    regoSource: draft.regoSource,
    enabled: draft.enabled,
    priority: draft.priority,
    appliedFacets:
      draft.appliedFacets.length > 0 ? [...draft.appliedFacets] : undefined,
  };
}

export function isUserOwnedRuleSet(record: RuleSetRecord): boolean {
  return !record.isManaged;
}
