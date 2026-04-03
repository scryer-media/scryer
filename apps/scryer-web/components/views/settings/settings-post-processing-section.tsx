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
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import {
  boxedActionButtonBaseClass,
  boxedActionButtonToneClass,
} from "@/lib/utils/action-button-styles";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Input,
  integerInputProps,
  sanitizeDigits,
} from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { LazyRegoEditor } from "@/components/common/lazy-rego-editor";
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

type SettingsPostProcessingSectionProps = {
  scripts: PPScript[];
  editingScriptId: string | null;
  scriptDraft: PPScriptDraft;
  setScriptDraft: React.Dispatch<React.SetStateAction<PPScriptDraft>>;
  submitScript: (event: React.FormEvent<HTMLFormElement>) => Promise<void> | void;
  mutatingScriptId: string | null;
  resetDraft: () => void;
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
  { value: "tv", label: "Series" },
  { value: "anime", label: "Anime" },
];

function FacetBadges({ facets }: { facets: string[] }) {
  if (facets.length === 0) {
    return (
      <span className="rounded bg-blue-900/40 px-1.5 py-0.5 text-xs text-blue-300">
        All
      </span>
    );
  }
  return (
    <div className="flex gap-1">
      {facets.map((f) => (
        <span
          key={f}
          className="rounded bg-muted px-1.5 py-0.5 text-xs text-muted-foreground capitalize"
        >
          {f}
        </span>
      ))}
    </div>
  );
}

function statusColor(status: string): string {
  switch (status) {
    case "success":
      return "text-emerald-400";
    case "failed":
      return "text-red-400";
    case "timeout":
      return "text-yellow-400";
    case "running":
      return "text-blue-400";
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
  runs,
  noRunsLabel,
  outputNotCapturedLabel,
}: {
  runs: PPScriptRun[];
  noRunsLabel: string;
  outputNotCapturedLabel: string;
}) {
  if (runs.length === 0) {
    return (
      <p className="px-3 py-4 text-xs text-muted-foreground">{noRunsLabel}</p>
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
            <TableRow key={run.id}>
              <TableCell className="text-xs">
                {run.titleName || run.titleId || "--"}
              </TableCell>
              <TableCell>
                <span className={`text-xs font-medium capitalize ${statusColor(run.status)}`}>
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
                      <pre className="max-h-24 overflow-auto whitespace-pre-wrap rounded bg-muted/50 p-1.5 font-mono text-[10px] leading-relaxed text-muted-foreground">
                        {run.stdoutTail}
                      </pre>
                    ) : null}
                    {run.stderrTail ? (
                      <pre className="max-h-24 overflow-auto whitespace-pre-wrap rounded bg-red-900/20 p-1.5 font-mono text-[10px] leading-relaxed text-red-300">
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
    editingScriptId,
    scriptDraft,
    setScriptDraft,
    submitScript,
    mutatingScriptId,
    resetDraft,
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
      <div className="space-y-4 text-sm">
        <CardTitle className="flex items-center gap-2 text-base">
          <Terminal className="h-4 w-4" />
          {t("settings.pp.title")}
        </CardTitle>

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
                        {script.executionMode === "blocking"
                          ? t("settings.pp.blocking")
                          : t("settings.pp.fireAndForget")}
                      </TableCell>
                      <TableCell className="text-muted-foreground">
                        {script.executionMode === "blocking"
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
                          <Button
                            type="button"
                            size="icon-sm"
                            variant="secondary"
                            title={script.enabled ? t("label.disable") : t("label.enable")}
                            aria-label={script.enabled ? t("label.disable") : t("label.enable")}
                            onClick={() => void toggleScript(script)}
                            disabled={mutatingScriptId === script.id}
                            className={cn(
                              boxedActionButtonBaseClass,
                              boxedActionButtonToneClass[script.enabled ? "disabled" : "enabled"],
                            )}
                          >
                            <Power className="h-4 w-4" />
                          </Button>
                          <Button
                            type="button"
                            size="icon-sm"
                            variant="secondary"
                            title={t("label.edit")}
                            aria-label={t("label.edit")}
                            onClick={() => editScript(script)}
                            className={cn(
                              boxedActionButtonBaseClass,
                              boxedActionButtonToneClass.edit,
                            )}
                          >
                            <Edit className="h-4 w-4" />
                          </Button>
                          <Button
                            type="button"
                            size="icon-sm"
                            variant="secondary"
                            title={t("label.delete")}
                            aria-label={t("label.delete")}
                            onClick={() => deleteScript(script)}
                            disabled={mutatingScriptId === script.id}
                            className={cn(
                              boxedActionButtonBaseClass,
                              boxedActionButtonToneClass.delete,
                            )}
                          >
                            <Trash2 className="h-4 w-4" />
                          </Button>
                        </div>
                      </TableCell>
                    </TableRow>
                    {expandedScriptId === script.id ? (
                      <TableRow>
                        <TableCell colSpan={7} className="bg-muted/30 p-0">
                          <div className="px-4 py-2">
                            <p className="mb-1 text-xs font-medium text-muted-foreground">
                              {t("settings.pp.runHistory")}
                            </p>
                            <ScriptRunsTable
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
            <form className="space-y-4" onSubmit={submitScript}>
              {/* Name + Description */}
              <div className="grid gap-3 md:grid-cols-2">
                <label>
                  <Label className="mb-2 block">{t("settings.pp.name")}</Label>
                  <Input
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
                    <LazyRegoEditor
                      value={scriptDraft.scriptContent}
                      onChange={(value) =>
                        setScriptDraft((prev) => ({ ...prev, scriptContent: value }))
                      }
                      height="200px"
                    />
                  </>
                ) : (
                  <>
                    <Label className="mb-2 block">
                      {t("settings.pp.filePathHelp")}
                    </Label>
                    <div className="flex gap-2">
                      <Input
                        value={scriptDraft.scriptContent}
                        onChange={(e) =>
                          setScriptDraft((prev) => ({
                            ...prev,
                            scriptContent: e.target.value,
                          }))
                        }
                        className="font-mono"
                        placeholder="/usr/local/bin/post-process.sh"
                      />
                      <Button
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
                      initialPath={scriptDraft.scriptContent || "/"}
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
                <div className="space-y-2">
                  <label className="flex items-center gap-2">
                    <input
                      type="radio"
                      name="executionMode"
                      value="blocking"
                      checked={scriptDraft.executionMode === "blocking"}
                      onChange={() =>
                        setScriptDraft((prev) => ({
                          ...prev,
                          executionMode: "blocking",
                        }))
                      }
                      className="accent-primary"
                    />
                    <span className="text-sm">{t("settings.pp.blocking")}</span>
                    <span className="text-xs text-muted-foreground">
                      {t("settings.pp.blockingHelp")}
                    </span>
                  </label>
                  <label className="flex items-center gap-2">
                    <input
                      type="radio"
                      name="executionMode"
                      value="fire_and_forget"
                      checked={scriptDraft.executionMode === "fire_and_forget"}
                      onChange={() =>
                        setScriptDraft((prev) => ({
                          ...prev,
                          executionMode: "fire_and_forget",
                        }))
                      }
                      className="accent-primary"
                    />
                    <span className="text-sm">
                      {t("settings.pp.fireAndForget")}
                    </span>
                    <span className="text-xs text-muted-foreground">
                      {t("settings.pp.fireAndForgetHelp")}
                    </span>
                  </label>
                </div>
              </div>

              {/* Timeout + Priority (only for blocking) */}
              {scriptDraft.executionMode === "blocking" ? (
                <div className="grid gap-3 md:grid-cols-2">
                  <label>
                    <Label className="mb-2 block">
                      {t("settings.pp.timeout")}
                    </Label>
                    <Input
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
                <Button type="submit" disabled={mutatingScriptId === "new"}>
                  {mutatingScriptId === "new"
                    ? t("label.saving")
                    : editingScriptId
                      ? t("label.update")
                      : t("label.create")}
                </Button>
                <Button
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

        {/* Environment Variables Reference */}
        <EnvVarsReference />
      </div>
    );
  },
);

function EnvVarsReference() {
  const t = useTranslate();
  const [open, setOpen] = React.useState(false);

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
        <CardContent className="text-sm">
          <pre className="rounded border border-border bg-muted/50 p-3 text-xs leading-relaxed">
{`{
  "event": "post_import",
  "facet": "tv",
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
    "episode": 5,
    "absolute": 5,
    "name": "..."
  },
  "release": {
    "raw_title": "...",
    "quality": "1080p",
    "source": "WEB-DL",
    "video_codec": "H.265",
    "audio": "DDP5.1 Atmos",
    "release_group": "...",
    "is_dual_audio": false,
    "is_dolby_vision": true,
    "is_hdr10plus": true,
    "languages_audio": ["English"],
    "streaming_service": "ATVP"
  },
  "mediainfo": {
    "video_codec": "HEVC",
    "video_width": 3840,
    "video_height": 2160,
    "video_bit_depth": 10,
    "video_hdr_format": "Dolby Vision",
    "audio_codec": "E-AC-3",
    "audio_channels": "5.1",
    "audio_languages": ["eng"],
    "subtitle_languages": ["eng", "spa"],
    "duration_seconds": 3600,
    "container_format": "Matroska"
  }
}`}
          </pre>
        </CardContent>
      ) : null}
    </Card>
  );
}
