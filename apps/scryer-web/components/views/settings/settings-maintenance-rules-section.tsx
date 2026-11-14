import {
  formatRegoValidationError,
  regoDiagnostics,
} from "@/lib/utils/rego-diagnostics";
import { formatBytes } from "@/lib/utils/activity-utils";
import * as React from "react";
import { MaintenanceSubjectPicker } from "@/components/common/maintenance-subject-picker";
import {
  BookOpen,
  ChevronDown,
  ChevronRight,
  Copy,
  Edit,
  Plus,
  Sparkles,
  Trash2,
} from "lucide-react";
import { AddNewButton } from "@/components/common/add-new-button";
import { HelpLabel } from "@/components/common/info-help";
import { RegoAiPromptDialog } from "@/components/common/rego-ai-prompt-dialog";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible";
import { IconButton } from "@/components/ui/icon-button";
import {
  Input,
  integerInputProps,
  sanitizeDigits,
} from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { LazyRegoEditor } from "@/components/common/lazy-rego-editor";
import { EditorValidationStatus } from "@/components/common/editor-validation-status";
import {
  MaintenanceDecidedActionsSummary,
  MaintenanceDeclaredActionsSummary,
  MaintenanceRuleSourceFindings,
} from "@/components/views/settings/maintenance-action-summary";
import { SingleSelectField } from "@/components/ui/select";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import type { MaintenanceRulesSection } from "@/components/root/types";
import maintenanceInputContract from "@/lib/contracts/maintenance-input-contract.json";
import {
  MAINTENANCE_RULE_TEMPLATES,
  maintenanceTemplateFacetLabelKey,
  type MaintenanceRuleTemplate,
} from "@/lib/constants/maintenance-rule-templates";
import { useUiDateTimeFormat } from "@/lib/context/ui-settings-context";
import { formatUiDateTime } from "@/lib/utils/date-format";
import { useTranslate } from "@/lib/context/translate-context";
import type {
  MaintenanceDeclaredActions,
  MaintenancePreviewResult,
  MaintenancePreviewSource,
  MaintenancePreviewTitle,
  MaintenanceRuleScope,
  MaintenanceTestSubject,
  MaintenanceRuleSetDraft,
  MaintenanceRuleSetRecord,
  MaintenanceRuleState,
  MaintenanceValidationResult,
} from "@/lib/types/maintenance-rule-sets";
import {
  MAINTENANCE_ACTION_HEAD_ORDER,
  MAINTENANCE_ACTION_HEAD_RULE_NAMES,
  MAINTENANCE_PREVIEW_LIMIT_MAX,
  actionHeadLabelKey,
  isDestructiveActionHead,
  previewOutcomeBadgeTone,
  previewOutcomeLabelKey,
  ruleStateLabelKey,
  ruleStateOptionsFor,
  templateActionHeads,
  withMaintenanceBuilderModel,
} from "@/lib/utils/maintenance-rule-sets";
import { selectorId } from "@/lib/utils/dom-ids";
import { cn } from "@/lib/utils";
import * as ToggleGroupPrimitive from "@radix-ui/react-toggle-group";
import { InfoHelp } from "@/components/common/info-help";
import {
  MaintenanceRuleBuilder,
  type MaintenanceBuilderOptions,
} from "@/components/views/settings/maintenance-rule-builder";
import { generateMaintenanceRuleSource } from "@/lib/utils/maintenance-rule-builder";
import {
  MaintenanceRuleScheduleField,
  MaintenanceRuleScheduleSummary,
} from "@/components/views/settings/maintenance-rule-schedule";
import { toScriptScheduleInput } from "@/lib/utils/script-schedule";

export type MaintenanceRuleEditorMode = "simple" | "advanced";
export type MaintenanceLibraryOption = { id: string; name: string };
export type MaintenanceStorageRootOption = {
  id: string;
  path: string;
  libraryName: string;
};

type SettingsMaintenanceRulesSectionProps = {
  isEditorOpen: boolean;
  editingRuleSetId: string | null;
  ruleSetDraft: MaintenanceRuleSetDraft;
  setRuleSetDraft: React.Dispatch<
    React.SetStateAction<MaintenanceRuleSetDraft>
  >;
  /// Changes with every editor opening; controls that keep their own typing
  /// state are keyed on it.
  editorSessionKey: number;
  /// Which editor the rule form shows. Simple is only offered while the draft
  /// is still the builder's.
  editorMode: MaintenanceRuleEditorMode;
  setEditorMode: (mode: MaintenanceRuleEditorMode) => void;
  /// A hand edit of the source; the container decides whether it detaches
  /// the rule from the builder.
  editRegoSource: (source: string) => void;
  /// Set once a hand edit in this editor session detached the rule.
  builderDetached: boolean;
  builderOptions: MaintenanceBuilderOptions;
  submitRuleSet: (
    event: React.FormEvent<HTMLFormElement>,
  ) => Promise<void> | void;
  mutatingRuleSetId: string | null;
  resetRuleSetDraft: () => void;
  startCreateRuleSet: () => void;
  /// Load a starter template into the create-rule editor. Nothing is created
  /// here: the container prefills the draft and the operator still saves it.
  applyTemplate: (template: MaintenanceRuleTemplate) => void;
  ruleSetRecords: MaintenanceRuleSetRecord[];
  libraries: MaintenanceLibraryOption[];
  storageRoots: MaintenanceStorageRootOption[];
  copyRuleSet: (record: MaintenanceRuleSetRecord) => void;
  editRuleSet: (record: MaintenanceRuleSetRecord) => void;
  deleteRuleSet: (record: MaintenanceRuleSetRecord) => Promise<void> | void;
  validating: boolean;
  validationResult: MaintenanceValidationResult | null;
  previewSource: MaintenancePreviewSource;
  setPreviewSource: (source: MaintenancePreviewSource) => void;
  previewRuleSetId: string;
  setPreviewRuleSetId: (id: string) => void;
  previewLibraryId: string;
  setPreviewLibraryId: (id: string) => void;
  previewLimit: number;
  setPreviewLimit: (limit: number) => void;
  runPreview: (subject?: MaintenanceTestSubject) => Promise<void> | void;
  previewing: boolean;
  previewResult: MaintenancePreviewResult | null;
  previewError: string | null;
  /// The two instance-wide switches, rendered by the container above every
  /// pane so what the instance may do is never scrolled past.
  instanceSwitches: React.ReactNode;
  setRuleState: (
    record: MaintenanceRuleSetRecord,
    state: MaintenanceRuleState,
  ) => void;
  /// Which pane of the maintenance gutter is open. Only the rule list wants the
  /// editor, the preview and the reference rail; the operational panes are a
  /// single panel each and read better across the full width.
  section: MaintenanceRulesSection;
  /// Panels rendered under the rules table. They are passed in rather than
  /// rendered here so the container owns their queries and this view stays a
  /// pure rendering of what it is handed.
  operationsPanels: React.ReactNode;
};

type RefField = { field: string; type: string; descKey: string };

/// How each action head is written in source, shown in the reference table.
const MAINTENANCE_ACTION_HEAD_SYNTAX: Record<
  (typeof MAINTENANCE_ACTION_HEAD_ORDER)[number],
  string
> = {
  ADD_TAGS: 'add_tags := ["tag"] if { ... }',
  REMOVE_TAGS: 'remove_tags := ["tag"] if { ... }',
  SET_QUALITY_PROFILE: 'set_quality_profile := "Profile name" if { ... }',
  MONITOR: "monitor if { ... }",
  SEARCH: "search if { ... }",
  FETCH_SUBTITLES: "fetch_subtitles if { ... }",
  MOVE_TO_LIBRARY: 'move_to_library := "Library name" if { ... }',
  MOVE_TO_ROOT: 'move_to_root := "/path/of/root" if { ... }',
  UNMONITOR: "unmonitor if { ... }",
  DELETE_FILES: "delete_files if { ... }",
  DELETE_TITLE: "delete_title if { ... }",
  NOTIFY: 'notify := "message text" if { ... }',
};
type RefSectionDef = { titleKey: string; path: string; fields: RefField[] };
const REF_SECTIONS = maintenanceInputContract.sections as RefSectionDef[];

function RefFieldTable({ section }: { section: RefSectionDef }) {
  const t = useTranslate();
  return (
    <div>
      <h4 className="mb-1 font-semibold">
        <code data-code-font className="rounded bg-muted px-1.5 py-0.5 text-xs">
          {section.path}
        </code>{" "}
        <span className="font-normal text-muted-foreground">
          {t(section.titleKey)}
        </span>
      </h4>
      <div className="overflow-x-auto">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead className="w-[260px]">
                {t("settings.refColField")}
              </TableHead>
              <TableHead className="w-[100px]">
                {t("settings.refColType")}
              </TableHead>
              <TableHead>{t("settings.refColDescription")}</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {section.fields.map((field) => (
              <TableRow key={field.field} data-ui="settings-table-row">
                <TableCell>
                  <code data-code-font className="text-xs">
                    {section.path}.{field.field}
                  </code>
                </TableCell>
                <TableCell>
                  <code
                    data-code-font
                    className="rounded bg-muted px-1 py-0.5 text-xs text-muted-foreground"
                  >
                    {field.type}
                  </code>
                </TableCell>
                <TableCell className="text-muted-foreground">
                  {t(field.descKey)}
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </div>
    </div>
  );
}

function MaintenanceContextReference() {
  const t = useTranslate();
  const [open, setOpen] = React.useState(false);

  return (
    <Card>
      <CardHeader
        className="cursor-pointer select-none"
        onClick={() => setOpen((prev) => !prev)}
      >
        <CardTitle className="flex items-center gap-2 text-base">
          <BookOpen className="h-4 w-4" />
          {t("settings.refMaintTitle")}
          <ChevronDown
            className={`ml-auto h-4 w-4 transition-transform ${open ? "rotate-180" : ""}`}
          />
        </CardTitle>
      </CardHeader>
      {open ? (
        <CardContent
          id="settings-maintenance-rules-reference"
          className="space-y-6 text-sm"
        >
          <p className="text-muted-foreground">{t("settings.refMaintIntro")}</p>

          <div>
            <h4 className="mb-1 font-semibold">
              {t("settings.refMaintOutputTitle")}
            </h4>
            <p className="mb-2 text-muted-foreground">
              {t("settings.refMaintOutputIntro")}
            </p>
            <ul className="list-disc space-y-1.5 pl-5 text-muted-foreground">
              <li>{t("settings.refMaintOutputFlagHeads")}</li>
              <li>{t("settings.refMaintOutputValueHeads")}</li>
              <li>{t("settings.refMaintOutputHelpers")}</li>
              <li>{t("settings.refMaintOutputUnknown")}</li>
              <li>{t("settings.refMaintOutputReasons")}</li>
              <li>{t("settings.refMaintOutputNoPackage")}</li>
            </ul>
          </div>

          <div>
            <h4 className="mb-1 font-semibold">
              <HelpLabel
                label={t("settings.refMaintActionsTitle")}
                help={t("settings.refMaintActionsOrder", {
                  order: MAINTENANCE_ACTION_HEAD_ORDER.map(
                    (head) => MAINTENANCE_ACTION_HEAD_RULE_NAMES[head],
                  ).join(", "),
                })}
              />
            </h4>
            <Table id="settings-maintenance-rules-reference-actions">
              <TableHeader>
                <TableRow>
                  <TableHead className="w-[260px]">
                    {t("settings.refColField")}
                  </TableHead>
                  <TableHead>{t("settings.refColDescription")}</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {MAINTENANCE_ACTION_HEAD_ORDER.map((head) => {
                  const key = actionHeadLabelKey(head);
                  return (
                    <TableRow key={head} data-ui="settings-table-row">
                      <TableCell>
                        <code data-code-font className="text-xs">
                          {MAINTENANCE_ACTION_HEAD_SYNTAX[head]}
                        </code>
                      </TableCell>
                      <TableCell className="text-muted-foreground">
                        {key ? t(key) : head}
                        {isDestructiveActionHead(head) ? (
                          <Badge tone="negative" className="ml-2 text-[10px]">
                            {t("settings.maintenanceTemplateDestructiveBadge")}
                          </Badge>
                        ) : null}
                      </TableCell>
                    </TableRow>
                  );
                })}
              </TableBody>
            </Table>
            <ul className="mt-2 list-disc space-y-1.5 pl-5 text-muted-foreground">
              <li>{t("settings.refMaintActionsDaysSince")}</li>
              <li>{t("settings.refMaintActionsNotify")}</li>
              <li>{t("settings.refMaintActionsConflicts")}</li>
            </ul>
          </div>

          <div>
            <h4 className="mb-1 font-semibold">
              {t("settings.refMaintTagActionsTitle")}
            </h4>
            <ul className="list-disc space-y-1.5 pl-5 text-muted-foreground">
              <li>{t("settings.refMaintTagActionsNote")}</li>
              <li>{t("settings.refMaintTagOscillationNote")}</li>
            </ul>
          </div>

          {REF_SECTIONS.map((section) => (
            <RefFieldTable key={section.path} section={section} />
          ))}
        </CardContent>
      ) : null}
    </Card>
  );
}

/// One starter template as a card. Everything on it is static: the gallery is
/// not gated on any backend flag, so it renders the same on an instance that
/// has never run an evaluation as on one that runs them nightly.
function MaintenanceTemplateCard({
  template,
  onApply,
}: {
  template: MaintenanceRuleTemplate;
  onApply: (template: MaintenanceRuleTemplate) => void;
}) {
  const t = useTranslate();
  const heads = templateActionHeads(template.regoSource);
  return (
    <button
      id={selectorId("settings-maintenance-template", template.id)}
      type="button"
      data-maintenance-template-destructive={
        template.destructive ? "true" : "false"
      }
      className="group flex flex-col rounded-lg border border-border bg-card/50 p-3 text-left transition-colors hover:border-primary/40 hover:bg-card"
      onClick={() => onApply(template)}
      title={t("settings.maintenanceTemplateApply")}
    >
      <p className="text-sm font-medium text-foreground group-hover:text-primary">
        {t(template.titleKey)}
      </p>
      <p className="mt-1 text-xs leading-5 text-muted-foreground">
        {t(template.descriptionKey)}
      </p>
      <div className="mt-2 flex flex-wrap items-center gap-1.5">
        {heads.map((head) => {
          const key = actionHeadLabelKey(head);
          return (
            <Badge key={head} tone="neutral" className="text-[10px]">
              {key ? t(key) : head}
            </Badge>
          );
        })}
        {template.subjectKind ? (
          <Badge tone="info" className="text-[10px]">
            {t(
              template.subjectKind === "EPISODE"
                ? "settings.maintenanceScopeEpisode"
                : "settings.maintenanceScopeSeason",
            )}
          </Badge>
        ) : null}
        {template.destructive ? (
          <Badge tone="negative" className="text-[10px]">
            {t("settings.maintenanceTemplateDestructiveBadge")}
          </Badge>
        ) : null}
        {template.requiresTargetQualityProfile ? (
          <Badge tone="warning" className="text-[10px]">
            {t("settings.maintenanceTemplateNeedsProfileBadge")}
          </Badge>
        ) : null}
        <Badge tone="neutral" className="text-[10px]">
          {template.graceDays > 0
            ? t("settings.maintenanceTemplateGraceBadge", {
                count: template.graceDays,
              })
            : t("settings.maintenanceTemplateNoGraceBadge")}
        </Badge>
        {template.subjectFacets.map((facet) => (
          <Badge key={facet} tone="info" className="text-[10px]">
            {t(maintenanceTemplateFacetLabelKey(facet))}
          </Badge>
        ))}
      </div>
    </button>
  );
}

function MaintenanceTemplateGallery({
  onApply,
}: {
  onApply: (template: MaintenanceRuleTemplate) => void;
}) {
  const t = useTranslate();
  const [open, setOpen] = React.useState(false);

  return (
    <Card>
      <CardHeader
        className="cursor-pointer select-none"
        onClick={() => setOpen((prev) => !prev)}
      >
        <CardTitle className="flex items-center gap-2 text-base">
          <Sparkles className="h-4 w-4" />
          {t("settings.maintenanceTemplateGallery")}
          <ChevronDown
            className={`ml-auto h-4 w-4 transition-transform ${open ? "rotate-180" : ""}`}
          />
        </CardTitle>
      </CardHeader>
      {open ? (
        <CardContent
          id="settings-maintenance-template-gallery"
          className="@container"
        >
          <div className="grid grid-cols-1 gap-2 @[560px]:grid-cols-2 @[900px]:grid-cols-3">
            {MAINTENANCE_RULE_TEMPLATES.map((template) => (
              <MaintenanceTemplateCard
                key={template.id}
                template={template}
                onApply={(applied) => {
                  onApply(applied);
                  setOpen(false);
                }}
              />
            ))}
          </div>
        </CardContent>
      ) : null}
    </Card>
  );
}

function PreviewOutcomeCell({ title }: { title: MaintenancePreviewTitle }) {
  const t = useTranslate();
  if (title.error) {
    return (
      <Badge tone="negative">
        {t("settings.maintenancePreviewOutcomeError")}
      </Badge>
    );
  }
  const labelKey = title.outcome ? previewOutcomeLabelKey(title.outcome) : null;
  return (
    <Badge tone={previewOutcomeBadgeTone(title.outcome)}>
      {labelKey ? t(labelKey) : (title.outcome ?? "—")}
    </Badge>
  );
}

function MaintenancePreviewPanel({
  isEditorOpen,
  draftName,
  draftScope,
  draftDeclaredActions,
  ruleSetRecords,
  libraries,
  previewSource,
  setPreviewSource,
  previewRuleSetId,
  setPreviewRuleSetId,
  previewLibraryId,
  setPreviewLibraryId,
  previewLimit,
  setPreviewLimit,
  runPreview,
  previewing,
  previewResult,
  previewError,
}: {
  draftName: string;
  draftScope: MaintenanceRuleScope;
  /// What the draft declares, from its latest validation.
  draftDeclaredActions: MaintenanceDeclaredActions | null | undefined;
} & Pick<
  SettingsMaintenanceRulesSectionProps,
  | "isEditorOpen"
  | "ruleSetRecords"
  | "libraries"
  | "previewSource"
  | "setPreviewSource"
  | "previewRuleSetId"
  | "setPreviewRuleSetId"
  | "previewLibraryId"
  | "setPreviewLibraryId"
  | "previewLimit"
  | "setPreviewLimit"
  | "runPreview"
  | "previewing"
  | "previewResult"
  | "previewError"
>) {
  const t = useTranslate();
  const dateTimeFormat = useUiDateTimeFormat();
  const storedRule = ruleSetRecords.find(
    (rule) => rule.id === previewRuleSetId,
  );
  const scope =
    previewSource === "draft"
      ? draftScope
      : (storedRule?.subjectKind ?? "TITLE");
  const declared =
    previewSource === "draft"
      ? draftDeclaredActions
      : storedRule?.declaredActions;
  const [selectedSubject, setSelectedSubject] = React.useState<
    MaintenanceTestSubject | undefined
  >();
  const [targetPending, setTargetPending] = React.useState(false);
  const [manualTarget, setManualTarget] = React.useState(false);
  React.useEffect(() => {
    setSelectedSubject(undefined);
    setTargetPending(false);
    setManualTarget(false);
  }, [previewSource, previewRuleSetId, previewLibraryId, draftName, scope]);
  const canPreviewDraft = isEditorOpen;
  const canRun =
    !previewing &&
    !targetPending &&
    Boolean(previewLibraryId || selectedSubject) &&
    (previewSource === "draft" ? canPreviewDraft : Boolean(previewRuleSetId));

  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-base">
          <HelpLabel
            label={t("settings.maintenancePreviewTitle")}
            help={t("settings.maintenancePreviewSubtitle")}
          />
        </CardTitle>
      </CardHeader>
      <CardContent className="space-y-3">
        <div className="grid gap-3 md:grid-cols-4">
          <SingleSelectField
            id="settings-maintenance-preview-source"
            label={t("settings.maintenancePreviewSource")}
            value={previewSource}
            onValueChange={(value) =>
              setPreviewSource(value as MaintenancePreviewSource)
            }
            options={[
              {
                value: "stored",
                label: t("settings.maintenancePreviewSourceStored"),
              },
              {
                value: "draft",
                label: t("settings.maintenancePreviewSourceDraft"),
                disabled: !canPreviewDraft,
              },
            ]}
          />
          {previewSource === "stored" ? (
            <SingleSelectField
              id="settings-maintenance-preview-rule"
              label={t("settings.maintenancePreviewRuleSelect")}
              placeholder={t(
                "settings.maintenancePreviewRuleSelectPlaceholder",
              )}
              value={previewRuleSetId}
              onValueChange={setPreviewRuleSetId}
              options={ruleSetRecords.map((record) => ({
                value: record.id,
                label: record.name,
              }))}
            />
          ) : (
            <div className="min-w-0 space-y-1.5">
              <Label className="block">
                {t("settings.maintenancePreviewRuleSelect")}
              </Label>
              <p
                id="settings-maintenance-preview-draft-name"
                className="truncate text-xs text-muted-foreground"
              >
                {draftName || t("settings.maintenancePreviewSourceDraft")}
              </p>
            </div>
          )}
          <SingleSelectField
            id="settings-maintenance-preview-library"
            label={t("settings.maintenancePreviewLibrary")}
            placeholder={t("settings.maintenancePreviewLibraryPlaceholder")}
            value={previewLibraryId}
            onValueChange={setPreviewLibraryId}
            options={libraries.map((library) => ({
              value: library.id,
              label: library.name,
            }))}
          />
          <label>
            <Label className="mb-2 block">
              <HelpLabel
                label={t("settings.maintenancePreviewLimit")}
                help={t("settings.maintenancePreviewLimitHelp", {
                  max: MAINTENANCE_PREVIEW_LIMIT_MAX,
                })}
              />
            </Label>
            <Input
              id="settings-maintenance-preview-limit"
              {...integerInputProps}
              value={previewLimit}
              onChange={(event) =>
                setPreviewLimit(Number(sanitizeDigits(event.target.value)) || 0)
              }
            />
          </label>
        </div>

        {declared ? (
          <div
            id="settings-maintenance-preview-declared-actions"
            className="rounded border border-border bg-muted/30 px-3 py-2"
          >
            <p className="mb-1 text-xs font-medium">
              {t("settings.maintenanceDeclaredActionsTitle")}
            </p>
            <MaintenanceDeclaredActionsSummary
              declared={declared}
              idPrefix="settings-maintenance-preview-declared-action"
            />
          </div>
        ) : null}

        <MaintenanceSubjectPicker
          key={`${previewSource}:${previewRuleSetId}:${previewLibraryId}:${scope}:${draftName}`}
          scope={scope}
          onChange={(subject, pending, active) => {
            setSelectedSubject(subject);
            setTargetPending(pending);
            setManualTarget(active);
          }}
        />
        {!manualTarget && previewResult?.titles.length ? (
          <SingleSelectField
            id="settings-maintenance-preview-subject"
            label={t("settings.maintenancePreviewSubject")}
            value={selectedSubject?.subjectId ?? "all"}
            onValueChange={(id) =>
              setSelectedSubject(
                previewResult.titles.find(
                  (subject) => subject.subjectId === id,
                ),
              )
            }
            options={[
              { value: "all", label: t("settings.maintenancePreviewSample") },
              ...previewResult.titles.map((subject) => ({
                value: subject.subjectId,
                label:
                  subject.subjectKind === "title"
                    ? subject.titleName
                    : `${subject.titleName} · ${subject.subjectLabel}`,
              })),
            ]}
          />
        ) : null}
        <Button
          id="settings-maintenance-preview-run"
          type="button"
          variant="secondary"
          onClick={() => void runPreview(selectedSubject)}
          disabled={!canRun}
        >
          {previewing
            ? t("settings.maintenancePreviewRunning")
            : t("settings.maintenancePreviewRun")}
        </Button>

        {previewError ? (
          <pre className="overflow-x-auto whitespace-pre-wrap rounded-[9px] border border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)] p-3 font-mono text-[12px] leading-5 text-[var(--scry-danger-text)]">
            {previewError}
          </pre>
        ) : null}

        {previewResult ? (
          <div className="space-y-2">
            <p className="text-xs text-muted-foreground">
              {t("settings.maintenancePreviewMeta", {
                time: formatUiDateTime(
                  previewResult.evaluatedAt,
                  dateTimeFormat,
                ),
                hash: previewResult.matcherContentHash.slice(0, 12),
              })}
            </p>
            <div className="overflow-x-auto">
              <Table id="settings-maintenance-preview-results">
                <TableHeader>
                  <TableRow>
                    <TableHead>{t("label.title")}</TableHead>
                    <TableHead className="w-[140px]">
                      {t("settings.maintenancePreviewColOutcome")}
                    </TableHead>
                    <TableHead>{t("settings.maintenanceRuleAction")}</TableHead>
                    <TableHead>
                      {t("settings.maintenancePreviewColReasons")}
                    </TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {previewResult.titles.map((title) => (
                    <TableRow
                      key={`${title.subjectKind}:${title.subjectId}`}
                      data-ui="settings-table-row"
                      id={selectorId(
                        "settings-maintenance-preview-row",
                        title.subjectId,
                      )}
                    >
                      <TableCell className="font-medium">
                        {title.titleName}
                        {title.subjectKind !== "title" && (
                          <div className="text-sm text-muted-foreground">
                            {title.subjectLabel}
                          </div>
                        )}
                        <div className="text-xs text-muted-foreground">
                          {t("settings.maintenanceFileSummary", {
                            count: title.fileCount,
                            size: formatBytes(title.totalSizeBytes),
                          })}
                        </div>
                        {title.storageRootFileCount !== null &&
                        title.storageRootTotalSizeBytes !== null ? (
                          <div className="text-xs text-muted-foreground">
                            {t("settings.maintenanceStorageRootFileSummary", {
                              count: title.storageRootFileCount,
                              size: formatBytes(
                                title.storageRootTotalSizeBytes,
                              ),
                            })}
                          </div>
                        ) : null}
                      </TableCell>
                      <TableCell>
                        <PreviewOutcomeCell title={title} />
                        {title.excluded && (
                          <div>
                            <Badge tone="warning">
                              {t("settings.maintenancePreviewExcluded")}
                            </Badge>
                          </div>
                        )}
                        {title.dueAt && (
                          <div className="mt-1 text-xs text-muted-foreground">
                            {t(
                              title.dueAtIsEstimate
                                ? "settings.maintenancePreviewEstimatedDue"
                                : "settings.maintenancePreviewDue",
                              {
                                time: formatUiDateTime(
                                  title.dueAt,
                                  dateTimeFormat,
                                ),
                              },
                            )}
                          </div>
                        )}
                      </TableCell>
                      <TableCell>
                        {title.actions ? (
                          <MaintenanceDecidedActionsSummary
                            actions={title.actions}
                            idPrefix={selectorId(
                              "settings-maintenance-preview-action",
                              title.subjectId,
                            )}
                          />
                        ) : (
                          <span className="text-muted-foreground">—</span>
                        )}
                      </TableCell>
                      <TableCell className="text-muted-foreground">
                        {title.error ? (
                          <span className="text-[var(--scry-danger-text)]">
                            {title.error}
                          </span>
                        ) : title.reasonCodes.length > 0 ? (
                          <div className="flex flex-wrap gap-1">
                            {title.reasonCodes.map((code) => (
                              <code
                                key={code}
                                data-code-font
                                className="rounded bg-muted px-1 py-0.5 text-xs"
                              >
                                {code}
                              </code>
                            ))}
                          </div>
                        ) : (
                          "—"
                        )}
                        {title.facts ? (
                          <PreviewFacts facts={title.facts} />
                        ) : null}
                      </TableCell>
                    </TableRow>
                  ))}
                  {previewResult.titles.length === 0 ? (
                    <TableRow>
                      <TableCell colSpan={4} className="text-muted-foreground">
                        {t("settings.maintenancePreviewNoTitles")}
                      </TableCell>
                    </TableRow>
                  ) : null}
                </TableBody>
              </Table>
            </div>
          </div>
        ) : null}
      </CardContent>
    </Card>
  );
}

/// The fact document the rule read for one subject, collapsed until asked
/// for so a long preview stays scannable.
function PreviewFacts({ facts }: { facts: Record<string, unknown> }) {
  const t = useTranslate();
  return (
    <details className="mt-1 whitespace-normal text-foreground">
      <summary className="cursor-pointer text-xs">
        {t("settings.maintenancePreviewFacts")}
      </summary>
      <pre
        data-code-font
        className="mt-1 max-h-80 max-w-xl overflow-auto rounded bg-muted p-2 text-xs"
      >
        {JSON.stringify(facts, null, 2)}
      </pre>
    </details>
  );
}

/// Rule scope and storage root. Templates set the scope, so this starts
/// collapsed, and opens by itself whenever either field holds a non-default
/// value so a rule never hides what it is scoped to.
function MaintenanceRuleAdvanced({
  hasNonDefault,
  children,
}: {
  hasNonDefault: boolean;
  children: React.ReactNode;
}) {
  const t = useTranslate();
  const [open, setOpen] = React.useState(hasNonDefault);
  const [seenNonDefault, setSeenNonDefault] = React.useState(hasNonDefault);
  if (hasNonDefault !== seenNonDefault) {
    setSeenNonDefault(hasNonDefault);
    if (hasNonDefault) setOpen(true);
  }
  return (
    <Collapsible open={open} onOpenChange={setOpen}>
      <CollapsibleTrigger asChild>
        <button
          id="settings-maintenance-rule-advanced-toggle"
          type="button"
          className="flex items-center gap-1.5 rounded-[8px] py-1 text-sm font-medium text-[var(--scry-muted)] transition-colors hover:text-[var(--scry-ink2)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
        >
          <ChevronRight
            className={cn("h-4 w-4 transition-transform", open && "rotate-90")}
          />
          {t("settings.advancedConfig")}
        </button>
      </CollapsibleTrigger>
      <CollapsibleContent
        id="settings-maintenance-rule-advanced"
        className="space-y-3 pt-3"
      >
        {children}
      </CollapsibleContent>
    </Collapsible>
  );
}

export function SettingsMaintenanceRulesSection({
  isEditorOpen,
  editingRuleSetId,
  ruleSetDraft,
  setRuleSetDraft,
  editorSessionKey,
  editorMode,
  setEditorMode,
  editRegoSource,
  builderDetached,
  builderOptions,
  submitRuleSet,
  mutatingRuleSetId,
  resetRuleSetDraft,
  startCreateRuleSet,
  applyTemplate,
  ruleSetRecords,
  libraries,
  storageRoots,
  copyRuleSet,
  editRuleSet,
  deleteRuleSet,
  validating,
  validationResult,
  instanceSwitches,
  setRuleState,
  section,
  operationsPanels,
  ...previewProps
}: SettingsMaintenanceRulesSectionProps) {
  const t = useTranslate();
  const editingRecord = editingRuleSetId
    ? ruleSetRecords.find((record) => record.id === editingRuleSetId)
    : undefined;
  const editingSystemOwned = editingRecord?.systemOwned === true;
  // The saved next run describes the saved schedule, so it shows only while
  // the editor still holds that schedule.
  const editingNextRunAt =
    editingRecord &&
    JSON.stringify(toScriptScheduleInput(editingRecord.schedule)) ===
      JSON.stringify(toScriptScheduleInput(ruleSetDraft.schedule))
      ? editingRecord.nextRunAt
      : null;
  const builderAvailable =
    ruleSetDraft.builderModel !== null && !editingSystemOwned;
  const showBuilder = builderAvailable && editorMode === "simple";
  const builderResult =
    showBuilder && ruleSetDraft.builderModel
      ? generateMaintenanceRuleSource(
          ruleSetDraft.builderModel,
          ruleSetDraft.subjectKind,
        )
      : null;
  const builderProblems =
    builderResult && !builderResult.ok ? builderResult.problems : [];
  const validationStatus = (
    <EditorValidationStatus
      id="settings-maintenance-rule-validation"
      checking={validating}
      result={validationResult}
      formatError={(error) =>
        formatRegoValidationError(error, ruleSetDraft.regoSource)
      }
    />
  );

  // Candidates and history are one pane each. They keep the instance switches,
  // because what the instance is allowed to do explains every row they show,
  // but they get the full column: there is no editor to sit beside and no rule
  // being written for the reference rail to describe.
  if (section !== "rules") {
    return (
      <div
        id="settings-maintenance-rules-section"
        className="space-y-4 text-sm"
      >
        <div className="mx-auto w-full max-w-[1600px] space-y-4">
          {instanceSwitches}
          {operationsPanels}
        </div>
      </div>
    );
  }

  return (
    <div id="settings-maintenance-rules-section" className="space-y-4 text-sm">
      <div className="mx-auto flex w-full max-w-[2176px] flex-col gap-4 2xl:flex-row 2xl:items-start">
        <div className="min-w-0 basis-[900px] flex-1">
          <div className="mx-auto w-full max-w-[1280px] space-y-4">
            {instanceSwitches}

            <div className="overflow-hidden rounded border border-border bg-card">
              <div className="flex items-center justify-between border-b border-border px-3 py-2">
                <CardTitle className="text-base text-foreground">
                  {t("settings.maintenanceRules")}
                </CardTitle>
                <RegoAiPromptDialog
                  id="settings-maintenance-rules-ai-prompt"
                  ruleKind="maintenance"
                  inputContract={maintenanceInputContract}
                  outputContract={
                    'Declare actions by defining rules with these names: unmonitor, monitor, search, fetch_subtitles, delete_files and delete_title as "name if { ... }"; set_quality_profile := "Profile name", move_to_library := "Library name", move_to_root := "/path/of/root" and notify := "message text" as plain string literals; add_tags := ["tag"] and remove_tags := ["tag"] as plain lists of string literals. delete_title, move_to_library and move_to_root act on whole titles. Any other rule name is a helper of your own. days_since(ts) returns whole days since an RFC3339 timestamp; do not define a rule with that name. You may add reasons contains "stable_reason" if { ... } and unknown if { ... } to hold a subject.'
                  }
                />
              </div>
              <div className="overflow-x-auto">
                <Table className="min-w-[min(900px,100%)]">
                  <TableHeader>
                    <TableRow>
                      <TableHead>{t("label.name")}</TableHead>
                      <TableHead>{t("settings.ruleDescription")}</TableHead>
                      <TableHead>
                        {t("settings.maintenanceRuleAction")}
                      </TableHead>
                      <TableHead className="text-center">
                        {t("settings.maintenanceRuleGraceDays")}
                      </TableHead>
                      <TableHead>
                        <HelpLabel
                          label={t("jobs.column.schedule")}
                          help={t("settings.maintenanceRuleScheduleHelp")}
                        />
                      </TableHead>
                      <TableHead className="w-[240px]">
                        <HelpLabel
                          label={t("settings.maintenanceRuleState")}
                          help={t("settings.maintenanceRuleStateHelp")}
                        />
                      </TableHead>
                      <TableHead className="text-center">
                        {t("settings.maintenanceRuleRevision")}
                      </TableHead>
                      <TableHead className="text-right">
                        {t("label.actions")}
                      </TableHead>
                    </TableRow>
                  </TableHeader>
                  <TableBody>
                    {ruleSetRecords.map((record) => {
                      return (
                        <TableRow
                          data-ui="settings-table-row"
                          key={record.id}
                          id={selectorId(
                            "settings-maintenance-rule-row",
                            record.id,
                          )}
                        >
                          <TableCell className="font-medium">
                            <span
                              id={selectorId(
                                "settings-maintenance-rule-name",
                                record.name,
                              )}
                            >
                              {record.name}
                            </span>
                            {record.systemOwned ? (
                              <Badge
                                tone="neutral"
                                className="ml-2 text-[10px]"
                              >
                                {t("settings.maintenanceRuleSystemOwned")}
                              </Badge>
                            ) : null}
                            {record.subjectKind !== "TITLE" && (
                              <div className="text-xs text-muted-foreground">
                                {t(
                                  record.subjectKind === "SEASON"
                                    ? "settings.maintenanceScopeSeason"
                                    : "settings.maintenanceScopeEpisode",
                                )}
                              </div>
                            )}
                          </TableCell>
                          <TableCell className="max-w-[200px] truncate text-muted-foreground">
                            {record.description || "—"}
                          </TableCell>
                          <TableCell className="max-w-[320px]">
                            <MaintenanceDeclaredActionsSummary
                              declared={record.declaredActions}
                              idPrefix={selectorId(
                                "settings-maintenance-rule-declared-action",
                                record.id,
                              )}
                            />
                          </TableCell>
                          <TableCell className="text-center">
                            {record.graceDays}
                          </TableCell>
                          <TableCell className="min-w-[10rem] text-muted-foreground">
                            <MaintenanceRuleScheduleSummary
                              record={record}
                              idPrefix={selectorId(
                                "settings-maintenance-rule-schedule",
                                record.id,
                              )}
                            />
                          </TableCell>
                          <TableCell>
                            <div className="space-y-1">
                              <SingleSelectField
                                id={selectorId(
                                  "settings-maintenance-rule-state",
                                  record.id,
                                )}
                                size="sm"
                                value={record.state}
                                disabled={mutatingRuleSetId === record.id}
                                onValueChange={(value) =>
                                  setRuleState(
                                    record,
                                    value as MaintenanceRuleState,
                                  )
                                }
                                options={ruleStateOptionsFor(record).map(
                                  (state) => {
                                    const key = ruleStateLabelKey(state);
                                    return {
                                      value: state,
                                      ariaLabel: state,
                                      label: key ? t(key) : state,
                                    };
                                  },
                                )}
                              />
                              {record.editedSinceEnabled ? (
                                <p
                                  id={selectorId(
                                    "settings-maintenance-rule-edited-since-enabled",
                                    record.id,
                                  )}
                                  className="rounded border border-[var(--scry-warning-border)] bg-[var(--scry-warning-bg)] px-2 py-1 text-xs text-[var(--scry-warning-text)]"
                                >
                                  {t(
                                    "settings.maintenanceRuleEditedSinceEnabled",
                                  )}
                                </p>
                              ) : null}
                            </div>
                          </TableCell>
                          <TableCell className="text-center">
                            {record.currentRevisionNumber}
                          </TableCell>
                          <TableCell className="text-right">
                            <div className="flex justify-end gap-1">
                              <IconButton
                                id={selectorId(
                                  "settings-maintenance-rule-copy",
                                  record.id,
                                )}
                                label={t("settings.maintenanceRuleCopy")}
                                tone="neutral"
                                onClick={() => copyRuleSet(record)}
                              >
                                <Copy className="h-4 w-4" />
                              </IconButton>
                              <IconButton
                                id={selectorId(
                                  "settings-maintenance-rule-edit",
                                  record.id,
                                )}
                                label={t("label.edit")}
                                tone="edit"
                                onClick={() => editRuleSet(record)}
                              >
                                <Edit className="h-4 w-4" />
                              </IconButton>
                              {record.systemOwned ? null : (
                                <IconButton
                                  id={selectorId(
                                    "settings-maintenance-rule-delete",
                                    record.id,
                                  )}
                                  label={t("label.delete")}
                                  tone="delete"
                                  onClick={() => void deleteRuleSet(record)}
                                  disabled={mutatingRuleSetId === record.id}
                                >
                                  <Trash2 className="h-4 w-4" />
                                </IconButton>
                              )}
                            </div>
                          </TableCell>
                        </TableRow>
                      );
                    })}
                    {ruleSetRecords.length === 0 ? (
                      <TableRow>
                        <TableCell
                          colSpan={9}
                          className="text-muted-foreground"
                        >
                          {t("settings.noMaintenanceRulesFound")}
                        </TableCell>
                      </TableRow>
                    ) : null}
                  </TableBody>
                </Table>
              </div>
            </div>

            {isEditorOpen ? (
              <Card>
                <CardHeader>
                  <CardTitle className="text-base">
                    {editingRuleSetId
                      ? t("settings.maintenanceRuleUpdate")
                      : t("settings.maintenanceRuleCreate")}
                  </CardTitle>
                </CardHeader>
                <CardContent>
                  <form
                    id="settings-maintenance-rule-form"
                    className="space-y-3"
                    onSubmit={submitRuleSet}
                  >
                    <div className="grid gap-3 md:grid-cols-2">
                      <label>
                        <Label className="mb-2 block">{t("label.name")}</Label>
                        <Input
                          id="settings-maintenance-rule-name"
                          value={ruleSetDraft.name}
                          readOnly={editingSystemOwned}
                          onChange={(event) =>
                            setRuleSetDraft((prev) => ({
                              ...prev,
                              name: event.target.value,
                            }))
                          }
                          required
                          placeholder="stale_unmonitored_titles"
                        />
                      </label>
                      <label>
                        <Label className="mb-2 block">
                          {t("settings.ruleDescription")}
                        </Label>
                        <Input
                          id="settings-maintenance-rule-description"
                          value={ruleSetDraft.description}
                          onChange={(event) =>
                            setRuleSetDraft((prev) => ({
                              ...prev,
                              description: event.target.value,
                            }))
                          }
                        />
                      </label>
                    </div>

                    {!editingRuleSetId ? (
                      <SingleSelectField
                        id="settings-maintenance-rule-template"
                        label={t("settings.maintenanceRuleTemplate")}
                        placeholder={t(
                          "settings.maintenanceRuleTemplatePlaceholder",
                        )}
                        value=""
                        onValueChange={(id) => {
                          const template = MAINTENANCE_RULE_TEMPLATES.find(
                            (candidate) => candidate.id === id,
                          );
                          if (template) applyTemplate(template);
                        }}
                        options={MAINTENANCE_RULE_TEMPLATES.map((template) => ({
                          value: template.id,
                          label: t(template.titleKey),
                        }))}
                      />
                    ) : null}

                    <div>
                      <div className="mb-2 flex flex-wrap items-center justify-between gap-2">
                        <Label className="block">
                          {showBuilder
                            ? t("settings.maintenanceBuilderTitle")
                            : t("settings.ruleRegoSource")}
                        </Label>
                        <div className="flex items-center gap-1">
                          <ToggleGroupPrimitive.Root
                            type="single"
                            aria-label={t(
                              "settings.maintenanceBuilderModeLabel",
                            )}
                            className="inline-flex rounded-md border border-border p-1"
                            value={showBuilder ? "simple" : "advanced"}
                            onValueChange={(value) => {
                              if (value === "simple" || value === "advanced")
                                setEditorMode(value);
                            }}
                          >
                            <ToggleGroupPrimitive.Item
                              value="simple"
                              disabled={!builderAvailable}
                              asChild
                            >
                              <Button
                                type="button"
                                size="sm"
                                id="settings-maintenance-rule-editor-simple"
                                variant={showBuilder ? "default" : "ghost"}
                              >
                                {t("settings.maintenanceBuilderModeSimple")}
                              </Button>
                            </ToggleGroupPrimitive.Item>
                            <ToggleGroupPrimitive.Item value="advanced" asChild>
                              <Button
                                type="button"
                                size="sm"
                                id="settings-maintenance-rule-editor-advanced"
                                variant={showBuilder ? "ghost" : "default"}
                              >
                                {t("label.advanced")}
                              </Button>
                            </ToggleGroupPrimitive.Item>
                          </ToggleGroupPrimitive.Root>
                          <InfoHelp
                            ariaLabel={t(
                              "settings.maintenanceBuilderModeLabel",
                            )}
                            text={
                              builderAvailable
                                ? t("settings.maintenanceBuilderModeHelp")
                                : t("settings.maintenanceBuilderUnavailable")
                            }
                          />
                        </div>
                      </div>
                      {builderDetached ? (
                        <p
                          id="settings-maintenance-rule-builder-detached"
                          role="status"
                          className="mb-2 rounded border border-[var(--scry-warning-border)] bg-[var(--scry-warning-bg)] px-2 py-1 text-xs text-[var(--scry-warning-text)]"
                        >
                          {t("settings.maintenanceBuilderDetached")}
                        </p>
                      ) : null}
                      {showBuilder && ruleSetDraft.builderModel ? (
                        <MaintenanceRuleBuilder
                          model={ruleSetDraft.builderModel}
                          scope={ruleSetDraft.subjectKind}
                          problems={builderProblems}
                          options={builderOptions}
                          onChange={(model) =>
                            setRuleSetDraft((prev) =>
                              withMaintenanceBuilderModel(prev, model),
                            )
                          }
                          generatedSource={ruleSetDraft.regoSource}
                          feedback={validationStatus}
                        />
                      ) : (
                        <LazyRegoEditor
                          ruleFamily="maintenance"
                          id="settings-maintenance-rule-rego-source"
                          value={ruleSetDraft.regoSource}
                          readOnly={editingSystemOwned}
                          onChange={editRegoSource}
                          diagnostics={regoDiagnostics(
                            ruleSetDraft.regoSource,
                            validationResult,
                            t,
                          )}
                          feedback={validationStatus}
                          minLines={12}
                          maxLines={35}
                        />
                      )}
                    </div>

                    <MaintenanceRuleSourceFindings
                      source={ruleSetDraft.regoSource}
                      checked={
                        !validating &&
                        validationResult !== null &&
                        !validationResult.unavailable
                      }
                      declared={validationResult?.declaredActions}
                      warnings={validationResult?.warnings ?? []}
                    />

                    <label className="block md:max-w-[50%]">
                      <Label className="mb-2 block">
                        <HelpLabel
                          label={t("settings.maintenanceRuleGraceDays")}
                          help={t("settings.maintenanceRuleGraceDaysHelp")}
                        />
                      </Label>
                      <Input
                        id="settings-maintenance-rule-grace-days"
                        {...integerInputProps}
                        min={0}
                        value={ruleSetDraft.graceDays}
                        onChange={(event) =>
                          setRuleSetDraft((prev) => ({
                            ...prev,
                            graceDays:
                              Number(sanitizeDigits(event.target.value)) || 0,
                          }))
                        }
                        placeholder="0"
                      />
                    </label>

                    <MaintenanceRuleScheduleField
                      key={editorSessionKey}
                      idPrefix="settings-maintenance-rule-form-schedule"
                      value={ruleSetDraft.schedule}
                      onChange={(schedule) =>
                        setRuleSetDraft((prev) => ({ ...prev, schedule }))
                      }
                      nextRunAt={editingNextRunAt}
                    />

                    <div>
                      <Label className="mb-2 block">
                        <HelpLabel
                          label={t("settings.maintenanceRuleLibraries")}
                          help={t("settings.maintenanceRuleLibrariesHelp")}
                        />
                      </Label>
                      <div className="flex flex-wrap items-center gap-4">
                        {libraries.length === 0 ? (
                          <span className="text-xs text-muted-foreground">
                            {t("settings.maintenanceRuleLibrariesAll")}
                          </span>
                        ) : null}
                        {libraries.map((library) => (
                          <label
                            key={library.id}
                            className="flex items-center gap-2"
                          >
                            <Checkbox
                              id={selectorId(
                                "settings-maintenance-rule-library",
                                library.id,
                              )}
                              checked={ruleSetDraft.libraryIds.includes(
                                library.id,
                              )}
                              onCheckedChange={(value) => {
                                setRuleSetDraft((prev) => ({
                                  ...prev,
                                  libraryIds:
                                    value === true
                                      ? [...prev.libraryIds, library.id]
                                      : prev.libraryIds.filter(
                                          (id) => id !== library.id,
                                        ),
                                }));
                              }}
                            />
                            <span className="text-sm">{library.name}</span>
                          </label>
                        ))}
                      </div>
                    </div>

                    <MaintenanceRuleAdvanced
                      key={editingRuleSetId ?? "new"}
                      hasNonDefault={
                        ruleSetDraft.subjectKind !== "TITLE" ||
                        Boolean(ruleSetDraft.storageRootId)
                      }
                    >
                      <SingleSelectField
                        id="settings-maintenance-rule-scope"
                        label={t("settings.maintenanceRuleScope")}
                        value={ruleSetDraft.subjectKind}
                        disabled={Boolean(editingRuleSetId)}
                        onValueChange={(value) =>
                          setRuleSetDraft((prev) => {
                            const next = {
                              ...prev,
                              subjectKind:
                                value as MaintenanceRuleSetDraft["subjectKind"],
                            };
                            // The builder writes for one scope, so a builder
                            // rule regenerates for the new one.
                            return prev.builderModel
                              ? withMaintenanceBuilderModel(
                                  next,
                                  prev.builderModel,
                                )
                              : next;
                          })
                        }
                        options={[
                          { value: "TITLE", label: t("label.title") },
                          {
                            value: "SEASON",
                            label: t("settings.maintenanceScopeSeason"),
                          },
                          {
                            value: "EPISODE",
                            label: t("settings.maintenanceScopeEpisode"),
                          },
                        ]}
                      />
                      <div>
                        <SingleSelectField
                          id="settings-maintenance-rule-storage-root"
                          label={
                            <HelpLabel
                              label={t("settings.maintenanceRuleStorageRoot")}
                              help={`${t("settings.maintenanceRuleStorageRootHelp")} ${t("settings.maintenanceRuleStorageRootUnmonitorHelp")}`}
                            />
                          }
                          placeholder={t(
                            "settings.maintenanceRuleStorageRootNone",
                          )}
                          value={ruleSetDraft.storageRootId}
                          onValueChange={(storageRootId) =>
                            setRuleSetDraft((prev) => ({
                              ...prev,
                              storageRootId,
                            }))
                          }
                          options={storageRoots.map((root) => ({
                            value: root.id,
                            label: `${root.libraryName} · ${root.path}`,
                          }))}
                        />
                        {ruleSetDraft.storageRootId ? (
                          <div className="mt-2">
                            <Button
                              id="settings-maintenance-rule-storage-root-clear"
                              type="button"
                              size="sm"
                              variant="secondary"
                              onClick={() =>
                                setRuleSetDraft((prev) => ({
                                  ...prev,
                                  storageRootId: "",
                                }))
                              }
                            >
                              {t("settings.maintenanceRuleStorageRootClear")}
                            </Button>
                          </div>
                        ) : null}
                      </div>
                    </MaintenanceRuleAdvanced>

                    <div className="flex gap-2">
                      <Button
                        id="settings-maintenance-rule-save"
                        type="submit"
                        disabled={mutatingRuleSetId !== null || validating}
                      >
                        {mutatingRuleSetId !== null
                          ? t("label.saving")
                          : editingRuleSetId
                            ? t("settings.maintenanceRuleUpdate")
                            : t("settings.maintenanceRuleCreate")}
                      </Button>
                      <Button
                        id="settings-maintenance-rule-cancel"
                        type="button"
                        variant="secondary"
                        onClick={resetRuleSetDraft}
                      >
                        {t("label.cancel")}
                      </Button>
                    </div>
                  </form>
                </CardContent>
              </Card>
            ) : null}

            <div className="flex justify-center">
              <AddNewButton
                id="settings-maintenance-rule-create"
                icon={Plus}
                label={t("settings.maintenanceRuleCreateNew")}
                onClick={startCreateRuleSet}
                disabled={mutatingRuleSetId !== null}
              />
            </div>

            <MaintenancePreviewPanel
              isEditorOpen={isEditorOpen}
              draftName={ruleSetDraft.name}
              draftScope={ruleSetDraft.subjectKind}
              draftDeclaredActions={validationResult?.declaredActions}
              ruleSetRecords={ruleSetRecords}
              libraries={libraries}
              {...previewProps}
            />

            {operationsPanels}
          </div>
        </div>
        <div className="@container w-full space-y-4 2xl:w-[44%] 2xl:max-w-[880px] 2xl:shrink-0">
          <MaintenanceTemplateGallery onApply={applyTemplate} />
          <MaintenanceContextReference />
        </div>
      </div>
    </div>
  );
}
