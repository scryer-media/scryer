import type {
  ArrSource,
  ArrSpecification,
  Diagnostic,
  ImportedCustomFormat,
  InspectionResult,
} from "./types";

export type { ArrSource, ArrSpecification, Diagnostic, ImportedCustomFormat, InspectionResult } from "./types";

type JsonRecord = Record<string, unknown>;
const MAX_PASTED_JSON_BYTES = 16 * 1024 * 1024;
const MAX_FORMATS = 1_000;
const MAX_SPECIFICATIONS = 1_000;

function isRecord(value: unknown): value is JsonRecord {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function stringValue(value: unknown): string | undefined {
  return typeof value === "string" && value.trim() ? value.trim() : undefined;
}

function numberValue(value: unknown): number | undefined {
  return typeof value === "number" && Number.isFinite(value) ? value : undefined;
}

function diagnostic(
  code: string,
  level: Diagnostic["level"],
  message: string,
  location?: string,
): Diagnostic {
  return { code, level, message, location };
}

function normalizeFields(value: unknown): Record<string, unknown> | undefined {
  if (isRecord(value)) return { ...value };
  if (!Array.isArray(value)) return undefined;

  const fields: Record<string, unknown> = {};
  for (const entry of value) {
    if (!isRecord(entry)) return undefined;
    const name = stringValue(entry.name) ?? stringValue(entry.key);
    if (!name || !("value" in entry)) return undefined;
    if (name in fields) return undefined;
    fields[name] = entry.value;
  }
  return fields;
}

function normalizeScores(raw: JsonRecord, diagnostics: Diagnostic[], location: string): Record<string, number> {
  const result: Record<string, number> = {};
  const candidate = raw.trash_scores ?? raw.trashScores ?? raw.scores;
  if (isRecord(candidate)) {
    for (const [name, score] of Object.entries(candidate)) {
      const numeric = numberValue(score);
      if (numeric !== undefined) result[name] = numeric;
    }
  }
  const direct = numberValue(raw.score);
  if (direct !== undefined) {
    if (result.default !== undefined && result.default !== direct) {
      diagnostics.push(diagnostic("arr.score.conflict", "warning", "The exported score conflicts with trash_scores.default; choose a score explicitly.", location));
      result["exported score"] = direct;
    } else {
      result.default = direct;
    }
  }
  return result;
}

function normalizeFormat(raw: unknown, source: ArrSource, location: string): {
  format?: ImportedCustomFormat;
  diagnostics: Diagnostic[];
} {
  const diagnostics: Diagnostic[] = [];
  if (!isRecord(raw)) {
    return { diagnostics: [diagnostic("arr.format.invalid", "error", "A custom format must be an object.", location)] };
  }
  const name = stringValue(raw.name);
  if (!name) {
    return { diagnostics: [diagnostic("arr.format.name_missing", "error", "Custom format is missing a name.", `${location}.name`)] };
  }
  if (!Array.isArray(raw.specifications) || raw.specifications.length === 0) {
    return { diagnostics: [diagnostic("arr.format.specifications_missing", "error", "Custom format is missing its specifications array.", `${location}.specifications`)] };
  }
  const specifications: ArrSpecification[] = [];
  if (raw.specifications.length > MAX_SPECIFICATIONS) {
    diagnostics.push(diagnostic("arr.specifications.too_many", "error", `A format may contain at most ${MAX_SPECIFICATIONS} specifications.`, `${location}.specifications`));
  }
  for (const [index, value] of raw.specifications.slice(0, MAX_SPECIFICATIONS).entries()) {
    const specLocation = `${location}.specifications[${index}]`;
    if (!isRecord(value)) {
      diagnostics.push(diagnostic("arr.specification.invalid", "error", "Specification must be an object.", specLocation));
      continue;
    }
    const implementation = stringValue(value.implementation);
    const specName = stringValue(value.name) ?? implementation;
    const fields = normalizeFields(value.fields);
    if (!implementation || !specName || !fields || ("negate" in value && typeof value.negate !== "boolean") || ("required" in value && typeof value.required !== "boolean")) {
      diagnostics.push(diagnostic("arr.specification.invalid", "error", "Specification needs implementation, name, and unique fields.", specLocation));
      continue;
    }
    specifications.push({
      implementation,
      name: specName,
      fields,
      negate: value.negate === true,
      required: value.required === true,
      index,
    });
  }
  const idCandidate = raw.trash_id ?? raw.id ?? raw.uuid ?? raw.name;
  const sourceId = typeof idCandidate === "number" || typeof idCandidate === "string" ? String(idCandidate) : name;
  const id = `${location}:${sourceId}`;
  const suggestedScores = normalizeScores(raw, diagnostics, `${location}.score`);
  const taggedDiagnostics = diagnostics.map((item) => ({ ...item, formatId: id }));
  return {
    format: {
      id,
      name,
      source,
      specifications,
      suggestedScores,
      description: stringValue(raw.description),
      inspectionDiagnostics: taggedDiagnostics,
    },
    diagnostics: taggedDiagnostics,
  };
}

/**
 * Parses and normalizes pasted Arr JSON. This module deliberately imports no
 * compiler, regex, or AST package so opening the dialog remains lightweight.
 */
export function inspectCustomFormats(json: unknown, source: ArrSource): InspectionResult {
  let document: unknown = json;
  const diagnostics: Diagnostic[] = [];
  if (typeof json === "string") {
    if (new TextEncoder().encode(json).byteLength > MAX_PASTED_JSON_BYTES) {
      return { formats: [], diagnostics: [diagnostic("arr.json.too_large", "error", `Paste no more than ${MAX_PASTED_JSON_BYTES} bytes of JSON.`)], fatal: true };
    }
    try {
      document = JSON.parse(json) as unknown;
    } catch {
      return {
        formats: [],
        diagnostics: [diagnostic("arr.json.invalid", "error", "Paste valid JSON exported by Sonarr or Radarr.")],
        fatal: true,
      };
    }
  }
  const rawFormats = Array.isArray(document) ? document : [document];
  if (rawFormats.length === 0) {
    return { formats: [], diagnostics: [diagnostic("arr.formats.empty", "error", "Paste at least one custom format.")], fatal: true };
  }
  if (rawFormats.length > MAX_FORMATS) {
    return { formats: [], diagnostics: [diagnostic("arr.formats.too_many", "error", `Paste no more than ${MAX_FORMATS} custom formats at once.`)], fatal: true };
  }
  const formats: ImportedCustomFormat[] = [];
  for (const [index, raw] of rawFormats.entries()) {
    const normalized = normalizeFormat(raw, source, Array.isArray(document) ? `[${index}]` : "$");
    diagnostics.push(...normalized.diagnostics);
    if (normalized.format) formats.push(normalized.format);
  }
  return { formats, diagnostics, fatal: formats.length === 0 };
}
