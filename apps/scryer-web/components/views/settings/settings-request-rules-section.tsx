import * as React from "react";
import {
  AlertTriangle,
  BookOpen,
  ChevronDown,
  Copy,
  Edit,
  Info,
  Loader2,
  Plus,
  Sparkles,
  Trash2,
  Users,
} from "lucide-react";
import { AddNewButton } from "@/components/common/add-new-button";
import { SettingsToggleSwitch } from "@/components/common/settings-toggle-switch";
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
import requestInputContract from "@/lib/contracts/request-input-contract.json";
import {
  REQUEST_RULE_TEMPLATES,
  type RequestRuleTemplate,
} from "@/lib/constants/request-rule-templates";
import { useTranslate } from "@/lib/context/translate-context";
import type { MetadataTvdbSearchItem } from "@/lib/graphql/smg-queries";
import type { LibraryRecord } from "@/lib/types";
import type {
  RequestDecisionOutcome,
  RequestEvaluationMode,
  RequestRuleDecisionRecord,
  RequestRuleInstanceGates,
  RequestRulePreviewResult,
  RequestRulePreviewSample,
  RequestRulePreviewSource,
  RequestRuleSetDraft,
  RequestRuleSetRecord,
  RequestRuleUserOption,
  RequestRuleValidationResult,
} from "@/lib/types/request-rule-sets";
import {
  REQUEST_EVALUATION_MODES,
  REQUEST_FILTER_ALL,
  REQUEST_LEASE_DAYS_MAX,
  requestDecisionOutcomeBadgeTone,
  requestDecisionOutcomeLabelKey,
  requestEvaluationModeBadgeTone,
  requestEvaluationModeHelpKey,
  requestEvaluationModeLabelKey,
  requestFallbackReasonLabelKey,
  isPersonTargetingRefusal,
  requestRuleNamesRequesters,
  requestVoteBadgeTone,
  requestVoteLabelKey,
} from "@/lib/utils/request-rule-sets";
import {
  settingsRequestPreviewTitleResultId,
  settingsRequestDecisionRowId,
  settingsRequestRuleCopyId,
  settingsRequestRuleDeleteId,
  settingsRequestRuleEditId,
  settingsRequestRuleLibraryId,
  settingsRequestRuleModeId,
  settingsRequestRuleNameId,
  settingsRequestRuleRowId,
  settingsRequestTemplateId,
  settingsRequestUserId,
} from "@/lib/utils/dom-ids";

export type RequestQualityProfileOption = { id: string; name: string };

const TITLE_SEARCH_DEBOUNCE_MS = 350;
const TITLE_SEARCH_MIN_LENGTH = 2;

const DECISION_OUTCOME_FILTERS: Array<RequestDecisionOutcome | "all"> = [
  REQUEST_FILTER_ALL as "all",
  "AUTO_APPROVE",
  "MANUAL_REVIEW",
  "DENY",
];

const PREVIEW_MONITOR_TYPES = [
  "MONITORED",
  "FUTURE_EPISODES",
  "MISSING_AND_FUTURE_EPISODES",
  "ALL_EPISODES",
  "NONE",
];

type SettingsRequestRulesSectionProps = {
  isEditorOpen: boolean;
  editingRuleSetId: string | null;
  ruleSetDraft: RequestRuleSetDraft;
  setRuleSetDraft: React.Dispatch<React.SetStateAction<RequestRuleSetDraft>>;
  submitRuleSet: (event: React.FormEvent<HTMLFormElement>) => Promise<void> | void;
  mutatingRuleSetId: string | null;
  resetRuleSetDraft: () => void;
  startCreateRuleSet: () => void;
  /// Load a starter template into the create-rule editor. Nothing is created
  /// here: the container prefills the draft and the operator still saves it.
  applyTemplate: (template: RequestRuleTemplate) => void;
  ruleSetRecords: RequestRuleSetRecord[];
  libraries: LibraryRecord[];
  qualityProfiles: RequestQualityProfileOption[];
  /// Accounts the picker can write into a matcher. Empty when the reader may
  /// not list users, which is a permission answer rather than a page failure.
  users: RequestRuleUserOption[];
  copyRuleSet: (record: RequestRuleSetRecord) => void;
  editRuleSet: (record: RequestRuleSetRecord) => void;
  deleteRuleSet: (record: RequestRuleSetRecord) => void;
  validateDraft: () => Promise<RequestRuleValidationResult | null>;
  validating: boolean;
  validationResult: RequestRuleValidationResult | null;
  /// Rewrite whichever way the draft names people with these usernames.
  applyRequesters: (usernames: string[]) => void;
  setRuleMode: (record: RequestRuleSetRecord, mode: RequestEvaluationMode) => void;
  /// Null when the gate query was refused, which on this page means the reader
  /// is not a system administrator and cannot see the instance state at all.
  gates: RequestRuleInstanceGates | null;
  gatesLocked: boolean;
  savingGate: boolean;
  setGate: (enabled: boolean) => void;
  previewSource: RequestRulePreviewSource;
  setPreviewSource: (source: RequestRulePreviewSource) => void;
  previewRuleSetId: string;
  setPreviewRuleSetId: (id: string) => void;
  previewSample: RequestRulePreviewSample;
  setPreviewSample: React.Dispatch<React.SetStateAction<RequestRulePreviewSample>>;
  searchPreviewTitles: (
    query: string,
    facet: string,
  ) => Promise<MetadataTvdbSearchItem[]>;
  runPreview: () => Promise<void> | void;
  previewing: boolean;
  previewResult: RequestRulePreviewResult | null;
  previewError: string | null;
  decisions: RequestRuleDecisionRecord[];
  decisionsLoading: boolean;
  decisionsError: string | null;
  decisionOutcomeFilter: string;
  setDecisionOutcomeFilter: (outcome: string) => void;
  refreshDecisions: () => void;
};

type RefField = { field: string; type: string; descKey: string };
type RefSectionDef = { titleKey: string; path: string; fields: RefField[] };
const REF_SECTIONS = requestInputContract.sections as RefSectionDef[];

/// What this instance will actually do with these rules right now. Three
/// controls have to agree before a request rule resolves anything, and this
/// line is derived from two of them: the instance gate and the rules' own
/// modes. It stays above the list where it cannot be scrolled past.
function RequestRulesStatusBanner({
  gates,
  ruleSetRecords,
}: {
  gates: RequestRuleInstanceGates | null;
  ruleSetRecords: RequestRuleSetRecord[];
}) {
  const t = useTranslate();
  const enforcing = ruleSetRecords.some(
    (record) => record.evaluationMode === "ENFORCE",
  );
  const shadowing = ruleSetRecords.some(
    (record) => record.evaluationMode === "SHADOW",
  );
  const gateOpen = gates?.evaluationEnabled === true;

  const variant = !gateOpen
    ? "gateClosed"
    : enforcing
      ? "enforcing"
      : shadowing
        ? "shadowing"
        : "idle";
  const warning = variant === "enforcing";
  const Icon = warning ? AlertTriangle : Info;

  return (
    <div
      id="settings-request-rules-status-notice"
      data-request-rules-status={variant}
      className={`flex items-start gap-2.5 rounded border px-3 py-2.5 ${
        warning
          ? "border-[var(--scry-warning-border)] bg-[var(--scry-warning-bg)] text-[var(--scry-warning-text)]"
          : "border-[var(--scry-info-border)] bg-[var(--scry-info-bg)] text-[var(--scry-info-text)]"
      }`}
    >
      <Icon className="mt-0.5 h-4 w-4 shrink-0" />
      <div className="space-y-1">
        <p className="font-semibold">
          {t(`settings.requestRulesStatus${capitalize(variant)}Title`)}
        </p>
        <p className="text-[13px] leading-5">
          {t(`settings.requestRulesStatus${capitalize(variant)}Body`)}
        </p>
      </div>
    </div>
  );
}

function capitalize(value: string): string {
  return value.charAt(0).toUpperCase() + value.slice(1);
}

function ModeBadge({ mode }: { mode: string }) {
  const t = useTranslate();
  const labelKey = requestEvaluationModeLabelKey(mode);
  return (
    <Badge tone={requestEvaluationModeBadgeTone(mode)}>
      {labelKey ? t(labelKey) : mode}
    </Badge>
  );
}

function OutcomeBadge({ outcome }: { outcome: string }) {
  const t = useTranslate();
  const labelKey = requestDecisionOutcomeLabelKey(outcome);
  return (
    <Badge tone={requestDecisionOutcomeBadgeTone(outcome)}>
      {labelKey ? t(labelKey) : outcome}
    </Badge>
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

function RequestContextReference() {
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
          {t("settings.refReqTitle")}
          <ChevronDown
            className={`ml-auto h-4 w-4 transition-transform ${open ? "rotate-180" : ""}`}
          />
        </CardTitle>
        <p className="text-xs text-muted-foreground">
          {t("settings.refReqSubtitle")}
        </p>
      </CardHeader>
      {open ? (
        <CardContent
          id="settings-request-rules-reference"
          className="space-y-6 text-sm"
        >
          <p className="text-muted-foreground">{t("settings.refReqIntro")}</p>

          <div>
            <h4 className="mb-1 font-semibold">
              {t("settings.refReqOutputTitle")}
            </h4>
            <p className="mb-2 text-muted-foreground">
              {t("settings.refReqOutputIntro")}
            </p>
            <ul className="list-disc space-y-1.5 pl-5 text-muted-foreground">
              <li>{t("settings.refReqOutputApprove")}</li>
              <li>{t("settings.refReqOutputDeny")}</li>
              <li>{t("settings.refReqOutputManual")}</li>
              <li>{t("settings.refReqOutputTags")}</li>
              <li>{t("settings.refReqOutputReasons")}</li>
              <li>{t("settings.refReqOutputNoPackage")}</li>
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

/// One starter template as a card. Everything on it is static: the gallery
/// renders the same on an instance that has never evaluated a request as on one
/// that decides every request automatically.
function RequestTemplateCard({
  template,
  onApply,
}: {
  template: RequestRuleTemplate;
  onApply: (template: RequestRuleTemplate) => void;
}) {
  const t = useTranslate();
  return (
    <button
      id={settingsRequestTemplateId(template.id)}
      type="button"
      data-request-template-person-targeted={
        template.personTargeted ? "true" : "false"
      }
      className="group flex flex-col rounded-lg border border-border bg-card/50 p-3 text-left transition-colors hover:border-primary/40 hover:bg-card"
      onClick={() => onApply(template)}
      title={t("settings.requestTemplateApply")}
    >
      <p className="text-sm font-medium text-foreground group-hover:text-primary">
        {t(template.titleKey)}
      </p>
      <p className="mt-1 text-xs leading-5 text-muted-foreground">
        {t(template.descriptionKey)}
      </p>
      {template.personTargeted ? (
        <div className="mt-2 flex flex-wrap items-center gap-1.5">
          <Badge tone="warning" className="text-[10px]">
            {t("settings.requestTemplatePersonTargetedBadge")}
          </Badge>
        </div>
      ) : null}
    </button>
  );
}

function RequestTemplateGallery({
  onApply,
}: {
  onApply: (template: RequestRuleTemplate) => void;
}) {
  const t = useTranslate();
  const [open, setOpen] = React.useState(false);

  return (
    <Card>
      <CardHeader
        id="settings-request-template-gallery-toggle"
        className="cursor-pointer select-none"
        onClick={() => setOpen((prev) => !prev)}
      >
        <CardTitle className="flex items-center gap-2 text-base">
          <Sparkles className="h-4 w-4" />
          {t("settings.requestTemplateGallery")}
          <ChevronDown
            className={`ml-auto h-4 w-4 transition-transform ${open ? "rotate-180" : ""}`}
          />
        </CardTitle>
        <p className="text-xs text-muted-foreground">
          {t("settings.requestTemplateGalleryDescription")}
        </p>
        <p className="text-xs text-muted-foreground">
          {t("settings.requestTemplateGalleryTagNote")}
        </p>
      </CardHeader>
      {open ? (
        <CardContent id="settings-request-template-gallery" className="@container">
          <div className="grid grid-cols-1 gap-2 @[560px]:grid-cols-2">
            {REQUEST_RULE_TEMPLATES.map((template) => (
              <RequestTemplateCard
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

/// Names the people a matcher is about. The picker never edits the matcher on
/// its own: the operator ticks accounts and presses apply, and the rewrite lands
/// in the editor where they can read it before saving.
function RequestUserPicker({
  users,
  regoSource,
  applyRequesters,
  disabled,
}: {
  users: RequestRuleUserOption[];
  regoSource: string;
  applyRequesters: (usernames: string[]) => void;
  disabled: boolean;
}) {
  const t = useTranslate();
  const [selected, setSelected] = React.useState<string[]>([]);
  const writable = requestRuleNamesRequesters(regoSource);

  return (
    <div id="settings-request-users-picker" className="space-y-2">
      <Label className="block">{t("settings.requestRuleRequesters")}</Label>
      <p className="text-xs text-muted-foreground">
        {t("settings.requestRuleRequestersHelp")}
      </p>
      {users.length === 0 ? (
        <p
          id="settings-request-users-empty"
          className="text-xs text-muted-foreground"
        >
          {t("settings.requestRuleRequestersUnavailable")}
        </p>
      ) : (
        <div className="flex flex-wrap items-center gap-4">
          {users.map((user) => (
            <label key={user.id} className="flex items-center gap-2">
              <Checkbox
                id={settingsRequestUserId(user.id)}
                checked={selected.includes(user.username)}
                onCheckedChange={(value) =>
                  setSelected((prev) =>
                    value === true
                      ? [...prev, user.username]
                      : prev.filter((name) => name !== user.username),
                  )
                }
              />
              <span className="text-sm">{user.username}</span>
            </label>
          ))}
        </div>
      )}
      {!writable ? (
        <p
          id="settings-request-users-no-target"
          className="text-xs text-[var(--scry-warning-text)]"
        >
          {t("settings.requestRuleRequestersNoTarget")}
        </p>
      ) : null}
      <Button
        id="settings-request-users-apply"
        type="button"
        variant="secondary"
        size="sm"
        disabled={disabled || selected.length === 0 || !writable}
        onClick={() => applyRequesters(selected)}
      >
        <Users className="h-4 w-4" />
        {t("settings.requestRuleRequestersApply")}
      </Button>
    </div>
  );
}

function DecisionVotes({ decision }: { decision: RequestRuleDecisionRecord }) {
  const t = useTranslate();
  if (decision.votes.length === 0) {
    return (
      <span className="text-muted-foreground">
        {t("settings.requestDecisionNoVotes")}
      </span>
    );
  }
  return (
    <div className="flex flex-wrap gap-1.5">
      {decision.votes.map((vote) => {
        const labelKey = requestVoteLabelKey(vote.vote);
        return (
          <Badge
            key={`${vote.ruleSetId}:${vote.revisionNumber}`}
            tone={requestVoteBadgeTone(vote.vote)}
            className="text-[10px]"
            title={vote.error ?? undefined}
          >
            {vote.ruleSetName}
            {": "}
            {labelKey ? t(labelKey) : (vote.vote ?? "—")}
            {vote.held ? ` · ${t("settings.requestDecisionHeld")}` : ""}
          </Badge>
        );
      })}
    </div>
  );
}

/// The author's preview. Unlike the requester's pre-flight it shows the input
/// document the matcher actually saw, which is the only honest answer to "why
/// did this not match".
function RequestPreviewPanel({
  isEditorOpen,
  draftName,
  ruleSetRecords,
  libraries,
  qualityProfiles,
  users,
  previewSource,
  setPreviewSource,
  previewRuleSetId,
  setPreviewRuleSetId,
  previewSample,
  setPreviewSample,
  searchPreviewTitles,
  runPreview,
  previewing,
  previewResult,
  previewError,
}: { draftName: string } & Pick<
  SettingsRequestRulesSectionProps,
  | "isEditorOpen"
  | "ruleSetRecords"
  | "libraries"
  | "qualityProfiles"
  | "users"
  | "previewSource"
  | "setPreviewSource"
  | "previewRuleSetId"
  | "setPreviewRuleSetId"
  | "previewSample"
  | "setPreviewSample"
  | "searchPreviewTitles"
  | "runPreview"
  | "previewing"
  | "previewResult"
  | "previewError"
>) {
  const t = useTranslate();
  const [titleQuery, setTitleQuery] = React.useState("");
  const [titleResults, setTitleResults] = React.useState<MetadataTvdbSearchItem[]>(
    [],
  );
  const [titleSearching, setTitleSearching] = React.useState(false);
  const [showInputDocument, setShowInputDocument] = React.useState(false);

  const selectedLibrary =
    libraries.find((library) => library.id === previewSample.libraryId) ?? null;
  const facet = selectedLibrary?.facet ?? "MOVIE";

  // The metadata search is a network call per keystroke otherwise, and the
  // author is typing a title rather than driving a live filter.
  React.useEffect(() => {
    const trimmed = titleQuery.trim();
    if (trimmed.length < TITLE_SEARCH_MIN_LENGTH) {
      setTitleResults([]);
      return;
    }
    let cancelled = false;
    const timer = setTimeout(() => {
      setTitleSearching(true);
      void searchPreviewTitles(trimmed, facet)
        .then((results) => {
          if (!cancelled) setTitleResults(results);
        })
        .finally(() => {
          if (!cancelled) setTitleSearching(false);
        });
    }, TITLE_SEARCH_DEBOUNCE_MS);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [facet, searchPreviewTitles, titleQuery]);

  const canPreviewDraft = isEditorOpen;
  const canRun =
    !previewing &&
    Boolean(previewSample.userId) &&
    Boolean(previewSample.libraryId) &&
    previewSample.externalIds.length > 0 &&
    (previewSource === "draft" ? canPreviewDraft : Boolean(previewRuleSetId));

  const decision = previewResult?.decision ?? null;

  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-base">
          {t("settings.requestPreviewTitle")}
        </CardTitle>
        <p className="text-xs text-muted-foreground">
          {t("settings.requestPreviewSubtitle")}
        </p>
      </CardHeader>
      <CardContent className="space-y-3">
        <div className="grid gap-3 md:grid-cols-3">
          <SingleSelectField
            id="settings-request-preview-source"
            label={t("settings.requestPreviewSource")}
            value={previewSource}
            onValueChange={(value) =>
              setPreviewSource(value as RequestRulePreviewSource)
            }
            options={[
              {
                value: "stored",
                label: t("settings.requestPreviewSourceStored"),
              },
              {
                value: "draft",
                label: t("settings.requestPreviewSourceDraft"),
                disabled: !canPreviewDraft,
              },
            ]}
          />
          {previewSource === "stored" ? (
            <SingleSelectField
              id="settings-request-preview-rule"
              label={t("settings.requestPreviewRuleSelect")}
              placeholder={t("settings.requestPreviewRuleSelectPlaceholder")}
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
                {t("settings.requestPreviewRuleSelect")}
              </Label>
              <p
                id="settings-request-preview-draft-name"
                className="truncate text-xs text-muted-foreground"
              >
                {draftName || t("settings.requestPreviewSourceDraft")}
              </p>
            </div>
          )}
          <SingleSelectField
            id="settings-request-preview-user"
            label={t("settings.requestPreviewUser")}
            placeholder={t("settings.requestPreviewUserPlaceholder")}
            value={previewSample.userId}
            onValueChange={(value) =>
              setPreviewSample((prev) => ({ ...prev, userId: value }))
            }
            options={users.map((user) => ({
              value: user.id,
              label: user.username,
            }))}
          />
        </div>

        <div className="grid gap-3 md:grid-cols-3">
          <SingleSelectField
            id="settings-request-preview-library"
            label={t("settings.requestPreviewLibrary")}
            placeholder={t("settings.requestPreviewLibraryPlaceholder")}
            value={previewSample.libraryId}
            onValueChange={(value) =>
              setPreviewSample((prev) => ({ ...prev, libraryId: value }))
            }
            options={libraries.map((library) => ({
              value: library.id,
              label: library.name,
            }))}
          />
          <SingleSelectField
            id="settings-request-preview-profile"
            label={t("settings.requestPreviewProfile")}
            placeholder={t("settings.requestPreviewProfilePlaceholder")}
            value={previewSample.qualityProfileId}
            onValueChange={(value) =>
              setPreviewSample((prev) => ({ ...prev, qualityProfileId: value }))
            }
            options={qualityProfiles.map((profile) => ({
              value: profile.id,
              label: profile.name,
            }))}
          />
          <SingleSelectField
            id="settings-request-preview-monitor"
            label={t("settings.requestPreviewMonitor")}
            value={previewSample.monitorType}
            onValueChange={(value) =>
              setPreviewSample((prev) => ({ ...prev, monitorType: value }))
            }
            options={PREVIEW_MONITOR_TYPES.map((monitorType) => ({
              value: monitorType,
              label: monitorType,
            }))}
          />
        </div>

        <div className="grid gap-3 md:grid-cols-2">
          <SingleSelectField
            id="settings-request-preview-lease-mode"
            label={t("settings.requestPreviewLease")}
            value={previewSample.leaseForever ? "forever" : "days"}
            onValueChange={(value) =>
              setPreviewSample((prev) => ({
                ...prev,
                leaseForever: value === "forever",
              }))
            }
            options={[
              { value: "forever", label: t("search.keepForForever") },
              { value: "days", label: t("settings.requestPreviewLeaseDays") },
            ]}
          />
          {previewSample.leaseForever ? null : (
            <label>
              <Label className="mb-2 block">
                {t("settings.requestPreviewLeaseDays")}
              </Label>
              <Input
                id="settings-request-preview-lease-days"
                {...integerInputProps}
                min={1}
                max={REQUEST_LEASE_DAYS_MAX}
                value={previewSample.leaseDays}
                onChange={(event) =>
                  setPreviewSample((prev) => ({
                    ...prev,
                    leaseDays: Number(sanitizeDigits(event.target.value)) || 0,
                  }))
                }
              />
            </label>
          )}
        </div>

        <div>
          <Label className="mb-2 block">{t("settings.requestPreviewMedia")}</Label>
          <Input
            id="settings-request-preview-title-search"
            value={titleQuery}
            placeholder={t("settings.requestPreviewMediaPlaceholder")}
            onChange={(event) => setTitleQuery(event.target.value)}
          />
          {previewSample.titleLabel ? (
            <p
              id="settings-request-preview-title-selected"
              className="mt-1 text-xs text-muted-foreground"
            >
              {t("settings.requestPreviewMediaSelected", {
                title: previewSample.titleLabel,
              })}
            </p>
          ) : null}
          {titleSearching ? (
            <p className="mt-1 text-xs text-muted-foreground">
              {t("label.searching")}
            </p>
          ) : null}
          {titleResults.length > 0 ? (
            <ul className="mt-2 space-y-1">
              {titleResults.slice(0, 8).map((result) => {
                const key = String(result.smgId ?? result.tvdbId ?? result.name);
                return (
                  <li key={key}>
                    <button
                      id={settingsRequestPreviewTitleResultId(key)}
                      type="button"
                      className="w-full rounded border border-border px-2 py-1 text-left text-xs transition-colors hover:border-primary/40"
                      onClick={() => {
                        setPreviewSample((prev) => ({
                          ...prev,
                          titleLabel: result.year
                            ? `${result.name} (${result.year})`
                            : result.name,
                          externalIds: metadataExternalIds(result),
                        }));
                        setTitleResults([]);
                        setTitleQuery("");
                      }}
                    >
                      {result.name}
                      {result.year ? ` (${result.year})` : ""}
                    </button>
                  </li>
                );
              })}
            </ul>
          ) : null}
        </div>

        <Button
          id="settings-request-preview-run"
          type="button"
          variant="secondary"
          onClick={() => void runPreview()}
          disabled={!canRun}
        >
          {previewing
            ? t("settings.requestPreviewRunning")
            : t("settings.requestPreviewRun")}
        </Button>

        {previewError ? (
          <pre
            id="settings-request-preview-error"
            className="overflow-x-auto whitespace-pre-wrap rounded-[9px] border border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)] p-3 font-mono text-[12px] leading-5 text-[var(--scry-danger-text)]"
          >
            {previewError}
          </pre>
        ) : null}

        {previewResult && decision ? (
          <div id="settings-request-preview-result" className="space-y-2">
            <div className="flex flex-wrap items-center gap-2">
              <OutcomeBadge outcome={decision.policyOutcome} />
              <ModeBadge mode={decision.mode} />
              {decision.votes[0]?.held ? (
                <Badge tone="warning">{t("settings.requestDecisionHeld")}</Badge>
              ) : null}
              {previewResult.metadataPartial ? (
                <Badge tone="warning">
                  {t("settings.requestPreviewMetadataPartial")}
                </Badge>
              ) : null}
            </div>
            <DecisionVotes decision={decision} />
            {decision.reasons.length > 0 ? (
              <div className="flex flex-wrap gap-1">
                {decision.reasons.map((reason) => (
                  <code
                    key={`${reason.ruleName}:${reason.code}`}
                    data-code-font
                    className="rounded bg-muted px-1 py-0.5 text-xs"
                  >
                    {reason.code}
                  </code>
                ))}
              </div>
            ) : null}
            {decision.tags.length > 0 ? (
              <div className="flex flex-wrap gap-1">
                {decision.tags.map((tag) => (
                  <Badge key={tag} tone="info" className="text-[10px]">
                    {tag}
                  </Badge>
                ))}
              </div>
            ) : null}
            {previewResult.undefinedTags.length > 0 ? (
              <div id="settings-request-preview-undefined-tags" className="space-y-1">
                <div className="flex flex-wrap gap-1">
                  {previewResult.undefinedTags.map((tag) => (
                    <Badge key={tag} tone="warning" className="text-[10px]">
                      {tag}
                    </Badge>
                  ))}
                </div>
                <p className="text-xs text-muted-foreground">
                  {t("settings.requestPreviewUndefinedTags")}
                </p>
              </div>
            ) : null}
            <Button
              id="settings-request-preview-input-toggle"
              type="button"
              variant="secondary"
              size="sm"
              onClick={() => setShowInputDocument((prev) => !prev)}
            >
              {showInputDocument
                ? t("settings.requestPreviewHideInput")
                : t("settings.requestPreviewShowInput")}
            </Button>
            {showInputDocument ? (
              <pre
                id="settings-request-preview-input-document"
                className="max-h-[420px] overflow-auto whitespace-pre-wrap rounded-[9px] border border-border bg-muted p-3 font-mono text-[12px] leading-5"
              >
                {previewResult.inputDocument === null ||
                previewResult.inputDocument === undefined
                  ? t("settings.requestPreviewInputUnavailable")
                  : JSON.stringify(previewResult.inputDocument, null, 2)}
              </pre>
            ) : null}
          </div>
        ) : previewError ? null : (
          <p className="text-xs text-muted-foreground">
            {t("settings.requestPreviewEmpty")}
          </p>
        )}
      </CardContent>
    </Card>
  );
}

function metadataExternalIds(
  result: MetadataTvdbSearchItem,
): Array<{ source: string; value: string }> {
  const ids = new Map<string, string>();
  for (const externalId of result.externalIds ?? []) {
    const source = externalId.source?.trim().toLowerCase();
    const value = externalId.value?.trim();
    if (source && value) {
      ids.set(source, value);
    }
  }
  const smgId = String(result.smgId ?? "").trim();
  if (smgId) ids.set("smg", smgId);
  const tvdbId = String(result.tvdbId ?? "").trim();
  if (tvdbId) ids.set("tvdb", tvdbId);
  const tmdbId = String(result.tmdbId ?? "").trim();
  if (tmdbId) ids.set("tmdb", tmdbId);
  const imdbId = result.imdbId?.trim();
  if (imdbId) ids.set("imdb", imdbId);
  return [...ids.entries()].map(([source, value]) => ({ source, value }));
}

function RequestDecisionsPanel({
  decisions,
  decisionsLoading,
  decisionsError,
  decisionOutcomeFilter,
  setDecisionOutcomeFilter,
  refreshDecisions,
}: Pick<
  SettingsRequestRulesSectionProps,
  | "decisions"
  | "decisionsLoading"
  | "decisionsError"
  | "decisionOutcomeFilter"
  | "setDecisionOutcomeFilter"
  | "refreshDecisions"
>) {
  const t = useTranslate();
  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-base">
          {t("settings.requestDecisionsTitle")}
        </CardTitle>
        <p className="text-xs text-muted-foreground">
          {t("settings.requestDecisionsSubtitle")}
        </p>
      </CardHeader>
      <CardContent className="space-y-3">
        <div className="flex flex-wrap items-end gap-3">
          <SingleSelectField
            id="settings-request-decisions-outcome"
            label={t("settings.requestDecisionsOutcomeFilter")}
            value={decisionOutcomeFilter}
            onValueChange={setDecisionOutcomeFilter}
            options={DECISION_OUTCOME_FILTERS.map((outcome) => {
              const labelKey =
                outcome === REQUEST_FILTER_ALL
                  ? "settings.requestDecisionsOutcomeAll"
                  : requestDecisionOutcomeLabelKey(outcome);
              return {
                value: outcome,
                ariaLabel: outcome,
                label: labelKey ? t(labelKey) : outcome,
              };
            })}
          />
          <Button
            id="settings-request-decisions-refresh"
            type="button"
            variant="secondary"
            onClick={refreshDecisions}
            disabled={decisionsLoading}
          >
            {decisionsLoading ? t("label.refreshing") : t("label.refresh")}
          </Button>
        </div>

        {decisionsError ? (
          <pre className="overflow-x-auto whitespace-pre-wrap rounded-[9px] border border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)] p-3 font-mono text-[12px] leading-5 text-[var(--scry-danger-text)]">
            {decisionsError}
          </pre>
        ) : null}

        <div className="overflow-x-auto">
          <Table id="settings-request-decisions">
            <TableHeader>
              <TableRow>
                <TableHead className="w-[190px]">
                  {t("settings.requestDecisionsColTime")}
                </TableHead>
                <TableHead className="w-[150px]">
                  {t("settings.requestDecisionsColOutcome")}
                </TableHead>
                <TableHead className="w-[120px]">
                  {t("settings.requestDecisionsColMode")}
                </TableHead>
                <TableHead>{t("settings.requestDecisionsColVotes")}</TableHead>
                <TableHead>{t("settings.requestDecisionsColReasons")}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {decisions.map((decision, index) => {
                const fallbackKey = decision.fallbackReason
                  ? requestFallbackReasonLabelKey(decision.fallbackReason)
                  : null;
                return (
                  <TableRow
                    key={decision.id ?? `${decision.evaluatedAt}:${index}`}
                    data-ui="settings-table-row"
                    id={settingsRequestDecisionRowId(index)}
                    data-request-decision-outcome={decision.effectiveOutcome}
                  >
                    <TableCell className="text-muted-foreground">
                      {decision.evaluatedAt}
                    </TableCell>
                    <TableCell>
                      <div className="space-y-1">
                        <OutcomeBadge outcome={decision.effectiveOutcome} />
                        {decision.policyOutcome !== decision.effectiveOutcome ? (
                          <p className="text-xs text-muted-foreground">
                            {t("settings.requestDecisionsPolicySaid", {
                              outcome: decision.policyOutcome,
                            })}
                          </p>
                        ) : null}
                      </div>
                    </TableCell>
                    <TableCell>
                      <ModeBadge mode={decision.mode} />
                    </TableCell>
                    <TableCell>
                      <DecisionVotes decision={decision} />
                    </TableCell>
                    <TableCell className="text-muted-foreground">
                      <div className="space-y-1">
                        {decision.reasons.length > 0 ? (
                          <div className="flex flex-wrap gap-1">
                            {decision.reasons.map((reason) => (
                              <code
                                key={`${reason.ruleName}:${reason.code}`}
                                data-code-font
                                className="rounded bg-muted px-1 py-0.5 text-xs"
                              >
                                {reason.code}
                              </code>
                            ))}
                          </div>
                        ) : null}
                        {decision.fallbackReason ? (
                          <p className="text-xs">
                            {fallbackKey
                              ? t(fallbackKey)
                              : decision.fallbackReason}
                          </p>
                        ) : null}
                        {decision.reasons.length === 0 &&
                        !decision.fallbackReason
                          ? "—"
                          : null}
                      </div>
                    </TableCell>
                  </TableRow>
                );
              })}
              {decisions.length === 0 ? (
                <TableRow>
                  <TableCell colSpan={5} className="text-muted-foreground">
                    {t("settings.requestDecisionsEmpty")}
                  </TableCell>
                </TableRow>
              ) : null}
            </TableBody>
          </Table>
        </div>
      </CardContent>
    </Card>
  );
}

/// The instance gate. It lives under system-settings management rather than the
/// catalog permission the rest of this page needs, so a catalog administrator
/// sees the switch locked and is told who can move it.
function RequestGatePanel({
  gates,
  gatesLocked,
  savingGate,
  setGate,
}: Pick<
  SettingsRequestRulesSectionProps,
  "gates" | "gatesLocked" | "savingGate" | "setGate"
>) {
  const t = useTranslate();
  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-base">
          {t("settings.requestGateTitle")}
        </CardTitle>
        <p className="text-xs text-muted-foreground">
          {t("settings.requestGateSubtitle")}
        </p>
      </CardHeader>
      <CardContent id="settings-request-rule-gate-panel">
        {gatesLocked || !gates ? (
          <p
            id="settings-request-rule-gate-locked"
            className="rounded border border-[var(--scry-info-border)] bg-[var(--scry-info-bg)] px-3 py-2 text-xs text-[var(--scry-info-text)]"
          >
            {t("settings.requestGateLocked")}
          </p>
        ) : (
          <div className="flex flex-col gap-2 rounded border border-border bg-muted/30 px-3 py-2.5 sm:flex-row sm:items-center sm:justify-between">
            <div className="space-y-1">
              <Label htmlFor="settings-request-rule-gate">
                {t("settings.requestGateEvaluation")}
              </Label>
              <p className="text-xs text-muted-foreground">
                {t("settings.requestGateEvaluationHelp")}
              </p>
            </div>
            <div className="flex shrink-0 items-center gap-2">
              {savingGate ? (
                <Loader2 className="h-4 w-4 animate-spin text-muted-foreground" />
              ) : null}
              <SettingsToggleSwitch
                id="settings-request-rule-gate"
                checked={gates.evaluationEnabled}
                disabled={savingGate}
                ariaLabel={t("settings.requestGateEvaluation")}
                onChange={setGate}
              />
            </div>
          </div>
        )}
      </CardContent>
    </Card>
  );
}

export function SettingsRequestRulesSection({
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
  libraries,
  qualityProfiles,
  users,
  copyRuleSet,
  editRuleSet,
  deleteRuleSet,
  validateDraft,
  validating,
  validationResult,
  applyRequesters,
  setRuleMode,
  gates,
  gatesLocked,
  savingGate,
  setGate,
  decisions,
  decisionsLoading,
  decisionsError,
  decisionOutcomeFilter,
  setDecisionOutcomeFilter,
  refreshDecisions,
  ...previewProps
}: SettingsRequestRulesSectionProps) {
  const t = useTranslate();

  return (
    <div id="settings-request-rules-section" className="space-y-4 text-sm">
      <div className="mx-auto flex w-full max-w-[2176px] flex-col gap-4 xl:flex-row xl:items-start">
        <div className="min-w-0 flex-1">
          <div className="mx-auto w-full max-w-[1280px] space-y-4">
            <RequestRulesStatusBanner
              gates={gates}
              ruleSetRecords={ruleSetRecords}
            />

            <div className="overflow-hidden rounded border border-border bg-card">
              <div className="flex items-center justify-between border-b border-border px-3 py-2">
                <CardTitle className="text-base text-foreground">
                  {t("settings.requestRules")}
                </CardTitle>
              </div>
              <div className="overflow-x-auto">
                <Table>
                  <TableHeader>
                    <TableRow>
                      <TableHead>{t("label.name")}</TableHead>
                      <TableHead>{t("settings.ruleDescription")}</TableHead>
                      <TableHead className="w-[200px]">
                        {t("settings.requestRuleMode")}
                      </TableHead>
                      <TableHead>
                        {t("settings.requestRuleLibraries")}
                      </TableHead>
                      <TableHead className="text-center">
                        {t("settings.requestRuleRevision")}
                      </TableHead>
                      <TableHead className="text-center">
                        {t("settings.requestRuleDecisionCount")}
                      </TableHead>
                      <TableHead className="text-right">
                        {t("label.actions")}
                      </TableHead>
                    </TableRow>
                  </TableHeader>
                  <TableBody>
                    {ruleSetRecords.map((record) => {
                      const modeHelpKey = requestEvaluationModeHelpKey(
                        record.evaluationMode,
                      );
                      const libraryNames =
                        record.libraryIds.length === 0
                          ? t("settings.requestRuleLibrariesAll")
                          : record.libraryIds
                              .map(
                                (libraryId) =>
                                  libraries.find(
                                    (library) => library.id === libraryId,
                                  )?.name ?? libraryId,
                              )
                              .join(", ");
                      return (
                        <TableRow
                          data-ui="settings-table-row"
                          key={record.id}
                          id={settingsRequestRuleRowId(record.id)}
                          data-request-rule-mode={record.evaluationMode}
                        >
                          <TableCell className="font-medium">
                            <span id={settingsRequestRuleNameId(record.name)}>
                              {record.name}
                            </span>
                          </TableCell>
                          <TableCell className="max-w-[220px] truncate text-muted-foreground">
                            {record.description || "—"}
                          </TableCell>
                          <TableCell>
                            <div className="space-y-1">
                              <SingleSelectField
                                id={settingsRequestRuleModeId(record.id)}
                                size="sm"
                                value={record.evaluationMode}
                                onValueChange={(value) =>
                                  setRuleMode(
                                    record,
                                    value as RequestEvaluationMode,
                                  )
                                }
                                options={REQUEST_EVALUATION_MODES.map((mode) => {
                                  const key = requestEvaluationModeLabelKey(mode);
                                  return {
                                    value: mode,
                                    ariaLabel: mode,
                                    label: key ? t(key) : mode,
                                  };
                                })}
                              />
                              <ModeBadge mode={record.evaluationMode} />
                              {modeHelpKey ? (
                                <p className="text-xs text-muted-foreground">
                                  {t(modeHelpKey)}
                                </p>
                              ) : null}
                            </div>
                          </TableCell>
                          <TableCell className="max-w-[220px] truncate text-muted-foreground">
                            {libraryNames}
                          </TableCell>
                          <TableCell className="text-center">
                            {record.currentRevisionNumber}
                          </TableCell>
                          <TableCell className="text-center">
                            {record.decisionCount}
                          </TableCell>
                          <TableCell className="text-right">
                            <div className="flex justify-end gap-1">
                              <IconButton
                                id={settingsRequestRuleCopyId(record.id)}
                                label={t("settings.requestRuleCopy")}
                                tone="neutral"
                                onClick={() => copyRuleSet(record)}
                              >
                                <Copy className="h-4 w-4" />
                              </IconButton>
                              <IconButton
                                id={settingsRequestRuleEditId(record.id)}
                                label={t("label.edit")}
                                tone="edit"
                                onClick={() => editRuleSet(record)}
                              >
                                <Edit className="h-4 w-4" />
                              </IconButton>
                              <IconButton
                                id={settingsRequestRuleDeleteId(record.id)}
                                label={t("label.delete")}
                                tone="delete"
                                onClick={() => deleteRuleSet(record)}
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
                        <TableCell colSpan={7} className="text-muted-foreground">
                          {t("settings.noRequestRulesFound")}
                        </TableCell>
                      </TableRow>
                    ) : null}
                  </TableBody>
                </Table>
              </div>
              <p className="border-t border-border px-3 py-2 text-xs text-muted-foreground">
                {t("settings.requestRuleModeHelp")}
              </p>
            </div>

            {isEditorOpen ? (
              <Card>
                <CardHeader>
                  <CardTitle className="text-base">
                    {editingRuleSetId
                      ? t("settings.requestRuleUpdate")
                      : t("settings.requestRuleCreate")}
                  </CardTitle>
                </CardHeader>
                <CardContent>
                  <form
                    id="settings-request-rule-form"
                    className="space-y-3"
                    onSubmit={submitRuleSet}
                  >
                    <div className="grid gap-3 md:grid-cols-2">
                      <label>
                        <Label className="mb-2 block">{t("label.name")}</Label>
                        <Input
                          id="settings-request-rule-name"
                          value={ruleSetDraft.name}
                          onChange={(event) =>
                            setRuleSetDraft((prev) => ({
                              ...prev,
                              name: event.target.value,
                            }))
                          }
                          required
                          placeholder="family_friendly_auto_approve"
                        />
                      </label>
                      <label>
                        <Label className="mb-2 block">
                          {t("settings.ruleDescription")}
                        </Label>
                        <Input
                          id="settings-request-rule-description"
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

                    <div>
                      <Label className="mb-2 block">
                        {t("settings.ruleRegoSource")}
                      </Label>
                      <LazyRegoEditor
                        id="settings-request-rule-rego-source"
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

                    <RequestUserPicker
                      users={users}
                      regoSource={ruleSetDraft.regoSource}
                      applyRequesters={applyRequesters}
                      disabled={mutatingRuleSetId !== null}
                    />

                    <div>
                      <Label className="mb-2 block">
                        {t("settings.requestRuleLibraries")}
                      </Label>
                      <p className="mb-2 text-xs text-muted-foreground">
                        {t("settings.requestRuleLibrariesHelp")}
                      </p>
                      <div className="flex flex-wrap items-center gap-4">
                        {libraries.length === 0 ? (
                          <span className="text-xs text-muted-foreground">
                            {t("settings.requestRuleLibrariesAll")}
                          </span>
                        ) : null}
                        {libraries.map((library) => (
                          <label
                            key={library.id}
                            className="flex items-center gap-2"
                          >
                            <Checkbox
                              id={settingsRequestRuleLibraryId(library.id)}
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

                    {validationResult ? (
                      <div
                        id="settings-request-rule-validation"
                        data-request-rule-valid={
                          validationResult.valid ? "true" : "false"
                        }
                        /* The API refuses a matcher that reads the requester
                           unless the author can manage permissions. Marking
                           that refusal lets the page point at the account
                           picker without rewording the server's message. */
                        data-request-rule-person-targeting-refusal={
                          validationResult.errors.some(isPersonTargetingRefusal)
                            ? "true"
                            : undefined
                        }
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
                            {/* The API's refusals are shown verbatim: the
                                person-targeting message names the permission an
                                author needs, and rewording it would lose that. */}
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
                      {t("settings.requestRuleSaveDisabledNote")}
                    </p>

                    <div className="flex gap-2">
                      <Button
                        id="settings-request-rule-save"
                        type="submit"
                        disabled={mutatingRuleSetId !== null || validating}
                      >
                        {mutatingRuleSetId !== null
                          ? t("label.saving")
                          : editingRuleSetId
                            ? t("settings.requestRuleUpdate")
                            : t("settings.requestRuleCreate")}
                      </Button>
                      <Button
                        id="settings-request-rule-validate"
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
                        id="settings-request-rule-cancel"
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
                id="settings-request-rule-create"
                icon={Plus}
                label={t("settings.requestRuleCreateNew")}
                onClick={startCreateRuleSet}
                disabled={mutatingRuleSetId !== null}
              />
            </div>

            <RequestPreviewPanel
              isEditorOpen={isEditorOpen}
              draftName={ruleSetDraft.name}
              ruleSetRecords={ruleSetRecords}
              libraries={libraries}
              qualityProfiles={qualityProfiles}
              users={users}
              {...previewProps}
            />

            <RequestGatePanel
              gates={gates}
              gatesLocked={gatesLocked}
              savingGate={savingGate}
              setGate={setGate}
            />

            <RequestDecisionsPanel
              decisions={decisions}
              decisionsLoading={decisionsLoading}
              decisionsError={decisionsError}
              decisionOutcomeFilter={decisionOutcomeFilter}
              setDecisionOutcomeFilter={setDecisionOutcomeFilter}
              refreshDecisions={refreshDecisions}
            />
          </div>
        </div>
        <div className="@container w-full space-y-4 xl:w-[44%] xl:max-w-[880px] xl:shrink-0">
          <RequestTemplateGallery onApply={applyTemplate} />
          <RequestContextReference />
        </div>
      </div>
    </div>
  );
}
