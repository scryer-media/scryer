import type { ConfigFieldDef } from "../types/index.ts";
import { providerConfigRecordToValues } from "./provider-config.ts";
import { normalizeIndexerConfigValues } from "./url-input.ts";

export function setupIndexerConfigFields(fields: ConfigFieldDef[]) {
  return fields.filter((field) => field.valueSource !== "HOST_BINDING");
}

export function buildSetupIndexerConfigValues(
  fields: ConfigFieldDef[],
): Record<string, string> {
  const values: Record<string, string> = {};
  for (const field of setupIndexerConfigFields(fields)) {
    values[field.key] =
      field.defaultValue ?? (field.fieldType === "BOOL" ? "false" : "");
  }
  return values;
}

export function applyIndexerConfigOption(
  fields: ConfigFieldDef[],
  currentValues: Record<string, string>,
  key: string,
  value: string,
): Record<string, string> {
  const nextValues = { ...currentValues, [key]: value };
  const declaredKeys = new Set(fields.map((field) => field.key));
  const selectedOption = fields
    .find((field) => field.key === key)
    ?.options.find((option) => option.value === value);

  for (const override of selectedOption?.configOverrides ?? []) {
    if (declaredKeys.has(override.key)) {
      nextValues[override.key] = override.value;
    }
  }
  return nextValues;
}

export function serializeSetupIndexerConfigValues(
  fields: ConfigFieldDef[],
  rawValues: Record<string, string>,
): ReturnType<typeof providerConfigRecordToValues> | undefined {
  // The first-run wizard is where a URL is most likely to arrive pasted out of
  // an email, so it forgives the same shapes the settings form does.
  const values = normalizeIndexerConfigValues(fields, rawValues);
  const entries: Record<string, string> = {};
  const fieldKeySet = new Set(fields.map((field) => field.key));

  for (const [key, value] of Object.entries(values)) {
    if (!fieldKeySet.has(key) && value.trim() !== "") {
      entries[key] = value;
    }
  }

  for (const field of setupIndexerConfigFields(fields)) {
    let value =
      values[field.key] ??
      field.defaultValue ??
      (field.fieldType === "BOOL" ? "false" : "");
    if (field.fieldType === "BOOL") {
      entries[field.key] = value.trim() || field.defaultValue || "false";
      continue;
    }
    if (value.trim() === "" && field.defaultValue) {
      value = field.defaultValue;
    }
    if (value.trim() !== "") {
      entries[field.key] = value;
    }
  }

  const secretInputKeys = setupIndexerConfigFields(fields)
    .filter((field) => field.fieldType === "PASSWORD")
    .map((field) => field.key);
  return Object.keys(entries).length > 0
    ? providerConfigRecordToValues(entries, secretInputKeys)
    : undefined;
}

export function findMissingSetupIndexerField(
  fields: ConfigFieldDef[],
  values: Record<string, string>,
): ConfigFieldDef | null {
  for (const field of setupIndexerConfigFields(fields)) {
    if (!field.required) {
      continue;
    }
    const value =
      values[field.key] ??
      field.defaultValue ??
      (field.fieldType === "BOOL" ? "false" : "");
    if (field.fieldType !== "BOOL" && value.trim() === "") {
      return field;
    }
  }
  return null;
}
