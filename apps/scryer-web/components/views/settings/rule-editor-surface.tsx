import * as React from "react";
import * as ToggleGroupPrimitive from "@radix-ui/react-toggle-group";
import { InfoHelp } from "@/components/common/info-help";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import {
  RuleBuilder,
  type RuleBuilderOptions,
} from "@/components/views/settings/rule-builder";
import { useTranslate } from "@/lib/context/translate-context";
import {
  generateRuleSource,
  type RuleBuilderModel,
  type RuleBuilderSchema,
} from "@/lib/utils/rule-builder";
import type { RuleEditorMode } from "@/lib/utils/rule-builder-mode";

/// A rule's source editor with its Simple/Advanced switch, the same for every
/// rule family: the builder while the draft is still the builder's and the
/// operator chose Simple, the Rego editor otherwise.
export function RuleEditorSurface({
  idPrefix,
  schema,
  variant,
  mode,
  onModeChange,
  available,
  detached,
  model,
  onModelChange,
  options,
  generatedSource,
  feedback,
  editor,
}: {
  /// Prefix of the switch's own element ids.
  idPrefix: string;
  schema: RuleBuilderSchema;
  variant: string | null;
  mode: RuleEditorMode;
  onModeChange: (mode: RuleEditorMode) => void;
  /// False when this rule can only be edited as Rego.
  available: boolean;
  /// Set once a hand edit detached the rule during this editing session.
  detached: boolean;
  model: RuleBuilderModel | null;
  onModelChange: (model: RuleBuilderModel) => void;
  options: RuleBuilderOptions;
  generatedSource: string;
  feedback: React.ReactNode;
  /// The Rego editor, shown whenever the builder is not.
  editor: React.ReactNode;
}) {
  const t = useTranslate();
  const builderModel = available ? model : null;
  const showBuilder = builderModel !== null && mode === "simple";
  const result =
    showBuilder && builderModel
      ? generateRuleSource(schema, builderModel, variant)
      : null;

  return (
    <div>
      <div className="mb-2 flex flex-wrap items-center justify-between gap-2">
        <Label className="block">
          {showBuilder
            ? t("settings.ruleBuilderTitle")
            : t("settings.ruleRegoSource")}
        </Label>
        <div className="flex items-center gap-1">
          <ToggleGroupPrimitive.Root
            type="single"
            aria-label={t("settings.ruleBuilderModeLabel")}
            className="inline-flex rounded-md border border-border p-1"
            value={showBuilder ? "simple" : "advanced"}
            onValueChange={(value) => {
              if (value === "simple" || value === "advanced")
                onModeChange(value);
            }}
          >
            <ToggleGroupPrimitive.Item
              value="simple"
              disabled={builderModel === null}
              asChild
            >
              <Button
                type="button"
                size="sm"
                id={`${idPrefix}-editor-simple`}
                variant={showBuilder ? "default" : "ghost"}
              >
                {t("settings.ruleBuilderModeSimple")}
              </Button>
            </ToggleGroupPrimitive.Item>
            <ToggleGroupPrimitive.Item value="advanced" asChild>
              <Button
                type="button"
                size="sm"
                id={`${idPrefix}-editor-advanced`}
                variant={showBuilder ? "ghost" : "default"}
              >
                {t("label.advanced")}
              </Button>
            </ToggleGroupPrimitive.Item>
          </ToggleGroupPrimitive.Root>
          <InfoHelp
            ariaLabel={t("settings.ruleBuilderModeLabel")}
            text={
              builderModel !== null
                ? t("settings.ruleBuilderModeHelp")
                : t("settings.ruleBuilderUnavailable")
            }
          />
        </div>
      </div>
      {detached ? (
        <p
          id={`${idPrefix}-builder-detached`}
          role="status"
          className="mb-2 rounded border border-[var(--scry-warning-border)] bg-[var(--scry-warning-bg)] px-2 py-1 text-xs text-[var(--scry-warning-text)]"
        >
          {t("settings.ruleBuilderDetached")}
        </p>
      ) : null}
      {showBuilder && builderModel ? (
        <RuleBuilder
          schema={schema}
          model={builderModel}
          variant={variant}
          problems={result && !result.ok ? result.problems : []}
          options={options}
          onChange={onModelChange}
          generatedSource={generatedSource}
          feedback={feedback}
        />
      ) : (
        editor
      )}
    </div>
  );
}
