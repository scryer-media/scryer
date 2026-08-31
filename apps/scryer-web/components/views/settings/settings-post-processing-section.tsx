import * as React from "react";
import {
  ChevronDown,
  ChevronRight,
  Edit,
  FolderOpen,
  Plus,
  Power,
  Terminal,
  Trash2,
} from "lucide-react";
import { AddNewButton } from "@/components/common/add-new-button";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { IconButton } from "@/components/ui/icon-button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Input,
  integerInputProps,
  sanitizeDigits,
} from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group";
import { LazyCodeEditor } from "@/components/common/lazy-code-editor";
import { RenderBooleanIcon } from "@/components/common/boolean-icon";
import { FolderBrowserDialog } from "@/components/setup/folder-browser-dialog";
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

const FACET_OPTIONS = [
  { value: "movie", label: "Movie" },
  { value: "series", label: "Series" },
  { value: "anime", label: "Anime" },
];

function FacetBadges({ facets }: { facets: string[] }) {
  if (facets.length === 0) {
    return (
      <Badge tone="info" className="capitalize">
        All
      </Badge>
    );
  }
  return (
    <div className="flex gap-1">
      {facets.map((f) => (
        <Badge key={f} tone="neutral" className="capitalize">
          {f}
        </Badge>
      ))}
    </div>
  );
}

function statusColor(status: string): string {
  switch (status) {
    case "success":
      return "text-[var(--scry-success-text-soft)]";
    case "failed":
      return "text-[var(--scry-danger-text-soft)]";
    case "timeout":
      return "text-[var(--scry-warning-text)]";
    case "running":
      return "text-[var(--scry-info-text-soft)]";
    default:
      return "text-muted-foreground";
  }
}

function formatDuration(ms: number | null): string {
  if (ms == null) return "--";
  if (ms < 1000) return `${ms}ms`;
  return `${(ms / 1000).toFixed(1)}s`;
}

function ScriptRunsTable({
  scriptId,
  runs,
  noRunsLabel,
  outputNotCapturedLabel,
}: {
  scriptId: string;
  runs: PPScriptRun[];
  noRunsLabel: string;
  outputNotCapturedLabel: string;
}) {
  if (runs.length === 0) {
    return (
      <p
        id={selectorId("settings-post-processing-no-runs", scriptId)}
        className="px-3 py-4 text-xs text-muted-foreground"
      >
        {noRunsLabel}
      </p>
    );
  }
  return (
    <Table>
      <TableHeader>
        <TableRow>
          <TableHead>Title</TableHead>
          <TableHead>Status</TableHead>
          <TableHead>Duration</TableHead>
          <TableHead>Output</TableHead>
        </TableRow>
      </TableHeader>
      <TableBody>
        {runs.map((run) => {
          const hasOutput = run.stdoutTail || run.stderrTail;
          return (
            <TableRow
              data-ui="settings-table-row"
              key={run.id}
              id={selectorId(
                "settings-post-processing-run-row",
                run.status,
                run.titleName || run.titleId || "unknown-title",
                run.id,
              )}
            >
              <TableCell className="text-xs">
                {run.titleName || run.titleId || "--"}
              </TableCell>
              <TableCell>
                <span
                  id={selectorId("settings-post-processing-run-status", run.id)}
                  className={`text-xs font-medium capitalize ${statusColor(run.status)}`}
                >
                  {run.status}
                  {run.exitCode != null && run.status === "failed"
                    ? ` (exit ${run.exitCode})`
                    : ""}
                </span>
              </TableCell>
              <TableCell className="text-xs">
                {formatDuration(run.durationMs)}
              </TableCell>
              <TableCell className="max-w-[400px]">
                {hasOutput ? (
                  <div className="space-y-1">
                    {run.stdoutTail ? (
                      <pre
                        id={selectorId("settings-post-processing-run-stdout", run.id)}
                        className="max-h-24 overflow-auto whitespace-pre-wrap rounded bg-muted/50 p-1.5 font-[var(--font-code)] text-[10px] leading-relaxed text-muted-foreground"
                      >
                        {run.stdoutTail}
                      </pre>
                    ) : null}
                    {run.stderrTail ? (
                      <pre
                        id={selectorId("settings-post-processing-run-stderr", run.id)}
                        className="max-h-24 overflow-auto whitespace-pre-wrap rounded bg-[var(--scry-danger-bg)] p-1.5 font-[var(--font-code)] text-[10px] leading-relaxed text-[var(--scry-danger-text)]"
                      >
                        {run.stderrTail}
                      </pre>
                    ) : null}
                  </div>
                ) : (
                  <span className="text-[10px] text-muted-foreground">
                    {outputNotCapturedLabel}
                  </span>
                )}
              </TableCell>
            </TableRow>
          );
        })}
      </TableBody>
    </Table>
  );
}

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
    const [folderBrowserOpen, setFolderBrowserOpen] = React.useState(false);

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
                        <FacetBadges facets={script.appliedFacets} />
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
            <form id="settings-post-processing-form" className="space-y-4" onSubmit={submitScript}>
              {/* Name + Description */}
              <div className="grid gap-3 md:grid-cols-2">
                <label>
                  <Label className="mb-2 block">{t("settings.pp.name")}</Label>
                  <Input
                    id="settings-post-processing-name"
                    value={scriptDraft.name}
                    onChange={(e) =>
                      setScriptDraft((prev) => ({ ...prev, name: e.target.value }))
                    }
                    required
                    placeholder={t("settings.pp.namePlaceholder")}
                  />
                </label>
                <label>
                  <Label className="mb-2 block">
                    {t("settings.pp.descriptionLabel")}
                  </Label>
                  <Input
                    id="settings-post-processing-description"
                    value={scriptDraft.description}
                    onChange={(e) =>
                      setScriptDraft((prev) => ({
                        ...prev,
                        description: e.target.value,
                      }))
                    }
                    placeholder={t("settings.pp.descriptionPlaceholder")}
                  />
                </label>
              </div>

              {/* Script Type */}
              <div>
                <Label className="mb-2 block">
                  {t("settings.pp.scriptType")}
                </Label>
                <div className="flex gap-2">
                  <Button
                    id="settings-post-processing-script-type-inline"
                    type="button"
                    size="sm"
                    variant={scriptDraft.scriptType === "inline" ? "default" : "secondary"}
                    onClick={() =>
                      setScriptDraft((prev) => ({ ...prev, scriptType: "inline" }))
                    }
                  >
                    {t("settings.pp.inline")}
                  </Button>
                  <Button
                    id="settings-post-processing-script-type-file"
                    type="button"
                    size="sm"
                    variant={scriptDraft.scriptType === "file" ? "default" : "secondary"}
                    onClick={() =>
                      setScriptDraft((prev) => ({ ...prev, scriptType: "file" }))
                    }
                  >
                    {t("settings.pp.filePath")}
                  </Button>
                </div>
              </div>

              {/* Script Content */}
              <div>
                {scriptDraft.scriptType === "inline" ? (
                  <>
                    <Label className="mb-2 block">
                      {t("settings.pp.inlineHelp")}
                    </Label>
                    <LazyCodeEditor
                      id="settings-post-processing-script-content"
                      value={scriptDraft.scriptContent}
                      onChange={(value) =>
                        setScriptDraft((prev) => ({ ...prev, scriptContent: value }))
                      }
                      language="shell"
                      minLines={10}
                      maxLines={35}
                    />
                  </>
                ) : (
                  <>
                    <Label className="mb-2 block">
                      {t("settings.pp.filePathHelp")}
                    </Label>
                    <div className="flex gap-2">
                      <Input
                        id="settings-post-processing-script-path"
                        value={scriptDraft.scriptContent}
                        onChange={(e) =>
                          setScriptDraft((prev) => ({
                            ...prev,
                            scriptContent: e.target.value,
                          }))
                        }
                        className="font-[var(--font-code)]"
                        placeholder="/usr/local/bin/post-process.sh"
                      />
                      <Button
                        id="settings-post-processing-browse"
                        type="button"
                        variant="outline"
                        onClick={() => setFolderBrowserOpen(true)}
                      >
                        <FolderOpen className="mr-1 h-4 w-4" />
                        Browse
                      </Button>
                    </div>
                    <FolderBrowserDialog
                      open={folderBrowserOpen}
                      onOpenChange={setFolderBrowserOpen}
                      onSelect={(path) =>
                        setScriptDraft((prev) => ({ ...prev, scriptContent: path }))
                      }
                      selectionTypes={["file"]}
                      initialPath={
                        scriptDraft.scriptContent.startsWith("/")
                          ? scriptDraft.scriptContent.replace(/\/[^/]+$/, "") || "/"
                          : "/"
                      }
                      title="Select script file"
                    />
                  </>
                )}
              </div>

              {/* Facets */}
              <div>
                <Label className="mb-2 block">{t("settings.pp.facets")}</Label>
                <div className="flex items-center gap-4">
                  {FACET_OPTIONS.map((opt) => (
                    <label key={opt.value} className="flex items-center gap-2">
                      <Checkbox
                        id={selectorId("settings-post-processing-facet", opt.value)}
                        checked={scriptDraft.appliedFacets.includes(opt.value)}
                        onCheckedChange={(checked) => {
                          setScriptDraft((prev) => {
                            const next = checked
                              ? [...prev.appliedFacets, opt.value]
                              : prev.appliedFacets.filter((f) => f !== opt.value);
                            return { ...prev, appliedFacets: next };
                          });
                        }}
                      />
                      <span className="text-sm">{opt.label}</span>
                    </label>
                  ))}
                </div>
              </div>

              {/* Execution Mode */}
              <div>
                <Label className="mb-2 block">
                  {t("settings.pp.executionMode")}
                </Label>
                <RadioGroup
                  value={scriptDraft.executionMode}
                  onValueChange={(value) =>
                    setScriptDraft((prev) => ({
                      ...prev,
                      executionMode: value,
                    }))
                  }
                >
                  <label
                    htmlFor="settings-post-processing-execution-blocking"
                    className="flex items-center gap-2"
                  >
                    <RadioGroupItem
                      id="settings-post-processing-execution-blocking"
                      value="BLOCKING"
                    />
                    <span className="text-sm">{t("settings.pp.blocking")}</span>
                    <span className="text-xs text-muted-foreground">
                      {t("settings.pp.blockingHelp")}
                    </span>
                  </label>
                  <label
                    htmlFor="settings-post-processing-execution-fire-and-forget"
                    className="flex items-center gap-2"
                  >
                    <RadioGroupItem
                      id="settings-post-processing-execution-fire-and-forget"
                      value="FIRE_AND_FORGET"
                    />
                    <span className="text-sm">
                      {t("settings.pp.fireAndForget")}
                    </span>
                    <span className="text-xs text-muted-foreground">
                      {t("settings.pp.fireAndForgetHelp")}
                    </span>
                  </label>
                </RadioGroup>
              </div>

              {/* Timeout + Priority (only for blocking) */}
              {scriptDraft.executionMode === "BLOCKING" ? (
                <div className="grid gap-3 md:grid-cols-2">
                  <label>
                    <Label className="mb-2 block">
                      {t("settings.pp.timeout")}
                    </Label>
                    <Input
                      id="settings-post-processing-timeout"
                      {...integerInputProps}
                      value={scriptDraft.timeoutSecs}
                      onChange={(e) =>
                        setScriptDraft((prev) => ({
                          ...prev,
                          timeoutSecs:
                            Number(sanitizeDigits(e.target.value)) || 0,
                        }))
                      }
                    />
                  </label>
                  <label>
                    <Label className="mb-2 block">
                      {t("settings.pp.priority")}
                    </Label>
                    <Input
                      id="settings-post-processing-priority"
                      {...integerInputProps}
                      value={scriptDraft.priority}
                      onChange={(e) =>
                        setScriptDraft((prev) => ({
                          ...prev,
                          priority:
                            Number(sanitizeDigits(e.target.value)) || 0,
                        }))
                      }
                    />
                    <p className="mt-1 text-xs text-muted-foreground">
                      {t("settings.pp.priorityHelp")}
                    </p>
                  </label>
                </div>
              ) : null}

              {/* Debug */}
              <label className="flex items-center gap-2">
                <Checkbox
                  id="settings-post-processing-debug"
                  checked={scriptDraft.debug}
                  onCheckedChange={(checked) =>
                    setScriptDraft((prev) => ({
                      ...prev,
                      debug: checked === true,
                    }))
                  }
                />
                <span className="text-sm">{t("settings.pp.debug")}</span>
              </label>
              <p className="-mt-2 pl-6 text-xs text-muted-foreground">
                {t("settings.pp.debugHelp")}
              </p>

              {/* Actions */}
              <div className="flex gap-2">
                <Button id="settings-post-processing-save" type="submit" disabled={mutatingScriptId !== null}>
                  {mutatingScriptId !== null
                    ? t("label.saving")
                    : editingScriptId
                      ? t("label.update")
                      : t("label.create")}
                </Button>
                <Button
                  id="settings-post-processing-cancel"
                  type="button"
                  variant="secondary"
                  onClick={resetDraft}
                >
                  {t("label.cancel")}
                </Button>
              </div>
            </form>
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
