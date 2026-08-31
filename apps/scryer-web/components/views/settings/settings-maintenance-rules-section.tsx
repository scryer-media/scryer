import * as React from "react";
import { BookOpen, ChevronDown, Copy, Edit, Info, Plus, Trash2 } from "lucide-react";
import { AddNewButton } from "@/components/common/add-new-button";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import { IconButton } from "@/components/ui/icon-button";
import { Input, integerInputProps, sanitizeDigits } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { LazyRegoEditor } from "@/components/common/lazy-rego-editor";
import { SingleSelectField } from "@/components/ui/select";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import maintenanceInputContract from "@/lib/contracts/maintenance-input-contract.json";
import { useTranslate } from "@/lib/context/translate-context";
import type {
  MaintenanceActionDescriptor,
  MaintenanceActionKind,
  MaintenancePreviewResult,
  MaintenancePreviewSource,
  MaintenancePreviewTitle,
  MaintenanceRuleSetDraft,
  MaintenanceRuleSetRecord,
  MaintenanceValidationResult,
} from "@/lib/types/maintenance-rule-sets";
import {
  MAINTENANCE_PREVIEW_LIMIT_MAX,
  actionKindLabelKey,
  actionRequiresTargetQualityProfile,
  descriptorForActionKind,
  evaluationModeLabelKey,
  previewOutcomeBadgeTone,
  previewOutcomeLabelKey,
  riskClassBadgeTone,
  riskClassLabelKey,
  titleScopedActionDescriptors,
} from "@/lib/utils/maintenance-rule-sets";
import { selectorId } from "@/lib/utils/dom-ids";

export type MaintenanceLibraryOption = { id: string; name: string };
export type MaintenanceQualityProfileOption = { id: string; name: string };

type SettingsMaintenanceRulesSectionProps = {
  isEditorOpen: boolean;
  editingRuleSetId: string | null;
  ruleSetDraft: MaintenanceRuleSetDraft;
  setRuleSetDraft: React.Dispatch<React.SetStateAction<MaintenanceRuleSetDraft>>;
  submitRuleSet: (event: React.FormEvent<HTMLFormElement>) => Promise<void> | void;
  mutatingRuleSetId: string | null;
  resetRuleSetDraft: () => void;
  startCreateRuleSet: () => void;
  ruleSetRecords: MaintenanceRuleSetRecord[];
  actionDescriptors: MaintenanceActionDescriptor[];
  libraries: MaintenanceLibraryOption[];
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
  runPreview: () => Promise<void> | void;
  previewing: boolean;
  previewResult: MaintenancePreviewResult | null;
  previewError: string | null;
};

type RefField = { field: string; type: string; descKey: string };
type RefSectionDef = { titleKey: string; path: string; fields: RefField[] };
const REF_SECTIONS = maintenanceInputContract.sections as RefSectionDef[];

/// The one thing every reader of this page needs to know: authoring a rule here
/// changes nothing yet. Rendered above the list so it cannot be scrolled past.
function ShippingDarkNotice() {
  const t = useTranslate();
  return (
    <div
      id="settings-maintenance-rules-dark-notice"
      className="flex items-start gap-2.5 rounded border border-[var(--scry-info-border)] bg-[var(--scry-info-bg)] px-3 py-2.5 text-[var(--scry-info-text)]"
    >
      <Info className="mt-0.5 h-4 w-4 shrink-0" />
      <div className="space-y-1">
        <p className="font-semibold">{t("settings.maintenanceRulesDarkTitle")}</p>
        <p className="text-[13px] leading-5">
          {t("settings.maintenanceRulesDarkBody")}
        </p>
      </div>
    </div>
  );
}

function RiskBadge({ risk }: { risk: string }) {
  const t = useTranslate();
  const labelKey = riskClassLabelKey(risk);
  return (
    <Badge tone={riskClassBadgeTone(risk)}>{labelKey ? t(labelKey) : risk}</Badge>
  );
}

function ActionLabel({ kind }: { kind: string }) {
  const t = useTranslate();
  const labelKey = actionKindLabelKey(kind);
  return <>{labelKey ? t(labelKey) : kind}</>;
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

          {REF_SECTIONS.map((section) => (
            <RefFieldTable key={section.path} section={section} />
          ))}
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
}: { draftName: string } & Pick<
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
  const canPreviewDraft = isEditorOpen;
  const canRun =
    !previewing &&
    Boolean(previewLibraryId) &&
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

        <Button
          id="settings-maintenance-preview-run"
          type="button"
          variant="secondary"
          onClick={() => void runPreview()}
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
                time: previewResult.evaluatedAt,
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
                      key={title.titleId}
                      data-ui="settings-table-row"
                      id={selectorId(
                        "settings-maintenance-preview-row",
                        title.titleId,
                      )}
                    >
                      <TableCell className="font-medium">
                        {title.titleName}
                      </TableCell>
                      <TableCell>
                        <PreviewOutcomeCell title={title} />
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
  ruleSetRecords,
  actionDescriptors,
  libraries,
  qualityProfiles,
  copyRuleSet,
  editRuleSet,
  deleteRuleSet,
  validateDraft,
  validating,
  validationResult,
  ...previewProps
}: SettingsMaintenanceRulesSectionProps) {
  const t = useTranslate();
  const offerableDescriptors = React.useMemo(
    () => titleScopedActionDescriptors(actionDescriptors),
    [actionDescriptors],
  );
  const needsTargetProfile = actionRequiresTargetQualityProfile(
    actionDescriptors,
    ruleSetDraft.actionKind,
  );
  const selectedDescriptor = descriptorForActionKind(
    actionDescriptors,
    ruleSetDraft.actionKind,
  );

  return (
    <div id="settings-maintenance-rules-section" className="space-y-4 text-sm">
      <div className="mx-auto flex w-full max-w-[2176px] flex-col gap-4 xl:flex-row xl:items-start">
        <div className="min-w-0 flex-1">
          <div className="mx-auto w-full max-w-[1280px] space-y-4">
            <ShippingDarkNotice />

            <div className="overflow-hidden rounded border border-border bg-card">
              <div className="flex items-center justify-between border-b border-border px-3 py-2">
                <CardTitle className="text-base text-foreground">
                  {t("settings.maintenanceRules")}
                </CardTitle>
              </div>
              <div className="overflow-x-auto">
                <Table>
                  <TableHeader>
                    <TableRow>
                      <TableHead>{t("label.name")}</TableHead>
                      <TableHead>{t("settings.ruleDescription")}</TableHead>
                      <TableHead>{t("settings.maintenanceRuleAction")}</TableHead>
                      <TableHead>{t("settings.maintenanceRuleRisk")}</TableHead>
                      <TableHead className="text-center">
                        {t("settings.maintenanceRuleGraceDays")}
                      </TableHead>
                      <TableHead className="text-center">
                        {t("settings.maintenanceRuleMode")}
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
                      const descriptor = descriptorForActionKind(
                        actionDescriptors,
                        record.actionSpec.kind,
                      );
                      const modeKey = evaluationModeLabelKey(record.evaluationMode);
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
                          </TableCell>
                          <TableCell className="max-w-[200px] truncate text-muted-foreground">
                            {record.description || "—"}
                          </TableCell>
                          <TableCell>
                            <ActionLabel kind={record.actionSpec.kind} />
                          </TableCell>
                          <TableCell>
                            {descriptor ? (
                              <RiskBadge risk={descriptor.riskClass} />
                            ) : (
                              "—"
                            )}
                          </TableCell>
                          <TableCell className="text-center">
                            {record.graceDays}
                          </TableCell>
                          <TableCell className="text-center">
                            <Badge tone="neutral">
                              {modeKey ? t(modeKey) : record.evaluationMode}
                            </Badge>
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
                        <TableCell colSpan={8} className="text-muted-foreground">
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

                    <div className="grid gap-3 md:grid-cols-2">
                      <div>
                        {offerableDescriptors.length > 0 ? (
                          <SingleSelectField
                            id="settings-maintenance-rule-action"
                            label={t("settings.maintenanceRuleAction")}
                            placeholder={t(
                              "settings.maintenanceRuleActionPlaceholder",
                            )}
                            value={ruleSetDraft.actionKind}
                            onValueChange={(value) =>
                              setRuleSetDraft((prev) => ({
                                ...prev,
                                actionKind: value as MaintenanceActionKind,
                              }))
                            }
                            options={offerableDescriptors.map((descriptor) => ({
                              value: descriptor.kind,
                              ariaLabel: descriptor.kind,
                              label: (
                                <span className="flex min-w-0 items-center gap-2">
                                  <span className="truncate">
                                    <ActionLabel kind={descriptor.kind} />
                                  </span>
                                  <RiskBadge risk={descriptor.riskClass} />
                                </span>
                              ),
                            }))}
                          />
                        ) : (
                          <p
                            id="settings-maintenance-rule-no-actions"
                            className="text-xs text-muted-foreground"
                          >
                            {t("settings.maintenanceRuleNoActions")}
                          </p>
                        )}
                        <p className="mt-1 text-xs text-muted-foreground">
                          {t("settings.maintenanceRuleActionHelp")}
                        </p>
                        {selectedDescriptor &&
                        selectedDescriptor.riskClass === "HIGH" ? (
                          <p className="mt-1 text-xs text-[var(--scry-danger-text)]">
                            {t("settings.maintenanceRiskHigh")}
                          </p>
                        ) : null}
                      </div>
                      <label>
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
                    </div>

                    {needsTargetProfile ? (
                      <div className="md:max-w-[50%]">
                        {qualityProfiles.length > 0 ? (
                          <SingleSelectField
                            id="settings-maintenance-rule-target-profile"
                            label={t("settings.maintenanceRuleTargetProfile")}
                            placeholder={t(
                              "settings.maintenanceRuleTargetProfileHelp",
                            )}
                            value={ruleSetDraft.targetQualityProfileId}
                            onValueChange={(value) =>
                              setRuleSetDraft((prev) => ({
                                ...prev,
                                targetQualityProfileId: value,
                              }))
                            }
                            options={qualityProfiles.map((profile) => ({
                              value: profile.id,
                              label: profile.name,
                            }))}
                          />
                        ) : (
                          <label>
                            <Label className="mb-2 block">
                              {t("settings.maintenanceRuleTargetProfile")}
                            </Label>
                            <Input
                              id="settings-maintenance-rule-target-profile-id"
                              value={ruleSetDraft.targetQualityProfileId}
                              onChange={(event) =>
                                setRuleSetDraft((prev) => ({
                                  ...prev,
                                  targetQualityProfileId: event.target.value,
                                }))
                              }
                            />
                            <p className="mt-1 text-xs text-muted-foreground">
                              {t("settings.maintenanceRuleTargetProfileIdHelp")}
                            </p>
                          </label>
                        )}
                        <p className="mt-1 text-xs text-muted-foreground">
                          {t("settings.maintenanceRuleTargetProfileHelp")}
                        </p>
                      </div>
                    ) : null}

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
              ruleSetRecords={ruleSetRecords}
              libraries={libraries}
              {...previewProps}
            />
          </div>
        </div>
        <div className="@container w-full space-y-4 xl:w-[44%] xl:max-w-[880px] xl:shrink-0">
          <MaintenanceContextReference />
        </div>
      </div>
    </div>
  );
}
