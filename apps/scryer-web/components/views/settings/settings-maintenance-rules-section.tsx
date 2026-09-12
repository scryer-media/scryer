import { formatBytes } from "@/lib/utils/activity-utils";
import * as React from "react";
import { MaintenanceSubjectPicker } from "@/components/common/maintenance-subject-picker";
import {
  AlertTriangle,
  BookOpen,
  ChevronDown,
  Copy,
  Edit,
  Info,
  Plus,
  Sparkles,
  Trash2,
} from "lucide-react";
import { AddNewButton } from "@/components/common/add-new-button";
import { RegoAiPromptDialog } from "@/components/common/rego-ai-prompt-dialog";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import { IconButton } from "@/components/ui/icon-button";
import { Input, integerInputProps, sanitizeDigits } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { LazyRegoEditor } from "@/components/common/lazy-rego-editor";
import { MaintenanceActionSequenceEditor } from "@/components/views/settings/maintenance-action-sequence-editor";
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
  MaintenanceActionDescriptor,
  MaintenanceActionSequence,
  MaintenanceActionStepDescriptor,
  MaintenanceEffectArming,
  MaintenanceEvaluationMode,
  MaintenanceInstanceGates,
  MaintenancePreviewResult,
  MaintenancePreviewSource,
  MaintenancePreviewTitle,
  MaintenanceRuleScope,
  MaintenanceTestSubject,
  MaintenanceRuleSetDraft,
  MaintenanceRuleSetRecord,
  MaintenanceValidationResult,
} from "@/lib/types/maintenance-rule-sets";
import {
  MAINTENANCE_PREVIEW_LIMIT_MAX,
  actionKindLabelKey,
  actionStepKindLabelKey,
  armingOptionsFor,
  effectArmingBadgeTone,
  effectArmingLabelKey,
  evaluationModeHelpKey,
  evaluationModeLabelKey,
  maintenanceStatusBanner,
  maintenanceStatusBannerKeys,
  previewOutcomeBadgeTone,
  previewOutcomeLabelKey,
} from "@/lib/utils/maintenance-rule-sets";
import { selectorId } from "@/lib/utils/dom-ids";

export type MaintenanceLibraryOption = { id: string; name: string };
export type MaintenanceQualityProfileOption = { id: string; name: string };
export type MaintenanceStorageRootOption = {
  id: string;
  path: string;
  libraryName: string;
};

const EVALUATION_MODE_OPTIONS: MaintenanceEvaluationMode[] = [
  "DISABLED",
  "SHADOW",
  "OBSERVE",
];

type SettingsMaintenanceRulesSectionProps = {
  isEditorOpen: boolean;
  editingRuleSetId: string | null;
  ruleSetDraft: MaintenanceRuleSetDraft;
  setRuleSetDraft: React.Dispatch<React.SetStateAction<MaintenanceRuleSetDraft>>;
  submitRuleSet: (event: React.FormEvent<HTMLFormElement>) => Promise<void> | void;
  mutatingRuleSetId: string | null;
  resetRuleSetDraft: () => void;
  startCreateRuleSet: () => void;
  /// Load a starter template into the create-rule editor. Nothing is created
  /// here: the container prefills the draft and the operator still saves it.
  applyTemplate: (template: MaintenanceRuleTemplate) => void;
  ruleSetRecords: MaintenanceRuleSetRecord[];
  actionDescriptors: MaintenanceActionDescriptor[];
  actionStepDescriptors: MaintenanceActionStepDescriptor[];
  actionStepDescriptorsLoaded: boolean;
  libraries: MaintenanceLibraryOption[];
  storageRoots: MaintenanceStorageRootOption[];
  qualityProfiles: MaintenanceQualityProfileOption[];
  copyRuleSet: (record: MaintenanceRuleSetRecord) => void;
  editRuleSet: (record: MaintenanceRuleSetRecord) => void;
  deleteRuleSet: (record: MaintenanceRuleSetRecord) => Promise<void> | void;
  validateDraft: () => Promise<MaintenanceValidationResult | null>;
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
  /// Null when the gates query was refused, which on this page means the reader
  /// is not a system administrator and cannot see the instance state at all.
  gates: MaintenanceInstanceGates | null;
  setRuleMode: (record: MaintenanceRuleSetRecord, mode: MaintenanceEvaluationMode) => void;
  setRuleArming: (record: MaintenanceRuleSetRecord, arming: MaintenanceEffectArming) => void;
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
type RefSectionDef = { titleKey: string; path: string; fields: RefField[] };
const REF_SECTIONS = maintenanceInputContract.sections as RefSectionDef[];

/// What this instance will actually do with these rules right now. This used to
/// be a permanent "nothing runs yet" notice; the pipeline is live, so the line
/// is derived from the instance gates and the rules' own modes instead, and it
/// stays above the list where it cannot be scrolled past.
function MaintenanceStatusBanner({
  gates,
  ruleSetRecords,
}: {
  gates: MaintenanceInstanceGates | null;
  ruleSetRecords: MaintenanceRuleSetRecord[];
}) {
  const t = useTranslate();
  const { variant, tone } = maintenanceStatusBanner(gates, ruleSetRecords);
  const { titleKey, bodyKey } = maintenanceStatusBannerKeys(variant);
  const warning = tone === "warning";
  const Icon = warning ? AlertTriangle : Info;

  return (
    <div
      id="settings-maintenance-rules-status-notice"
      data-maintenance-status={variant}
      className={`flex items-start gap-2.5 rounded border px-3 py-2.5 ${
        warning
          ? "border-[var(--scry-warning-border)] bg-[var(--scry-warning-bg)] text-[var(--scry-warning-text)]"
          : "border-[var(--scry-info-border)] bg-[var(--scry-info-bg)] text-[var(--scry-info-text)]"
      }`}
    >
      <Icon className="mt-0.5 h-4 w-4 shrink-0" />
      <div className="space-y-1">
        <p className="font-semibold">{t(titleKey)}</p>
        <p className="text-[13px] leading-5">{t(bodyKey)}</p>
      </div>
    </div>
  );
}

function ArmingBadge({ arming }: { arming: string }) {
  const t = useTranslate();
  const labelKey = effectArmingLabelKey(arming);
  return (
    <Badge tone={effectArmingBadgeTone(arming)}>
      {labelKey ? t(labelKey) : arming}
    </Badge>
  );
}

function ActionLabel({ kind }: { kind: string }) {
  const t = useTranslate();
  const labelKey = actionKindLabelKey(kind);
  return <>{labelKey ? t(labelKey) : kind}</>;
}

function ActionSummary({
  record,
}: {
  record: MaintenanceRuleSetRecord;
}) {
  const t = useTranslate();
  if (!record.actionSequence) {
    return <ActionLabel kind={record.actionSpec.kind} />;
  }
  if (record.actionSequence.steps.length === 0) {
    return <>{t("settings.maintenanceSequenceObserveOnly")}</>;
  }
  const labels = record.actionSequence.steps
    .map((step) => {
      const key = actionStepKindLabelKey(step.kind);
      return key ? t(key) : step.kind;
    })
    .join(" → ");
  return (
    <span className="block max-w-[260px] truncate" title={labels}>
      {labels}
    </span>
  );
}

function ActionSequencePreview({
  sequence,
}: {
  sequence: MaintenanceActionSequence;
}) {
  const t = useTranslate();
  return (
    <div
      id="settings-maintenance-preview-action-sequence"
      className="rounded border border-border bg-muted/30 px-3 py-2"
    >
      <p className="text-xs font-medium">
        {t("settings.maintenanceSequencePreviewPlan")}
      </p>
      {sequence.steps.length === 0 ? (
        <p className="mt-1 text-xs text-muted-foreground">
          {t("settings.maintenanceSequenceObserveOnly")}
        </p>
      ) : (
        <ol className="mt-1 list-decimal space-y-1 pl-5 text-xs text-muted-foreground">
          {sequence.steps.map((step) => {
            const key = actionStepKindLabelKey(step.kind);
            let parameters: string | null = null;
            if (step.kind === "UNMONITOR") {
              parameters = step.parameters.includeDescendants
                ? t("settings.maintenanceSequencePreviewDescendants")
                : t("settings.maintenanceSequencePreviewMatchedScope");
            } else if (step.kind === "CHANGE_QUALITY_PROFILE") {
              parameters = step.parameters.targetQualityProfileId ?? "—";
            } else if (step.kind === "SEARCH") {
              parameters =
                step.parameters.searchCondition === "PREVIOUS_PROFILE_CHANGED"
                  ? t("settings.maintenanceSequenceSearchPreviousProfileChanged")
                  : t("settings.maintenanceSequenceSearchUnconditional");
            } else if (step.kind === "ADD_TAGS" || step.kind === "REMOVE_TAGS") {
              parameters = step.parameters.tags.join(", ") || "—";
            }
            return (
              <li key={step.id}>
                {key ? t(key) : step.kind}
                {parameters ? ` · ${parameters}` : ""}
              </li>
            );
          })}
        </ol>
      )}
    </div>
  );
}

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
        <p className="text-xs text-muted-foreground">
          {t("settings.refMaintSubtitle")}
        </p>
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
              <li>{t("settings.refMaintOutputMatch")}</li>
              <li>{t("settings.refMaintOutputUnknown")}</li>
              <li>{t("settings.refMaintOutputReasons")}</li>
              <li>{t("settings.refMaintOutputNoPackage")}</li>
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
  return (
    <button
      id={selectorId("settings-maintenance-template", template.id)}
      type="button"
      data-maintenance-template-destructive={template.destructive ? "true" : "false"}
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
        <Badge tone="neutral" className="text-[10px]">
          <ActionLabel kind={template.actionKind} />
        </Badge>
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
        <p className="text-xs text-muted-foreground">
          {t("settings.maintenanceTemplateGalleryDescription")}
        </p>
      </CardHeader>
      {open ? (
        <CardContent id="settings-maintenance-template-gallery" className="@container">
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
      <Badge tone="negative">{t("settings.maintenancePreviewOutcomeError")}</Badge>
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
  draftActionSequence,
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
  draftActionSequence: MaintenanceActionSequence;
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
  const storedRule = ruleSetRecords.find((rule) => rule.id === previewRuleSetId);
  const scope = previewSource === "draft" ? draftScope : storedRule?.subjectKind ?? "TITLE";
  const sequence = previewSource === "draft" ? draftActionSequence : storedRule?.actionSequence;
  const [selectedSubject, setSelectedSubject] = React.useState<MaintenanceTestSubject | undefined>();
  const [targetPending, setTargetPending] = React.useState(false);
  const [manualTarget, setManualTarget] = React.useState(false);
  React.useEffect(() => { setSelectedSubject(undefined); setTargetPending(false); setManualTarget(false); }, [previewSource, previewRuleSetId, previewLibraryId, draftName, scope]);
  const canPreviewDraft = isEditorOpen;
  const canRun =
    !previewing && !targetPending &&
    Boolean(previewLibraryId || selectedSubject) &&
    (previewSource === "draft" ? canPreviewDraft : Boolean(previewRuleSetId));

  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-base">
          {t("settings.maintenancePreviewTitle")}
        </CardTitle>
        <p className="text-xs text-muted-foreground">
          {t("settings.maintenancePreviewSubtitle")}
        </p>
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
              placeholder={t("settings.maintenancePreviewRuleSelectPlaceholder")}
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
              {t("settings.maintenancePreviewLimit")}
            </Label>
            <Input
              id="settings-maintenance-preview-limit"
              {...integerInputProps}
              value={previewLimit}
              onChange={(event) =>
                setPreviewLimit(Number(sanitizeDigits(event.target.value)) || 0)
              }
            />
            <p className="mt-1 text-xs text-muted-foreground">
              {t("settings.maintenancePreviewLimitHelp", {
                max: MAINTENANCE_PREVIEW_LIMIT_MAX,
              })}
            </p>
          </label>
        </div>

        {sequence ? <ActionSequencePreview sequence={sequence} /> : null}

        <MaintenanceSubjectPicker key={`${previewSource}:${previewRuleSetId}:${previewLibraryId}:${scope}:${draftName}`}
          scope={scope} onChange={(subject, pending, active) => { setSelectedSubject(subject); setTargetPending(pending); setManualTarget(active); }} />
        {!manualTarget && previewResult?.titles.length ? (
          <SingleSelectField
            id="settings-maintenance-preview-subject"
            label={t("settings.maintenancePreviewSubject")}
            value={selectedSubject?.subjectId ?? "all"}
            onValueChange={(id) => setSelectedSubject(previewResult.titles.find((subject) => subject.subjectId === id))}
            options={[
              { value: "all", label: t("settings.maintenancePreviewSample") },
              ...previewResult.titles.map((subject) => ({ value: subject.subjectId, label: subject.subjectKind === "title" ? subject.titleName : `${subject.titleName} · ${subject.subjectLabel}` })),
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
                time: formatUiDateTime(previewResult.evaluatedAt, dateTimeFormat),
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
                        {title.subjectKind !== "title" && <div className="text-sm text-muted-foreground">{title.subjectLabel}</div>}
                        <div className="text-xs text-muted-foreground">{t("settings.maintenanceFileSummary", { count: title.fileCount, size: formatBytes(title.totalSizeBytes) })}</div>
                        {title.storageRootFileCount !== null &&
                        title.storageRootTotalSizeBytes !== null ? (
                          <div className="text-xs text-muted-foreground">
                            {t("settings.maintenanceStorageRootFileSummary", {
                              count: title.storageRootFileCount,
                              size: formatBytes(title.storageRootTotalSizeBytes),
                            })}
                          </div>
                        ) : null}
                      </TableCell>
                      <TableCell>
                        <PreviewOutcomeCell title={title} />
                        {title.excluded && <div><Badge tone="warning">{t("settings.maintenancePreviewExcluded")}</Badge></div>}
                        {title.dueAt && <div className="mt-1 text-xs text-muted-foreground">{t(title.dueAtIsEstimate ? "settings.maintenancePreviewEstimatedDue" : "settings.maintenancePreviewDue", { time: formatUiDateTime(title.dueAt, dateTimeFormat) })}</div>}
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
                      </TableCell>
                    </TableRow>
                  ))}
                  {previewResult.titles.length === 0 ? (
                    <TableRow>
                      <TableCell colSpan={3} className="text-muted-foreground">
                        {t("settings.maintenancePreviewNoTitles")}
                      </TableCell>
                    </TableRow>
                  ) : null}
                </TableBody>
              </Table>
            </div>
          </div>
        ) : previewError ? null : (
          <p className="text-xs text-muted-foreground">
            {t("settings.maintenancePreviewEmpty")}
          </p>
        )}
      </CardContent>
    </Card>
  );
}

export function SettingsMaintenanceRulesSection({
  isEditorOpen,
  editingRuleSetId,
  ruleSetDraft,
  setRuleSetDraft,
  submitRuleSet,
  mutatingRuleSetId,
  resetRuleSetDraft,
  startCreateRuleSet,
  applyTemplate,
  ruleSetRecords,
  actionDescriptors,
  actionStepDescriptors,
  actionStepDescriptorsLoaded,
  libraries,
  storageRoots,
  qualityProfiles,
  copyRuleSet,
  editRuleSet,
  deleteRuleSet,
  validateDraft,
  validating,
  validationResult,
  gates,
  setRuleMode,
  setRuleArming,
  section,
  operationsPanels,
  ...previewProps
}: SettingsMaintenanceRulesSectionProps) {
  const t = useTranslate();

  // Candidates, history and gates are one panel each. They keep the status
  // banner, because what the instance is allowed to do explains every row they
  // show, but they get the full column: there is no editor to sit beside and no
  // rule being written for the reference rail to describe.
  if (section !== "rules") {
    return (
      <div id="settings-maintenance-rules-section" className="space-y-4 text-sm">
        <div className="mx-auto w-full max-w-[1600px] space-y-4">
          <MaintenanceStatusBanner gates={gates} ruleSetRecords={ruleSetRecords} />
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
            <MaintenanceStatusBanner
              gates={gates}
              ruleSetRecords={ruleSetRecords}
            />

            <div className="overflow-hidden rounded border border-border bg-card">
              <div className="flex items-center justify-between border-b border-border px-3 py-2">
                <CardTitle className="text-base text-foreground">
                  {t("settings.maintenanceRules")}
                </CardTitle>
                <RegoAiPromptDialog
                  id="settings-maintenance-rules-ai-prompt"
                  ruleKind="maintenance"
                  inputContract={maintenanceInputContract}
                  outputContract={'Define match if { ... }. You may also add reasons contains "stable_reason" if { match }.'}
                />
              </div>
              <div className="overflow-x-auto">
                <Table className="min-w-[min(900px,100%)]">
                  <TableHeader>
                    <TableRow>
                      <TableHead>{t("label.name")}</TableHead>
                      <TableHead>{t("settings.ruleDescription")}</TableHead>
                      <TableHead>{t("settings.maintenanceRuleAction")}</TableHead>
                      <TableHead className="text-center">
                        {t("settings.maintenanceRuleGraceDays")}
                      </TableHead>
                      <TableHead className="w-[190px]">
                        {t("settings.maintenanceRuleMode")}
                      </TableHead>
                      <TableHead className="w-[210px]">
                        {t("settings.maintenanceRuleArming")}
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
                      const sequenceAllowsFileDeletion =
                        record.actionSequence?.steps.some((step) =>
                          actionStepDescriptors
                            .find((descriptor) => descriptor.kind === step.kind)
                            ?.effectClasses.includes("DELETE_FILES"),
                        ) ?? false;
                      const modeHelpKey = evaluationModeHelpKey(record.evaluationMode);
                      const armingOptions = record.actionSequence
                        ? sequenceAllowsFileDeletion
                          ? ["NONE", "REVERSIBLE", "DESTRUCTIVE"]
                          : ["NONE", "REVERSIBLE"]
                        : armingOptionsFor(actionDescriptors, record.actionSpec.kind);
                      return (
                        <TableRow
                          data-ui="settings-table-row"
                          key={record.id}
                          id={selectorId("settings-maintenance-rule-row", record.id)}
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
                            {record.subjectKind !== "TITLE" && <div className="text-xs text-muted-foreground">{t(record.subjectKind === "SEASON" ? "settings.maintenanceScopeSeason" : "settings.maintenanceScopeEpisode")}</div>}
                          </TableCell>
                          <TableCell className="max-w-[200px] truncate text-muted-foreground">
                            {record.description || "—"}
                          </TableCell>
                          <TableCell>
                            <ActionSummary record={record} />
                          </TableCell>
                          <TableCell className="text-center">
                            {record.graceDays}
                          </TableCell>
                          <TableCell>
                            <div className="space-y-1">
                              <SingleSelectField
                                id={selectorId(
                                  "settings-maintenance-rule-mode",
                                  record.id,
                                )}
                                size="sm"
                                value={record.evaluationMode}
                                onValueChange={(value) =>
                                  setRuleMode(
                                    record,
                                    value as MaintenanceEvaluationMode,
                                  )
                                }
                                options={EVALUATION_MODE_OPTIONS.map((mode) => {
                                  const key = evaluationModeLabelKey(mode);
                                  return {
                                    value: mode,
                                    label: key ? t(key) : mode,
                                  };
                                })}
                              />
                              {modeHelpKey ? (
                                <p className="text-xs text-muted-foreground">
                                  {t(modeHelpKey)}
                                </p>
                              ) : null}
                            </div>
                          </TableCell>
                          <TableCell>
                            <div className="space-y-1">
                              <SingleSelectField
                                id={selectorId(
                                  "settings-maintenance-rule-arming",
                                  record.id,
                                )}
                                size="sm"
                                value={record.effectArming}
                                onValueChange={(value) =>
                                  setRuleArming(
                                    record,
                                    value as MaintenanceEffectArming,
                                  )
                                }
                                options={armingOptions.map((arming) => {
                                  const key = effectArmingLabelKey(arming);
                                  return {
                                    value: arming,
                                    ariaLabel: arming,
                                    label: key ? t(key) : arming,
                                  };
                                })}
                              />
                              <ArmingBadge arming={record.effectArming} />
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
                            </div>
                          </TableCell>
                        </TableRow>
                      );
                    })}
                    {ruleSetRecords.length === 0 ? (
                      <TableRow>
                        <TableCell colSpan={9} className="text-muted-foreground">
                          {t("settings.noMaintenanceRulesFound")}
                        </TableCell>
                      </TableRow>
                    ) : null}
                  </TableBody>
                </Table>
              </div>
              <p className="border-t border-border px-3 py-2 text-xs text-muted-foreground">
                {t("settings.maintenanceRuleArmingHelp")}
              </p>
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

                    <SingleSelectField
                      id="settings-maintenance-rule-scope"
                      label={t("settings.maintenanceRuleScope")}
                      value={ruleSetDraft.subjectKind}
                      disabled={Boolean(editingRuleSetId)}
                      onValueChange={(value) => setRuleSetDraft((prev) => ({
                        ...prev,
                        subjectKind: value as MaintenanceRuleSetDraft["subjectKind"],
                      }))}
                      options={[
                        { value: "TITLE", label: t("label.title") },
                        { value: "SEASON", label: t("settings.maintenanceScopeSeason") },
                        { value: "EPISODE", label: t("settings.maintenanceScopeEpisode") },
                      ]}
                    />
                    {!actionStepDescriptorsLoaded ? (
                      <p
                        id="settings-maintenance-sequence-catalog-unavailable"
                        className="rounded border border-[var(--scry-warning-border)] bg-[var(--scry-warning-bg)] px-3 py-2 text-xs text-[var(--scry-warning-text)]"
                      >
                        {t("settings.maintenanceSequenceCatalogUnavailable")}
                      </p>
                    ) : null}
                    <MaintenanceActionSequenceEditor
                      sequence={ruleSetDraft.actionSequence}
                      descriptors={actionStepDescriptors}
                      subjectKind={ruleSetDraft.subjectKind}
                      storageRootId={ruleSetDraft.storageRootId}
                      qualityProfiles={qualityProfiles}
                      onChange={(actionSequence) =>
                        setRuleSetDraft((prev) => ({ ...prev, actionSequence }))
                      }
                    />

                    <label className="block md:max-w-[50%]">
                      <Label className="mb-2 block">
                        {t("settings.maintenanceRuleGraceDays")}
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
                      <p className="mt-1 text-xs text-muted-foreground">
                        {t("settings.maintenanceRuleGraceDaysHelp")}
                      </p>
                    </label>

                    <div>
                      <SingleSelectField
                        id="settings-maintenance-rule-storage-root"
                        label={t("settings.maintenanceRuleStorageRoot")}
                        placeholder={t("settings.maintenanceRuleStorageRootNone")}
                        value={ruleSetDraft.storageRootId}
                        onValueChange={(storageRootId) =>
                          setRuleSetDraft((prev) => ({ ...prev, storageRootId }))
                        }
                        options={storageRoots.map((root) => ({
                          value: root.id,
                          label: `${root.libraryName} · ${root.path}`,
                        }))}
                      />
                      <p className="mt-1 text-xs text-muted-foreground">
                        {t("settings.maintenanceRuleStorageRootHelp")}
                      </p>
                      {ruleSetDraft.storageRootId ? (
                        <div className="mt-1 space-y-1">
                          <p
                            id="settings-maintenance-rule-storage-root-retention"
                            className="text-xs text-[var(--scry-warning-text)]"
                          >
                            {t("settings.maintenanceRuleStorageRootUnmonitorHelp")}
                          </p>
                          <Button
                            id="settings-maintenance-rule-storage-root-clear"
                            type="button"
                            size="sm"
                            variant="secondary"
                            onClick={() =>
                              setRuleSetDraft((prev) => ({ ...prev, storageRootId: "" }))
                            }
                          >
                            {t("settings.maintenanceRuleStorageRootClear")}
                          </Button>
                        </div>
                      ) : null}
                    </div>

                    <div>
                      <Label className="mb-2 block">
                        {t("settings.ruleRegoSource")}
                      </Label>
                      <LazyRegoEditor
                        id="settings-maintenance-rule-rego-source"
                        value={ruleSetDraft.regoSource}
                        onChange={(value) =>
                          setRuleSetDraft((prev) => ({
                            ...prev,
                            regoSource: value,
                          }))
                        }
                        minLines={12}
                        maxLines={35}
                      />
                    </div>

                    <div>
                      <Label className="mb-2 block">
                        {t("settings.maintenanceRuleLibraries")}
                      </Label>
                      <p className="mb-2 text-xs text-muted-foreground">
                        {t("settings.maintenanceRuleLibrariesHelp")}
                      </p>
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
                              checked={ruleSetDraft.libraryIds.includes(library.id)}
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

                    {validationResult ? (
                      <div
                        id="settings-maintenance-rule-validation"
                        className={`rounded border px-3 py-2 text-sm ${
                          validationResult.valid
                            ? "border-[var(--scry-success-border)] bg-[var(--scry-success-bg)] text-[var(--scry-success-text)]"
                            : "border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)] text-[var(--scry-danger-text)]"
                        }`}
                      >
                        {validationResult.valid ? (
                          t("settings.ruleValid")
                        ) : (
                          <div className="space-y-2">
                            {validationResult.errors.map((error, index) => (
                              <pre
                                key={index}
                                className="overflow-x-auto whitespace-pre-wrap rounded-[9px] border border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)] p-3 font-mono text-[12px] leading-5 text-[var(--scry-danger-text)]"
                              >
                                {error}
                              </pre>
                            ))}
                          </div>
                        )}
                      </div>
                    ) : null}

                    <p className="text-xs text-muted-foreground">
                      {t("settings.maintenanceRuleSaveDisabledNote")}
                    </p>

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
                        id="settings-maintenance-rule-validate"
                        type="button"
                        variant="secondary"
                        onClick={() => void validateDraft()}
                        disabled={validating || !ruleSetDraft.regoSource.trim()}
                      >
                        {validating
                          ? t("settings.ruleValidating")
                          : t("settings.ruleValidate")}
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
              draftActionSequence={ruleSetDraft.actionSequence}
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
