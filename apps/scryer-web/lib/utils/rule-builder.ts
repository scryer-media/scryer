/// The simple rule builder writes a rule's Rego for an operator who never
/// writes Rego. Everything that differs between rule families lives in a
/// schema: which input fields a condition can read, how each comparison is
/// written, which outputs a rule can declare and how they combine. The model
/// is what the builder edits and what the server stores beside the rule; the
/// source generated from it is what actually runs.

export const RULE_BUILDER_MODEL_VERSION = 2;

// ── Model ─────────────────────────────────────────────────────────────

export type RuleBuilderJoin = "ALL" | "ANY";

/// One condition. `field` is a full input path such as `input.facts.tags`;
/// `value` is the text the operator typed, kept as text so the model is
/// exactly what was entered and the generator is the one place that decides
/// whether it can be written. An empty `field` or `operator` is a row the
/// operator has not finished.
export type RuleBuilderCondition = {
  field: string;
  operator: string;
  value: string;
};

/// A chosen output's value: `true` for a flag, the typed text for a text,
/// choice or number output, a list for a list output, and one typed text per
/// part for a structured output.
export type RuleBuilderOutputValue =
  | true
  | string
  | string[]
  | { [part: string]: string };

/// What the server stores as an opaque JSON document next to the rule. A key
/// present in `outputs` is a chosen output.
export type RuleBuilderModel = {
  version: typeof RULE_BUILDER_MODEL_VERSION;
  family: string;
  join: RuleBuilderJoin;
  conditions: RuleBuilderCondition[];
  outputs: Record<string, RuleBuilderOutputValue>;
};

// ── Schema ────────────────────────────────────────────────────────────

/// What a comparison takes as its value: nothing, a decimal number, a whole
/// number of days, free text, or one of the field's known values.
export type RuleBuilderConditionValue =
  | "none"
  | "number"
  | "days"
  | "text"
  | "choice";

/// One comparison. `template` is the Rego it writes, one body line per line
/// of the template: `{path}` is the field's input path and `{value}` the
/// value's literal (a number as typed, anything else as a string literal).
export type RuleBuilderOperator = {
  id: string;
  labelKey: string;
  value: RuleBuilderConditionValue;
  template: string;
};

/// How fields of one kind are compared.
export type RuleBuilderKind = { operators: readonly RuleBuilderOperator[] };

export type RuleBuilderChoice = { value: string; labelKey: string };

export type RuleBuilderFieldOverride = {
  /// A kind to use instead of the one the field's contract type maps to.
  kind?: string;
  /// The values a `choice` comparison offers.
  choices?: readonly RuleBuilderChoice[];
  /// Variants whose input carries the field. Absent means every variant.
  variants?: readonly string[];
};

/// One contract section a condition can read from.
export type RuleBuilderConditionSource = {
  section: string;
  /// How a field is named in the picker: by its own name, or by its path
  /// below `input.`.
  label: "field" | "path";
  /// Variants whose input carries the section. Absent means every variant.
  variants?: readonly string[];
  /// Fields of the section the builder does not offer.
  exclude?: readonly string[];
  fields?: Readonly<Record<string, RuleBuilderFieldOverride>>;
};

/// Maps a contract type to a kind. The first rule whose type matches (and
/// whose suffix the field name ends with, when it has one) wins; a type no
/// rule maps is not offered.
export type RuleBuilderTypeRule = {
  type: string;
  suffix?: string;
  kind: string;
};

/// Limits a text value must meet. `pattern` is a regular expression the whole
/// value must match; `maxBytes` counts UTF-8 bytes, as the server does.
export type RuleBuilderTextConstraints = {
  maxBytes?: number;
  pattern?: string;
};

export type RuleBuilderOutputPart =
  | ({
      id: string;
      kind: "text";
      labelKey: string;
      placeholderKey?: string;
    } & RuleBuilderTextConstraints)
  | {
      id: string;
      kind: "number";
      labelKey: string;
      placeholderKey?: string;
      min: number;
      max: number;
    };

/// One value a page supplies for an output's catalog: the literal the rule
/// writes, shown as itself, or with a label when the literal is a code.
export type RuleBuilderOptionValue = string | { value: string; label: string };

/// What an output carries. `options` names a catalog the page supplies, which
/// the builder loads only once an output that needs it is chosen. A catalog
/// the page does not supply is typed in as free text instead; the value is
/// written the same way either way.
export type RuleBuilderOutputValueSpec =
  | { kind: "flag" }
  | ({ kind: "text" } & RuleBuilderTextConstraints)
  | ({ kind: "choice"; options: string } & RuleBuilderTextConstraints)
  | { kind: "number"; min: number; max: number }
  | ({
      kind: "text_list";
      options?: string;
      maxItems?: number;
    } & RuleBuilderTextConstraints)
  | { kind: "structured"; parts: readonly RuleBuilderOutputPart[] };

/// One output a rule can declare. `template` is the Rego it writes:
/// `{head}` and `{helper}` are the head and helper rule names, `{value}` a
/// text, choice or number value's literal, `{items}` a list's literals joined
/// by commas, `{item}` one list item (with `perItem`, the template is written
/// once per item), and `{part.<id>}` a structured part's literal.
export type RuleBuilderOutput = {
  id: string;
  head: string;
  /// Translated through the normal i18n path. Without a translation the
  /// output is shown by its head.
  labelKey: string;
  /// Help without a translation is left out.
  helpKey?: string | null;
  value: RuleBuilderOutputValueSpec;
  template: string;
  perItem?: boolean;
  destructive?: boolean;
  /// Variants that may declare the output. Absent means every variant.
  variants?: readonly string[];
  /// Outputs this one is never chosen with. Read in both directions.
  conflictsWith?: readonly string[];
  /// Outputs that must be chosen whenever this one is.
  requires?: readonly string[];
};

/// A rule about which outputs a rule chooses. `of` absent means every output.
export type RuleBuilderOutputRule =
  | { rule: "at_least_one"; of?: readonly string[] }
  | { rule: "at_most_one"; of: readonly string[] }
  | { rule: "exactly_one"; of: readonly string[] };

export type RuleBuilderContractField = {
  field: string;
  type: string;
  descKey: string;
  /// Variants whose input carries the field. Absent means every variant.
  scopes?: readonly string[];
};

export type RuleBuilderContract = {
  sections: readonly {
    path: string;
    fields: readonly RuleBuilderContractField[];
  }[];
};

export type RuleBuilderSchema = {
  /// Also the family the server's `ruleBuilderSchema` query is asked for.
  family: string;
  /// Prefix of every element id the builder renders.
  idPrefix: string;
  /// The lines every generated source starts with.
  preamble: string;
  /// The helper rule holding the conditions, which every output refers to.
  helperRule: string;
  contract: RuleBuilderContract;
  conditionSources: readonly RuleBuilderConditionSource[];
  typeRules: readonly RuleBuilderTypeRule[];
  kinds: Readonly<Record<string, RuleBuilderKind>>;
  /// In the order they are listed and written.
  outputs: readonly RuleBuilderOutput[];
  outputRules: readonly RuleBuilderOutputRule[];
  labels: {
    outputs: string;
    outputsHelp: string;
    conditionsHelp: string;
    /// Shown on an output the rule's variant cannot declare.
    outputNotInVariant?: string;
  };
};

/// Everything about a family that is not its outputs. The outputs come from
/// the server, so a new action needs no change here.
export type RuleBuilderBaseSchema = Omit<RuleBuilderSchema, "outputs" | "outputRules">;

// ── Served outputs ────────────────────────────────────────────────────

/// The value kinds the server describes an output with. A kind this build
/// does not know is left out of the builder.
export type RuleBuilderServedValueKind =
  | "FLAG"
  | "TEXT"
  | "CHOICE"
  | "NUMBER"
  | "TEXT_LIST";

/// One output as the `ruleBuilderSchema` query describes it.
export type RuleBuilderOutputDescriptor = {
  id: string;
  head: string;
  /// A translation tag, `ruleOutput.<family>.<head>`.
  labelKey: string;
  /// `ruleOutput.<family>.<head>.help`, when the output has help.
  helpKey?: string | null;
  valueKind: string;
  options?: string | null;
  min?: number | null;
  max?: number | null;
  maxItems?: number | null;
  maxBytes?: number | null;
  pattern?: string | null;
  template: string;
  perItem: boolean;
  destructive: boolean;
  /// Null means every variant.
  variants?: readonly string[] | null;
  conflictsWith: readonly string[];
};

export type RuleBuilderServedOutputRule = {
  rule: string;
  /// Null means every output.
  of?: readonly string[] | null;
};

/// What the `ruleBuilderSchema` query returns for one family.
export type RuleBuilderSchemaPayload = {
  family: string;
  outputs: readonly RuleBuilderOutputDescriptor[];
  outputRules: readonly RuleBuilderServedOutputRule[];
};

function constraints(descriptor: RuleBuilderOutputDescriptor) {
  return {
    ...(descriptor.maxBytes != null ? { maxBytes: descriptor.maxBytes } : {}),
    ...(descriptor.pattern != null ? { pattern: descriptor.pattern } : {}),
  };
}

function servedValue(
  descriptor: RuleBuilderOutputDescriptor,
): RuleBuilderOutputValueSpec | null {
  switch (descriptor.valueKind) {
    case "FLAG":
      return { kind: "flag" };
    case "TEXT":
      return { kind: "text", ...constraints(descriptor) };
    case "CHOICE":
      // A choice without a catalog can only be typed in.
      return descriptor.options
        ? { kind: "choice", options: descriptor.options, ...constraints(descriptor) }
        : { kind: "text", ...constraints(descriptor) };
    case "NUMBER":
      return {
        kind: "number",
        min: descriptor.min ?? Number.MIN_SAFE_INTEGER,
        max: descriptor.max ?? Number.MAX_SAFE_INTEGER,
      };
    case "TEXT_LIST":
      return {
        kind: "text_list",
        ...(descriptor.options ? { options: descriptor.options } : {}),
        ...(descriptor.maxItems != null ? { maxItems: descriptor.maxItems } : {}),
        ...constraints(descriptor),
      };
    default:
      return null;
  }
}

/// The builder outputs a served schema describes, in the order it lists them.
/// An output whose value kind this build does not know is left out, so a rule
/// that chooses it opens as Rego; `skipped` names those outputs.
export function ruleBuilderOutputsFromPayload(
  payload: RuleBuilderSchemaPayload,
): {
  outputs: RuleBuilderOutput[];
  outputRules: RuleBuilderOutputRule[];
  skipped: string[];
} {
  const outputs: RuleBuilderOutput[] = [];
  const skipped: string[] = [];
  for (const descriptor of payload.outputs) {
    const value = servedValue(descriptor);
    if (!value) {
      skipped.push(descriptor.id);
      continue;
    }
    outputs.push({
      id: descriptor.id,
      head: descriptor.head,
      labelKey: descriptor.labelKey,
      ...(descriptor.helpKey ? { helpKey: descriptor.helpKey } : {}),
      value,
      template: descriptor.template,
      ...(descriptor.perItem ? { perItem: true } : {}),
      ...(descriptor.destructive ? { destructive: true } : {}),
      ...(descriptor.variants ? { variants: [...descriptor.variants] } : {}),
      ...(descriptor.conflictsWith.length > 0
        ? { conflictsWith: [...descriptor.conflictsWith] }
        : {}),
    });
  }
  const ids = outputs.map((output) => output.id);
  const outputRules: RuleBuilderOutputRule[] = [];
  for (const served of payload.outputRules) {
    const of = served.of ? [...served.of] : null;
    switch (served.rule) {
      case "AT_LEAST_ONE":
        outputRules.push(of ? { rule: "at_least_one", of } : { rule: "at_least_one" });
        break;
      case "AT_MOST_ONE":
        outputRules.push({ rule: "at_most_one", of: of ?? ids });
        break;
      case "EXACTLY_ONE":
        outputRules.push({ rule: "exactly_one", of: of ?? ids });
        break;
    }
  }
  return { outputs, outputRules, skipped };
}

/// A family's whole schema: its own base and the outputs the server serves.
export function ruleBuilderSchemaFromPayload(
  base: RuleBuilderBaseSchema,
  payload: RuleBuilderSchemaPayload,
): RuleBuilderSchema {
  const { outputs, outputRules } = ruleBuilderOutputsFromPayload(payload);
  return { ...base, outputs, outputRules };
}

/// A translation, or `fallback` when the locale and the fallback locale have
/// none. The translate helper hands a missing key back as itself.
export function translatedOr(
  t: (key: string) => string,
  key: string,
  fallback: string,
): string {
  const text = t(key);
  return text === key ? fallback : text;
}

/// An output's label in the UI language, or its head when the tag has no
/// translation yet, so an output the server just added is still usable.
export function ruleBuilderOutputLabel(
  t: (key: string) => string,
  output: Pick<RuleBuilderOutput, "labelKey" | "head">,
): string {
  return translatedOr(t, output.labelKey, output.head);
}

/// An output's help in the UI language; null when it has none translated.
export function ruleBuilderOutputHelp(
  t: (key: string) => string,
  output: Pick<RuleBuilderOutput, "helpKey">,
): string | null {
  if (!output.helpKey) return null;
  return translatedOr(t, output.helpKey, "") || null;
}

/// The placeholder of an output typed in rather than picked, when the locale
/// has one: `<labelKey>.placeholder`.
export function ruleBuilderOutputPlaceholder(
  t: (key: string) => string,
  output: Pick<RuleBuilderOutput, "labelKey">,
): string | undefined {
  return translatedOr(t, `${output.labelKey}.placeholder`, "") || undefined;
}

// ── Shared vocabulary ─────────────────────────────────────────────────

/// Labels for the comparisons every schema builds its kinds from.
export const RULE_BUILDER_OPERATOR_LABEL_KEYS = {
  IS_TRUE: "settings.ruleBuilderOpIsTrue",
  IS_FALSE: "settings.ruleBuilderOpIsFalse",
  EQUALS: "settings.ruleBuilderOpEquals",
  NOT_EQUALS: "settings.ruleBuilderOpNotEquals",
  GREATER_THAN: "settings.ruleBuilderOpGreaterThan",
  GREATER_OR_EQUAL: "settings.ruleBuilderOpGreaterOrEqual",
  LESS_THAN: "settings.ruleBuilderOpLessThan",
  LESS_OR_EQUAL: "settings.ruleBuilderOpLessOrEqual",
  CONTAINS: "settings.ruleBuilderOpContains",
  NOT_CONTAINS: "settings.ruleBuilderOpNotContains",
  IS_EMPTY: "settings.ruleBuilderOpIsEmpty",
  IS_NOT_EMPTY: "settings.ruleBuilderOpIsNotEmpty",
  MORE_THAN_DAYS_AGO: "settings.ruleBuilderOpMoreThanDaysAgo",
  LESS_THAN_DAYS_AGO: "settings.ruleBuilderOpLessThanDaysAgo",
  HAS_VALUE: "settings.ruleBuilderOpHasValue",
  HAS_NO_VALUE: "settings.ruleBuilderOpHasNoValue",
} as const;

export type RuleBuilderOperatorId = keyof typeof RULE_BUILDER_OPERATOR_LABEL_KEYS;

/// A comparison with its shared label.
export function ruleBuilderOperator(
  id: RuleBuilderOperatorId,
  value: RuleBuilderConditionValue,
  template: string,
): RuleBuilderOperator {
  return { id, labelKey: RULE_BUILDER_OPERATOR_LABEL_KEYS[id], value, template };
}

// ── Fields ────────────────────────────────────────────────────────────

export type RuleBuilderField = {
  path: string;
  label: string;
  kind: string;
  descKey: string;
  choices?: readonly RuleBuilderChoice[];
};

const FIELD_NAME = /^[a-z_][a-z0-9_]*$/;

function availableIn(
  variants: readonly string[] | undefined,
  variant: string | null,
): boolean {
  return !variants || (variant !== null && variants.includes(variant));
}

function fieldKind(
  schema: RuleBuilderSchema,
  field: RuleBuilderContractField,
): string | null {
  for (const rule of schema.typeRules) {
    if (rule.type !== field.type) continue;
    if (rule.suffix && !field.field.endsWith(rule.suffix)) continue;
    return rule.kind;
  }
  return null;
}

const FIELD_CACHE = new WeakMap<
  RuleBuilderSchema,
  Map<string | null, RuleBuilderField[]>
>();

/// Every field a condition can read for a rule of this variant, in contract
/// order within each source.
export function ruleBuilderFields(
  schema: RuleBuilderSchema,
  variant: string | null,
): RuleBuilderField[] {
  let byVariant = FIELD_CACHE.get(schema);
  if (!byVariant) {
    byVariant = new Map();
    FIELD_CACHE.set(schema, byVariant);
  }
  const cached = byVariant.get(variant);
  if (cached) return cached;

  const fields: RuleBuilderField[] = [];
  for (const source of schema.conditionSources) {
    if (!availableIn(source.variants, variant)) continue;
    const section = schema.contract.sections.find(
      (entry) => entry.path === source.section,
    );
    for (const field of section?.fields ?? []) {
      if (!FIELD_NAME.test(field.field)) continue;
      if (source.exclude?.includes(field.field)) continue;
      if (!availableIn(field.scopes, variant)) continue;
      const override = source.fields?.[field.field];
      if (!availableIn(override?.variants, variant)) continue;
      const kind = override?.kind ?? fieldKind(schema, field);
      if (!kind || !schema.kinds[kind]) continue;
      const path = `${source.section}.${field.field}`;
      fields.push({
        path,
        label: source.label === "field" ? field.field : path.replace(/^input\./, ""),
        kind,
        descKey: field.descKey,
        ...(override?.choices ? { choices: override.choices } : {}),
      });
    }
  }
  byVariant.set(variant, fields);
  return fields;
}

export function ruleBuilderOperators(
  schema: RuleBuilderSchema,
  kind: string,
): readonly RuleBuilderOperator[] {
  return schema.kinds[kind]?.operators ?? [];
}

// ── Outputs ───────────────────────────────────────────────────────────

/// The chosen outputs in the order they are written.
export function ruleBuilderChosenOutputs(
  schema: RuleBuilderSchema,
  outputs: RuleBuilderModel["outputs"],
): RuleBuilderOutput[] {
  return schema.outputs.filter((output) => Object.hasOwn(outputs, output.id));
}

export function isRuleBuilderOutputInVariant(
  output: RuleBuilderOutput,
  variant: string | null,
): boolean {
  return availableIn(output.variants, variant);
}

function conflictPartners(
  schema: RuleBuilderSchema,
  outputId: string,
): Set<string> {
  const partners = new Set<string>();
  for (const output of schema.outputs) {
    if (output.id === outputId) {
      for (const other of output.conflictsWith ?? []) partners.add(other);
    } else if (output.conflictsWith?.includes(outputId)) {
      partners.add(output.id);
    }
  }
  for (const rule of schema.outputRules) {
    if (rule.rule === "at_least_one" || !rule.of.includes(outputId)) continue;
    for (const other of rule.of) if (other !== outputId) partners.add(other);
  }
  return partners;
}

/// The chosen outputs that cannot be combined with `outputId`, in write order.
export function ruleBuilderConflicts(
  schema: RuleBuilderSchema,
  outputId: string,
  outputs: RuleBuilderModel["outputs"],
): string[] {
  const partners = conflictPartners(schema, outputId);
  return schema.outputs
    .filter(
      (output) => partners.has(output.id) && Object.hasOwn(outputs, output.id),
    )
    .map((output) => output.id);
}

/// The outputs `outputId` needs that are not chosen, in write order.
export function ruleBuilderMissingRequirements(
  schema: RuleBuilderSchema,
  outputId: string,
  outputs: RuleBuilderModel["outputs"],
): string[] {
  const output = schema.outputs.find((entry) => entry.id === outputId);
  const required = new Set(output?.requires ?? []);
  return schema.outputs
    .filter(
      (entry) => required.has(entry.id) && !Object.hasOwn(outputs, entry.id),
    )
    .map((entry) => entry.id);
}

/// The value a newly chosen output starts with.
export function initialRuleBuilderOutputValue(
  output: RuleBuilderOutput,
): RuleBuilderOutputValue {
  switch (output.value.kind) {
    case "flag":
      return true;
    case "text_list":
      return [];
    case "structured":
      return Object.fromEntries(output.value.parts.map((part) => [part.id, ""]));
    default:
      return "";
  }
}

/// The catalogs the chosen outputs need, in write order without repeats.
export function ruleBuilderOptionSources(
  schema: RuleBuilderSchema,
  model: RuleBuilderModel | null,
): string[] {
  if (!model) return [];
  const sources: string[] = [];
  for (const output of ruleBuilderChosenOutputs(schema, model.outputs)) {
    const value = output.value;
    const source =
      value.kind === "choice" || value.kind === "text_list"
        ? value.options
        : undefined;
    if (source && !sources.includes(source)) sources.push(source);
  }
  return sources;
}

// ── Problems ──────────────────────────────────────────────────────────

/// Why the builder cannot write a source for the model yet. The builder shows
/// each one next to the row or output it names.
export const RULE_BUILDER_PROBLEM_CODES = [
  /// The rule has no condition.
  "BUILDER_NO_CONDITIONS",
  /// The rule chooses no output.
  "BUILDER_NO_OUTPUTS",
  /// The rule chooses none of the outputs it needs one of.
  "BUILDER_OUTPUT_REQUIRED",
  /// A condition has no field chosen.
  "BUILDER_FACT_REQUIRED",
  /// A condition names a field the schema does not offer for this variant.
  "BUILDER_FACT_UNKNOWN",
  /// A condition has no comparison, or one that does not fit its field.
  "BUILDER_OPERATOR_INVALID",
  /// A condition or output needs a value and has none.
  "BUILDER_VALUE_REQUIRED",
  /// A number condition's value is not a plain decimal number.
  "BUILDER_VALUE_NOT_NUMBER",
  /// A day count is not a whole number of days.
  "BUILDER_VALUE_NOT_DAYS",
  /// A number output's value is not a whole number.
  "BUILDER_VALUE_NOT_WHOLE",
  /// A number output's value is outside the range its output accepts.
  "BUILDER_VALUE_OUT_OF_RANGE",
  /// A condition's value is not one of its field's known values.
  "BUILDER_VALUE_NOT_OFFERED",
  /// A value holds characters a Rego string literal cannot carry faithfully.
  "BUILDER_VALUE_UNREPRESENTABLE",
  /// A value does not have the shape its output accepts.
  "BUILDER_VALUE_INVALID",
  /// A value is longer than its output accepts.
  "BUILDER_VALUE_TOO_LONG",
  /// A list holds more entries than its output accepts.
  "BUILDER_VALUE_TOO_MANY",
  /// A list names the same entry twice.
  "BUILDER_VALUE_DUPLICATE",
  /// The model chooses an output the schema does not know.
  "BUILDER_OUTPUT_UNKNOWN",
  /// An output is not available for the rule's variant.
  "BUILDER_OUTPUT_NOT_IN_SCOPE",
  /// Two chosen outputs are ones a rule never declares together.
  "BUILDER_OUTPUTS_CONFLICT",
  /// An output needs another output that is not chosen.
  "BUILDER_OUTPUT_NEEDS_OTHER",
  /// A picked value is not in its catalog as loaded. Shown, never blocking:
  /// the rule keeps the value.
  "BUILDER_VALUE_NOT_IN_CATALOG",
] as const;

export type RuleBuilderProblemCode = (typeof RULE_BUILDER_PROBLEM_CODES)[number];

export type RuleBuilderProblem = {
  code: RuleBuilderProblemCode;
  /// The condition row the problem belongs to.
  conditionIndex?: number;
  /// The output the problem belongs to.
  output?: string;
  /// The structured part of that output.
  part?: string;
  /// Outputs the problem refers to, for its message.
  outputs?: string[];
};

export const RULE_BUILDER_PROBLEM_KEYS: Record<RuleBuilderProblemCode, string> =
  {
    BUILDER_NO_CONDITIONS: "settings.ruleBuilderProblemNoConditions",
    BUILDER_NO_OUTPUTS: "settings.ruleBuilderProblemNoOutputs",
    BUILDER_OUTPUT_REQUIRED: "settings.ruleBuilderProblemOutputRequired",
    BUILDER_FACT_REQUIRED: "settings.ruleBuilderProblemFactRequired",
    BUILDER_FACT_UNKNOWN: "settings.ruleBuilderProblemFactUnknown",
    BUILDER_OPERATOR_INVALID: "settings.ruleBuilderProblemOperatorInvalid",
    BUILDER_VALUE_REQUIRED: "settings.ruleBuilderProblemValueRequired",
    BUILDER_VALUE_NOT_NUMBER: "settings.ruleBuilderProblemValueNotNumber",
    BUILDER_VALUE_NOT_DAYS: "settings.ruleBuilderProblemValueNotDays",
    BUILDER_VALUE_NOT_WHOLE: "settings.ruleBuilderProblemValueNotWhole",
    BUILDER_VALUE_OUT_OF_RANGE: "settings.ruleBuilderProblemValueOutOfRange",
    BUILDER_VALUE_NOT_OFFERED: "settings.ruleBuilderProblemValueNotOffered",
    BUILDER_VALUE_UNREPRESENTABLE:
      "settings.ruleBuilderProblemValueUnrepresentable",
    BUILDER_VALUE_INVALID: "settings.ruleBuilderProblemValueInvalid",
    BUILDER_VALUE_TOO_LONG: "settings.ruleBuilderProblemValueTooLong",
    BUILDER_VALUE_TOO_MANY: "settings.ruleBuilderProblemValueTooMany",
    BUILDER_VALUE_DUPLICATE: "settings.ruleBuilderProblemValueDuplicate",
    BUILDER_OUTPUT_UNKNOWN: "settings.ruleBuilderProblemOutputUnknown",
    BUILDER_OUTPUT_NOT_IN_SCOPE: "settings.ruleBuilderProblemOutputNotInScope",
    BUILDER_OUTPUTS_CONFLICT: "settings.ruleBuilderProblemOutputsConflict",
    BUILDER_OUTPUT_NEEDS_OTHER: "settings.ruleBuilderProblemOutputNeedsOther",
    BUILDER_VALUE_NOT_IN_CATALOG: "settings.ruleBuilderProblemValueNotInCatalog",
  };

export type RuleBuilderResult =
  | { ok: true; source: string }
  | { ok: false; problems: RuleBuilderProblem[] };

// ── Values ────────────────────────────────────────────────────────────

/// Characters a Rego string literal cannot carry as typed: control characters,
/// line separators and lone surrogates.
function hasUnrepresentable(value: string): boolean {
  // Iterating a string yields code points, so a lone surrogate arrives alone.
  for (const char of value) {
    const code = char.codePointAt(0) ?? 0;
    if (code <= 0x1f || (code >= 0x7f && code <= 0x9f)) return true;
    if (code === 0x2028 || code === 0x2029) return true;
    if (code >= 0xd800 && code <= 0xdfff) return true;
  }
  return false;
}

const DECIMAL = /^-?(0|[1-9][0-9]*)(\.[0-9]{1,6})?$/;
const WHOLE = /^-?(0|[1-9][0-9]*)$/;
const WHOLE_DAYS = /^(0|[1-9][0-9]*)$/;
const MAX_DAYS = 100_000;

const UTF8 = new TextEncoder();

function textProblem(
  value: string,
  constraints: RuleBuilderTextConstraints = {},
): RuleBuilderProblemCode | null {
  if (value.trim() === "") return "BUILDER_VALUE_REQUIRED";
  if (hasUnrepresentable(value)) return "BUILDER_VALUE_UNREPRESENTABLE";
  if (
    constraints.maxBytes !== undefined &&
    UTF8.encode(value).length > constraints.maxBytes
  )
    return "BUILDER_VALUE_TOO_LONG";
  if (
    constraints.pattern !== undefined &&
    !new RegExp(`^(?:${constraints.pattern})$`, "u").test(value)
  )
    return "BUILDER_VALUE_INVALID";
  return null;
}

/// A Rego string literal for a value `textProblem` accepted. Rego string
/// literals use JSON's escapes, and with control characters refused the only
/// escapes left are the quote and the backslash.
function regoString(value: string): string {
  return JSON.stringify(value);
}

function decimalProblem(value: string): RuleBuilderProblemCode | null {
  if (value.trim() === "") return "BUILDER_VALUE_REQUIRED";
  if (!DECIMAL.test(value)) return "BUILDER_VALUE_NOT_NUMBER";
  const whole = value.split(".")[0].replace("-", "");
  if (!Number.isSafeInteger(Number(whole))) return "BUILDER_VALUE_NOT_NUMBER";
  return null;
}

function daysProblem(value: string): RuleBuilderProblemCode | null {
  if (value.trim() === "") return "BUILDER_VALUE_REQUIRED";
  if (!WHOLE_DAYS.test(value) || Number(value) > MAX_DAYS)
    return "BUILDER_VALUE_NOT_DAYS";
  return null;
}

function wholeProblem(
  value: string,
  min: number,
  max: number,
): RuleBuilderProblemCode | null {
  if (value.trim() === "") return "BUILDER_VALUE_REQUIRED";
  if (!WHOLE.test(value)) return "BUILDER_VALUE_NOT_WHOLE";
  const number = Number(value);
  if (!Number.isSafeInteger(number) || number < min || number > max)
    return "BUILDER_VALUE_OUT_OF_RANGE";
  return null;
}

/// Fill a template's placeholders in one pass, so a value that happens to
/// contain placeholder text is never read as one.
function fill(template: string, values: Readonly<Record<string, string>>) {
  return template.replace(/\{([a-z]+(?:\.[a-z_]+)?)\}/g, (whole, name: string) =>
    Object.hasOwn(values, name) ? values[name] : whole,
  );
}

// ── Conditions ────────────────────────────────────────────────────────

function checkCondition(
  schema: RuleBuilderSchema,
  condition: RuleBuilderCondition,
  fields: ReadonlyMap<string, RuleBuilderField>,
): { problem: RuleBuilderProblemCode } | { lines: string[] } {
  if (!condition.field) return { problem: "BUILDER_FACT_REQUIRED" };
  const field = fields.get(condition.field);
  if (!field) return { problem: "BUILDER_FACT_UNKNOWN" };
  const operator = ruleBuilderOperators(schema, field.kind).find(
    (candidate) => candidate.id === condition.operator,
  );
  if (!operator) return { problem: "BUILDER_OPERATOR_INVALID" };

  const value = condition.value;
  let literal = "";
  switch (operator.value) {
    case "none":
      break;
    case "number": {
      const problem = decimalProblem(value);
      if (problem) return { problem };
      literal = value;
      break;
    }
    case "days": {
      const problem = daysProblem(value);
      if (problem) return { problem };
      literal = value;
      break;
    }
    case "choice": {
      if (value === "") return { problem: "BUILDER_VALUE_REQUIRED" };
      if (!field.choices?.some((choice) => choice.value === value))
        return { problem: "BUILDER_VALUE_NOT_OFFERED" };
      literal = regoString(value);
      break;
    }
    case "text": {
      const problem = textProblem(value);
      if (problem) return { problem };
      literal = regoString(value);
      break;
    }
  }
  return {
    lines: fill(operator.template, { path: field.path, value: literal }).split(
      "\n",
    ),
  };
}

// ── Outputs ───────────────────────────────────────────────────────────

type Checked =
  | { problem: RuleBuilderProblemCode; part?: string }
  | { blocks: string[] };

function outputBlocks(
  schema: RuleBuilderSchema,
  output: RuleBuilderOutput,
  value: RuleBuilderOutputValue,
): Checked {
  const base = { head: output.head, helper: schema.helperRule };
  const spec = output.value;
  switch (spec.kind) {
    case "flag":
      if (value !== true) return { problem: "BUILDER_VALUE_REQUIRED" };
      return { blocks: [fill(output.template, base)] };
    case "text":
    case "choice": {
      const text = typeof value === "string" ? value : "";
      const problem = textProblem(text, spec);
      if (problem) return { problem };
      return {
        blocks: [fill(output.template, { ...base, value: regoString(text) })],
      };
    }
    case "number": {
      const text = typeof value === "string" ? value : "";
      const problem = wholeProblem(text, spec.min, spec.max);
      if (problem) return { problem };
      return { blocks: [fill(output.template, { ...base, value: text })] };
    }
    case "text_list": {
      const items = Array.isArray(value) ? value : [];
      if (items.length === 0) return { problem: "BUILDER_VALUE_REQUIRED" };
      if (spec.maxItems !== undefined && items.length > spec.maxItems)
        return { problem: "BUILDER_VALUE_TOO_MANY" };
      for (const item of items) {
        const problem = textProblem(item, spec);
        if (problem) return { problem };
      }
      if (new Set(items).size !== items.length)
        return { problem: "BUILDER_VALUE_DUPLICATE" };
      const literals = items.map(regoString);
      if (output.perItem) {
        return {
          blocks: literals.map((item) =>
            fill(output.template, { ...base, item }),
          ),
        };
      }
      return {
        blocks: [fill(output.template, { ...base, items: literals.join(", ") })],
      };
    }
    case "structured": {
      const parts =
        typeof value === "object" && value !== null && !Array.isArray(value)
          ? value
          : {};
      const values: Record<string, string> = { ...base };
      for (const part of spec.parts) {
        const text = parts[part.id] ?? "";
        const problem =
          part.kind === "number"
            ? wholeProblem(text, part.min, part.max)
            : textProblem(text, part);
        if (problem) return { problem, part: part.id };
        values[`part.${part.id}`] =
          part.kind === "number" ? text : regoString(text);
      }
      return { blocks: [fill(output.template, values)] };
    }
  }
}

// ── Generation ────────────────────────────────────────────────────────

/// Write the Rego source for a model: one helper rule holding the conditions
/// (one body for ALL, one body per condition for ANY), then each chosen
/// output in the schema's order. The same schema, model and variant always
/// give the same bytes; a model that cannot be written gives its problems
/// instead of a source.
export function generateRuleSource(
  schema: RuleBuilderSchema,
  model: RuleBuilderModel,
  variant: string | null,
): RuleBuilderResult {
  const problems: RuleBuilderProblem[] = [];
  const fields = new Map(
    ruleBuilderFields(schema, variant).map((field) => [field.path, field]),
  );

  if (model.conditions.length === 0)
    problems.push({ code: "BUILDER_NO_CONDITIONS" });
  const bodies: string[][] = [];
  model.conditions.forEach((condition, conditionIndex) => {
    const checked = checkCondition(schema, condition, fields);
    if ("problem" in checked)
      problems.push({ code: checked.problem, conditionIndex });
    else bodies.push(checked.lines);
  });

  const chosen = ruleBuilderChosenOutputs(schema, model.outputs);
  const chosenIds = new Set(chosen.map((output) => output.id));
  for (const rule of schema.outputRules) {
    if (rule.rule === "at_most_one") continue;
    const of = rule.of ?? schema.outputs.map((output) => output.id);
    if (of.some((id) => chosenIds.has(id))) continue;
    problems.push(
      rule.of
        ? { code: "BUILDER_OUTPUT_REQUIRED", outputs: [...rule.of] }
        : { code: "BUILDER_NO_OUTPUTS" },
    );
  }
  const known = new Set(schema.outputs.map((output) => output.id));
  for (const id of Object.keys(model.outputs).sort()) {
    if (!known.has(id))
      problems.push({ code: "BUILDER_OUTPUT_UNKNOWN", output: id });
  }

  const blocks: string[] = [];
  for (const output of chosen) {
    if (!isRuleBuilderOutputInVariant(output, variant)) {
      problems.push({ code: "BUILDER_OUTPUT_NOT_IN_SCOPE", output: output.id });
      continue;
    }
    const conflicts = ruleBuilderConflicts(schema, output.id, model.outputs);
    if (conflicts.length > 0) {
      problems.push({
        code: "BUILDER_OUTPUTS_CONFLICT",
        output: output.id,
        outputs: conflicts,
      });
      continue;
    }
    const missing = ruleBuilderMissingRequirements(
      schema,
      output.id,
      model.outputs,
    );
    if (missing.length > 0) {
      problems.push({
        code: "BUILDER_OUTPUT_NEEDS_OTHER",
        output: output.id,
        outputs: missing,
      });
      continue;
    }
    const checked = outputBlocks(schema, output, model.outputs[output.id]);
    if ("problem" in checked)
      problems.push({
        code: checked.problem,
        output: output.id,
        ...(checked.part ? { part: checked.part } : {}),
      });
    else blocks.push(...checked.blocks);
  }

  if (problems.length > 0) return { ok: false, problems };

  const helper = schema.helperRule;
  const joined =
    model.join === "ALL" ? [bodies.flat()] : bodies.map((lines) => lines);
  const helperBlocks = joined.map(
    (lines) => `${helper} if {\n${lines.map((line) => `\t${line}`).join("\n")}\n}`,
  );
  return {
    ok: true,
    source: `${schema.preamble}${[...helperBlocks, ...blocks].join("\n\n")}\n`,
  };
}

/// True when `source` is what the builder writes for this model, either as
/// generated or as the API hands a stored source back to the editor, with the
/// preamble removed.
export function isRuleBuilderSource(
  schema: RuleBuilderSchema,
  model: RuleBuilderModel,
  variant: string | null,
  source: string,
): boolean {
  const result = generateRuleSource(schema, model, variant);
  if (!result.ok) return false;
  return (
    source === result.source ||
    source === result.source.slice(schema.preamble.length)
  );
}

// ── Model documents ───────────────────────────────────────────────────

/// A new rule's starting point: one unfinished condition and no output.
export function emptyRuleBuilderModel(
  schema: RuleBuilderSchema,
): RuleBuilderModel {
  return {
    version: RULE_BUILDER_MODEL_VERSION,
    family: schema.family,
    join: "ALL",
    conditions: [{ field: "", operator: "", value: "" }],
    outputs: {},
  };
}

/// Nothing has been chosen yet, so leaving the builder loses nothing.
export function isBlankRuleBuilderModel(model: RuleBuilderModel): boolean {
  return (
    Object.keys(model.outputs).length === 0 &&
    model.conditions.every(
      (condition) =>
        !condition.field && !condition.operator && !condition.value,
    )
  );
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isStringList(value: unknown): value is string[] {
  return Array.isArray(value) && value.every((item) => typeof item === "string");
}

function parseOutputValue(
  output: RuleBuilderOutput,
  value: unknown,
): RuleBuilderOutputValue | null {
  switch (output.value.kind) {
    case "flag":
      return value === true ? true : null;
    case "text_list":
      return isStringList(value) ? [...value] : null;
    case "structured": {
      if (!isRecord(value)) return null;
      const ids = output.value.parts.map((part) => part.id);
      if (Object.keys(value).some((key) => !ids.includes(key))) return null;
      const parts: Record<string, string> = {};
      for (const id of ids) {
        if (typeof value[id] !== "string") return null;
        parts[id] = value[id];
      }
      return parts;
    }
    default:
      return typeof value === "string" ? value : null;
  }
}

/// Read a stored model document for this schema. Anything that is not the
/// current version of this family, or that this build does not recognise, is
/// null: the rule then opens as Rego.
export function parseRuleBuilderModel(
  schema: RuleBuilderSchema,
  document: unknown,
): RuleBuilderModel | null {
  if (!isRecord(document)) return null;
  if (
    document.version !== RULE_BUILDER_MODEL_VERSION ||
    document.family !== schema.family
  )
    return null;
  const value = document;
  if (value.join !== "ALL" && value.join !== "ANY") return null;
  if (!Array.isArray(value.conditions)) return null;

  const operators = new Set(
    Object.values(schema.kinds).flatMap((kind) =>
      kind.operators.map((operator) => operator.id),
    ),
  );
  const conditions: RuleBuilderCondition[] = [];
  for (const entry of value.conditions) {
    if (!isRecord(entry)) return null;
    const { field, operator, value: text } = entry;
    if (typeof field !== "string" || typeof text !== "string") return null;
    if (typeof operator !== "string") return null;
    if (operator !== "" && !operators.has(operator)) return null;
    conditions.push({ field, operator, value: text });
  }

  if (!isRecord(value.outputs)) return null;
  const outputs: Record<string, RuleBuilderOutputValue> = {};
  for (const [id, entry] of Object.entries(value.outputs)) {
    const output = schema.outputs.find((candidate) => candidate.id === id);
    if (!output) return null;
    const parsed = parseOutputValue(output, entry);
    if (parsed === null) return null;
    outputs[id] = parsed;
  }

  return {
    version: RULE_BUILDER_MODEL_VERSION,
    family: schema.family,
    join: value.join,
    conditions,
    outputs,
  };
}
