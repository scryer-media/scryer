import type { ConfigFieldDef } from "../types/indexers.ts";

/// Fields a plugin adds for the values entered so far, and the static fields
/// it hides for them. The host has already validated both.
export type DynamicConfigFields = {
  fields: ConfigFieldDef[];
  hiddenFieldKeys: string[];
};

/// Where the form's request for the plugin's fields stands.
export type DynamicConfigFieldsStatus =
  | { state: "idle" }
  | { state: "loading" }
  | { state: "error"; message: string };

/// The field list a form works from: the provider's static fields minus the
/// ones the plugin hides, then the plugin's fields. A dynamic field never
/// shadows a static one, whatever the plugin sent.
export function mergeDynamicConfigFields(
  staticFields: ConfigFieldDef[],
  dynamic: DynamicConfigFields | null,
): ConfigFieldDef[] {
  if (!dynamic) {
    return staticFields;
  }
  const hidden = new Set(dynamic.hiddenFieldKeys);
  const staticKeys = new Set(staticFields.map((field) => field.key));
  return [
    ...staticFields.filter((field) => !hidden.has(field.key)),
    ...dynamic.fields.filter((field) => !staticKeys.has(field.key)),
  ];
}

/// The trigger values the plugin is asked about: trimmed, falling back to the
/// static field's default, and left out when blank. Only these leave the form.
export function dynamicTriggerValues(
  triggerKeys: string[],
  staticFields: ConfigFieldDef[],
  values: Record<string, string>,
): Record<string, string> {
  const result: Record<string, string> = {};
  for (const key of triggerKeys) {
    const value =
      (values[key] ?? "").trim() ||
      (
        staticFields.find((field) => field.key === key)?.defaultValue ?? ""
      ).trim();
    if (value !== "") {
      result[key] = value;
    }
  }
  return result;
}

/// A stable string for a set of trigger values, so an effect re-runs only when
/// one of them actually changes.
export function dynamicTriggerSignature(
  values: Record<string, string>,
): string {
  return JSON.stringify(
    Object.keys(values)
      .sort()
      .map((key) => [key, values[key]]),
  );
}

/// Bring draft values in line with a new answer: values of dynamic fields the
/// plugin no longer offers are dropped, so they are not saved against a
/// selection they do not belong to, and newly offered fields start from their
/// default. Values the operator already typed are kept. Returns `values`
/// itself when nothing changes.
export function reconcileDynamicConfigValues(
  values: Record<string, string>,
  previous: DynamicConfigFields | null,
  next: DynamicConfigFields | null,
): Record<string, string> {
  const nextKeys = new Set((next?.fields ?? []).map((field) => field.key));
  let changed = false;
  const result = { ...values };
  for (const field of previous?.fields ?? []) {
    if (!nextKeys.has(field.key) && field.key in result) {
      delete result[field.key];
      changed = true;
    }
  }
  for (const field of next?.fields ?? []) {
    if (
      field.key in result ||
      field.valueSource === "HOST_BINDING" ||
      field.fieldType === "INFO"
    ) {
      continue;
    }
    result[field.key] =
      field.defaultValue ?? (field.fieldType === "BOOL" ? "false" : "");
    changed = true;
  }
  return changed ? result : values;
}

/// Draft values without the static fields the plugin hides. A hidden field's
/// draft value is kept so it returns if the selection changes back, but it is
/// not part of what gets tested or saved.
export function omitHiddenConfigValues(
  values: Record<string, string>,
  dynamic: DynamicConfigFields | null,
): Record<string, string> {
  if (!dynamic || dynamic.hiddenFieldKeys.length === 0) {
    return values;
  }
  const result = { ...values };
  for (const key of dynamic.hiddenFieldKeys) {
    delete result[key];
  }
  return result;
}

/// Orders overlapping requests: only the answer to the newest one may land, so
/// a slow reply for an earlier selection never overwrites a later one.
export function createRequestEpoch() {
  let current = 0;
  return {
    next(): number {
      current += 1;
      return current;
    },
    isCurrent(epoch: number): boolean {
      return epoch === current;
    },
  };
}
