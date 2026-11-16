import * as React from "react";
import { ChevronRight, Plus, Trash2 } from "lucide-react";
import { HelpLabel } from "@/components/common/info-help";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible";
import { IconButton } from "@/components/ui/icon-button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { MultiSelectDropdown } from "@/components/ui/multi-select-dropdown";
import { SingleSelectField } from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";
import { useTranslate } from "@/lib/context/translate-context";
import {
  RULE_BUILDER_PROBLEM_KEYS,
  addRuleBuilderAction,
  initialRuleBuilderOutputValue,
  isRuleBuilderOutputInVariant,
  ruleBuilderConflicts,
  ruleBuilderOutputHelp,
  ruleBuilderOutputLabel,
  ruleBuilderOutputPlaceholder,
  ruleBuilderActionLabel,
  ruleBuilderActionRows,
  ruleBuilderFields,
  ruleBuilderOperators,
  ruleBuilderRowSubject,
  ruleBuilderSubjectFields,
  removeRuleBuilderAction,
  setRuleBuilderActionConditions,
  setRuleBuilderActionOutput,
  setRuleBuilderActionSubject,
  translatedOr,
  type RuleBuilderActionRow,
  type RuleBuilderCondition,
  type RuleBuilderField,
  type RuleBuilderJoin,
  type RuleBuilderModel,
  type RuleBuilderOutput,
  type RuleBuilderOutputValue,
  type RuleBuilderOptionValue,
  type RuleBuilderProblem,
  type RuleBuilderSchema,
} from "@/lib/utils/rule-builder";
import {
  RULE_BUILDER_CATALOGS,
  ruleBuilderCatalogMisses,
} from "@/lib/utils/rule-builder-catalog-presentation";
import { selectorId } from "@/lib/utils/dom-ids";
import { cn } from "@/lib/utils";

/// Values for the builder's pickers, by the option source a schema names.
/// Null while a catalog is still loading.
export type RuleBuilderOptions = Readonly<
  Record<string, readonly RuleBuilderOptionValue[] | null>
>;

type Translate = ReturnType<typeof useTranslate>;

function outputLabel(schema: RuleBuilderSchema, id: string, t: Translate) {
  const output = schema.outputs.find((entry) => entry.id === id);
  return output ? ruleBuilderOutputLabel(t, output) : id;
}

/// Options for a picker, keeping a stored value that the catalog no longer
/// lists so the operator still sees what the rule holds.
function pickerOptions(
  names: readonly RuleBuilderOptionValue[],
  kept: readonly string[],
) {
  const all = names.map((name) =>
    typeof name === "string" ? { value: name, label: name } : name,
  );
  for (const value of kept)
    if (value && !all.some((option) => option.value === value))
      all.push({ value, label: value });
  return all;
}

/// The label a picker shows for each selected literal.
function pickerLabels(
  names: readonly RuleBuilderOptionValue[],
  selected: readonly string[],
): string {
  const options = pickerOptions(names, selected);
  return selected
    .map((value) => options.find((option) => option.value === value)?.label)
    .join(", ");
}

function ProblemText({
  id,
  schema,
  problems,
}: {
  id: string;
  schema: RuleBuilderSchema;
  problems: RuleBuilderProblem[];
}) {
  const t = useTranslate();
  if (problems.length === 0) return null;
  const messages = problems.map((problem) =>
    t(RULE_BUILDER_PROBLEM_KEYS[problem.code], {
      outputs: (problem.outputs ?? [])
        .map((other) => outputLabel(schema, other, t))
        .join(", "),
    }),
  );
  return (
    <p id={id} className="text-xs text-[var(--scry-warning-text)]">
      {[...new Set(messages)].join(" ")}
    </p>
  );
}

/// One condition. `idBase` and `keys` name its elements: the rule's shared
/// conditions are `<prefix>-condition-*` by index, an action row's own are
/// `<prefix>-action-condition-*` by row and index.
function ConditionRow({
  schema,
  fields,
  idBase,
  keys,
  condition,
  problems,
  readOnly,
  onChange,
  onRemove,
}: {
  schema: RuleBuilderSchema;
  fields: readonly RuleBuilderField[];
  idBase: string;
  keys: readonly (string | number)[];
  condition: RuleBuilderCondition;
  problems: RuleBuilderProblem[];
  readOnly: boolean;
  onChange: (condition: RuleBuilderCondition) => void;
  onRemove: () => void;
}) {
  const t = useTranslate();
  const field = fields.find((candidate) => candidate.path === condition.field);
  const operators = field ? ruleBuilderOperators(schema, field.kind) : [];
  const operator = operators.find(
    (candidate) => candidate.id === condition.operator,
  );
  const valueKind = operator?.value ?? "none";
  const valueId = selectorId(`${idBase}-value`, ...keys);

  return (
    <div
      id={selectorId(idBase, ...keys)}
      className="space-y-1 rounded border border-border p-2"
    >
      <div className="grid gap-2 md:grid-cols-[minmax(0,1.2fr)_minmax(0,1fr)_minmax(0,1fr)_auto] md:items-end">
        <SingleSelectField
          id={selectorId(`${idBase}-fact`, ...keys)}
          label={t("settings.ruleBuilderFact")}
          placeholder={t("settings.ruleBuilderFactPlaceholder")}
          valueKind="code"
          value={condition.field}
          disabled={readOnly}
          onValueChange={(path) => {
            const next = fields.find((candidate) => candidate.path === path);
            onChange({
              field: path,
              operator: next
                ? (ruleBuilderOperators(schema, next.kind)[0]?.id ?? "")
                : "",
              value: "",
            });
          }}
          options={[
            ...fields.map((candidate) => ({
              value: candidate.path,
              label: candidate.label,
            })),
            // A field this row cannot read stays visible, flagged below,
            // rather than vanishing from the condition.
            ...(condition.field && !field
              ? [{ value: condition.field, label: condition.field.replace(/^input\./, "") }]
              : []),
          ]}
        />
        <SingleSelectField
          id={selectorId(`${idBase}-operator`, ...keys)}
          label={t("settings.ruleBuilderOperator")}
          value={condition.operator}
          disabled={readOnly || !field}
          onValueChange={(next) => {
            const nextOperator = operators.find(
              (candidate) => candidate.id === next,
            );
            onChange({
              ...condition,
              operator: next,
              value: nextOperator?.value === valueKind ? condition.value : "",
            });
          }}
          options={operators.map((candidate) => ({
            value: candidate.id,
            label: t(candidate.labelKey),
          }))}
        />
        {valueKind === "choice" ? (
          <SingleSelectField
            id={valueId}
            label={t("settings.ruleBuilderValue")}
            value={condition.value}
            disabled={readOnly}
            onValueChange={(value) => onChange({ ...condition, value })}
            options={(field?.choices ?? []).map((choice) => ({
              value: choice.value,
              label: t(choice.labelKey),
            }))}
          />
        ) : valueKind !== "none" ? (
          <label>
            <Label className="mb-2 block">
              {valueKind === "days"
                ? t("settings.ruleBuilderDays")
                : t("settings.ruleBuilderValue")}
            </Label>
            <Input
              id={valueId}
              value={condition.value}
              readOnly={readOnly}
              inputMode={valueKind === "text" ? undefined : "decimal"}
              onChange={(event) =>
                onChange({ ...condition, value: event.target.value })
              }
            />
          </label>
        ) : (
          <div />
        )}
        <IconButton
          id={selectorId(`${idBase}-remove`, ...keys)}
          label={t("label.remove")}
          tone="delete"
          disabled={readOnly}
          onClick={onRemove}
        >
          <Trash2 className="h-4 w-4" />
        </IconButton>
      </div>
      {field ? (
        <p className="text-xs text-muted-foreground">{t(field.descKey)}</p>
      ) : null}
      <ProblemText
        id={selectorId(`${idBase}-problem`, ...keys)}
        schema={schema}
        problems={problems}
      />
    </div>
  );
}

function Loading() {
  const t = useTranslate();
  return <p className="text-xs text-muted-foreground">{t("label.loading")}</p>;
}

function OutputValuePicker({
  schema,
  output,
  value,
  options,
  readOnly,
  onChange,
}: {
  schema: RuleBuilderSchema;
  output: RuleBuilderOutput;
  value: RuleBuilderOutputValue;
  options: RuleBuilderOptions;
  readOnly: boolean;
  onChange: (value: RuleBuilderOutputValue) => void;
}) {
  const t = useTranslate();
  const id = selectorId(`${schema.idPrefix}-action-value`, output.id);
  const spec = output.value;
  const catalog =
    spec.kind === "choice" || spec.kind === "text_list"
      ? spec.options
      : undefined;
  // A catalog the page cannot supply is typed in, as is one this build does
  // not know how to present.
  const presentation =
    catalog && Object.hasOwn(options, catalog)
      ? RULE_BUILDER_CATALOGS[catalog]
      : undefined;
  const names = catalog && presentation ? (options[catalog] ?? null) : null;
  const placeholder = presentation
    ? t(presentation.placeholderKey)
    : ruleBuilderOutputPlaceholder(t, output);
  const stored =
    typeof value === "string" ? [value] : Array.isArray(value) ? value : [];
  const misses = presentation ? ruleBuilderCatalogMisses(names, stored) : [];
  const missing =
    misses.length > 0 ? (
      <p
        id={selectorId(`${schema.idPrefix}-action-catalog-missing`, output.id)}
        data-problem-code="BUILDER_VALUE_NOT_IN_CATALOG"
        className="mt-1 text-xs text-[var(--scry-warning-text)]"
      >
        {t(RULE_BUILDER_PROBLEM_KEYS.BUILDER_VALUE_NOT_IN_CATALOG, {
          values: misses.join(", "),
        })}
      </p>
    ) : null;

  switch (spec.kind) {
    case "flag":
      return null;
    case "text":
    case "number":
      return (
        <Input
          id={id}
          value={typeof value === "string" ? value : ""}
          readOnly={readOnly}
          placeholder={placeholder}
          inputMode={spec.kind === "number" ? "numeric" : undefined}
          onChange={(event) => onChange(event.target.value)}
        />
      );
    case "choice": {
      const text = typeof value === "string" ? value : "";
      if (!presentation) {
        return (
          <Input
            id={id}
            value={text}
            readOnly={readOnly}
            placeholder={placeholder}
            onChange={(event) => onChange(event.target.value)}
          />
        );
      }
      if (names === null) return <Loading />;
      return (
        <>
          <SingleSelectField
            id={id}
            value={text}
            disabled={readOnly}
            valueKind={presentation.display ?? "text"}
            placeholder={placeholder}
            onValueChange={onChange}
            options={pickerOptions(names, [text])}
          />
          {missing}
        </>
      );
    }
    case "text_list": {
      const selected = Array.isArray(value) ? value : [];
      if (!presentation) {
        // One entry per line; the text is split exactly as typed so it
        // round-trips while the operator is still typing.
        return (
          <Textarea
            id={id}
            value={selected.join("\n")}
            readOnly={readOnly}
            placeholder={placeholder}
            onChange={(event) =>
              onChange(
                event.target.value === "" ? [] : event.target.value.split("\n"),
              )
            }
          />
        );
      }
      if (names === null) return <Loading />;
      if (names.length === 0 && selected.length === 0) {
        return (
          <p id={id} className="text-xs text-muted-foreground">
            {presentation.emptyKey ? t(presentation.emptyKey) : null}
          </p>
        );
      }
      return (
        <>
          <MultiSelectDropdown
            id={id}
            optionIdPrefix={id}
            disabled={readOnly}
            triggerLabel={
              selected.length > 0 ? pickerLabels(names, selected) : placeholder
            }
            ariaLabel={placeholder}
            options={pickerOptions(names, selected)}
            selectedValues={selected}
            onSelectedValuesChange={onChange}
          />
          {missing}
        </>
      );
    }
    case "structured": {
      const parts =
        typeof value === "object" && !Array.isArray(value) ? value : {};
      return (
        <div className="grid gap-2 md:grid-cols-2">
          {spec.parts.map((part) => (
            <label key={part.id}>
              <Label className="mb-2 block">{t(part.labelKey)}</Label>
              <Input
                id={selectorId(
                  `${schema.idPrefix}-action-value`,
                  output.id,
                  part.id,
                )}
                value={parts[part.id] ?? ""}
                readOnly={readOnly}
                placeholder={
                  part.placeholderKey ? t(part.placeholderKey) : undefined
                }
                inputMode={part.kind === "number" ? "numeric" : undefined}
                onChange={(event) =>
                  onChange({ ...parts, [part.id]: event.target.value })
                }
              />
            </label>
          ))}
        </div>
      );
    }
  }
}

function OutputRow({
  schema,
  variant,
  output,
  outputs,
  options,
  problems,
  readOnly,
  onChange,
}: {
  schema: RuleBuilderSchema;
  variant: string | null;
  output: RuleBuilderOutput;
  outputs: RuleBuilderModel["outputs"];
  options: RuleBuilderOptions;
  problems: RuleBuilderProblem[];
  readOnly: boolean;
  onChange: (outputs: RuleBuilderModel["outputs"]) => void;
}) {
  const t = useTranslate();
  const prefix = schema.idPrefix;
  const chosen = Object.hasOwn(outputs, output.id);
  const inVariant = isRuleBuilderOutputInVariant(output, variant);
  const conflicts = ruleBuilderConflicts(schema, output.id, outputs);
  const label = ruleBuilderOutputLabel(t, output);
  const help = ruleBuilderOutputHelp(t, output);
  // A chosen output can always be cleared, even once it no longer fits.
  const blocked = !chosen && (!inVariant || conflicts.length > 0);

  const toggle = (checked: boolean) => {
    const next = { ...outputs };
    if (!checked) delete next[output.id];
    else next[output.id] = initialRuleBuilderOutputValue(output);
    onChange(next);
  };

  return (
    <div className="space-y-1">
      <label
        className={cn(
          "flex items-center gap-2",
          blocked && "text-muted-foreground",
        )}
      >
        <Checkbox
          id={selectorId(`${prefix}-action`, output.id)}
          checked={chosen}
          disabled={readOnly || blocked}
          onCheckedChange={(value) => toggle(value === true)}
        />
        <span className="text-sm">
          {help ? (
            <HelpLabel label={label} help={help} />
          ) : (
            label
          )}
        </span>
      </label>
      {!inVariant ? (
        <p
          id={selectorId(`${prefix}-action-scope`, output.id)}
          className="pl-6 text-xs text-muted-foreground"
        >
          {schema.labels.outputNotInVariant
            ? t(schema.labels.outputNotInVariant)
            : null}
        </p>
      ) : conflicts.length > 0 ? (
        <p
          id={selectorId(`${prefix}-action-conflict`, output.id)}
          className="pl-6 text-xs text-muted-foreground"
        >
          {t("settings.ruleBuilderOutputConflict", {
            outputs: conflicts
              .map((other) => outputLabel(schema, other, t))
              .join(", "),
          })}
        </p>
      ) : null}
      {chosen && output.value.kind !== "flag" ? (
        <div
          className={cn(
            "pl-6",
            output.value.kind !== "structured" && "md:max-w-[50%]",
          )}
        >
          <OutputValuePicker
            schema={schema}
            output={output}
            value={outputs[output.id]}
            options={options}
            readOnly={readOnly}
            onChange={(value) => onChange({ ...outputs, [output.id]: value })}
          />
        </div>
      ) : null}
      <div className="pl-6">
        <ProblemText
          id={selectorId(`${prefix}-action-problem`, output.id)}
          schema={schema}
          problems={problems}
        />
      </div>
    </div>
  );
}

/// A list of conditions with its join and its add button.
function ConditionList({
  schema,
  fields,
  idBase,
  keys,
  joinId,
  join,
  conditions,
  problems,
  readOnly,
  label,
  onChange,
}: {
  schema: RuleBuilderSchema;
  fields: readonly RuleBuilderField[];
  idBase: string;
  keys: readonly (string | number)[];
  joinId: string;
  join: RuleBuilderJoin;
  conditions: RuleBuilderCondition[];
  problems: RuleBuilderProblem[];
  readOnly: boolean;
  label: React.ReactNode;
  onChange: (change: {
    join?: RuleBuilderJoin;
    conditions?: RuleBuilderCondition[];
  }) => void;
}) {
  const t = useTranslate();
  return (
    <div className="space-y-2">
      <div className="flex flex-wrap items-end justify-between gap-2">
        <Label className="block">{label}</Label>
        <SingleSelectField
          id={joinId}
          size="sm"
          className="w-56"
          value={join}
          disabled={readOnly}
          onValueChange={(next) => onChange({ join: next as RuleBuilderJoin })}
          options={[
            { value: "ALL", label: t("settings.ruleBuilderJoinAll") },
            { value: "ANY", label: t("settings.ruleBuilderJoinAny") },
          ]}
        />
      </div>
      {conditions.map((condition, index) => (
        <ConditionRow
          key={index}
          schema={schema}
          fields={fields}
          idBase={idBase}
          keys={[...keys, index]}
          condition={condition}
          readOnly={readOnly}
          problems={problems.filter((problem) => problem.conditionIndex === index)}
          onChange={(next) =>
            onChange({
              conditions: conditions.map((existing, at) =>
                at === index ? next : existing,
              ),
            })
          }
          onRemove={() =>
            onChange({ conditions: conditions.filter((_, at) => at !== index) })
          }
        />
      ))}
      <Button
        id={selectorId(`${idBase}-add`, ...keys)}
        type="button"
        size="sm"
        variant="secondary"
        disabled={readOnly}
        onClick={() =>
          onChange({
            conditions: [...conditions, { field: "", operator: "", value: "" }],
          })
        }
      >
        <Plus className="mr-1 h-4 w-4" />
        {t("settings.ruleBuilderAddCondition")}
      </Button>
    </div>
  );
}

/// One action of a family with subjects: what it acts on, the action, its
/// value, and the conditions that apply to it alone.
function ActionRow({
  schema,
  variant,
  model,
  row,
  index,
  options,
  problems,
  readOnly,
  onChange,
}: {
  schema: RuleBuilderSchema;
  variant: string | null;
  model: RuleBuilderModel;
  row: RuleBuilderActionRow;
  index: number;
  options: RuleBuilderOptions;
  problems: RuleBuilderProblem[];
  readOnly: boolean;
  onChange: (model: RuleBuilderModel) => void;
}) {
  const t = useTranslate();
  const prefix = schema.idPrefix;
  const subject = ruleBuilderRowSubject(schema, row);
  const rows = ruleBuilderActionRows(schema, model);
  const taken = new Set(
    rows.filter((_, at) => at !== index).map((other) => other.output),
  );
  const offered = schema.outputs.filter(
    (output) =>
      subject?.outputs.includes(output.id) &&
      (!taken.has(output.id) || output.id === row.output),
  );
  const output = schema.outputs.find((entry) => entry.id === row.output);
  const help = output ? ruleBuilderOutputHelp(t, output) : null;
  const rowProblems = problems.filter(
    (problem) => problem.conditionIndex === undefined,
  );
  return (
    <div
      id={selectorId(`${prefix}-action-row`, index)}
      className="space-y-2 rounded border border-border p-2"
    >
      <div className="grid gap-2 md:grid-cols-[minmax(0,1fr)_minmax(0,1.4fr)_auto] md:items-end">
        <SingleSelectField
          id={selectorId(`${prefix}-action-subject`, index)}
          label={t("settings.ruleBuilderActionSubject")}
          value={subject?.id ?? row.subject ?? ""}
          disabled={readOnly}
          onValueChange={(next) =>
            onChange(setRuleBuilderActionSubject(schema, model, index, next))
          }
          options={(schema.subjects ?? []).map((entry) => ({
            value: entry.id,
            label: translatedOr(t, entry.labelKey, entry.id),
          }))}
        />
        <SingleSelectField
          id={selectorId(`${prefix}-action-output`, index)}
          label={t("settings.ruleBuilderAction")}
          placeholder={t("settings.ruleBuilderActionPlaceholder")}
          value={row.output}
          disabled={readOnly || !subject}
          onValueChange={(next) =>
            onChange(setRuleBuilderActionOutput(schema, model, index, next))
          }
          options={offered.map((entry) => ({
            value: entry.id,
            label: ruleBuilderActionLabel(t, entry),
          }))}
        />
        <IconButton
          id={selectorId(`${prefix}-action-remove`, index)}
          label={t("label.remove")}
          tone="delete"
          disabled={readOnly}
          onClick={() => onChange(removeRuleBuilderAction(schema, model, index))}
        >
          <Trash2 className="h-4 w-4" />
        </IconButton>
      </div>
      {help ? <p className="text-xs text-muted-foreground">{help}</p> : null}
      {output && output.value.kind !== "flag" ? (
        <div className={cn(output.value.kind !== "structured" && "md:max-w-[50%]")}>
          <OutputValuePicker
            schema={schema}
            output={output}
            value={model.outputs[output.id] ?? initialRuleBuilderOutputValue(output)}
            options={options}
            readOnly={readOnly}
            onChange={(value) =>
              onChange({ ...model, outputs: { ...model.outputs, [output.id]: value } })
            }
          />
        </div>
      ) : null}
      <ConditionList
        schema={schema}
        fields={ruleBuilderSubjectFields(schema, variant, subject)}
        idBase={`${prefix}-action-condition`}
        keys={[index]}
        joinId={selectorId(`${prefix}-action-join`, index)}
        join={row.join}
        conditions={row.conditions}
        problems={problems.filter((problem) => problem.conditionIndex !== undefined)}
        readOnly={readOnly}
        label={t("settings.ruleBuilderActionConditions")}
        onChange={(change) =>
          onChange(setRuleBuilderActionConditions(schema, model, index, change))
        }
      />
      <ProblemText
        id={selectorId(`${prefix}-action-row-problem`, index)}
        schema={schema}
        problems={rowProblems}
      />
    </div>
  );
}

/// Conditions, then outputs, written into Rego by the generator. Everything
/// here edits the model; the source the rule saves is always generated. What
/// the builder offers comes entirely from the schema.
export function RuleBuilder({
  schema,
  model,
  variant,
  problems,
  options,
  readOnly = false,
  onChange,
  generatedSource,
  feedback,
}: {
  schema: RuleBuilderSchema;
  model: RuleBuilderModel;
  variant: string | null;
  problems: RuleBuilderProblem[];
  options: RuleBuilderOptions;
  readOnly?: boolean;
  onChange: (model: RuleBuilderModel) => void;
  /// What the rule will save; empty while the model cannot be written.
  generatedSource: string;
  /// Validation status of the generated source.
  feedback: React.ReactNode;
}) {
  const t = useTranslate();
  const prefix = schema.idPrefix;
  const [sourceOpen, setSourceOpen] = React.useState(false);
  const hasSubjects = (schema.subjects?.length ?? 0) > 0;
  const general = problems.filter(
    (problem) =>
      problem.conditionIndex === undefined &&
      problem.output === undefined &&
      problem.actionIndex === undefined,
  );
  const sharedProblems = problems.filter(
    (problem) => problem.actionIndex === undefined,
  );

  return (
    <div id={prefix} className="space-y-4">
      <ConditionList
        schema={schema}
        fields={ruleBuilderFields(schema, variant)}
        idBase={`${prefix}-condition`}
        keys={[]}
        joinId={`${prefix}-join`}
        join={model.join}
        conditions={model.conditions}
        problems={sharedProblems}
        readOnly={readOnly}
        label={
          <HelpLabel
            label={t("settings.ruleBuilderConditions")}
            help={t(schema.labels.conditionsHelp)}
          />
        }
        onChange={(change) => onChange({ ...model, ...change })}
      />

      <div className="space-y-2">
        <Label className="block">
          <HelpLabel
            label={t(schema.labels.outputs)}
            help={t(schema.labels.outputsHelp)}
          />
        </Label>
        {hasSubjects ? (
          <div className="space-y-2">
            {ruleBuilderActionRows(schema, model).map((row, index) => (
              <ActionRow
                key={index}
                schema={schema}
                variant={variant}
                model={model}
                row={row}
                index={index}
                options={options}
                readOnly={readOnly}
                problems={problems.filter(
                  (problem) => problem.actionIndex === index,
                )}
                onChange={onChange}
              />
            ))}
            <Button
              id={`${prefix}-action-add`}
              type="button"
              size="sm"
              variant="secondary"
              disabled={readOnly}
              onClick={() => onChange(addRuleBuilderAction(schema, model))}
            >
              <Plus className="mr-1 h-4 w-4" />
              {t("settings.ruleBuilderAddAction")}
            </Button>
          </div>
        ) : (
        <div className="grid gap-2 md:grid-cols-2">
          {schema.outputs.map((output) => (
            <OutputRow
              key={output.id}
              schema={schema}
              variant={variant}
              output={output}
              outputs={model.outputs}
              options={options}
              readOnly={readOnly}
              problems={problems.filter(
                (problem) => problem.output === output.id,
              )}
              onChange={(outputs) => onChange({ ...model, outputs })}
            />
          ))}
        </div>
        )}
      </div>

      <ProblemText
        id={`${prefix}-problems`}
        schema={schema}
        problems={general}
      />

      <Collapsible open={sourceOpen} onOpenChange={setSourceOpen}>
        <CollapsibleTrigger asChild>
          <button
            id={`${prefix}-source-toggle`}
            type="button"
            className="flex items-center gap-1.5 rounded-[8px] py-1 text-sm font-medium text-[var(--scry-muted)] transition-colors hover:text-[var(--scry-ink2)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          >
            <ChevronRight
              className={cn(
                "h-4 w-4 transition-transform",
                sourceOpen && "rotate-90",
              )}
            />
            <HelpLabel
              label={t("settings.ruleBuilderGeneratedSource")}
              help={t("settings.ruleBuilderGeneratedSourceHelp")}
            />
          </button>
        </CollapsibleTrigger>
        <CollapsibleContent className="pt-2">
          {generatedSource ? (
            <pre
              id={`${prefix}-source`}
              data-code-font
              className="max-h-96 overflow-auto rounded bg-muted p-2 text-xs"
            >
              {generatedSource}
            </pre>
          ) : (
            <p
              id={`${prefix}-source-empty`}
              className="text-xs text-muted-foreground"
            >
              {t("settings.ruleBuilderGeneratedSourceEmpty")}
            </p>
          )}
        </CollapsibleContent>
      </Collapsible>
      {feedback}
    </div>
  );
}
