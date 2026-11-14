import {
  emptyRuleBuilderModel,
  generateRuleSource,
  isBlankRuleBuilderModel,
  isRuleBuilderSource,
  parseRuleBuilderModel,
  type RuleBuilderModel,
  type RuleBuilderSchema,
} from "./rule-builder.ts";

/// Simple and Advanced editing, the same for every rule family. A draft is
/// attached to the builder while it holds a model; the source is then exactly
/// what the builder writes. A hand edit that leaves anything else detaches it
/// for good, and only an attached draft sends its model with a save.

export type RuleEditorMode = "simple" | "advanced";

/// The part of a family's draft the builder reads and writes.
export type RuleBuilderDraft = {
  regoSource: string;
  builderModel: RuleBuilderModel | null;
};

/// How one family's drafts meet the builder.
export type RuleBuilderBinding<D extends RuleBuilderDraft> = {
  schema: RuleBuilderSchema;
  /// The variant a draft is written for, such as a maintenance rule's scope.
  variant: (draft: D) => string | null;
  /// Where the Rego editor starts when an untouched builder is left.
  starterSource: string;
};

/// The mode an editor opens in: the builder for an attached draft, Rego for
/// anything else.
export function initialRuleEditorMode(draft: RuleBuilderDraft): RuleEditorMode {
  return draft.builderModel ? "simple" : "advanced";
}

/// Apply a builder change: the source becomes exactly what the builder writes
/// for the model, or nothing while the model cannot be written yet.
export function withRuleBuilderModel<D extends RuleBuilderDraft>(
  binding: RuleBuilderBinding<D>,
  draft: D,
  model: RuleBuilderModel,
): D {
  const result = generateRuleSource(binding.schema, model, binding.variant(draft));
  return {
    ...draft,
    builderModel: model,
    regoSource: result.ok ? result.source : "",
  };
}

/// A new rule in the builder with nothing chosen, so no source yet.
export function withNewRuleBuilderModel<D extends RuleBuilderDraft>(
  binding: RuleBuilderBinding<D>,
  draft: D,
): D {
  return withRuleBuilderModel(binding, draft, emptyRuleBuilderModel(binding.schema));
}

/// Rewrite an attached draft after something the source depends on changed,
/// such as its variant. A detached draft is left as it is.
export function withRegeneratedRuleSource<D extends RuleBuilderDraft>(
  binding: RuleBuilderBinding<D>,
  draft: D,
): D {
  return draft.builderModel
    ? withRuleBuilderModel(binding, draft, draft.builderModel)
    : draft;
}

/// Apply a hand edit of the source. An edit that leaves anything other than
/// the builder's own source detaches the draft from the builder for good.
export function withRuleSourceEdit<D extends RuleBuilderDraft>(
  binding: RuleBuilderBinding<D>,
  draft: D,
  regoSource: string,
): { draft: D; detached: boolean } {
  const detached =
    draft.builderModel !== null &&
    !isRuleBuilderSource(
      binding.schema,
      draft.builderModel,
      binding.variant(draft),
      regoSource,
    );
  return {
    draft: {
      ...draft,
      regoSource,
      builderModel: detached ? null : draft.builderModel,
    },
    detached,
  };
}

/// Leaving the builder for the Rego editor keeps the generated source. A
/// builder nobody has touched yet has nothing to keep, so the editor starts
/// from the starter source instead and the rule is Rego from then on.
export function withRuleAdvancedEditor<D extends RuleBuilderDraft>(
  binding: RuleBuilderBinding<D>,
  draft: D,
): D {
  if (
    draft.builderModel &&
    draft.regoSource === "" &&
    isBlankRuleBuilderModel(draft.builderModel)
  ) {
    return { ...draft, regoSource: binding.starterSource, builderModel: null };
  }
  return draft;
}

/// The model to store with a save: only when the saved source is exactly
/// what the builder writes from it. Null clears a stored model.
export function ruleBuilderModelToSave<D extends RuleBuilderDraft>(
  binding: RuleBuilderBinding<D>,
  draft: D,
): RuleBuilderModel | null {
  return draft.builderModel &&
    isRuleBuilderSource(
      binding.schema,
      draft.builderModel,
      binding.variant(draft),
      draft.regoSource,
    )
    ? draft.builderModel
    : null;
}

/// True when the builder cannot write the attached draft yet, so saving it
/// would save an empty rule.
export function isIncompleteRuleBuilderDraft<D extends RuleBuilderDraft>(
  binding: RuleBuilderBinding<D>,
  draft: D,
): boolean {
  return (
    draft.builderModel !== null &&
    !generateRuleSource(binding.schema, draft.builderModel, binding.variant(draft))
      .ok
  );
}

/// A stored model counts only while regenerating it reproduces the stored
/// source; otherwise the rule opens as Rego and the stored model is left
/// alone until the next save.
export function attachedRuleBuilderModel(
  schema: RuleBuilderSchema,
  stored: unknown,
  variant: string | null,
  regoSource: string,
): RuleBuilderModel | null {
  const model = parseRuleBuilderModel(schema, stored);
  return model !== null && isRuleBuilderSource(schema, model, variant, regoSource)
    ? model
    : null;
}
