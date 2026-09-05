import type { ConfigFieldDef, FieldCondition } from "../types/indexers.ts";

/// Whether a condition holds, given the current value of the field it names.
///
/// Mirrors `FieldCondition::holds` in scryer-domain. The host validates with
/// that one, so any drift here shows up as a form that will not submit and
/// cannot be fixed from the UI — change both or neither.
export function fieldConditionHolds(
  condition: FieldCondition,
  values: Record<string, string>,
): boolean {
  const value = (values[condition.key] ?? "").trim();
  switch (condition.op) {
    case "EQ":
      return condition.values[0] === value;
    case "NE":
      return condition.values[0] !== value;
    case "IN":
      return condition.values.includes(value);
    case "NOT_IN":
      return !condition.values.includes(value);
    case "NON_EMPTY":
      return value !== "";
  }
}

/// Whether a field should be shown at all.
export function isConfigFieldVisible(
  field: ConfigFieldDef,
  values: Record<string, string>,
): boolean {
  return field.visibleWhen === null
    ? true
    : fieldConditionHolds(field.visibleWhen, values);
}

/// Whether a field must carry a value. A hidden field never does, whatever it
/// declared — otherwise a form could block on something it is not showing.
export function isConfigFieldRequired(
  field: ConfigFieldDef,
  values: Record<string, string>,
): boolean {
  if (!isConfigFieldVisible(field, values)) {
    return false;
  }
  return (
    field.required ||
    (field.requiredWhen !== null &&
      fieldConditionHolds(field.requiredWhen, values))
  );
}

/// The fields a form should render right now, each carrying the requiredness it
/// actually has. Renderers stay dumb: they honour `required` and never evaluate
/// a condition themselves.
export function resolveConfigFieldsForValues(
  fields: ConfigFieldDef[],
  values: Record<string, string>,
): ConfigFieldDef[] {
  return fields
    .filter((field) => isConfigFieldVisible(field, values))
    .map((field) => {
      const required = isConfigFieldRequired(field, values);
      return required === field.required ? field : { ...field, required };
    });
}

export type SplitConfigFields = {
  /// Shown up front.
  standard: ConfigFieldDef[];
  /// Shown behind the form's advanced disclosure.
  advanced: ConfigFieldDef[];
};

/// Split a provider's fields into what a form shows up front and what it keeps
/// behind a disclosure.
///
/// The plugin decides which is which; a form that hard-codes the split ends up
/// disagreeing with the next provider that ships.
export function splitAdvancedConfigFields(
  fields: ConfigFieldDef[],
): SplitConfigFields {
  const standard: ConfigFieldDef[] = [];
  const advanced: ConfigFieldDef[] = [];
  for (const field of fields) {
    (field.advanced ? advanced : standard).push(field);
  }
  return { standard, advanced };
}
