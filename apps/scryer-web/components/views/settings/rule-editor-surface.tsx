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
import type { RuleBuilderSchemaStatus } from "@/lib/hooks/use-rule-builder-schema";

/// A rule's source editor, the same for every rule family. A rule still made
/// by the builder is shown in the builder (Simple) or as its generated Rego
/// (Advanced), and the operator moves freely between the two. While the
/// builder has written nothing yet, because nothing is chosen or the choices
/// are incomplete, Advanced shows the family's starter source instead. The
/// first edit there detaches the rule from the builder, which is one-way, so
/// it asks first when that gives up choices made in the builder. Any other
/// rule shows only the Rego editor.
export function RuleEditorSurface({
  idPrefix,
  schema,
  builderStatus = "ready",
  variant,
  mode,
  onModeChange,
  onEditAsRego,
  available,
  model,
  onModelChange,
  options,
  generatedSource,
  starterSource,
  feedback,
  editor,
}: {
  /// Prefix of the surface's own element ids.
  idPrefix: string;
  /// Null until the family's outputs are read; only the Rego editor is shown
  /// meanwhile, and after a failed read.
  schema: RuleBuilderSchema | null;
  builderStatus?: RuleBuilderSchemaStatus;
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
  /// Shown on Advanced while the builder has written nothing yet.
  starterSource: string;
  feedback: React.ReactNode;
  /// The Rego editor, shown whenever the builder is not, with the source it
  /// shows. `readOnly` is set while it shows the builder's own source.
  editor: (readOnly: boolean, source: string) => React.ReactNode;
}) {
  const t = useTranslate();
  const [confirming, setConfirming] = React.useState(false);
  const builderModel =
    available && mode !== "edit" && schema !== null ? model : null;
  const showBuilder = builderModel !== null && mode === "simple";
  const result =
    showBuilder && builderModel && schema
      ? generateRuleSource(schema, builderModel, variant)
      : null;

  const shownSource =
    builderModel !== null && generatedSource === ""
      ? starterSource
      : generatedSource;
  // Editing gives up nothing chosen in the builder: the first edit detaches
  // the rule and goes through to the editor.
  const editsFreely =
    builderModel !== null &&
    !ruleRegoEditorNeedsConfirmation({ regoSource: "", builderModel });

  /// Stops an edit of the builder's Rego before the editor applies it and
  /// asks first. Copying, selecting and moving around stay free.
  const holdEdit = (event: React.SyntheticEvent) => {
    if (editsFreely) {
      onEditAsRego();
      return;
    }
    event.preventDefault();
    event.stopPropagation();
    setConfirming(true);
  };
  const holdEditKey = (event: React.KeyboardEvent) => {
    const modified = event.metaKey || event.ctrlKey;
    const key = event.key.toLowerCase();
    const edits = modified
      ? key === "v" || key === "x" || key === "z" || key === "y"
      : event.key.length === 1 ||
        event.key === "Backspace" ||
        event.key === "Delete" ||
        event.key === "Enter";
    if (edits && !event.altKey) holdEdit(event);
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
      {builderStatus !== "ready" ? (
        <p
          id={selectorId(idPrefix, "editor-builder-status")}
          data-builder-status={builderStatus}
          className="mb-2 text-xs text-muted-foreground"
        >
          {builderStatus === "loading"
            ? t("settings.ruleBuilderSchemaLoading")
            : t("settings.ruleBuilderSchemaFailed")}
        </p>
      ) : null}
      {showBuilder && builderModel && schema ? (
        <RuleBuilder
          schema={schema}
          model={builderModel}
          variant={variant}
          problems={result && !result.ok ? result.problems : []}
          options={options}
          onChange={onModelChange}
          feedback={feedback}
        />
      ) : (
        // One wrapper for both cases, so confirming keeps the same editor.
        <div
          id={selectorId(idPrefix, "editor-source")}
          {...(builderModel !== null && !confirming
            ? {
                onKeyDownCapture: holdEditKey,
                onBeforeInputCapture: holdEdit,
                onPasteCapture: holdEdit,
                onCutCapture: holdEdit,
                onDropCapture: holdEdit,
              }
            : {})}
        >
          {editor(false, shownSource)}
        </div>
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
