import * as React from "react";
import { ChevronRight, Plus, Trash2 } from "lucide-react";
import { HelpLabel } from "@/components/common/info-help";
import { Badge } from "@/components/ui/badge";
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
  initialRuleBuilderOutputValue,
  isRuleBuilderOutputInVariant,
  ruleBuilderConflicts,
  ruleBuilderFields,
  ruleBuilderOperators,
  type RuleBuilderCondition,
  type RuleBuilderModel,
  type RuleBuilderOutput,
  type RuleBuilderOutputValue,
  type RuleBuilderProblem,
  type RuleBuilderSchema,
} from "@/lib/utils/rule-builder";
import { selectorId } from "@/lib/utils/dom-ids";
import { cn } from "@/lib/utils";

/// Values for the builder's pickers, by the option source a schema names.
/// Null while a catalog is still loading.
export type RuleBuilderOptions = Readonly<
  Record<string, readonly string[] | null>
>;

type Translate = ReturnType<typeof useTranslate>;

function outputLabel(schema: RuleBuilderSchema, id: string, t: Translate) {
  const output = schema.outputs.find((entry) => entry.id === id);
  return output ? t(output.labelKey) : id;
}

/// Options for a picker, keeping a stored value that the catalog no longer
/// lists so the operator still sees what the rule holds.
function pickerOptions(names: readonly string[], kept: readonly string[]) {
  const all = [...names];
  for (const value of kept) if (value && !all.includes(value)) all.push(value);
  return all.map((name) => ({ value: name, label: name }));
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

function ConditionRow({
  schema,
  variant,
  index,
  condition,
  problems,
  readOnly,
  onChange,
  onRemove,
}: {
  schema: RuleBuilderSchema;
  variant: string | null;
  index: number;
  condition: RuleBuilderCondition;
  problems: RuleBuilderProblem[];
  readOnly: boolean;
  onChange: (condition: RuleBuilderCondition) => void;
  onRemove: () => void;
}) {
  const t = useTranslate();
  const prefix = schema.idPrefix;
  const fields = ruleBuilderFields(schema, variant);
  const field = fields.find((candidate) => candidate.path === condition.field);
  const operators = field ? ruleBuilderOperators(schema, field.kind) : [];
  const operator = operators.find(
    (candidate) => candidate.id === condition.operator,
  );
  const valueKind = operator?.value ?? "none";
  const valueId = selectorId(`${prefix}-condition-value`, index);

  return (
    <div
      id={selectorId(`${prefix}-condition`, index)}
      className="space-y-1 rounded border border-border p-2"
    >
      <div className="grid gap-2 md:grid-cols-[minmax(0,1.2fr)_minmax(0,1fr)_minmax(0,1fr)_auto] md:items-end">
        <SingleSelectField
          id={selectorId(`${prefix}-condition-fact`, index)}
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
          options={fields.map((candidate) => ({
            value: candidate.path,
            label: candidate.label,
          }))}
        />
        <SingleSelectField
          id={selectorId(`${prefix}-condition-operator`, index)}
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
          id={selectorId(`${prefix}-condition-remove`, index)}
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
        id={selectorId(`${prefix}-condition-problem`, index)}
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
  const placeholder =
    "placeholderKey" in spec && spec.placeholderKey
      ? t(spec.placeholderKey)
      : undefined;

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
      const names = options[spec.options] ?? null;
      if (names === null) return <Loading />;
      const text = typeof value === "string" ? value : "";
      return (
        <SingleSelectField
          id={id}
          value={text}
          disabled={readOnly}
          valueKind={spec.display ?? "text"}
          placeholder={placeholder}
          onValueChange={onChange}
          options={pickerOptions(names, [text])}
        />
      );
    }
    case "text_list": {
      const selected = Array.isArray(value) ? value : [];
      if (!spec.options) {
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
      const names = options[spec.options] ?? null;
      if (names === null) return <Loading />;
      if (names.length === 0 && selected.length === 0) {
        return (
          <p id={id} className="text-xs text-muted-foreground">
            {spec.emptyKey ? t(spec.emptyKey) : null}
          </p>
        );
      }
      return (
        <MultiSelectDropdown
          id={id}
          optionIdPrefix={id}
          disabled={readOnly}
          triggerLabel={
            selected.length > 0 ? selected.join(", ") : (placeholder ?? "")
          }
          ariaLabel={placeholder ?? t(output.labelKey)}
          options={pickerOptions(names, selected)}
          selectedValues={selected}
          onSelectedValuesChange={onChange}
        />
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
        <span className="text-sm">{t(output.labelKey)}</span>
        {output.destructive && schema.labels.destructive ? (
          <Badge tone="negative" className="text-[10px]">
            {t(schema.labels.destructive)}
          </Badge>
        ) : null}
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
  const general = problems.filter(
    (problem) =>
      problem.conditionIndex === undefined && problem.output === undefined,
  );

  const setConditions = (conditions: RuleBuilderCondition[]) =>
    onChange({ ...model, conditions });

  return (
    <div id={prefix} className="space-y-4">
      <div className="space-y-2">
        <div className="flex flex-wrap items-end justify-between gap-2">
          <Label className="block">
            <HelpLabel
              label={t("settings.ruleBuilderConditions")}
              help={t(schema.labels.conditionsHelp)}
            />
          </Label>
          <SingleSelectField
            id={`${prefix}-join`}
            size="sm"
            className="w-56"
            value={model.join}
            disabled={readOnly}
            onValueChange={(join) =>
              onChange({ ...model, join: join as RuleBuilderModel["join"] })
            }
            options={[
              { value: "ALL", label: t("settings.ruleBuilderJoinAll") },
              { value: "ANY", label: t("settings.ruleBuilderJoinAny") },
            ]}
          />
        </div>
        {model.conditions.map((condition, index) => (
          <ConditionRow
            key={index}
            schema={schema}
            variant={variant}
            index={index}
            condition={condition}
            readOnly={readOnly}
            problems={problems.filter(
              (problem) => problem.conditionIndex === index,
            )}
            onChange={(next) =>
              setConditions(
                model.conditions.map((existing, at) =>
                  at === index ? next : existing,
                ),
              )
            }
            onRemove={() =>
              setConditions(model.conditions.filter((_, at) => at !== index))
            }
          />
        ))}
        <Button
          id={`${prefix}-condition-add`}
          type="button"
          size="sm"
          variant="secondary"
          disabled={readOnly}
          onClick={() =>
            setConditions([
              ...model.conditions,
              { field: "", operator: "", value: "" },
            ])
          }
        >
          <Plus className="mr-1 h-4 w-4" />
          {t("settings.ruleBuilderAddCondition")}
        </Button>
      </div>

      <div className="space-y-2">
        <Label className="block">
          <HelpLabel
            label={t(schema.labels.outputs)}
            help={t(schema.labels.outputsHelp)}
          />
        </Label>
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
