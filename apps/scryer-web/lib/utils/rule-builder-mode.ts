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
/// attached to the builder while it holds a model; its source is then exactly
/// what the builder writes. An attached draft is shown in the builder
/// (`simple`) or as its generated Rego, read-only (`view`). Choosing to edit
/// the Rego (`edit`) detaches the draft for good: it drops the model, a save
/// then clears the stored one, and the rule never offers the builder again.

export type RuleEditorMode = "simple" | "view" | "edit";

/// The part of a family's draft the builder reads and writes.
export type RuleBuilderDraft = {
  regoSource: string;
  builderModel: RuleBuilderModel | null;
};

/// How one family's drafts meet the builder.
export type RuleBuilderBinding<D extends RuleBuilderDraft> = {
  /// Null until the family's outputs are read from the server, and after a
  /// failed read: a draft then can only be edited as Rego.
  schema: RuleBuilderSchema | null;
  /// The variant a draft is written for, such as a maintenance rule's scope.
  variant: (draft: D) => string | null;
  /// Where the Rego editor starts when an untouched builder is left.
  starterSource: string;
};

/// The mode an editor opens in: the builder for an attached draft, the Rego
/// editor for anything else.
export function initialRuleEditorMode(draft: RuleBuilderDraft): RuleEditorMode {
  return draft.builderModel ? "simple" : "edit";
}

/// The mode an editor can move to from `current` without detaching the draft.
/// Only an attached draft moves between the builder and its read-only Rego;
/// `edit` is reached only through `withRuleRegoEditor`.
export function nextRuleEditorMode(
  draft: RuleBuilderDraft,
  current: RuleEditorMode,
  requested: "simple" | "view",
): RuleEditorMode {
  return draft.builderModel !== null && current !== "edit" ? requested : current;
}

/// Apply a builder change: the source becomes exactly what the builder writes
/// for the model, or nothing while the model cannot be written yet.
export function withRuleBuilderModel<D extends RuleBuilderDraft>(
  binding: RuleBuilderBinding<D>,
  draft: D,
  model: RuleBuilderModel,
): D {
  if (!binding.schema) return draft;
  const result = generateRuleSource(binding.schema, model, binding.variant(draft));
  return {
    ...draft,
    builderModel: model,
    regoSource: result.ok ? result.source : "",
  };
}

/// A new rule in the builder with nothing chosen, so no source yet. Without a
/// schema the draft is left as written, for the Rego editor.
export function withNewRuleBuilderModel<D extends RuleBuilderDraft>(
  binding: RuleBuilderBinding<D>,
  draft: D,
): D {
  if (!binding.schema) return draft;
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

/// Apply a hand edit of the source. Only a detached draft takes one: an
/// attached draft's source is the builder's, and changes only with its model.
export function withRuleSourceEdit<D extends RuleBuilderDraft>(
  draft: D,
  regoSource: string,
): D {
  return draft.builderModel === null ? { ...draft, regoSource } : draft;
}

/// True when editing the draft as Rego gives up something the operator chose
/// in the builder, so they confirm it first. An untouched builder loses
/// nothing.
export function ruleRegoEditorNeedsConfirmation(draft: RuleBuilderDraft): boolean {
  return draft.builderModel !== null && !isBlankRuleBuilderModel(draft.builderModel);
}

/// Detach the draft for editing as Rego, whatever its source. The generated
/// source is kept; a builder that has written nothing yet leaves the editor
/// the family's starter source instead.
export function withRuleRegoEditor<D extends RuleBuilderDraft>(
  binding: RuleBuilderBinding<D>,
  draft: D,
): D {
  if (draft.builderModel === null) return draft;
  return {
    ...draft,
    regoSource: draft.regoSource === "" ? binding.starterSource : draft.regoSource,
    builderModel: null,
  };
}

/// The model to store with a save: only when the saved source is exactly
/// what the builder writes from it. Null clears a stored model.
export function ruleBuilderModelToSave<D extends RuleBuilderDraft>(
  binding: RuleBuilderBinding<D>,
  draft: D,
): RuleBuilderModel | null {
  return draft.builderModel &&
    binding.schema &&
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
  if (draft.builderModel === null) return false;
  return (
    !binding.schema ||
    !generateRuleSource(binding.schema, draft.builderModel, binding.variant(draft))
      .ok
  );
}

/// A stored model counts only while regenerating it reproduces the stored
/// source; otherwise the rule opens as Rego and the stored model is left
/// alone until the next save.
export function attachedRuleBuilderModel(
  schema: RuleBuilderSchema | null,
  stored: unknown,
  variant: string | null,
  regoSource: string,
): RuleBuilderModel | null {
  if (!schema) return null;
  const model = parseRuleBuilderModel(schema, stored);
  return model !== null && isRuleBuilderSource(schema, model, variant, regoSource)
    ? model
    : null;
}
