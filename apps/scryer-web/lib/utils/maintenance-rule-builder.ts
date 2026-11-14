import maintenanceContract from "../contracts/maintenance-input-contract.json" with { type: "json" };
import type {
  MaintenanceActionHead,
  MaintenanceRuleScope,
} from "@/lib/types/maintenance-rule-sets";
import {
  MAINTENANCE_ACTION_HEAD_ORDER,
  MAINTENANCE_ACTION_HEAD_RULE_NAMES,
} from "./maintenance-rule-sets.ts";

/// The simple builder writes a maintenance rule's Rego for an operator who
/// never writes Rego. The model below is what it edits and what the server
/// stores beside the rule; the source it generates is what actually runs.

export const MAINTENANCE_BUILDER_MODEL_VERSION = 1;

/// How a condition reads its fact. Derived from the input contract's type, so
/// the operators offered always fit the fact.
export type MaintenanceBuilderFactKind =
  | "bool"
  | "number"
  | "string"
  | "string_list"
  | "timestamp";

export type MaintenanceBuilderOperator =
  | "IS_TRUE"
  | "IS_FALSE"
  | "EQUALS"
  | "NOT_EQUALS"
  | "GREATER_THAN"
  | "GREATER_OR_EQUAL"
  | "LESS_THAN"
  | "LESS_OR_EQUAL"
  | "CONTAINS"
  | "NOT_CONTAINS"
  | "IS_EMPTY"
  | "IS_NOT_EMPTY"
  | "MORE_THAN_DAYS_AGO"
  | "LESS_THAN_DAYS_AGO"
  | "HAS_VALUE"
  | "HAS_NO_VALUE";

/// One condition. `fact` is a name under `input.facts`; `value` is the text the
/// operator typed, kept as text so the model is exactly what was entered and
/// the generator is the one place that decides whether it can be written.
/// An empty `fact` or `operator` is a row the operator has not finished.
export type MaintenanceBuilderCondition = {
  fact: string;
  operator: MaintenanceBuilderOperator | "";
  value: string;
};

export type MaintenanceBuilderFlagHead =
  | "UNMONITOR"
  | "MONITOR"
  | "DELETE_FILES"
  | "DELETE_TITLE"
  | "SEARCH"
  | "FETCH_SUBTITLES";
export type MaintenanceBuilderTextHead =
  | "SET_QUALITY_PROFILE"
  | "MOVE_TO_LIBRARY"
  | "MOVE_TO_ROOT"
  | "NOTIFY";
export type MaintenanceBuilderListHead = "ADD_TAGS" | "REMOVE_TAGS";

/// The chosen actions. A key that is present is a chosen action: flags carry
/// `true`, valued heads carry their literal.
export type MaintenanceBuilderActions = {
  [H in MaintenanceBuilderFlagHead]?: true;
} & { [H in MaintenanceBuilderTextHead]?: string } & {
  [H in MaintenanceBuilderListHead]?: string[];
};

export type MaintenanceBuilderJoin = "ALL" | "ANY";

/// What the server stores as an opaque JSON document next to the rule.
export type MaintenanceBuilderModel = {
  version: typeof MAINTENANCE_BUILDER_MODEL_VERSION;
  join: MaintenanceBuilderJoin;
  conditions: MaintenanceBuilderCondition[];
  actions: MaintenanceBuilderActions;
};

export const MAINTENANCE_BUILDER_FLAG_HEADS: readonly MaintenanceBuilderFlagHead[] =
  [
    "UNMONITOR",
    "MONITOR",
    "DELETE_FILES",
    "DELETE_TITLE",
    "SEARCH",
    "FETCH_SUBTITLES",
  ];
export const MAINTENANCE_BUILDER_TEXT_HEADS: readonly MaintenanceBuilderTextHead[] =
  ["SET_QUALITY_PROFILE", "MOVE_TO_LIBRARY", "MOVE_TO_ROOT", "NOTIFY"];
export const MAINTENANCE_BUILDER_LIST_HEADS: readonly MaintenanceBuilderListHead[] =
  ["ADD_TAGS", "REMOVE_TAGS"];

/// Heads that act on whole titles, so only a title-scoped rule may declare them.
const TITLE_ONLY_HEADS: ReadonlySet<MaintenanceActionHead> = new Set([
  "DELETE_TITLE",
  "MOVE_TO_LIBRARY",
  "MOVE_TO_ROOT",
]);

/// Pairs the server holds for every subject that decides both, so a rule whose
/// only condition set declares both could never act.
const CONFLICTING_HEADS: readonly (readonly [
  MaintenanceActionHead,
  MaintenanceActionHead,
])[] = [
  ["MONITOR", "UNMONITOR"],
  ["MONITOR", "DELETE_FILES"],
  ["MONITOR", "DELETE_TITLE"],
  ["MOVE_TO_LIBRARY", "MOVE_TO_ROOT"],
  ["MOVE_TO_LIBRARY", "DELETE_FILES"],
  ["MOVE_TO_LIBRARY", "DELETE_TITLE"],
  ["MOVE_TO_ROOT", "DELETE_FILES"],
  ["MOVE_TO_ROOT", "DELETE_TITLE"],
];

/// The helper rule every generated action head refers to.
export const MAINTENANCE_BUILDER_HELPER_RULE = "rule_applies";

const SOURCE_PREAMBLE = "package rules\nimport rego.v1\n\n";

// ── Facts ─────────────────────────────────────────────────────────────

export type MaintenanceBuilderFact = {
  name: string;
  kind: MaintenanceBuilderFactKind;
  descKey: string;
};

type ContractField = {
  field: string;
  type: string;
  descKey: string;
  /// Rule scopes that carry this fact. The contract lists none today, which
  /// means every scope's input carries it.
  scopes?: string[];
};

const FACT_NAME = /^[a-z_][a-z0-9_]*$/;

/// The contract types a single condition can read. Object lists such as
/// `files` need a nested condition the builder does not model.
function factKind(field: ContractField): MaintenanceBuilderFactKind | null {
  switch (field.type) {
    case "bool":
      return "bool";
    case "number":
      return "number";
    case "string[]":
      return "string_list";
    case "string":
      // Timestamps travel as RFC 3339 strings, and every timestamp fact in the
      // contract is named for the moment it records.
      return field.field.endsWith("_at") ? "timestamp" : "string";
    default:
      return null;
  }
}

const FACT_SECTION = maintenanceContract.sections.find(
  (section) => section.path === "input.facts",
);

/// Every fact a condition can read for a rule of this scope, in contract order.
export function maintenanceBuilderFacts(
  scope: MaintenanceRuleScope,
): MaintenanceBuilderFact[] {
  const fields = (FACT_SECTION?.fields ?? []) as ContractField[];
  const facts: MaintenanceBuilderFact[] = [];
  for (const field of fields) {
    if (!FACT_NAME.test(field.field)) continue;
    if (field.scopes && !field.scopes.includes(scope)) continue;
    const kind = factKind(field);
    if (kind) facts.push({ name: field.field, kind, descKey: field.descKey });
  }
  return facts;
}

const OPERATORS_BY_KIND: Record<
  MaintenanceBuilderFactKind,
  readonly MaintenanceBuilderOperator[]
> = {
  bool: ["IS_TRUE", "IS_FALSE"],
  number: [
    "EQUALS",
    "NOT_EQUALS",
    "GREATER_THAN",
    "GREATER_OR_EQUAL",
    "LESS_THAN",
    "LESS_OR_EQUAL",
    "HAS_VALUE",
    "HAS_NO_VALUE",
  ],
  string: ["EQUALS", "NOT_EQUALS", "HAS_VALUE", "HAS_NO_VALUE"],
  string_list: ["CONTAINS", "NOT_CONTAINS", "IS_EMPTY", "IS_NOT_EMPTY"],
  timestamp: [
    "MORE_THAN_DAYS_AGO",
    "LESS_THAN_DAYS_AGO",
    "HAS_VALUE",
    "HAS_NO_VALUE",
  ],
};

export function maintenanceBuilderOperators(
  kind: MaintenanceBuilderFactKind,
): readonly MaintenanceBuilderOperator[] {
  return OPERATORS_BY_KIND[kind];
}

/// What kind of value an operator takes, or null when it takes none.
export function maintenanceBuilderValueKind(
  kind: MaintenanceBuilderFactKind,
  operator: MaintenanceBuilderOperator,
): "number" | "text" | "days" | null {
  switch (operator) {
    case "IS_TRUE":
    case "IS_FALSE":
    case "IS_EMPTY":
    case "IS_NOT_EMPTY":
    case "HAS_VALUE":
    case "HAS_NO_VALUE":
      return null;
    case "MORE_THAN_DAYS_AGO":
    case "LESS_THAN_DAYS_AGO":
      return "days";
    default:
      return kind === "number" ? "number" : "text";
  }
}

// ── Actions ───────────────────────────────────────────────────────────

/// The chosen heads in the order the server applies them.
export function maintenanceBuilderChosenHeads(
  actions: MaintenanceBuilderActions,
): MaintenanceActionHead[] {
  return MAINTENANCE_ACTION_HEAD_ORDER.filter((head) =>
    Object.hasOwn(actions, head),
  );
}

export function isMaintenanceBuilderHeadInScope(
  head: MaintenanceActionHead,
  scope: MaintenanceRuleScope,
): boolean {
  return scope === "TITLE" || !TITLE_ONLY_HEADS.has(head);
}

/// The chosen heads that cannot be combined with `head`.
export function maintenanceBuilderConflicts(
  head: MaintenanceActionHead,
  actions: MaintenanceBuilderActions,
): MaintenanceActionHead[] {
  const chosen = new Set(maintenanceBuilderChosenHeads(actions));
  const conflicts = new Set<MaintenanceActionHead>();
  for (const [left, right] of CONFLICTING_HEADS) {
    if (left === head && chosen.has(right)) conflicts.add(right);
    if (right === head && chosen.has(left)) conflicts.add(left);
  }
  return MAINTENANCE_ACTION_HEAD_ORDER.filter((other) => conflicts.has(other));
}

// ── Generation ────────────────────────────────────────────────────────

/// Why the builder cannot write a source for the model yet. The builder shows
/// each one next to the row or action it names.
export const MAINTENANCE_BUILDER_PROBLEM_CODES = [
  /// The rule has no condition.
  "BUILDER_NO_CONDITIONS",
  /// The rule chooses no action.
  "BUILDER_NO_ACTIONS",
  /// A condition has no fact chosen.
  "BUILDER_FACT_REQUIRED",
  /// A condition names a fact the contract does not offer for this scope.
  "BUILDER_FACT_UNKNOWN",
  /// A condition has no operator, or one that does not fit its fact.
  "BUILDER_OPERATOR_INVALID",
  /// A condition or action needs a value and has none.
  "BUILDER_VALUE_REQUIRED",
  /// A number condition's value is not a plain decimal number.
  "BUILDER_VALUE_NOT_NUMBER",
  /// A day count is not a whole number of days.
  "BUILDER_VALUE_NOT_DAYS",
  /// A value holds characters a Rego string literal cannot carry faithfully.
  "BUILDER_VALUE_UNREPRESENTABLE",
  /// A tag list names the same tag twice.
  "BUILDER_VALUE_DUPLICATE",
  /// An action acts on whole titles and the rule is not title-scoped.
  "BUILDER_ACTION_NOT_IN_SCOPE",
  /// Two chosen actions are ones the server never applies together.
  "BUILDER_ACTIONS_CONFLICT",
] as const;

export type MaintenanceBuilderProblemCode =
  (typeof MAINTENANCE_BUILDER_PROBLEM_CODES)[number];

export type MaintenanceBuilderProblem = {
  code: MaintenanceBuilderProblemCode;
  /// The condition row the problem belongs to.
  conditionIndex?: number;
  /// The action the problem belongs to.
  head?: MaintenanceActionHead;
};

export const MAINTENANCE_BUILDER_PROBLEM_KEYS: Record<
  MaintenanceBuilderProblemCode,
  string
> = {
  BUILDER_NO_CONDITIONS: "settings.maintenanceBuilderProblemNoConditions",
  BUILDER_NO_ACTIONS: "settings.maintenanceBuilderProblemNoActions",
  BUILDER_FACT_REQUIRED: "settings.maintenanceBuilderProblemFactRequired",
  BUILDER_FACT_UNKNOWN: "settings.maintenanceBuilderProblemFactUnknown",
  BUILDER_OPERATOR_INVALID: "settings.maintenanceBuilderProblemOperatorInvalid",
  BUILDER_VALUE_REQUIRED: "settings.maintenanceBuilderProblemValueRequired",
  BUILDER_VALUE_NOT_NUMBER: "settings.maintenanceBuilderProblemValueNotNumber",
  BUILDER_VALUE_NOT_DAYS: "settings.maintenanceBuilderProblemValueNotDays",
  BUILDER_VALUE_UNREPRESENTABLE:
    "settings.maintenanceBuilderProblemValueUnrepresentable",
  BUILDER_VALUE_DUPLICATE: "settings.maintenanceBuilderProblemValueDuplicate",
  BUILDER_ACTION_NOT_IN_SCOPE:
    "settings.maintenanceBuilderProblemActionNotInScope",
  BUILDER_ACTIONS_CONFLICT: "settings.maintenanceBuilderProblemActionsConflict",
};

export const MAINTENANCE_BUILDER_OPERATOR_KEYS: Record<
  MaintenanceBuilderOperator,
  string
> = {
  IS_TRUE: "settings.maintenanceBuilderOpIsTrue",
  IS_FALSE: "settings.maintenanceBuilderOpIsFalse",
  EQUALS: "settings.maintenanceBuilderOpEquals",
  NOT_EQUALS: "settings.maintenanceBuilderOpNotEquals",
  GREATER_THAN: "settings.maintenanceBuilderOpGreaterThan",
  GREATER_OR_EQUAL: "settings.maintenanceBuilderOpGreaterOrEqual",
  LESS_THAN: "settings.maintenanceBuilderOpLessThan",
  LESS_OR_EQUAL: "settings.maintenanceBuilderOpLessOrEqual",
  CONTAINS: "settings.maintenanceBuilderOpContains",
  NOT_CONTAINS: "settings.maintenanceBuilderOpNotContains",
  IS_EMPTY: "settings.maintenanceBuilderOpIsEmpty",
  IS_NOT_EMPTY: "settings.maintenanceBuilderOpIsNotEmpty",
  MORE_THAN_DAYS_AGO: "settings.maintenanceBuilderOpMoreThanDaysAgo",
  LESS_THAN_DAYS_AGO: "settings.maintenanceBuilderOpLessThanDaysAgo",
  HAS_VALUE: "settings.maintenanceBuilderOpHasValue",
  HAS_NO_VALUE: "settings.maintenanceBuilderOpHasNoValue",
};

export type MaintenanceBuilderResult =
  | { ok: true; source: string }
  | { ok: false; problems: MaintenanceBuilderProblem[] };

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
const WHOLE_DAYS = /^(0|[1-9][0-9]*)$/;
const MAX_DAYS = 100_000;

function stringProblem(
  value: string,
): MaintenanceBuilderProblemCode | null {
  if (value.trim() === "") return "BUILDER_VALUE_REQUIRED";
  if (hasUnrepresentable(value)) return "BUILDER_VALUE_UNREPRESENTABLE";
  return null;
}

/// A Rego string literal for a value `stringProblem` accepted. Rego string
/// literals use JSON's escapes, and with control characters refused the only
/// escapes left are the quote and the backslash.
function regoString(value: string): string {
  return JSON.stringify(value);
}

function numberProblem(value: string): MaintenanceBuilderProblemCode | null {
  if (value.trim() === "") return "BUILDER_VALUE_REQUIRED";
  if (!DECIMAL.test(value)) return "BUILDER_VALUE_NOT_NUMBER";
  const whole = value.split(".")[0].replace("-", "");
  if (!Number.isSafeInteger(Number(whole))) return "BUILDER_VALUE_NOT_NUMBER";
  return null;
}

function daysProblem(value: string): MaintenanceBuilderProblemCode | null {
  if (value.trim() === "") return "BUILDER_VALUE_REQUIRED";
  if (!WHOLE_DAYS.test(value) || Number(value) > MAX_DAYS)
    return "BUILDER_VALUE_NOT_DAYS";
  return null;
}

const COMPARISON: Partial<Record<MaintenanceBuilderOperator, string>> = {
  EQUALS: "==",
  NOT_EQUALS: "!=",
  GREATER_THAN: ">",
  GREATER_OR_EQUAL: ">=",
  LESS_THAN: "<",
  LESS_OR_EQUAL: "<=",
};

function conditionExpression(
  condition: MaintenanceBuilderCondition,
  fact: MaintenanceBuilderFact,
  operator: MaintenanceBuilderOperator,
): string {
  const path = `input.facts.${fact.name}`;
  const value = condition.value;
  switch (operator) {
    case "IS_TRUE":
    case "HAS_VALUE":
      return path;
    case "IS_FALSE":
    case "HAS_NO_VALUE":
      return `not ${path}`;
    case "CONTAINS":
      return `${regoString(value)} in ${path}`;
    case "NOT_CONTAINS":
      return `not ${regoString(value)} in ${path}`;
    case "IS_EMPTY":
      return `count(${path}) == 0`;
    case "IS_NOT_EMPTY":
      return `count(${path}) > 0`;
    case "MORE_THAN_DAYS_AGO":
      return `days_since(${path}) > ${value}`;
    case "LESS_THAN_DAYS_AGO":
      return `days_since(${path}) < ${value}`;
    default: {
      const literal = fact.kind === "number" ? value : regoString(value);
      return `${path} ${COMPARISON[operator]} ${literal}`;
    }
  }
}

function checkCondition(
  condition: MaintenanceBuilderCondition,
  facts: ReadonlyMap<string, MaintenanceBuilderFact>,
): { problem: MaintenanceBuilderProblemCode } | { expression: string } {
  if (!condition.fact) return { problem: "BUILDER_FACT_REQUIRED" };
  const fact = facts.get(condition.fact);
  if (!fact) return { problem: "BUILDER_FACT_UNKNOWN" };
  const operator = condition.operator;
  if (!operator || !OPERATORS_BY_KIND[fact.kind].includes(operator))
    return { problem: "BUILDER_OPERATOR_INVALID" };
  const valueKind = maintenanceBuilderValueKind(fact.kind, operator);
  const problem =
    valueKind === "number"
      ? numberProblem(condition.value)
      : valueKind === "days"
        ? daysProblem(condition.value)
        : valueKind === "text"
          ? stringProblem(condition.value)
          : null;
  if (problem) return { problem };
  return { expression: conditionExpression(condition, fact, operator) };
}

function headLine(
  head: MaintenanceActionHead,
  actions: MaintenanceBuilderActions,
): { problem: MaintenanceBuilderProblemCode } | { line: string } {
  const name = MAINTENANCE_ACTION_HEAD_RULE_NAMES[head];
  const tail = ` if ${MAINTENANCE_BUILDER_HELPER_RULE}`;
  if ((MAINTENANCE_BUILDER_FLAG_HEADS as readonly string[]).includes(head)) {
    return { line: `${name}${tail}` };
  }
  if ((MAINTENANCE_BUILDER_LIST_HEADS as readonly string[]).includes(head)) {
    const values = actions[head as MaintenanceBuilderListHead] ?? [];
    if (values.length === 0) return { problem: "BUILDER_VALUE_REQUIRED" };
    for (const value of values) {
      const problem = stringProblem(value);
      if (problem) return { problem };
    }
    if (new Set(values).size !== values.length)
      return { problem: "BUILDER_VALUE_DUPLICATE" };
    return {
      line: `${name} := [${values.map(regoString).join(", ")}]${tail}`,
    };
  }
  const value = actions[head as MaintenanceBuilderTextHead] ?? "";
  const problem = stringProblem(value);
  if (problem) return { problem };
  return { line: `${name} := ${regoString(value)}${tail}` };
}

/// Write the Rego source for a model: one helper rule holding the conditions
/// (one body for ALL, one body per condition for ANY), then one head per
/// chosen action in the order the server applies them. The same model and
/// scope always give the same bytes; a model that cannot be written gives its
/// problems instead of a source.
export function generateMaintenanceRuleSource(
  model: MaintenanceBuilderModel,
  scope: MaintenanceRuleScope,
): MaintenanceBuilderResult {
  const problems: MaintenanceBuilderProblem[] = [];
  const facts = new Map(
    maintenanceBuilderFacts(scope).map((fact) => [fact.name, fact]),
  );

  if (model.conditions.length === 0)
    problems.push({ code: "BUILDER_NO_CONDITIONS" });
  const expressions: string[] = [];
  model.conditions.forEach((condition, conditionIndex) => {
    const checked = checkCondition(condition, facts);
    if ("problem" in checked)
      problems.push({ code: checked.problem, conditionIndex });
    else expressions.push(checked.expression);
  });

  const heads = maintenanceBuilderChosenHeads(model.actions);
  if (heads.length === 0) problems.push({ code: "BUILDER_NO_ACTIONS" });
  const lines: string[] = [];
  for (const head of heads) {
    if (!isMaintenanceBuilderHeadInScope(head, scope)) {
      problems.push({ code: "BUILDER_ACTION_NOT_IN_SCOPE", head });
      continue;
    }
    if (maintenanceBuilderConflicts(head, model.actions).length > 0) {
      problems.push({ code: "BUILDER_ACTIONS_CONFLICT", head });
      continue;
    }
    const checked = headLine(head, model.actions);
    if ("problem" in checked) problems.push({ code: checked.problem, head });
    else lines.push(checked.line);
  }

  if (problems.length > 0) return { ok: false, problems };

  const helper = MAINTENANCE_BUILDER_HELPER_RULE;
  const bodies =
    model.join === "ALL"
      ? [expressions]
      : expressions.map((expression) => [expression]);
  const blocks = [
    ...bodies.map(
      (body) =>
        `${helper} if {\n${body.map((line) => `\t${line}`).join("\n")}\n}`,
    ),
    ...lines,
  ];
  return { ok: true, source: `${SOURCE_PREAMBLE}${blocks.join("\n\n")}\n` };
}

/// True when `source` is what the builder writes for this model, either as
/// generated or as the API hands a stored source back to the editor, with the
/// package and import lines it generates removed.
export function isMaintenanceBuilderSource(
  model: MaintenanceBuilderModel,
  scope: MaintenanceRuleScope,
  source: string,
): boolean {
  const result = generateMaintenanceRuleSource(model, scope);
  if (!result.ok) return false;
  return (
    source === result.source ||
    source === result.source.slice(SOURCE_PREAMBLE.length)
  );
}

// ── Model documents ───────────────────────────────────────────────────

/// A new rule's starting point: one unfinished condition and no action.
export function emptyMaintenanceBuilderModel(): MaintenanceBuilderModel {
  return {
    version: MAINTENANCE_BUILDER_MODEL_VERSION,
    join: "ALL",
    conditions: [{ fact: "", operator: "", value: "" }],
    actions: {},
  };
}

/// Nothing has been chosen yet, so leaving the builder loses nothing.
export function isBlankMaintenanceBuilderModel(
  model: MaintenanceBuilderModel,
): boolean {
  return (
    maintenanceBuilderChosenHeads(model.actions).length === 0 &&
    model.conditions.every(
      (condition) => !condition.fact && !condition.operator && !condition.value,
    )
  );
}

const ALL_OPERATORS: ReadonlySet<string> = new Set(
  Object.values(OPERATORS_BY_KIND).flat(),
);

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function parseCondition(value: unknown): MaintenanceBuilderCondition | null {
  if (!isRecord(value)) return null;
  const { fact, operator, value: text } = value;
  if (typeof fact !== "string" || typeof text !== "string") return null;
  if (operator !== "" && !ALL_OPERATORS.has(String(operator))) return null;
  return {
    fact,
    operator: operator as MaintenanceBuilderOperator | "",
    value: text,
  };
}

function parseActions(value: unknown): MaintenanceBuilderActions | null {
  if (!isRecord(value)) return null;
  const actions: MaintenanceBuilderActions = {};
  for (const [key, entry] of Object.entries(value)) {
    if ((MAINTENANCE_BUILDER_FLAG_HEADS as readonly string[]).includes(key)) {
      if (entry !== true) return null;
      actions[key as MaintenanceBuilderFlagHead] = true;
    } else if (
      (MAINTENANCE_BUILDER_TEXT_HEADS as readonly string[]).includes(key)
    ) {
      if (typeof entry !== "string") return null;
      actions[key as MaintenanceBuilderTextHead] = entry;
    } else if (
      (MAINTENANCE_BUILDER_LIST_HEADS as readonly string[]).includes(key)
    ) {
      if (
        !Array.isArray(entry) ||
        !entry.every((item) => typeof item === "string")
      )
        return null;
      actions[key as MaintenanceBuilderListHead] = [...entry];
    } else {
      return null;
    }
  }
  return actions;
}

/// Read a stored model document. Anything this build does not recognise,
/// including a later version, is null: the rule then opens as Rego.
export function parseMaintenanceBuilderModel(
  value: unknown,
): MaintenanceBuilderModel | null {
  if (!isRecord(value)) return null;
  if (value.version !== MAINTENANCE_BUILDER_MODEL_VERSION) return null;
  if (value.join !== "ALL" && value.join !== "ANY") return null;
  if (!Array.isArray(value.conditions)) return null;
  const conditions: MaintenanceBuilderCondition[] = [];
  for (const entry of value.conditions) {
    const condition = parseCondition(entry);
    if (!condition) return null;
    conditions.push(condition);
  }
  const actions = parseActions(value.actions);
  if (!actions) return null;
  return {
    version: MAINTENANCE_BUILDER_MODEL_VERSION,
    join: value.join,
    conditions,
    actions,
  };
}
