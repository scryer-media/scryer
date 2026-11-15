import type { RuleSetDraft, RuleSetRecord } from "@/lib/types/rule-sets";
import { formatRegoCopy } from "./format-rego-copy.ts";
import type { RuleBuilderSchema } from "./rule-builder.ts";
import {
  attachedRuleBuilderModel,
  ruleBuilderModelToSave,
  withNewRuleBuilderModel,
  type RuleBuilderBinding,
} from "./rule-builder-mode.ts";

/// Where the Rego editor starts for a new scoring rule written by hand.
export const SCORING_STARTER_SOURCE =
  'import rego.v1\n\nscore_entry["size_guard"] := scryer.block_score() if {\n    scryer.size_gib(input.release.size_bytes) > 100\n}\n';

export type ScoringRuleBuilderBinding = RuleBuilderBinding<RuleSetDraft>;

/// How scoring drafts meet the builder. A scoring rule has no variant.
export function scoringRuleBuilder(
  schema: RuleBuilderSchema | null,
): ScoringRuleBuilderBinding {
  return { schema, variant: () => null, starterSource: SCORING_STARTER_SOURCE };
}

/// A draft written by hand: the starter source and no builder model.
export const RULE_SET_INITIAL_DRAFT: RuleSetDraft = {
  name: "",
  description: "",
  regoSource: SCORING_STARTER_SOURCE,
  enabled: true,
  priority: 0,
  appliedFacets: [],
  builderModel: null,
};

/// A new rule opens in the builder with nothing chosen yet.
export function newScoringRuleBuilderDraft(
  binding: ScoringRuleBuilderBinding,
): RuleSetDraft {
  return withNewRuleBuilderModel(binding, {
    ...RULE_SET_INITIAL_DRAFT,
  });
}

/// The editor draft for a saved rule. Its stored builder model counts only
/// while it still regenerates the stored source.
export function ruleSetDraftFromRecord(
  record: RuleSetRecord,
  binding: ScoringRuleBuilderBinding,
): RuleSetDraft {
  return {
    name: record.name,
    description: record.description,
    regoSource: record.regoSource,
    enabled: record.enabled,
    priority: record.priority,
    appliedFacets: [...record.appliedFacets],
    builderModel: attachedRuleBuilderModel(
      binding.schema,
      record.builderModel,
      null,
      record.regoSource,
    ),
  };
}

/// A copy is a new rule. A rule still made by the builder stays in it with its
/// source as written; any other source is tidied for editing.
export function copyRuleSetDraft(
  record: RuleSetRecord,
  binding: ScoringRuleBuilderBinding,
): RuleSetDraft {
  const builderModel = attachedRuleBuilderModel(
    binding.schema,
    record.builderModel,
    null,
    record.regoSource,
  );
  return {
    name: `Copy of ${record.name}`,
    description: record.description,
    regoSource: builderModel
      ? record.regoSource
      : formatRegoCopy(record.regoSource),
    enabled: record.enabled,
    priority: record.priority,
    appliedFacets: [...record.appliedFacets],
    builderModel,
  };
}

export function createRuleSetInput(
  draft: RuleSetDraft,
  binding: ScoringRuleBuilderBinding,
) {
  return {
    name: draft.name.trim(),
    description: draft.description.trim() || undefined,
    regoSource: draft.regoSource,
    enabled: draft.enabled,
    priority: draft.priority,
    appliedFacets:
      draft.appliedFacets.length > 0 ? [...draft.appliedFacets] : undefined,
    builderModel: ruleBuilderModelToSave(binding, draft),
  };
}

/// Without a schema nothing could be checked, so `builderModel` is left out
/// and the stored model stays as it is.
export function updateRuleSetInput(
  id: string,
  draft: RuleSetDraft,
  binding: ScoringRuleBuilderBinding,
) {
  return {
    id,
    name: draft.name.trim(),
    description: draft.description.trim(),
    regoSource: draft.regoSource,
    priority: draft.priority,
    appliedFacets: draft.appliedFacets,
    ...(binding.schema
      ? { builderModel: ruleBuilderModelToSave(binding, draft) }
      : {}),
  };
}

export function isUserOwnedRuleSet(record: RuleSetRecord): boolean {
  return !record.isManaged;
}
