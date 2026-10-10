import * as React from "react";
import { FolderOpen } from "lucide-react";

import { FacetSelect } from "@/components/common/facet-select";
import { LazyCodeEditor, type CodeEditorLanguage } from "@/components/common/lazy-code-editor";
import { ScriptScheduleEditor } from "@/components/common/script-schedule-editor";
import { FolderBrowserDialog } from "@/components/setup/folder-browser-dialog";
import { Button } from "@/components/ui/button";
import { ScriptChoiceGroup } from "@/components/common/script-choice-group";
import { ScriptLanguageIcon } from "@/components/common/script-language-icon";
import { Checkbox } from "@/components/ui/checkbox";
import { Input, integerInputProps, sanitizeDigits } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group";
import { useTranslate } from "@/lib/context/translate-context";
import { LOWERCASE_FACET_IDS } from "@/lib/facets/selection";
import {
  SCRIPT_LANGUAGES,
  type PostProcessingScriptDraft,
  type ScriptLanguageValue,
  type ScriptTriggerValue,
} from "@/lib/types/scripts";
import { defaultScriptSchedule } from "@/lib/utils/script-schedule";

function editorLanguageFor(language: ScriptLanguageValue): CodeEditorLanguage {
  return language === "SHELL" ? "shell" : "plain";
}

type ScriptEditorFormProps = {
  /** POST_IMPORT edits a post-processing script; SCHEDULE edits a scheduled job. */
  trigger: ScriptTriggerValue;
  formId: string;
  draft: PostProcessingScriptDraft;
  setDraft: React.Dispatch<React.SetStateAction<PostProcessingScriptDraft>>;
  isEditing: boolean;
  isSaving: boolean;
  onSubmit: (event: React.FormEvent<HTMLFormElement>) => Promise<void> | void;
  onCancel: () => void;
  /**
   * Keeps the element ids a page already published on its controls. The
   * shared `script-editor-*` ids then sit on the wrapper around each control.
   */
  legacyIdPrefix?: string;
};

/**
 * The fields of one user script: what runs, how, and either the facets it
 * applies to after an import or the schedule it runs on.
 */
export function ScriptEditorForm({
  trigger,
  formId,
  draft,
  setDraft,
  isEditing,
  isSaving,
  onSubmit,
  onCancel,
  legacyIdPrefix,
}: ScriptEditorFormProps) {
  const t = useTranslate();
  const [folderBrowserOpen, setFolderBrowserOpen] = React.useState(false);
  const isScheduled = trigger === "SCHEDULE";

  /** The control id and, when a legacy id holds the control, the wrapper id. */
  const ids = (key: string, legacySuffix: string) =>
    legacyIdPrefix
      ? { control: `${legacyIdPrefix}-${legacySuffix}`, wrapper: `script-editor-${key}` }
      : { control: `script-editor-${key}`, wrapper: undefined };
  const nameIds = ids("name", "name");
  const descriptionIds = ids("description", "description");
  const contentIds = ids("content", "script-content");
  const pathIds = ids("path", "script-path");
  const timeoutIds = ids("timeout", "timeout");
  const debugIds = ids("debug", "debug");
  const saveIds = ids("save", "save");
  const cancelIds = ids("cancel", "cancel");
  const inlineTypeId = ids("type-inline", "script-type-inline").control;
  const fileTypeId = ids("type-file", "script-type-file").control;
  const blockingId = ids("execution-mode-blocking", "execution-blocking").control;
  const fireAndForgetId = ids(
    "execution-mode-fire-and-forget",
    "execution-fire-and-forget",
  ).control;

  const saveButton = (
    <Button id={saveIds.control} type="submit" disabled={isSaving}>
      {isSaving ? t("label.saving") : isEditing ? t("label.update") : t("label.create")}
    </Button>
  );
  const cancelButton = (
    <Button id={cancelIds.control} type="button" variant="secondary" onClick={onCancel}>
      {t("label.cancel")}
    </Button>
  );

  return (
    <form id={formId} className="space-y-4" onSubmit={onSubmit}>
      {/* Name + Description */}
      <div className="grid gap-3 md:grid-cols-2">
        <label id={nameIds.wrapper}>
          <Label className="mb-2 block">{t("settings.pp.name")}</Label>
          <Input
            id={nameIds.control}
            value={draft.name}
            onChange={(e) => setDraft((prev) => ({ ...prev, name: e.target.value }))}
            required
            placeholder={
              isScheduled ? t("script.editor.jobNamePlaceholder") : t("settings.pp.namePlaceholder")
            }
          />
        </label>
        <label id={descriptionIds.wrapper}>
          <Label className="mb-2 block">{t("settings.pp.descriptionLabel")}</Label>
          <Input
            id={descriptionIds.control}
            value={draft.description}
            onChange={(e) => setDraft((prev) => ({ ...prev, description: e.target.value }))}
            placeholder={t("settings.pp.descriptionPlaceholder")}
          />
        </label>
      </div>

      {/* Script Type + Interpreter */}
      <div className="flex flex-wrap gap-x-4 gap-y-4">
        <ScriptChoiceGroup
          id="script-editor-type"
          label={t("settings.pp.scriptType")}
          value={draft.scriptType}
          onValueChange={(scriptType) =>
            setDraft((prev) => ({
              ...prev,
              scriptType: scriptType as PostProcessingScriptDraft["scriptType"],
            }))
          }
          options={[
            {
              id: inlineTypeId,
              value: "inline",
              label: isScheduled ? t("script.editor.inline") : t("settings.pp.inline"),
            },
            { id: fileTypeId, value: "file", label: t("settings.pp.filePath") },
          ]}
        />

        <ScriptChoiceGroup
          id="script-editor-language"
          label={t("script.editor.interpreter")}
          help={t("script.editor.interpreterHelp")}
          value={draft.language}
          onValueChange={(language) =>
            setDraft((prev) => ({ ...prev, language: language as ScriptLanguageValue }))
          }
          options={SCRIPT_LANGUAGES.map((language) => ({
            value: language,
            label: t(`script.language.${language.toLowerCase()}`),
            icon: <ScriptLanguageIcon language={language} />,
          }))}
        />
      </div>

      {/* Script Content */}
      <div>
        {draft.scriptType === "inline" ? (
          <>
            <Label className="mb-2 block">
              {isScheduled ? t("script.editor.inlineHelp") : t("settings.pp.inlineHelp")}
            </Label>
            <div id={contentIds.wrapper}>
              <LazyCodeEditor
                id={contentIds.control}
                value={draft.scriptContent}
                onChange={(value) => setDraft((prev) => ({ ...prev, scriptContent: value }))}
                language={editorLanguageFor(draft.language)}
                minLines={10}
                maxLines={35}
              />
            </div>
          </>
        ) : (
          <>
            <Label htmlFor={pathIds.control} className="mb-2 block">
              {isScheduled ? t("script.editor.filePathHelp") : t("settings.pp.filePathHelp")}
            </Label>
            {/* The path is always one picked from the host, never typed. */}
            <div id={pathIds.wrapper} className="flex">
              <Button
                id={pathIds.control}
                type="button"
                variant="outline"
                aria-haspopup="dialog"
                onClick={() => setFolderBrowserOpen(true)}
                className="min-w-0 flex-1 justify-start font-[var(--font-code)]"
                title={draft.scriptContent || t("script.editor.selectFile")}
                data-value={draft.scriptContent}
              >
                <FolderOpen className="mr-2 h-4 w-4 shrink-0" />
                <span className="truncate">
                  {draft.scriptContent || t("script.editor.selectFile")}
                </span>
              </Button>
            </div>
            <FolderBrowserDialog
              open={folderBrowserOpen}
              onOpenChange={setFolderBrowserOpen}
              onSelect={(path) => setDraft((prev) => ({ ...prev, scriptContent: path }))}
              selectionTypes={["file"]}
              initialPath={
                draft.scriptContent.startsWith("/")
                  ? draft.scriptContent.replace(/\/[^/]+$/, "") || "/"
                  : "/"
              }
              title={t("script.editor.selectFile")}
            />
          </>
        )}
      </div>

      {isScheduled ? (
        <div>
          <Label className="mb-2 block">{t("script.editor.schedule")}</Label>
          <ScriptScheduleEditor
            value={draft.schedule ?? defaultScriptSchedule()}
            onChange={(schedule) => setDraft((prev) => ({ ...prev, schedule }))}
            runOnStartup={draft.runOnStartup}
            onRunOnStartupChange={(runOnStartup) =>
              setDraft((prev) => ({ ...prev, runOnStartup }))
            }
          />
        </div>
      ) : (
        /* Facets */
        <div>
          <Label className="mb-2 block">{t("settings.pp.facets")}</Label>
          <FacetSelect
            idPrefix={`${legacyIdPrefix ?? "script-editor"}-facet`}
            values={LOWERCASE_FACET_IDS}
            selected={draft.appliedFacets}
            onChange={(next) => {
              setDraft((prev) => ({ ...prev, appliedFacets: next }));
            }}
          />
        </div>
      )}

      {/* Execution Mode */}
      <div>
        <Label className="mb-2 block">{t("settings.pp.executionMode")}</Label>
        <RadioGroup
          id="script-editor-execution-mode"
          value={draft.executionMode}
          onValueChange={(value) => setDraft((prev) => ({ ...prev, executionMode: value }))}
        >
          <label htmlFor={blockingId} className="flex items-center gap-2">
            <RadioGroupItem id={blockingId} value="BLOCKING" />
            <span className="text-sm">{t("settings.pp.blocking")}</span>
            <span className="text-xs text-muted-foreground">
              {isScheduled ? t("script.editor.blockingHelp") : t("settings.pp.blockingHelp")}
            </span>
          </label>
          <label htmlFor={fireAndForgetId} className="flex items-center gap-2">
            <RadioGroupItem id={fireAndForgetId} value="FIRE_AND_FORGET" />
            <span className="text-sm">{t("settings.pp.fireAndForget")}</span>
            <span className="text-xs text-muted-foreground">
              {isScheduled ? t("script.editor.fireAndForgetHelp") : t("settings.pp.fireAndForgetHelp")}
            </span>
          </label>
        </RadioGroup>
      </div>

      {/* Timeout + Priority (only for blocking; priority only after imports) */}
      {draft.executionMode === "BLOCKING" ? (
        <div className="grid gap-3 md:grid-cols-2">
          <label id={timeoutIds.wrapper}>
            <Label className="mb-2 block">{t("settings.pp.timeout")}</Label>
            <Input
              id={timeoutIds.control}
              {...integerInputProps}
              value={draft.timeoutSecs}
              onChange={(e) =>
                setDraft((prev) => ({
                  ...prev,
                  timeoutSecs: Number(sanitizeDigits(e.target.value)) || 0,
                }))
              }
            />
          </label>
          {isScheduled ? null : (
            <label>
              <Label className="mb-2 block">{t("settings.pp.priority")}</Label>
              <Input
                id={`${legacyIdPrefix ?? "script-editor"}-priority`}
                {...integerInputProps}
                value={draft.priority}
                onChange={(e) =>
                  setDraft((prev) => ({
                    ...prev,
                    priority: Number(sanitizeDigits(e.target.value)) || 0,
                  }))
                }
              />
              <p className="mt-1 text-xs text-muted-foreground">{t("settings.pp.priorityHelp")}</p>
            </label>
          )}
        </div>
      ) : null}

      {/* Enabled (scheduled jobs; post-processing scripts toggle from their table) */}
      {isScheduled ? (
        <label className="flex items-center gap-2">
          <Checkbox
            id="script-editor-enabled"
            checked={draft.enabled}
            onCheckedChange={(checked) =>
              setDraft((prev) => ({ ...prev, enabled: checked === true }))
            }
          />
          <span className="text-sm">{t("label.enabled")}</span>
        </label>
      ) : null}

      {/* Debug */}
      <label id={debugIds.wrapper} className="flex items-center gap-2">
        <Checkbox
          id={debugIds.control}
          checked={draft.debug}
          onCheckedChange={(checked) =>
            setDraft((prev) => ({ ...prev, debug: checked === true }))
          }
        />
        <span className="text-sm">{t("settings.pp.debug")}</span>
      </label>
      <p className="-mt-2 pl-6 text-xs text-muted-foreground">{t("settings.pp.debugHelp")}</p>

      {/* Actions */}
      <div className="flex gap-2">
        {saveIds.wrapper ? (
          <span id={saveIds.wrapper} className="inline-flex">
            {saveButton}
          </span>
        ) : (
          saveButton
        )}
        {cancelIds.wrapper ? (
          <span id={cancelIds.wrapper} className="inline-flex">
            {cancelButton}
          </span>
        ) : (
          cancelButton
        )}
      </div>
    </form>
  );
}
