import {
  formatRegoValidationError,
  regoDiagnostics,
} from "@/lib/utils/rego-diagnostics";
import { formatBytes } from "@/lib/utils/activity-utils";
import * as React from "react";
import { LifecycleSubjectPicker } from "@/components/common/lifecycle-subject-picker";
import {
  BookOpen,
  ChevronDown,
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
  LifecycleDecidedActionsSummary,
  LifecycleDeclaredActionsSummary,
  LifecycleRuleSourceFindings,
} from "@/components/views/settings/lifecycle-action-summary";
import { SingleSelectField } from "@/components/ui/select";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import type { LifecycleRulesSection } from "@/components/root/types";
import lifecycleInputContract from "@/lib/contracts/lifecycle-input-contract.json";
import {
  LIFECYCLE_RULE_TEMPLATES,
  lifecycleTemplateFacetLabelKey,
  type LifecycleRuleTemplate,
} from "@/lib/constants/lifecycle-rule-templates";
import { useUiDateTimeFormat } from "@/lib/context/ui-settings-context";
import { formatUiDateTime } from "@/lib/utils/date-format";
import { useTranslate } from "@/lib/context/translate-context";
import type {
  LifecycleDeclaredActions,
  LifecyclePreviewResult,
  LifecyclePreviewSource,
  LifecyclePreviewTitle,
  LifecycleTestSubject,
  LifecycleRuleSetDraft,
  LifecycleRuleSetRecord,
  LifecycleRuleState,
  LifecycleValidationResult,
} from "@/lib/types/lifecycle-rule-sets";
import {
  LIFECYCLE_PREVIEW_LIMIT_MAX,
  previewOutcomeBadgeTone,
  previewOutcomeLabelKey,
  ruleStateLabelKey,
  ruleStateOptionsFor,
  lifecycleLoadRefusalKey,
  lifecyclePreviewErrorText,
} from "@/lib/utils/lifecycle-rule-sets";
import { useRuleOutputDescriptors } from "@/lib/hooks/use-rule-builder-schema";
import {
  ruleOutputHeadHelp,
  ruleOutputHeadLabel,
  ruleOutputHeadNames,
  ruleOutputSyntax,
  templateRuleOutputs,
} from "@/lib/utils/rule-output-vocabulary";
import { selectorId } from "@/lib/utils/dom-ids";
import type { RuleBuilderOptions } from "@/components/views/settings/rule-builder";
import type { RuleBuilderSchema } from "@/lib/utils/rule-builder";
import type { RuleBuilderSchemaStatus } from "@/lib/hooks/use-rule-builder-schema";
import { RuleEditorSurface } from "@/components/views/settings/rule-editor-surface";
import type { RuleBuilderModel } from "@/lib/utils/rule-builder";
import type { RuleEditorMode } from "@/lib/utils/rule-builder-mode";
import {
  LifecycleRuleScheduleField,
  LifecycleRuleScheduleSummary,
} from "@/components/views/settings/lifecycle-rule-schedule";
import { toScriptScheduleInput } from "@/lib/utils/script-schedule";

export type LifecycleLibraryOption = { id: string; name: string };

type SettingsLifecycleRulesSectionProps = {
  isEditorOpen: boolean;
  editingRuleSetId: string | null;
  ruleSetDraft: LifecycleRuleSetDraft;
  setRuleSetDraft: React.Dispatch<
    React.SetStateAction<LifecycleRuleSetDraft>
  >;
  /// Changes with every editor opening; controls that keep their own typing
  /// state are keyed on it.
  editorSessionKey: number;
  /// Which editor the rule form shows. Simple is only offered while the draft
  /// is still the builder's.
  editorMode: RuleEditorMode;
  setEditorMode: (mode: "simple" | "view") => void;
  /// A hand edit of the source; the container decides whether it detaches
  /// the rule from the builder.
  editRegoSource: (source: string) => void;
  setBuilderModel: (model: RuleBuilderModel) => void;
  /// Set once a hand edit in this editor session detached the rule.
  editAsRego: () => void;
  builderOptions: RuleBuilderOptions;
  /// Null until the served outputs are read, and after a failed read.
  builderSchema: RuleBuilderSchema | null;
  builderStatus: RuleBuilderSchemaStatus;
  submitRuleSet: (
    event: React.FormEvent<HTMLFormElement>,
  ) => Promise<void> | void;
  mutatingRuleSetId: string | null;
  resetRuleSetDraft: () => void;
  startCreateRuleSet: () => void;
  /// Load a starter template into the create-rule editor. Nothing is created
  /// here: the container prefills the draft and the operator still saves it.
  applyTemplate: (template: LifecycleRuleTemplate) => void;
  ruleSetRecords: LifecycleRuleSetRecord[];
  libraries: LifecycleLibraryOption[];
  copyRuleSet: (record: LifecycleRuleSetRecord) => void;
  editRuleSet: (record: LifecycleRuleSetRecord) => void;
  deleteRuleSet: (record: LifecycleRuleSetRecord) => Promise<void> | void;
  validating: boolean;
  validationResult: LifecycleValidationResult | null;
  previewSource: LifecyclePreviewSource;
  setPreviewSource: (source: LifecyclePreviewSource) => void;
  previewRuleSetId: string;
  setPreviewRuleSetId: (id: string) => void;
  previewLibraryId: string;
  setPreviewLibraryId: (id: string) => void;
  previewLimit: number;
  setPreviewLimit: (limit: number) => void;
  runPreview: (subject?: LifecycleTestSubject) => Promise<void> | void;
  previewing: boolean;
  previewResult: LifecyclePreviewResult | null;
  previewError: string | null;
  /// The two instance-wide switches, rendered by the container above every
  /// pane so what the instance may do is never scrolled past.
  instanceSwitches: React.ReactNode;
  setRuleState: (
    record: LifecycleRuleSetRecord,
    state: LifecycleRuleState,
  ) => void;
  /// Which pane of the lifecycle gutter is open. Only the rule list wants the
  /// editor, the preview and the reference rail; the operational panes are a
  /// single panel each and read better across the full width.
  section: LifecycleRulesSection;
  /// Panels rendered under the rules table. They are passed in rather than
  /// rendered here so the container owns their queries and this view stays a
  /// pure rendering of what it is handed.
  operationsPanels: React.ReactNode;
};

type RefField = { field: string; type: string; descKey: string };

type RefSectionDef = { titleKey: string; path: string; fields: RefField[] };
const REF_SECTIONS = lifecycleInputContract.sections as RefSectionDef[];

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

function LifecycleContextReference() {
  const t = useTranslate();
  const [open, setOpen] = React.useState(false);
  const descriptors = useRuleOutputDescriptors("lifecycle");

  return (
    <Card>
      <CardHeader
        className="cursor-pointer select-none"
        onClick={() => setOpen((prev) => !prev)}
      >
        <CardTitle className="flex items-center gap-2 text-base">
          <BookOpen className="h-4 w-4" />
          {t("settings.refLifecycleTitle")}
          <ChevronDown
            className={`ml-auto h-4 w-4 transition-transform ${open ? "rotate-180" : ""}`}
          />
        </CardTitle>
      </CardHeader>
      {open ? (
        <CardContent
          id="settings-lifecycle-rules-reference"
          className="space-y-6 text-sm"
        >
          <p className="text-muted-foreground">{t("settings.refLifecycleIntro")}</p>

          <div>
            <h4 className="mb-1 font-semibold">
              {t("settings.refLifecycleOutputTitle")}
            </h4>
            <p className="mb-2 text-muted-foreground">
              {t("settings.refLifecycleOutputIntro")}
            </p>
            <ul className="list-disc space-y-1.5 pl-5 text-muted-foreground">
              <li>{t("settings.refLifecycleOutputFlagHeads")}</li>
              <li>{t("settings.refLifecycleOutputValueHeads")}</li>
              <li>{t("settings.refLifecycleOutputHelpers")}</li>
              <li>{t("settings.refLifecycleOutputUnknown")}</li>
              <li>{t("settings.refLifecycleOutputReasons")}</li>
              <li>{t("settings.refLifecycleOutputNoPackage")}</li>
            </ul>
          </div>

          <div>
            <h4 className="mb-1 font-semibold">
              <HelpLabel
                label={t("settings.refLifecycleActionsTitle")}
                help={t("settings.refLifecycleActionsOrder", {
                  order: ruleOutputHeadNames(descriptors),
                })}
              />
            </h4>
            <Table id="settings-lifecycle-rules-reference-actions">
              <TableHeader>
                <TableRow>
                  <TableHead className="w-[260px]">
                    {t("settings.refColField")}
                  </TableHead>
                  <TableHead>{t("settings.refColDescription")}</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {descriptors.map((descriptor) => {
                  const help = ruleOutputHeadHelp(t, descriptor);
                  return (
                    <TableRow
                      key={descriptor.id}
                      id={selectorId(
                        "settings-lifecycle-rules-reference-action",
                        descriptor.id,
                      )}
                      data-ui="settings-table-row"
                    >
                      <TableCell>
                        <code data-code-font className="text-xs">
                          {ruleOutputSyntax(descriptor)}
                        </code>
                      </TableCell>
                      <TableCell className="text-muted-foreground">
                        {ruleOutputHeadLabel(t, descriptors, descriptor.id)}
                        {help ? (
                          <span className="block text-xs">{help}</span>
                        ) : null}
                      </TableCell>
                    </TableRow>
                  );
                })}
              </TableBody>
            </Table>
            <ul className="mt-2 list-disc space-y-1.5 pl-5 text-muted-foreground">
              <li>{t("settings.refLifecycleActionsDaysSince")}</li>
              <li>{t("settings.refLifecycleActionsNotify")}</li>
              <li>{t("settings.refLifecycleActionsConflicts")}</li>
            </ul>
          </div>

          <div>
            <h4 className="mb-1 font-semibold">
              {t("settings.refLifecycleTagActionsTitle")}
            </h4>
            <ul className="list-disc space-y-1.5 pl-5 text-muted-foreground">
              <li>{t("settings.refLifecycleTagActionsNote")}</li>
              <li>{t("settings.refLifecycleTagOscillationNote")}</li>
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
function LifecycleTemplateCard({
  template,
  onApply,
}: {
  template: LifecycleRuleTemplate;
  onApply: (template: LifecycleRuleTemplate) => void;
}) {
  const t = useTranslate();
  const descriptors = useRuleOutputDescriptors("lifecycle");
  const heads = templateRuleOutputs(template.regoSource, descriptors);
  return (
    <button
      id={selectorId("settings-lifecycle-template", template.id)}
      type="button"
      data-lifecycle-template-destructive={
        template.destructive ? "true" : "false"
      }
      className="group flex flex-col rounded-lg border border-border bg-card/50 p-3 text-left transition-colors hover:border-primary/40 hover:bg-card"
      onClick={() => onApply(template)}
      title={t("settings.lifecycleTemplateApply")}
    >
      <p className="text-sm font-medium text-foreground group-hover:text-primary">
        {t(template.titleKey)}
      </p>
      <p className="mt-1 text-xs leading-5 text-muted-foreground">
        {t(template.descriptionKey)}
      </p>
      <div className="mt-2 flex flex-wrap items-center gap-1.5">
        {heads.map((head) => (
          <Badge key={head} tone="neutral" className="text-[10px]">
            {ruleOutputHeadLabel(t, descriptors, head)}
          </Badge>
        ))}
        {template.requiresTargetQualityProfile ? (
          <Badge tone="warning" className="text-[10px]">
            {t("settings.lifecycleTemplateNeedsProfileBadge")}
          </Badge>
        ) : null}
        <Badge tone="neutral" className="text-[10px]">
          {template.graceDays > 0
            ? t("settings.lifecycleTemplateGraceBadge", {
                count: template.graceDays,
              })
            : t("settings.lifecycleTemplateNoGraceBadge")}
        </Badge>
        {template.subjectFacets.map((facet) => (
          <Badge key={facet} tone="info" className="text-[10px]">
            {t(lifecycleTemplateFacetLabelKey(facet))}
          </Badge>
        ))}
      </div>
    </button>
  );
}

function LifecycleTemplateGallery({
  onApply,
}: {
  onApply: (template: LifecycleRuleTemplate) => void;
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
          {t("settings.lifecycleTemplateGallery")}
          <ChevronDown
            className={`ml-auto h-4 w-4 transition-transform ${open ? "rotate-180" : ""}`}
          />
        </CardTitle>
      </CardHeader>
      {open ? (
        <CardContent
          id="settings-lifecycle-template-gallery"
          className="@container"
        >
          <div className="grid grid-cols-1 gap-2 @[560px]:grid-cols-2 @[900px]:grid-cols-3">
            {LIFECYCLE_RULE_TEMPLATES.map((template) => (
              <LifecycleTemplateCard
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

function PreviewOutcomeCell({ title }: { title: LifecyclePreviewTitle }) {
  const t = useTranslate();
  if (title.error) {
    return (
      <Badge tone="negative">
        {t("settings.lifecyclePreviewOutcomeError")}
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

function LifecyclePreviewPanel({
  isEditorOpen,
  draftName,
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
  /// What the draft declares, from its latest validation.
  draftDeclaredActions: LifecycleDeclaredActions | null | undefined;
} & Pick<
  SettingsLifecycleRulesSectionProps,
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
  const declared =
    previewSource === "draft"
      ? draftDeclaredActions
      : storedRule?.declaredActions;
  const [selectedSubject, setSelectedSubject] = React.useState<
    LifecycleTestSubject | undefined
  >();
  const [targetPending, setTargetPending] = React.useState(false);
  const [manualTarget, setManualTarget] = React.useState(false);
  React.useEffect(() => {
    setSelectedSubject(undefined);
    setTargetPending(false);
    setManualTarget(false);
  }, [previewSource, previewRuleSetId, previewLibraryId, draftName]);
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
            label={t("settings.lifecyclePreviewTitle")}
            help={t("settings.lifecyclePreviewSubtitle")}
          />
        </CardTitle>
      </CardHeader>
      <CardContent className="space-y-3">
        <div className="grid gap-3 md:grid-cols-4">
          <SingleSelectField
            id="settings-lifecycle-preview-source"
            label={t("settings.lifecyclePreviewSource")}
            value={previewSource}
            onValueChange={(value) =>
              setPreviewSource(value as LifecyclePreviewSource)
            }
            options={[
              {
                value: "stored",
                label: t("settings.lifecyclePreviewSourceStored"),
              },
              {
                value: "draft",
                label: t("settings.lifecyclePreviewSourceDraft"),
                disabled: !canPreviewDraft,
              },
            ]}
          />
          {previewSource === "stored" ? (
            <SingleSelectField
              id="settings-lifecycle-preview-rule"
              label={t("settings.lifecyclePreviewRuleSelect")}
              placeholder={t(
                "settings.lifecyclePreviewRuleSelectPlaceholder",
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
                {t("settings.lifecyclePreviewRuleSelect")}
              </Label>
              <p
                id="settings-lifecycle-preview-draft-name"
                className="truncate text-xs text-muted-foreground"
              >
                {draftName || t("settings.lifecyclePreviewSourceDraft")}
              </p>
            </div>
          )}
          <SingleSelectField
            id="settings-lifecycle-preview-library"
            label={t("settings.lifecyclePreviewLibrary")}
            placeholder={t("settings.lifecyclePreviewLibraryPlaceholder")}
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
                label={t("settings.lifecyclePreviewLimit")}
                help={t("settings.lifecyclePreviewLimitHelp", {
                  max: LIFECYCLE_PREVIEW_LIMIT_MAX,
                })}
              />
            </Label>
            <Input
              id="settings-lifecycle-preview-limit"
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
            id="settings-lifecycle-preview-declared-actions"
            className="rounded border border-border bg-muted/30 px-3 py-2"
          >
            <p className="mb-1 text-xs font-medium">
              {t("settings.lifecycleDeclaredActionsTitle")}
            </p>
            <LifecycleDeclaredActionsSummary
              declared={declared}
              idPrefix="settings-lifecycle-preview-declared-action"
            />
          </div>
        ) : null}

        <LifecycleSubjectPicker
          key={`${previewSource}:${previewRuleSetId}:${previewLibraryId}:${draftName}`}
          scope="TITLE"
          onChange={(subject, pending, active) => {
            setSelectedSubject(subject);
            setTargetPending(pending);
            setManualTarget(active);
          }}
        />
        {!manualTarget && previewResult?.titles.length ? (
          <SingleSelectField
            id="settings-lifecycle-preview-subject"
            label={t("settings.lifecyclePreviewSubject")}
            value={selectedSubject?.subjectId ?? "all"}
            onValueChange={(id) =>
              setSelectedSubject(
                previewResult.titles.find(
                  (subject) => subject.subjectId === id,
                ),
              )
            }
            options={[
              { value: "all", label: t("settings.lifecyclePreviewSample") },
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
          id="settings-lifecycle-preview-run"
          type="button"
          variant="secondary"
          onClick={() => void runPreview(selectedSubject)}
          disabled={!canRun}
        >
          {previewing
            ? t("settings.lifecyclePreviewRunning")
            : t("settings.lifecyclePreviewRun")}
        </Button>

        {previewError ? (
          <pre className="overflow-x-auto whitespace-pre-wrap rounded-[9px] border border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)] p-3 font-mono text-[12px] leading-5 text-[var(--scry-danger-text)]">
            {previewError}
          </pre>
        ) : null}

        {previewResult ? (
          <div className="space-y-2">
            <p className="text-xs text-muted-foreground">
              {t("settings.lifecyclePreviewMeta", {
                time: formatUiDateTime(
                  previewResult.evaluatedAt,
                  dateTimeFormat,
                ),
                hash: previewResult.matcherContentHash.slice(0, 12),
              })}
            </p>
            <div className="overflow-x-auto">
              <Table id="settings-lifecycle-preview-results">
                <TableHeader>
                  <TableRow>
                    <TableHead>{t("label.title")}</TableHead>
                    <TableHead className="w-[140px]">
                      {t("settings.lifecyclePreviewColOutcome")}
                    </TableHead>
                    <TableHead>{t("settings.lifecycleRuleAction")}</TableHead>
                    <TableHead>
                      {t("settings.lifecyclePreviewColReasons")}
                    </TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {previewResult.titles.map((title) => (
                    <TableRow
                      key={`${title.subjectKind}:${title.subjectId}`}
                      data-ui="settings-table-row"
                      id={selectorId(
                        "settings-lifecycle-preview-row",
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
                          {t("settings.lifecycleFileSummary", {
                            count: title.fileCount,
                            size: formatBytes(title.totalSizeBytes),
                          })}
                        </div>
                      </TableCell>
                      <TableCell>
                        <PreviewOutcomeCell title={title} />
                        {title.excluded && (
                          <div>
                            <Badge tone="warning">
                              {t("settings.lifecyclePreviewExcluded")}
                            </Badge>
                          </div>
                        )}
                        {title.dueAt && (
                          <div className="mt-1 text-xs text-muted-foreground">
                            {t(
                              title.dueAtIsEstimate
                                ? "settings.lifecyclePreviewEstimatedDue"
                                : "settings.lifecyclePreviewDue",
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
                          <LifecycleDecidedActionsSummary
                            actions={title.actions}
                            idPrefix={selectorId(
                              "settings-lifecycle-preview-action",
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
                            {lifecyclePreviewErrorText(title, t)}
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
                        {t("settings.lifecyclePreviewNoTitles")}
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
        {t("settings.lifecyclePreviewFacts")}
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

export function SettingsLifecycleRulesSection({
  isEditorOpen,
  editingRuleSetId,
  ruleSetDraft,
  setRuleSetDraft,
  editorSessionKey,
  editorMode,
  setEditorMode,
  editRegoSource,
  setBuilderModel,
  editAsRego,
  builderOptions,
  builderSchema,
  builderStatus,
  submitRuleSet,
  mutatingRuleSetId,
  resetRuleSetDraft,
  startCreateRuleSet,
  applyTemplate,
  ruleSetRecords,
  libraries,
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
}: SettingsLifecycleRulesSectionProps) {
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
  const validationStatus = (
    <EditorValidationStatus
      id="settings-lifecycle-rule-validation"
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
        id="settings-lifecycle-rules-section"
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
    <div id="settings-lifecycle-rules-section" className="space-y-4 text-sm">
      <div className="mx-auto flex w-full max-w-[2176px] flex-col gap-4 2xl:flex-row 2xl:items-start">
        <div className="min-w-0 basis-[900px] flex-1">
          <div className="mx-auto w-full max-w-[1280px] space-y-4">
            {instanceSwitches}

            <div className="overflow-hidden rounded border border-border bg-card">
              <div className="flex items-center justify-between border-b border-border px-3 py-2">
                <CardTitle className="text-base text-foreground">
                  {t("settings.lifecycleRules")}
                </CardTitle>
                <RegoAiPromptDialog
                  id="settings-lifecycle-rules-ai-prompt"
                  ruleKind="lifecycle"
                  inputContract={lifecycleInputContract}
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
                      <TableHead className="text-center">
                        {t("settings.lifecycleRuleGraceDays")}
                      </TableHead>
                      <TableHead>
                        <HelpLabel
                          label={t("jobs.column.schedule")}
                          help={t("settings.lifecycleRuleScheduleHelp")}
                        />
                      </TableHead>
                      <TableHead className="w-[240px]">
                        <HelpLabel
                          label={t("settings.lifecycleRuleState")}
                          help={t("settings.lifecycleRuleStateHelp")}
                        />
                      </TableHead>
                      <TableHead className="text-center">
                        {t("settings.lifecycleRuleRevision")}
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
                            "settings-lifecycle-rule-row",
                            record.id,
                          )}
                        >
                          <TableCell className="font-medium">
                            <span
                              id={selectorId(
                                "settings-lifecycle-rule-name",
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
                                {t("settings.lifecycleRuleSystemOwned")}
                              </Badge>
                            ) : null}
                            {record.loadRefusal ? (
                              <p
                                id={selectorId(
                                  "settings-lifecycle-rule-load-refusal",
                                  record.id,
                                )}
                                className="mt-1 text-xs text-[var(--scry-danger-text)]"
                              >
                                {t(
                                  lifecycleLoadRefusalKey(record.loadRefusal),
                                )}
                              </p>
                            ) : null}
                          </TableCell>
                          <TableCell className="max-w-[200px] truncate text-muted-foreground">
                            {record.description || "—"}
                          </TableCell>
                          <TableCell className="text-center">
                            {record.graceDays}
                          </TableCell>
                          <TableCell className="min-w-[10rem] text-muted-foreground">
                            <LifecycleRuleScheduleSummary
                              record={record}
                              idPrefix={selectorId(
                                "settings-lifecycle-rule-schedule",
                                record.id,
                              )}
                            />
                          </TableCell>
                          <TableCell>
                            <div className="space-y-1">
                              <SingleSelectField
                                id={selectorId(
                                  "settings-lifecycle-rule-state",
                                  record.id,
                                )}
                                size="sm"
                                value={record.state}
                                disabled={mutatingRuleSetId === record.id}
                                onValueChange={(value) =>
                                  setRuleState(
                                    record,
                                    value as LifecycleRuleState,
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
                                    "settings-lifecycle-rule-edited-since-enabled",
                                    record.id,
                                  )}
                                  className="rounded border border-[var(--scry-warning-border)] bg-[var(--scry-warning-bg)] px-2 py-1 text-xs text-[var(--scry-warning-text)]"
                                >
                                  {t(
                                    "settings.lifecycleRuleEditedSinceEnabled",
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
                                  "settings-lifecycle-rule-copy",
                                  record.id,
                                )}
                                label={t("settings.lifecycleRuleCopy")}
                                tone="neutral"
                                onClick={() => copyRuleSet(record)}
                              >
                                <Copy className="h-4 w-4" />
                              </IconButton>
                              <IconButton
                                id={selectorId(
                                  "settings-lifecycle-rule-edit",
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
                                    "settings-lifecycle-rule-delete",
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
                          {t("settings.noLifecycleRulesFound")}
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
                      ? t("settings.lifecycleRuleUpdate")
                      : t("settings.lifecycleRuleCreate")}
                  </CardTitle>
                </CardHeader>
                <CardContent>
                  <form
                    id="settings-lifecycle-rule-form"
                    className="space-y-3"
                    onSubmit={submitRuleSet}
                  >
                    <div className="grid gap-3 md:grid-cols-2">
                      <label>
                        <Label className="mb-2 block">{t("label.name")}</Label>
                        <Input
                          id="settings-lifecycle-rule-name"
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
                          id="settings-lifecycle-rule-description"
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
                        id="settings-lifecycle-rule-template"
                        label={t("settings.lifecycleRuleTemplate")}
                        placeholder={t(
                          "settings.lifecycleRuleTemplatePlaceholder",
                        )}
                        value=""
                        onValueChange={(id) => {
                          const template = LIFECYCLE_RULE_TEMPLATES.find(
                            (candidate) => candidate.id === id,
                          );
                          if (template) applyTemplate(template);
                        }}
                        options={LIFECYCLE_RULE_TEMPLATES.map((template) => ({
                          value: template.id,
                          label: t(template.titleKey),
                        }))}
                      />
                    ) : null}

                    <RuleEditorSurface
                      idPrefix="settings-lifecycle-rule"
                      schema={builderSchema}
                      builderStatus={builderStatus}
                      variant={null}
                      mode={editorMode}
                      onModeChange={setEditorMode}
                      available={!editingSystemOwned}
                      onEditAsRego={editAsRego}
                      model={ruleSetDraft.builderModel}
                      onModelChange={setBuilderModel}
                      options={builderOptions}
                      generatedSource={ruleSetDraft.regoSource}
                      feedback={validationStatus}
                      editor={(readOnly) => (
                        <LazyRegoEditor
                          ruleFamily="lifecycle"
                          ruleVariant={null}
                          id="settings-lifecycle-rule-rego-source"
                          value={ruleSetDraft.regoSource}
                          readOnly={editingSystemOwned || readOnly}
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
                    />

                    <LifecycleRuleSourceFindings
                      source={ruleSetDraft.regoSource}
                      checked={
                        !validating &&
                        validationResult !== null &&
                        !validationResult.unavailable
                      }
                      declared={validationResult?.declaredActions}
                      warnings={validationResult?.warnings ?? []}
                      showDeclared={
                        editorMode === "simple" &&
                        ruleSetDraft.builderModel !== null &&
                        builderSchema !== null &&
                        !editingSystemOwned
                      }
                    />

                    <label className="block md:max-w-[50%]">
                      <Label className="mb-2 block">
                        <HelpLabel
                          label={t("settings.lifecycleRuleGraceDays")}
                          help={t("settings.lifecycleRuleGraceDaysHelp")}
                        />
                      </Label>
                      <Input
                        id="settings-lifecycle-rule-grace-days"
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

                    <label className="block md:max-w-[50%]">
                      <Label className="mb-2 block">
                        <HelpLabel
                          label={t("settings.lifecycleRuleMaxTitlesPerRun")}
                          help={t("settings.lifecycleRuleMaxTitlesPerRunHelp")}
                        />
                      </Label>
                      <Input
                        id="settings-lifecycle-rule-max-titles-per-run"
                        {...integerInputProps}
                        min={1}
                        required={
                          validationResult?.declaredActions?.destructive === true
                        }
                        value={ruleSetDraft.maxTitlesPerRun ?? ""}
                        onChange={(event) => {
                          const digits = sanitizeDigits(event.target.value);
                          setRuleSetDraft((prev) => ({
                            ...prev,
                            maxTitlesPerRun: digits === "" ? null : Number(digits),
                          }));
                        }}
                        placeholder={t("settings.lifecycleRuleMaxTitlesPerRunNone")}
                      />
                    </label>

                    <LifecycleRuleScheduleField
                      key={editorSessionKey}
                      idPrefix="settings-lifecycle-rule-form-schedule"
                      value={ruleSetDraft.schedule}
                      onChange={(schedule) =>
                        setRuleSetDraft((prev) => ({ ...prev, schedule }))
                      }
                      nextRunAt={editingNextRunAt}
                    />

                    <div>
                      <Label className="mb-2 block">
                        <HelpLabel
                          label={t("settings.lifecycleRuleLibraries")}
                          help={t("settings.lifecycleRuleLibrariesHelp")}
                        />
                      </Label>
                      <div className="flex flex-wrap items-center gap-4">
                        {libraries.length === 0 ? (
                          <span className="text-xs text-muted-foreground">
                            {t("settings.lifecycleRuleLibrariesAll")}
                          </span>
                        ) : null}
                        {libraries.map((library) => (
                          <label
                            key={library.id}
                            className="flex items-center gap-2"
                          >
                            <Checkbox
                              id={selectorId(
                                "settings-lifecycle-rule-library",
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

                    <div className="flex gap-2">
                      <Button
                        id="settings-lifecycle-rule-save"
                        type="submit"
                        disabled={mutatingRuleSetId !== null || validating}
                      >
                        {mutatingRuleSetId !== null
                          ? t("label.saving")
                          : editingRuleSetId
                            ? t("settings.lifecycleRuleUpdate")
                            : t("settings.lifecycleRuleCreate")}
                      </Button>
                      <Button
                        id="settings-lifecycle-rule-cancel"
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
                id="settings-lifecycle-rule-create"
                icon={Plus}
                label={t("settings.lifecycleRuleCreateNew")}
                onClick={startCreateRuleSet}
                disabled={mutatingRuleSetId !== null}
              />
            </div>

            <LifecyclePreviewPanel
              isEditorOpen={isEditorOpen}
              draftName={ruleSetDraft.name}
              draftDeclaredActions={validationResult?.declaredActions}
              ruleSetRecords={ruleSetRecords}
              libraries={libraries}
              {...previewProps}
            />

            {operationsPanels}
          </div>
        </div>
        <div className="@container w-full space-y-4 2xl:w-[44%] 2xl:max-w-[880px] 2xl:shrink-0">
          <LifecycleTemplateGallery onApply={applyTemplate} />
          <LifecycleContextReference />
        </div>
      </div>
    </div>
  );
}
