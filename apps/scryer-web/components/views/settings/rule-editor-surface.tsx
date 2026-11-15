import * as React from "react";
import * as ToggleGroupPrimitive from "@radix-ui/react-toggle-group";
import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { InfoHelp } from "@/components/common/info-help";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import {
  RuleBuilder,
  type RuleBuilderOptions,
} from "@/components/views/settings/rule-builder";
import { useTranslate } from "@/lib/context/translate-context";
import { selectorId } from "@/lib/utils/dom-ids";
import {
  generateRuleSource,
  type RuleBuilderModel,
  type RuleBuilderSchema,
} from "@/lib/utils/rule-builder";
import {
  ruleRegoEditorNeedsConfirmation,
  type RuleEditorMode,
} from "@/lib/utils/rule-builder-mode";

/// A rule's source editor, the same for every rule family. A rule still made
/// by the builder is shown in the builder (Simple) or as its generated Rego,
/// read-only (Advanced), with a confirmed, one-way step to edit that Rego.
/// Any other rule shows only the Rego editor.
export function RuleEditorSurface({
  idPrefix,
  schema,
  variant,
  mode,
  onModeChange,
  onEditAsRego,
  available,
  model,
  onModelChange,
  options,
  generatedSource,
  feedback,
  editor,
}: {
  /// Prefix of the surface's own element ids.
  idPrefix: string;
  schema: RuleBuilderSchema;
  variant: string | null;
  mode: RuleEditorMode;
  onModeChange: (mode: "simple" | "view") => void;
  /// Detach the rule from the builder and open the Rego editor.
  onEditAsRego: () => void;
  /// False when this rule can only be shown as Rego.
  available: boolean;
  model: RuleBuilderModel | null;
  onModelChange: (model: RuleBuilderModel) => void;
  options: RuleBuilderOptions;
  generatedSource: string;
  feedback: React.ReactNode;
  /// The Rego editor, shown whenever the builder is not. `readOnly` is set
  /// while it shows the builder's own source.
  editor: (readOnly: boolean) => React.ReactNode;
}) {
  const t = useTranslate();
  const [confirming, setConfirming] = React.useState(false);
  const builderModel = available && mode !== "edit" ? model : null;
  const showBuilder = builderModel !== null && mode === "simple";
  const result =
    showBuilder && builderModel
      ? generateRuleSource(schema, builderModel, variant)
      : null;

  const editAsRego = () => {
    if (ruleRegoEditorNeedsConfirmation({ regoSource: "", builderModel })) {
      setConfirming(true);
    } else {
      onEditAsRego();
    }
  };

  return (
    <div>
      <div className="mb-2 flex flex-wrap items-center justify-between gap-2">
        <Label className="block">
          {showBuilder
            ? t("settings.ruleBuilderTitle")
            : t("settings.ruleRegoSource")}
        </Label>
        <div className="flex items-center gap-1">
          {builderModel !== null ? (
            <>
              {mode === "view" ? (
                <Button
                  type="button"
                  size="sm"
                  variant="outline"
                  id={selectorId(idPrefix, "editor-edit-as-rego")}
                  onClick={editAsRego}
                >
                  {t("settings.ruleBuilderEditAsRego")}
                </Button>
              ) : null}
              <ToggleGroupPrimitive.Root
                type="single"
                aria-label={t("settings.ruleBuilderModeLabel")}
                className="inline-flex rounded-md border border-border p-1"
                value={showBuilder ? "simple" : "view"}
                onValueChange={(value) => {
                  if (value === "simple" || value === "view")
                    onModeChange(value);
                }}
              >
                <ToggleGroupPrimitive.Item value="simple" asChild>
                  <Button
                    type="button"
                    size="sm"
                    id={selectorId(idPrefix, "editor-simple")}
                    variant={showBuilder ? "default" : "ghost"}
                  >
                    {t("settings.ruleBuilderModeSimple")}
                  </Button>
                </ToggleGroupPrimitive.Item>
                <ToggleGroupPrimitive.Item value="view" asChild>
                  <Button
                    type="button"
                    size="sm"
                    id={selectorId(idPrefix, "editor-advanced")}
                    variant={showBuilder ? "ghost" : "default"}
                  >
                    {t("label.advanced")}
                  </Button>
                </ToggleGroupPrimitive.Item>
              </ToggleGroupPrimitive.Root>
            </>
          ) : null}
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
      ) : builderModel !== null && generatedSource === "" ? (
        <p
          id={selectorId(idPrefix, "editor-view-empty")}
          className="rounded border border-border px-2 py-3 text-xs text-muted-foreground"
        >
          {t("settings.ruleBuilderGeneratedSourceEmpty")}
        </p>
      ) : (
        editor(builderModel !== null)
      )}
      <ConfirmDialog
        open={confirming}
        contentId={selectorId(idPrefix, "edit-as-rego-confirm")}
        confirmButtonId={selectorId(idPrefix, "edit-as-rego-confirm-accept")}
        cancelButtonId={selectorId(idPrefix, "edit-as-rego-confirm-cancel")}
        title={t("settings.ruleBuilderEditAsRegoTitle")}
        description={t("settings.ruleBuilderEditAsRegoIntro")}
        confirmLabel={t("settings.ruleBuilderEditAsRego")}
        cancelLabel={t("label.cancel")}
        confirmButtonVariant="default"
        onConfirm={() => {
          setConfirming(false);
          onEditAsRego();
        }}
        onCancel={() => setConfirming(false)}
      />
    </div>
  );
}
