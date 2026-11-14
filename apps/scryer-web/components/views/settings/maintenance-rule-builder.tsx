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
import { useTranslate } from "@/lib/context/translate-context";
import type {
  MaintenanceActionHead,
  MaintenanceRuleScope,
} from "@/lib/types/maintenance-rule-sets";
import {
  MAINTENANCE_BUILDER_LIST_HEADS,
  MAINTENANCE_BUILDER_OPERATOR_KEYS,
  MAINTENANCE_BUILDER_PROBLEM_KEYS,
  MAINTENANCE_BUILDER_TEXT_HEADS,
  isMaintenanceBuilderHeadInScope,
  maintenanceBuilderConflicts,
  maintenanceBuilderFacts,
  maintenanceBuilderOperators,
  maintenanceBuilderValueKind,
  type MaintenanceBuilderActions,
  type MaintenanceBuilderCondition,
  type MaintenanceBuilderListHead,
  type MaintenanceBuilderModel,
  type MaintenanceBuilderProblem,
  type MaintenanceBuilderTextHead,
} from "@/lib/utils/maintenance-rule-builder";
import {
  MAINTENANCE_ACTION_HEAD_ORDER,
  actionHeadLabelKey,
  isDestructiveActionHead,
} from "@/lib/utils/maintenance-rule-sets";
import { selectorId } from "@/lib/utils/dom-ids";
import { cn } from "@/lib/utils";

/// Values the builder's pickers offer. Null while a catalog has not loaded.
export type MaintenanceBuilderOptions = {
  qualityProfileNames: string[] | null;
  tagLabels: string[] | null;
  libraryNames: string[];
  rootPaths: string[];
};

function isListHead(head: MaintenanceActionHead): head is MaintenanceBuilderListHead {
  return (MAINTENANCE_BUILDER_LIST_HEADS as readonly string[]).includes(head);
}

function isTextHead(head: MaintenanceActionHead): head is MaintenanceBuilderTextHead {
  return (MAINTENANCE_BUILDER_TEXT_HEADS as readonly string[]).includes(head);
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
  problems,
}: {
  id: string;
  problems: MaintenanceBuilderProblem[];
}) {
  const t = useTranslate();
  if (problems.length === 0) return null;
  return (
    <p id={id} className="text-xs text-[var(--scry-warning-text)]">
      {[...new Set(problems.map((problem) => problem.code))]
        .map((code) => t(MAINTENANCE_BUILDER_PROBLEM_KEYS[code]))
        .join(" ")}
    </p>
  );
}

function ConditionRow({
  index,
  condition,
  scope,
  problems,
  readOnly,
  onChange,
  onRemove,
}: {
  index: number;
  condition: MaintenanceBuilderCondition;
  scope: MaintenanceRuleScope;
  problems: MaintenanceBuilderProblem[];
  readOnly: boolean;
  onChange: (condition: MaintenanceBuilderCondition) => void;
  onRemove: () => void;
}) {
  const t = useTranslate();
  const facts = maintenanceBuilderFacts(scope);
  const fact = facts.find((candidate) => candidate.name === condition.fact);
  const operators = fact ? maintenanceBuilderOperators(fact.kind) : [];
  const valueKind =
    fact && condition.operator
      ? maintenanceBuilderValueKind(fact.kind, condition.operator)
      : null;

  return (
    <div
      id={selectorId("settings-maintenance-builder-condition", index)}
      className="space-y-1 rounded border border-border p-2"
    >
      <div className="grid gap-2 md:grid-cols-[minmax(0,1.2fr)_minmax(0,1fr)_minmax(0,1fr)_auto] md:items-end">
        <SingleSelectField
          id={selectorId("settings-maintenance-builder-condition-fact", index)}
          label={t("settings.maintenanceBuilderFact")}
          placeholder={t("settings.maintenanceBuilderFactPlaceholder")}
          valueKind="code"
          value={condition.fact}
          disabled={readOnly}
          onValueChange={(name) => {
            const next = facts.find((candidate) => candidate.name === name);
            onChange({
              fact: name,
              operator: next ? maintenanceBuilderOperators(next.kind)[0] : "",
              value: "",
            });
          }}
          options={facts.map((candidate) => ({
            value: candidate.name,
            label: candidate.name,
          }))}
        />
        <SingleSelectField
          id={selectorId(
            "settings-maintenance-builder-condition-operator",
            index,
          )}
          label={t("settings.maintenanceBuilderOperator")}
          value={condition.operator}
          disabled={readOnly || !fact}
          onValueChange={(operator) => {
            const nextOperator = operator as MaintenanceBuilderCondition["operator"];
            const keepValue =
              fact &&
              nextOperator &&
              maintenanceBuilderValueKind(fact.kind, nextOperator) ===
                valueKind;
            onChange({
              ...condition,
              operator: nextOperator,
              value: keepValue ? condition.value : "",
            });
          }}
          options={operators.map((operator) => ({
            value: operator,
            label: t(MAINTENANCE_BUILDER_OPERATOR_KEYS[operator]),
          }))}
        />
        {valueKind ? (
          <label>
            <Label className="mb-2 block">
              {valueKind === "days"
                ? t("settings.maintenanceBuilderDays")
                : t("settings.maintenanceBuilderValue")}
            </Label>
            <Input
              id={selectorId(
                "settings-maintenance-builder-condition-value",
                index,
              )}
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
          id={selectorId("settings-maintenance-builder-condition-remove", index)}
          label={t("label.remove")}
          tone="delete"
          disabled={readOnly}
          onClick={onRemove}
        >
          <Trash2 className="h-4 w-4" />
        </IconButton>
      </div>
      {fact ? (
        <p className="text-xs text-muted-foreground">{t(fact.descKey)}</p>
      ) : null}
      <ProblemText
        id={selectorId("settings-maintenance-builder-condition-problem", index)}
        problems={problems}
      />
    </div>
  );
}

function ActionValuePicker({
  head,
  actions,
  options,
  readOnly,
  onChange,
}: {
  head: MaintenanceActionHead;
  actions: MaintenanceBuilderActions;
  options: MaintenanceBuilderOptions;
  readOnly: boolean;
  onChange: (actions: MaintenanceBuilderActions) => void;
}) {
  const t = useTranslate();
  const id = selectorId("settings-maintenance-builder-action-value", head);

  if (isListHead(head)) {
    const selected = actions[head] ?? [];
    if (options.tagLabels === null) {
      return (
        <p className="text-xs text-muted-foreground">{t("label.loading")}</p>
      );
    }
    if (options.tagLabels.length === 0 && selected.length === 0) {
      return (
        <p id={id} className="text-xs text-muted-foreground">
          {t("settings.maintenanceBuilderNoTags")}
        </p>
      );
    }
    return (
      <MultiSelectDropdown
        id={id}
        optionIdPrefix={id}
        disabled={readOnly}
        triggerLabel={
          selected.length > 0
            ? selected.join(", ")
            : t("settings.maintenanceBuilderTagsPlaceholder")
        }
        ariaLabel={t("settings.maintenanceBuilderTagsPlaceholder")}
        options={pickerOptions(options.tagLabels, selected)}
        selectedValues={selected}
        onSelectedValuesChange={(values) =>
          onChange({ ...actions, [head]: values })
        }
      />
    );
  }

  if (!isTextHead(head)) return null;
  const value = actions[head] ?? "";
  if (head === "NOTIFY") {
    return (
      <Input
        id={id}
        value={value}
        readOnly={readOnly}
        placeholder={t("settings.maintenanceBuilderNotifyPlaceholder")}
        onChange={(event) =>
          onChange({ ...actions, NOTIFY: event.target.value })
        }
      />
    );
  }

  const names =
    head === "SET_QUALITY_PROFILE"
      ? options.qualityProfileNames
      : head === "MOVE_TO_LIBRARY"
        ? options.libraryNames
        : options.rootPaths;
  if (names === null) {
    return <p className="text-xs text-muted-foreground">{t("label.loading")}</p>;
  }
  return (
    <SingleSelectField
      id={id}
      value={value}
      disabled={readOnly}
      valueKind={head === "MOVE_TO_ROOT" ? "path" : "text"}
      placeholder={t(
        head === "SET_QUALITY_PROFILE"
          ? "settings.maintenanceBuilderQualityProfilePlaceholder"
          : head === "MOVE_TO_LIBRARY"
            ? "settings.maintenanceBuilderLibraryPlaceholder"
            : "settings.maintenanceBuilderRootPlaceholder",
      )}
      onValueChange={(next) => onChange({ ...actions, [head]: next })}
      options={pickerOptions(names, [value])}
    />
  );
}

function ActionRow({
  head,
  scope,
  actions,
  options,
  problems,
  readOnly,
  onChange,
}: {
  head: MaintenanceActionHead;
  scope: MaintenanceRuleScope;
  actions: MaintenanceBuilderActions;
  options: MaintenanceBuilderOptions;
  problems: MaintenanceBuilderProblem[];
  readOnly: boolean;
  onChange: (actions: MaintenanceBuilderActions) => void;
}) {
  const t = useTranslate();
  const chosen = Object.hasOwn(actions, head);
  const inScope = isMaintenanceBuilderHeadInScope(head, scope);
  const conflicts = maintenanceBuilderConflicts(head, actions);
  // A chosen action can always be cleared, even once it no longer fits.
  const blocked = !chosen && (!inScope || conflicts.length > 0);
  const labelKey = actionHeadLabelKey(head);
  const label = labelKey ? t(labelKey) : head;
  const checkboxId = selectorId("settings-maintenance-builder-action", head);

  const toggle = (checked: boolean) => {
    const next = { ...actions } as Record<string, unknown>;
    if (!checked) delete next[head];
    else next[head] = isListHead(head) ? [] : isTextHead(head) ? "" : true;
    onChange(next as MaintenanceBuilderActions);
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
          id={checkboxId}
          checked={chosen}
          disabled={readOnly || blocked}
          onCheckedChange={(value) => toggle(value === true)}
        />
        <span className="text-sm">{label}</span>
        {isDestructiveActionHead(head) ? (
          <Badge tone="negative" className="text-[10px]">
            {t("settings.maintenanceTemplateDestructiveBadge")}
          </Badge>
        ) : null}
      </label>
      {!inScope ? (
        <p
          id={selectorId("settings-maintenance-builder-action-scope", head)}
          className="pl-6 text-xs text-muted-foreground"
        >
          {t("settings.maintenanceBuilderActionTitleOnly")}
        </p>
      ) : conflicts.length > 0 ? (
        <p
          id={selectorId("settings-maintenance-builder-action-conflict", head)}
          className="pl-6 text-xs text-muted-foreground"
        >
          {t("settings.maintenanceBuilderActionConflict", {
            actions: conflicts
              .map((other) => {
                const key = actionHeadLabelKey(other);
                return key ? t(key) : other;
              })
              .join(", "),
          })}
        </p>
      ) : null}
      {chosen && (isListHead(head) || isTextHead(head)) ? (
        <div className="pl-6 md:max-w-[50%]">
          <ActionValuePicker
            head={head}
            actions={actions}
            options={options}
            readOnly={readOnly}
            onChange={onChange}
          />
        </div>
      ) : null}
      <div className="pl-6">
        <ProblemText
          id={selectorId("settings-maintenance-builder-action-problem", head)}
          problems={problems}
        />
      </div>
    </div>
  );
}

/// Conditions, then actions, written into Rego by the generator. Everything
/// here edits the model; the source the rule saves is always generated.
export function MaintenanceRuleBuilder({
  model,
  scope,
  problems,
  options,
  readOnly = false,
  onChange,
  generatedSource,
  feedback,
}: {
  model: MaintenanceBuilderModel;
  scope: MaintenanceRuleScope;
  problems: MaintenanceBuilderProblem[];
  options: MaintenanceBuilderOptions;
  readOnly?: boolean;
  onChange: (model: MaintenanceBuilderModel) => void;
  /// What the rule will save; empty while the model cannot be written.
  generatedSource: string;
  /// Validation status of the generated source.
  feedback: React.ReactNode;
}) {
  const t = useTranslate();
  const [sourceOpen, setSourceOpen] = React.useState(false);
  const general = problems.filter(
    (problem) =>
      problem.conditionIndex === undefined && problem.head === undefined,
  );

  const setConditions = (conditions: MaintenanceBuilderCondition[]) =>
    onChange({ ...model, conditions });

  return (
    <div id="settings-maintenance-builder" className="space-y-4">
      <div className="space-y-2">
        <div className="flex flex-wrap items-end justify-between gap-2">
          <Label className="block">
            <HelpLabel
              label={t("settings.maintenanceBuilderConditions")}
              help={t("settings.maintenanceBuilderConditionsHelp")}
            />
          </Label>
          <SingleSelectField
            id="settings-maintenance-builder-join"
            size="sm"
            className="w-56"
            value={model.join}
            disabled={readOnly}
            onValueChange={(join) =>
              onChange({ ...model, join: join as MaintenanceBuilderModel["join"] })
            }
            options={[
              { value: "ALL", label: t("settings.maintenanceBuilderJoinAll") },
              { value: "ANY", label: t("settings.maintenanceBuilderJoinAny") },
            ]}
          />
        </div>
        {model.conditions.map((condition, index) => (
          <ConditionRow
            key={index}
            index={index}
            condition={condition}
            scope={scope}
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
          id="settings-maintenance-builder-condition-add"
          type="button"
          size="sm"
          variant="secondary"
          disabled={readOnly}
          onClick={() =>
            setConditions([
              ...model.conditions,
              { fact: "", operator: "", value: "" },
            ])
          }
        >
          <Plus className="mr-1 h-4 w-4" />
          {t("settings.maintenanceBuilderAddCondition")}
        </Button>
      </div>

      <div className="space-y-2">
        <Label className="block">
          <HelpLabel
            label={t("label.actions")}
            help={t("settings.maintenanceBuilderActionsHelp")}
          />
        </Label>
        <div className="grid gap-2 md:grid-cols-2">
          {MAINTENANCE_ACTION_HEAD_ORDER.map((head) => (
            <ActionRow
              key={head}
              head={head}
              scope={scope}
              actions={model.actions}
              options={options}
              readOnly={readOnly}
              problems={problems.filter((problem) => problem.head === head)}
              onChange={(actions) => onChange({ ...model, actions })}
            />
          ))}
        </div>
      </div>

      <ProblemText
        id="settings-maintenance-builder-problems"
        problems={general}
      />

      <Collapsible open={sourceOpen} onOpenChange={setSourceOpen}>
        <CollapsibleTrigger asChild>
          <button
            id="settings-maintenance-builder-source-toggle"
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
              label={t("settings.maintenanceBuilderGeneratedSource")}
              help={t("settings.maintenanceBuilderGeneratedSourceHelp")}
            />
          </button>
        </CollapsibleTrigger>
        <CollapsibleContent className="pt-2">
          {generatedSource ? (
            <pre
              id="settings-maintenance-builder-source"
              data-code-font
              className="max-h-96 overflow-auto rounded bg-muted p-2 text-xs"
            >
              {generatedSource}
            </pre>
          ) : (
            <p
              id="settings-maintenance-builder-source-empty"
              className="text-xs text-muted-foreground"
            >
              {t("settings.maintenanceBuilderGeneratedSourceEmpty")}
            </p>
          )}
        </CollapsibleContent>
      </Collapsible>
      {feedback}
    </div>
  );
}
