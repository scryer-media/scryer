import * as React from "react";
import {
  ChevronDown,
  ChevronRight,
  Edit,
  Plus,
  Power,
  Terminal,
  Trash2,
} from "lucide-react";
import { AddNewButton } from "@/components/common/add-new-button";
import { FacetTags } from "@/components/common/facet-select";
import { IconButton } from "@/components/ui/icon-button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { LazyCodeEditor } from "@/components/common/lazy-code-editor";
import { RenderBooleanIcon } from "@/components/common/boolean-icon";
import { ScriptEditorForm } from "@/components/common/script-editor-form";
import { ScriptRunsTable } from "@/components/common/script-runs-table";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { useTranslate } from "@/lib/context/translate-context";
import type {
  PPScript,
  PPScriptDraft,
  PPScriptRun,
} from "@/components/containers/settings/settings-post-processing-container";
import { selectorId } from "@/lib/utils/dom-ids";

type SettingsPostProcessingSectionProps = {
  scripts: PPScript[];
  isEditorOpen: boolean;
  editorMode: "create" | "edit";
  editingScriptId: string | null;
  scriptDraft: PPScriptDraft;
  setScriptDraft: React.Dispatch<React.SetStateAction<PPScriptDraft>>;
  submitScript: (event: React.FormEvent<HTMLFormElement>) => Promise<void> | void;
  mutatingScriptId: string | null;
  resetDraft: () => void;
  startCreateScript: () => void;
  editScript: (record: PPScript) => void;
  toggleScript: (record: PPScript) => Promise<void> | void;
  deleteScript: (record: PPScript) => void;
  expandedScriptId: string | null;
  setExpandedScriptId: (id: string | null) => void;
  scriptRuns: Record<string, PPScriptRun[]>;
  loadRunsForScript: (scriptId: string) => Promise<void> | void;
};

export const SettingsPostProcessingSection = React.memo(
  function SettingsPostProcessingSection({
    scripts,
    isEditorOpen,
    editorMode,
    editingScriptId,
    scriptDraft,
    setScriptDraft,
    submitScript,
    mutatingScriptId,
    resetDraft,
    startCreateScript,
    editScript,
    toggleScript,
    deleteScript,
    expandedScriptId,
    setExpandedScriptId,
    scriptRuns,
    loadRunsForScript,
  }: SettingsPostProcessingSectionProps) {
    const t = useTranslate();

    const handleToggleExpand = React.useCallback(
      (scriptId: string) => {
        if (expandedScriptId === scriptId) {
          setExpandedScriptId(null);
        } else {
          setExpandedScriptId(scriptId);
          void loadRunsForScript(scriptId);
        }
      },
      [expandedScriptId, setExpandedScriptId, loadRunsForScript],
    );

    return (
      <div id="settings-post-processing-section" className="space-y-4 text-sm">
        <div className="mx-auto flex w-full max-w-[2176px] flex-col gap-4 xl:flex-row xl:items-start">
          <div className="min-w-0 flex-1">
            <div className="mx-auto w-full max-w-[1280px] space-y-4">
        {/* Scripts Table */}
        <div className="rounded border border-border">
          <div className="flex items-center justify-between border-b border-border px-3 py-2">
            <div>
              <CardTitle className="text-base">
                {t("settings.pp.title")}
              </CardTitle>
              <p className="mt-0.5 text-xs text-muted-foreground">
                {t("settings.pp.description")}
              </p>
            </div>
          </div>
          <div className="overflow-x-auto">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead className="w-8" />
                  <TableHead>{t("settings.pp.name")}</TableHead>
                  <TableHead>{t("settings.pp.facets")}</TableHead>
                  <TableHead>{t("settings.pp.executionMode")}</TableHead>
                  <TableHead>{t("settings.pp.timeout")}</TableHead>
                  <TableHead className="text-center">
                    {t("label.enabled")}
                  </TableHead>
                  <TableHead className="text-right">
                    {t("label.actions")}
                  </TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {scripts.map((script) => (
                  <React.Fragment key={script.id}>
                    <TableRow
                      data-ui="settings-table-row"
                      id={selectorId("settings-post-processing-row", script.name)}
                      className="cursor-pointer"
                      onClick={() => handleToggleExpand(script.id)}
                    >
                      <TableCell className="w-8">
                        {expandedScriptId === script.id ? (
                          <ChevronDown className="h-3.5 w-3.5 text-muted-foreground" />
                        ) : (
                          <ChevronRight className="h-3.5 w-3.5 text-muted-foreground" />
                        )}
                      </TableCell>
                      <TableCell className="font-medium">
                        {script.name}
                      </TableCell>
                      <TableCell>
                        <FacetTags
                          values={script.appliedFacets}
                          emptyLabel="All"
                        />
                      </TableCell>
                      <TableCell className="text-muted-foreground">
                        {script.executionMode === "BLOCKING"
                          ? t("settings.pp.blocking")
                          : t("settings.pp.fireAndForget")}
                      </TableCell>
                      <TableCell className="text-muted-foreground">
                        {script.executionMode === "BLOCKING"
                          ? `${script.timeoutSecs}s`
                          : "--"}
                      </TableCell>
                      <TableCell className="text-center">
                        <RenderBooleanIcon
                          value={script.enabled}
                          label={`${t("label.enabled")}: ${script.name}`}
                        />
                      </TableCell>
                      <TableCell className="text-right">
                        <div
                          className="flex justify-end gap-1"
                          onClick={(e) => e.stopPropagation()}
                        >
                          <IconButton
                            id={selectorId("settings-post-processing-toggle", script.id)}
                            label={script.enabled ? t("label.disable") : t("label.enable")}
                            tone={script.enabled ? "disabled" : "enabled"}
                            onClick={() => void toggleScript(script)}
                            disabled={mutatingScriptId === script.id}
                          >
                            <Power className="h-4 w-4" />
                          </IconButton>
                          <IconButton
                            id={selectorId("settings-post-processing-edit", script.id)}
                            label={t("label.edit")}
                            tone="edit"
                            onClick={() => editScript(script)}
                          >
                            <Edit className="h-4 w-4" />
                          </IconButton>
                          <IconButton
                            id={selectorId("settings-post-processing-delete", script.id)}
                            label={t("label.delete")}
                            tone="delete"
                            onClick={() => deleteScript(script)}
                            disabled={mutatingScriptId === script.id}
                          >
                            <Trash2 className="h-4 w-4" />
                          </IconButton>
                        </div>
                      </TableCell>
                    </TableRow>
                    {expandedScriptId === script.id ? (
                      <TableRow>
                        <TableCell colSpan={7} className="bg-muted/30 p-0">
                          <div
                            id={selectorId(
                              "settings-post-processing-run-history",
                              script.id,
                            )}
                            className="px-4 py-2"
                          >
                            <p className="mb-1 text-xs font-medium text-muted-foreground">
                              {t("settings.pp.runHistory")}
                            </p>
                            <ScriptRunsTable
                              scriptId={script.id}
                              runs={scriptRuns[script.id] || []}
                              noRunsLabel={t("settings.pp.noRuns")}
                              outputNotCapturedLabel={t("settings.pp.outputNotCaptured")}
                            />
                          </div>
                        </TableCell>
                      </TableRow>
                    ) : null}
                  </React.Fragment>
                ))}
                {scripts.length === 0 ? (
                  <TableRow>
                    <TableCell colSpan={7} className="text-muted-foreground">
                      {t("settings.pp.noScripts")}
                    </TableCell>
                  </TableRow>
                ) : null}
              </TableBody>
            </Table>
          </div>
        </div>

        {isEditorOpen ? (
          <>
        {/* Create / Edit Form */}
        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-2 text-base">
              <Plus className="h-4 w-4" />
              {editingScriptId
                ? t("label.update")
                : t("label.create")}
            </CardTitle>
          </CardHeader>
          <CardContent>
            <ScriptEditorForm
              trigger="POST_IMPORT"
              formId="settings-post-processing-form"
              legacyIdPrefix="settings-post-processing"
              draft={scriptDraft}
              setDraft={setScriptDraft}
              isEditing={editingScriptId !== null}
              isSaving={mutatingScriptId !== null}
              onSubmit={submitScript}
              onCancel={resetDraft}
            />
          </CardContent>
        </Card>
        {editorMode === "edit" ? (
          <div className="flex justify-center">
            <AddNewButton
              id="settings-post-processing-create-new"
              icon={Plus}
              label={t("settings.pp.createNewScript")}
              onClick={startCreateScript}
              disabled={mutatingScriptId !== null}
            />
          </div>
        ) : null}
          </>
        ) : (
          <div className="flex justify-center">
            <AddNewButton
              id="settings-post-processing-create"
              icon={Plus}
              label={t("settings.pp.createNewScript")}
              onClick={startCreateScript}
              disabled={mutatingScriptId !== null}
            />
          </div>
        )}
            </div>
          </div>
          <div className="@container w-full space-y-4 xl:w-[44%] xl:max-w-[880px] xl:shrink-0">

        {/* Environment Variables Reference */}
        <EnvVarsReference />
          </div>
        </div>
      </div>
    );
  },
);

const ENV_METADATA_EXAMPLE = `{
  "event": "post_import",
  "facet": "series",
  "file_path": "/data/series/...",
  "title": {
    "id": "...",
    "name": "...",
    "year": 2024,
    "imdb_id": "tt...",
    "tvdb_id": "..."
  },
  "episode": {
    "season": 1,
    "episode": 5
  },
  "release": {
    "quality": "1080p"
  }
}`;

const ENV_VARIABLES_EXAMPLE = `SCRYER_METADATA={...}
SCRYER_EVENT=post_import
SCRYER_FILE_PATH=/data/series/...
SCRYER_FACET=series
SCRYER_TITLE_NAME=Example Title
SCRYER_TITLE_ID=...`;

const ignoreEnvReferenceCodeChange = (_value: string) => undefined;

function EnvVarsReference() {
  const t = useTranslate();
  const [open, setOpen] = React.useState(true);

  return (
    <Card>
      <CardHeader
        className="cursor-pointer select-none"
        onClick={() => setOpen((prev) => !prev)}
      >
        <CardTitle className="flex items-center gap-2 text-base">
          <Terminal className="h-4 w-4" />
          {t("settings.pp.envHeading")}
          <ChevronDown
            className={`ml-auto h-4 w-4 transition-transform ${open ? "rotate-180" : ""}`}
          />
        </CardTitle>
        <p className="text-xs text-muted-foreground">
          {t("settings.pp.envDescription")}
        </p>
      </CardHeader>
      {open ? (
        <CardContent className="space-y-3 text-sm">
          <LazyCodeEditor
            id="settings-post-processing-env-metadata-example"
            value={ENV_METADATA_EXAMPLE}
            onChange={ignoreEnvReferenceCodeChange}
            readOnly
            language="javascript"
            minLines={21}
            maxLines={21}
          />
          <LazyCodeEditor
            id="settings-post-processing-env-variables-example"
            value={ENV_VARIABLES_EXAMPLE}
            onChange={ignoreEnvReferenceCodeChange}
            readOnly
            language="shell"
            minLines={8}
            maxLines={8}
          />
        </CardContent>
      ) : null}
    </Card>
  );
}
