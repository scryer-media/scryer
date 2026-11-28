import {
  formatRegoValidationError,
  regoDiagnostics,
} from "@/lib/utils/rego-diagnostics";
import * as React from "react";
import { BookOpen, Copy, Edit, Plus, Sparkles, Trash2 } from "lucide-react";
import { AddNewButton } from "@/components/common/add-new-button";
import { HelpLabel } from "@/components/common/info-help";
import {
  GutterPanel,
  GutterPanelButton,
} from "@/components/common/gutter-panel";
import {
  RULE_REFERENCE_BODY_CLASS,
  RuleSidePanelHeader,
  useRuleSidePanels,
} from "@/components/views/settings/rule-side-panels";
import { RegoAiPromptDialog } from "@/components/common/rego-ai-prompt-dialog";
import type { RequestRulesSection } from "@/components/root/types";
import { RequestRulesTabs } from "@/components/views/settings/request-rules-tabs";
import {
  FORM_SHELL,
  PaneEmpty,
  PaneError,
  PaneSection,
  RefreshButton,
  TABLE_SHELL,
  TOOLBAR_SELECT_LABEL,
  TOOLBAR_SHELL,
} from "@/components/views/settings/rule-pane-section";
import { useUiDateTimeFormat } from "@/lib/context/ui-settings-context";
import { formatUiDateTime } from "@/lib/utils/date-format";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import { LibrarySelect } from "@/components/common/facet-select";
import { IconButton } from "@/components/ui/icon-button";
import {
  Input,
  integerInputProps,
  sanitizeDigits,
} from "@/components/ui/input";
import { LibrarySingleSelect } from "@/components/common/library-single-select";
import { FilterableSelect } from "@/components/ui/filterable-select";
import { Label } from "@/components/ui/label";
import { LazyRegoEditor } from "@/components/common/lazy-rego-editor";
import { EditorValidationStatus } from "@/components/common/editor-validation-status";
import { SingleSelectField } from "@/components/ui/select";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { TableDisclosureButton } from "@/components/ui/table-disclosure-button";
import { RuleTablePager } from "@/components/views/settings/rule-table-pager";
import { SortableHead } from "@/components/views/lists/sortable-head";
import {
  nextRuleTableSort,
  ruleTablePage,
  sortRequestRules,
  type RequestRuleSortKey,
  type RuleTableSort,
} from "@/lib/utils/rule-table";
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
  RequestRuleDecisionRecord,
  RequestRulePreviewResult,
  RequestRulePreviewSample,
  RequestRuleSetDraft,
  RequestRuleSetRecord,
  RequestRuleUserOption,
  RequestRuleValidationResult,
  RequestRuleVoteRecord,
} from "@/lib/types/request-rule-sets";
import {
  REQUEST_FILTER_ALL,
  REQUEST_LEASE_DAYS_MAX,
  requestDecisionOutcomeBadgeTone,
  requestDecisionOutcomeLabelKey,
  requestFallbackReasonLabelKey,
  isPersonTargetingRefusal,
  requestRuleReadsRequester,
  partitionRequestVotes,
  requestVoteBadgeTone,
  requestVoteLabelKey,
  REQUEST_STARTER_SOURCE,
} from "@/lib/utils/request-rule-sets";
import type { RuleBuilderOptions } from "@/components/views/settings/rule-builder";
import type { RuleBuilderSchema } from "@/lib/utils/rule-builder";
import type { RuleBuilderSchemaStatus } from "@/lib/hooks/use-rule-builder-schema";
import { RuleEditorSurface } from "@/components/views/settings/rule-editor-surface";
import {
  RequestDeclaredActionsSummary,
  RequestRuleSourceFindings,
  RequestShapingSummary,
} from "@/components/views/settings/lifecycle-action-summary";
import type { RuleBuilderModel } from "@/lib/utils/rule-builder";
import type { RuleEditorMode } from "@/lib/utils/rule-builder-mode";
import {
  settingsRequestPreviewTitleResultId,
  settingsRequestDecisionRowId,
  settingsRequestRuleCopyId,
  settingsRequestRuleDeleteId,
  settingsRequestRuleEditId,
  settingsRequestRuleEnabledId,
  settingsRequestRuleNameId,
  settingsRequestRuleRowId,
  settingsRequestTemplateId,
  selectorId,
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
  /// Which pane of the page is open: the rule list, a request to try the
  /// rules against, or the decisions they made.
  section: RequestRulesSection;
  isEditorOpen: boolean;
  editingRuleSetId: string | null;
  ruleSetDraft: RequestRuleSetDraft;
  setRuleSetDraft: React.Dispatch<React.SetStateAction<RequestRuleSetDraft>>;
  /// Which source editor the form shows. Simple is only offered while the
  /// draft is still the builder's.
  builderMode: RuleEditorMode;
  setBuilderMode: (mode: "simple" | "view") => void;
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
  applyTemplate: (template: RequestRuleTemplate) => void;
  ruleSetRecords: RequestRuleSetRecord[];
  libraries: LibraryRecord[];
  qualityProfiles: RequestQualityProfileOption[];
  /// Accounts a preview can run as. Empty when the reader may not list users,
  /// which is a permission answer rather than a page failure.
  users: RequestRuleUserOption[];
  copyRuleSet: (record: RequestRuleSetRecord) => void;
  editRuleSet: (record: RequestRuleSetRecord) => void;
  deleteRuleSet: (record: RequestRuleSetRecord) => void;
  validating: boolean;
  validationResult: RequestRuleValidationResult | null;
  setRuleEnabled: (record: RequestRuleSetRecord, enabled: boolean) => void;
  /// Saving a rule that reads requester fields needs Manage permissions; the
  /// editor warns before save rather than after the API refuses.
  canManagePermissions: boolean;
  previewRuleSetId: string;
  setPreviewRuleSetId: (id: string) => void;
  previewSample: RequestRulePreviewSample;
  setPreviewSample: React.Dispatch<
    React.SetStateAction<RequestRulePreviewSample>
  >;
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

/// What submitting the sample report would do, for a report rule's preview.
const REPORT_OUTCOME_KEYS = {
  REVIEW: "settings.requestPreviewReportReview",
  REJECT: "settings.requestPreviewReportReject",
  MARK_BAD: "settings.requestPreviewReportMarkBad",
} as const;

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

const REQUEST_TEMPLATES_PANEL_ID = "settings-request-template-gallery";
const REQUEST_REFERENCE_PANEL_ID = "settings-request-reference-panel";
type RequestSidePanel = "templates" | "reference";

function RequestContextReference() {
  const t = useTranslate();

  return (
    <div>
      <RuleSidePanelHeader
        icon={<BookOpen className="h-4 w-4" />}
        title={t("settings.refReqTitle")}
      />
      <div
        id="settings-request-rules-reference"
        className={RULE_REFERENCE_BODY_CLASS}
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
            <li>{t("settings.refReqOutputSetQualityProfile")}</li>
            <li>{t("settings.refReqOutputSetLeaseDays")}</li>
            <li>{t("settings.refReqOutputSetMonitor")}</li>
            <li>{t("settings.refReqOutputSetAudioLanguages")}</li>
            <li>{t("settings.refReqOutputTags")}</li>
            <li>{t("settings.refReqOutputNotify")}</li>
            <li>{t("settings.refReqOutputReasons")}</li>
            <li>{t("settings.refReqOutputShapingOutcome")}</li>
            <li>{t("settings.refReqOutputNoSchedule")}</li>
            <li>{t("settings.refReqOutputNoPackage")}</li>
          </ul>
        </div>

        {REF_SECTIONS.map((section) => (
          <RefFieldTable key={section.path} section={section} />
        ))}
      </div>
    </div>
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
  return (
    <div>
      <RuleSidePanelHeader
        icon={<Sparkles className="h-4 w-4" />}
        title={
          <HelpLabel
            label={t("settings.requestTemplateGallery")}
            help={t("settings.requestTemplateGalleryTagNote")}
          />
        }
      />
      <div className="grid grid-cols-1 gap-2 p-3">
        {REQUEST_RULE_TEMPLATES.map((template) => (
          <RequestTemplateCard
            key={template.id}
            template={template}
            onApply={onApply}
          />
        ))}
      </div>
    </div>
  );
}

function DecisionVoteItem({
  vote,
  showError,
}: {
  vote: RequestRuleVoteRecord;
  showError: boolean;
}) {
  const t = useTranslate();
  const failed = vote.vote === null && vote.error !== null;
  const labelKey = failed
    ? "requests.voteFailed"
    : requestVoteLabelKey(vote.vote);
  return (
    <li>
      <span className="text-[var(--scry-ink2)]">{vote.ruleSetName}</span>{" "}
      <Badge
        tone={failed ? "negative" : requestVoteBadgeTone(vote.vote)}
        className="text-[10px]"
        title={vote.error ?? undefined}
      >
        {labelKey ? t(labelKey) : (vote.vote ?? "—")}
        {vote.held ? ` · ${t("settings.requestDecisionHeld")}` : ""}
      </Badge>
      {vote.error && showError ? (
        <p
          data-code-font
          className="mt-0.5 whitespace-pre-wrap text-xs text-[var(--scry-muted3)] [overflow-wrap:anywhere]"
        >
          {vote.error}
        </p>
      ) : null}
    </li>
  );
}

/// The votes that decided, always shown; every other rule's vote only once the
/// row is opened.
function DecisionVotes({
  decision,
  expanded,
  onExpand,
}: {
  decision: RequestRuleDecisionRecord;
  expanded: boolean;
  onExpand?: () => void;
}) {
  const t = useTranslate();
  const { signal, rest } = partitionRequestVotes(decision.votes);
  return (
    <div className="space-y-1.5">
      {signal.length > 0 ? (
        <ul className="list-disc space-y-1 pl-4 text-sm">
          {signal.map((vote) => (
            <DecisionVoteItem
              key={`${vote.ruleSetId}:${vote.revisionNumber}`}
              vote={vote}
              showError={expanded}
            />
          ))}
        </ul>
      ) : (
        <p className="text-muted-foreground">
          {t("settings.requestDecisionNoVotes")}
        </p>
      )}
      {rest.length > 0 && expanded ? (
        <ul className="list-disc space-y-1 pl-4 text-sm text-muted-foreground">
          {rest.map((vote) => (
            <DecisionVoteItem
              key={`${vote.ruleSetId}:${vote.revisionNumber}`}
              vote={vote}
              showError={expanded}
            />
          ))}
        </ul>
      ) : null}
      {rest.length > 0 && !expanded ? (
        <button
          type="button"
          onClick={onExpand}
          className="text-xs text-[var(--scry-accent-text)] underline-offset-2 hover:underline focus-visible:underline focus-visible:outline-none"
        >
          {t("settings.requestDecisionOtherRules", { count: rest.length })}
        </button>
      ) : null}
    </div>
  );
}

function DecisionRow({
  decision,
  index,
  expanded,
  onToggle,
}: {
  decision: RequestRuleDecisionRecord;
  index: number;
  expanded: boolean;
  onToggle: () => void;
}) {
  const t = useTranslate();
  const dateTimeFormat = useUiDateTimeFormat();
  const { rest } = partitionRequestVotes(decision.votes);
  const discloses =
    rest.length > 0 || decision.votes.some((vote) => vote.error !== null);
  const failedRules = decision.votes
    .filter((vote) => vote.error !== null)
    .map((vote) => vote.ruleSetName)
    .sort((a, b) => a.localeCompare(b));
  const fallbackText =
    decision.fallbackReason === "error" && failedRules.length > 0
      ? t("requests.fallbackErrorRules", { rules: failedRules.join(", ") })
      : decision.fallbackReason
        ? (() => {
            const key = requestFallbackReasonLabelKey(decision.fallbackReason);
            return key ? t(key) : decision.fallbackReason;
          })()
        : null;
  const rowId = settingsRequestDecisionRowId(index);
  return (
    <TableRow
      data-ui="settings-table-row"
      id={rowId}
      data-request-decision-outcome={decision.effectiveOutcome}
      className="[&>td]:align-middle"
    >
      <TableCell className="w-[1%] pr-0">
        {discloses ? (
          <TableDisclosureButton
            id={`${rowId}-toggle`}
            expanded={expanded}
            aria-label={t("settings.requestDecisionsColVotes")}
            onClick={onToggle}
          />
        ) : null}
      </TableCell>
      <TableCell className="whitespace-nowrap text-[var(--scry-muted)]">
        {formatUiDateTime(decision.evaluatedAt, dateTimeFormat, {
          fallback: decision.evaluatedAt,
        })}
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
        <DecisionVotes
          decision={decision}
          expanded={expanded}
          onExpand={onToggle}
        />
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
          {fallbackText ? <p className="text-xs">{fallbackText}</p> : null}
          {decision.reasons.length === 0 && !fallbackText ? "—" : null}
        </div>
      </TableCell>
    </TableRow>
  );
}

/// The author's preview. Unlike the requester's pre-flight it shows the input
/// document the matcher actually saw, which is the only honest answer to "why
/// did this not match".
function RequestPreviewPanel({
  ruleSetRecords,
  libraries,
  qualityProfiles,
  users,
  previewRuleSetId,
  setPreviewRuleSetId,
  previewSample,
  setPreviewSample,
  searchPreviewTitles,
  runPreview,
  previewing,
  previewResult,
  previewError,
}: Pick<
  SettingsRequestRulesSectionProps,
  | "ruleSetRecords"
  | "libraries"
  | "qualityProfiles"
  | "users"
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
  const [titleResults, setTitleResults] = React.useState<
    MetadataTvdbSearchItem[]
  >([]);
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

  const canRun =
    !previewing &&
    Boolean(previewSample.userId) &&
    Boolean(previewSample.libraryId) &&
    previewSample.externalIds.length > 0 &&
    Boolean(previewRuleSetId);

  const decision = previewResult?.decision ?? null;

  return (
    <PaneSection
      id="settings-request-preview-section"
      title={t("settings.requestPreviewTitle")}
      help={t("settings.requestPreviewSubtitle")}
    >
      <div className={FORM_SHELL}>
        <div className="grid gap-3 md:grid-cols-2">
          <div className="flex min-w-0 flex-col gap-1.5">
            <label
              htmlFor="settings-request-preview-rule"
              className="block text-sm font-medium text-[var(--scry-ink2)]"
            >
              {t("settings.requestPreviewRuleSelect")}
            </label>
            <FilterableSelect
              id="settings-request-preview-rule"
              optionIdPrefix="settings-request-preview-rule"
              filterByValue={false}
              placeholder={t("settings.requestPreviewRuleSelectPlaceholder")}
              value={previewRuleSetId}
              onValueChange={setPreviewRuleSetId}
              options={ruleSetRecords.map((record) => ({
                value: record.id,
                label: record.name,
              }))}
            />
          </div>
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
          <LibrarySingleSelect
            id="settings-request-preview-library"
            label={t("settings.requestPreviewLibrary")}
            placeholder={t("settings.requestPreviewLibraryPlaceholder")}
            value={previewSample.libraryId}
            onValueChange={(value) =>
              setPreviewSample((prev) => ({ ...prev, libraryId: value }))
            }
            libraries={libraries}
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
            <div className="flex min-w-0 flex-col gap-1.5">
              <label
                htmlFor="settings-request-preview-lease-days"
                className="block text-sm font-medium text-[var(--scry-ink2)]"
              >
                {t("settings.requestPreviewLeaseDays")}
              </label>
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
            </div>
          )}
        </div>

        <div>
          <Label className="mb-2 block">
            {t("settings.requestPreviewMedia")}
          </Label>
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
                const key = String(
                  result.smgId ?? result.tvdbId ?? result.name,
                );
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
            {previewResult.reportOutcome ? (
              <p
                id="settings-request-preview-report-outcome"
                className="text-sm font-medium"
              >
                {t(REPORT_OUTCOME_KEYS[previewResult.reportOutcome])}
              </p>
            ) : null}
            <div className="flex flex-wrap items-center gap-2">
              <OutcomeBadge outcome={decision.policyOutcome} />
              {decision.votes[0]?.held ? (
                <Badge tone="warning">
                  {t("settings.requestDecisionHeld")}
                </Badge>
              ) : null}
              {previewResult.metadataPartial ? (
                <Badge tone="warning">
                  {t("settings.requestPreviewMetadataPartial")}
                </Badge>
              ) : null}
            </div>
            <DecisionVotes decision={decision} expanded />
            <RequestShapingSummary
              shaping={decision.shaping}
              idPrefix="settings-request-preview-shaping"
            />
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
              <div
                id="settings-request-preview-undefined-tags"
                className="space-y-1"
              >
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
        ) : null}
      </div>
    </PaneSection>
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
  const [expandedDecisions, setExpandedDecisions] = React.useState<
    ReadonlySet<string>
  >(() => new Set());
  const toggleDecision = (key: string) =>
    setExpandedDecisions((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  return (
    <PaneSection
      id="settings-request-decisions-section"
      title={t("settings.requestDecisionsTitle")}
      count={
        decisionsLoading && decisions.length === 0
          ? undefined
          : decisions.length
      }
      actions={
        <RefreshButton
          id="settings-request-decisions-refresh"
          loading={decisionsLoading}
          onClick={refreshDecisions}
        />
      }
    >
      <div className={TOOLBAR_SHELL}>
        <SingleSelectField
          id="settings-request-decisions-outcome"
          label={t("settings.requestDecisionsOutcomeFilter")}
          labelClassName={TOOLBAR_SELECT_LABEL}
          size="compact"
          chrome="toolbar"
          className="md:w-[240px]"
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
      </div>

      <PaneError message={decisionsError} />

      {decisions.length === 0 ? (
        <PaneEmpty
          id="settings-request-decisions-empty"
          loading={decisionsLoading}
        >
          {t("settings.requestDecisionsEmpty")}
        </PaneEmpty>
      ) : (
        <Table id="settings-request-decisions" wrapperClassName={TABLE_SHELL}>
          <TableHeader>
            <TableRow>
              <TableHead className="w-[1%]" />
              <TableHead className="w-[190px]">
                {t("settings.requestDecisionsColTime")}
              </TableHead>
              <TableHead className="w-[150px]">
                {t("settings.requestDecisionsColOutcome")}
              </TableHead>
              <TableHead className="min-w-[320px]">
                {t("settings.requestDecisionsColVotes")}
              </TableHead>
              <TableHead>{t("settings.requestDecisionsColReasons")}</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {decisions.map((decision, index) => {
              const key = decision.id ?? `${decision.evaluatedAt}:${index}`;
              return (
                <DecisionRow
                  key={key}
                  decision={decision}
                  index={index}
                  expanded={expandedDecisions.has(key)}
                  onToggle={() => toggleDecision(key)}
                />
              );
            })}
          </TableBody>
        </Table>
      )}
    </PaneSection>
  );
}

export function SettingsRequestRulesSection({
  section,
  isEditorOpen,
  editingRuleSetId,
  ruleSetDraft,
  setRuleSetDraft,
  builderMode,
  setBuilderMode,
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
  qualityProfiles,
  users,
  copyRuleSet,
  editRuleSet,
  deleteRuleSet,
  validating,
  validationResult,
  setRuleEnabled,
  canManagePermissions,
  decisions,
  decisionsLoading,
  decisionsError,
  decisionOutcomeFilter,
  setDecisionOutcomeFilter,
  refreshDecisions,
  ...previewProps
}: SettingsRequestRulesSectionProps) {
  const t = useTranslate();
  const {
    rootRef: sidePanelRootRef,
    columnRef: sidePanelColumnRef,
    buttonProps: sidePanelButtonProps,
    panelProps: sidePanelProps,
    close: closeSidePanel,
  } = useRuleSidePanels<RequestSidePanel>(section === "rules");
  // The rule list arrives whole; sorting and paging stay in the page.
  const [ruleSort, setRuleSort] = React.useState<
    RuleTableSort<RequestRuleSortKey>
  >({ key: "name", descending: false });
  const [rulePageIndex, setRulePageIndex] = React.useState(0);
  const libraryNameById = React.useMemo(
    () => new Map(libraries.map((library) => [library.id, library.name])),
    [libraries],
  );
  const libraryLabel = React.useCallback(
    (record: RequestRuleSetRecord) =>
      record.libraryIds.length === 0
        ? t("settings.requestRuleLibrariesAll")
        : record.libraryIds
            .map((libraryId) => libraryNameById.get(libraryId) ?? libraryId)
            .join(", "),
    [libraryNameById, t],
  );
  const sortedRules = React.useMemo(
    () => sortRequestRules(ruleSetRecords, ruleSort, libraryLabel),
    [ruleSetRecords, ruleSort, libraryLabel],
  );
  const rulePage = ruleTablePage(sortedRules, rulePageIndex);
  const validationStatus = (
    <EditorValidationStatus
      id="settings-request-rule-validation"
      checking={validating}
      result={validationResult}
      formatError={(error) =>
        formatRegoValidationError(error, ruleSetDraft.regoSource)
      }
      data-request-rule-valid={
        validationResult ? String(validationResult.valid) : undefined
      }
      data-request-rule-person-targeting-refusal={
        validationResult?.diagnostics?.some(isPersonTargetingRefusal)
          ? "true"
          : undefined
      }
    />
  );

  // The starter rules and the input reference help write a rule, so their
  // buttons sit on the tab strip only while the rule list is open.
  const pageHead = (
    <RequestRulesTabs
      activeSection={section}
      actions={
        section === "rules" ? (
          <>
            <GutterPanelButton
              panelId={REQUEST_TEMPLATES_PANEL_ID}
              label={t("settings.requestTemplateGallery")}
              icon={<Sparkles className="h-4 w-4" />}
              {...sidePanelButtonProps("templates")}
            />
            <GutterPanelButton
              panelId={REQUEST_REFERENCE_PANEL_ID}
              label={t("settings.refReqTitle")}
              icon={<BookOpen className="h-4 w-4" />}
              {...sidePanelButtonProps("reference")}
            />
          </>
        ) : null
      }
    />
  );

  // Trying a request and reading decisions are one pane each, in the same
  // column width as the rule list so the tabs do not move between them. A
  // draft open in the rule editor stays open, so it can still be tried here.
  if (section !== "rules") {
    return (
      <div id="settings-request-rules-section" className="text-sm">
        <div className="mx-auto w-full max-w-[1280px] space-y-6">
          {pageHead}
          {section === "try" ? (
            <RequestPreviewPanel
              ruleSetRecords={ruleSetRecords}
              libraries={libraries}
              qualityProfiles={qualityProfiles}
              users={users}
              {...previewProps}
            />
          ) : (
            <RequestDecisionsPanel
              decisions={decisions}
              decisionsLoading={decisionsLoading}
              decisionsError={decisionsError}
              decisionOutcomeFilter={decisionOutcomeFilter}
              setDecisionOutcomeFilter={setDecisionOutcomeFilter}
              refreshDecisions={refreshDecisions}
            />
          )}
        </div>
      </div>
    );
  }

  return (
    <div
      ref={sidePanelRootRef}
      id="settings-request-rules-section"
      className="space-y-4 text-sm"
    >
      <div
        ref={sidePanelColumnRef}
        className="mx-auto w-full max-w-[1280px] space-y-4"
      >
        <div className="pb-2">{pageHead}</div>

        <div className="overflow-hidden rounded border border-border bg-card">
          <div className="flex items-center justify-between border-b border-border px-3 py-2">
            <CardTitle className="text-base text-foreground">
              <HelpLabel
                label={t("settings.requestRules")}
                help={t("settings.requestRulesDefault")}
              />
            </CardTitle>
            <RegoAiPromptDialog
              id="settings-request-rules-ai-prompt"
              ruleKind="request decision"
              inputContract={requestInputContract}
              outputContract={
                'At least one vote or change. Votes: approve, deny, manual if { ... }. Changes, literals only: set_quality_profile := "name"; set_lease_days := 30 (0 forever, else 1-3650); set_monitor := MONITORED|UNMONITORED (movies), FUTURE_EPISODES|MISSING_AND_FUTURE_EPISODES|ALL_EPISODES|NONE (series, anime); set_required_audio_languages, set_preferred_audio_languages := ["eng"] (ISO 639-2); add_tags := ["tag"]; notify := "text". Optional reasons contains "id" if { ... }. Only reasons or notify is refused. No schedule.'
              }
            />
          </div>
          <div className="overflow-x-auto">
            <Table>
              <TableHeader>
                <TableRow>
                  {(
                    [
                      ["name", t("label.name"), undefined],
                      ["description", t("settings.ruleDescription"), undefined],
                      ["enabled", t("label.enabled"), "w-[1%] text-center"],
                      [
                        "libraries",
                        t("settings.requestRuleLibraries"),
                        undefined,
                      ],
                    ] as const
                  ).map(([key, label, className]) => (
                    <SortableHead
                      key={key}
                      id={selectorId("settings-request-rules-sort", key)}
                      label={label}
                      className={className}
                      active={ruleSort.key === key}
                      descending={ruleSort.descending}
                      onSort={() => {
                        setRuleSort((current) =>
                          nextRuleTableSort(current, key),
                        );
                        setRulePageIndex(0);
                      }}
                    />
                  ))}
                  <TableHead>
                    {t("settings.requestDeclaredActionsTitle")}
                  </TableHead>
                  <TableHead className="text-right">
                    {t("label.actions")}
                  </TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {rulePage.rows.map((record) => {
                  const libraryNames = libraryLabel(record);
                  return (
                    <TableRow
                      data-ui="settings-table-row"
                      key={record.id}
                      id={settingsRequestRuleRowId(record.id)}
                      data-request-rule-enabled={
                        record.enabled ? "true" : "false"
                      }
                    >
                      <TableCell className="font-medium">
                        <span id={settingsRequestRuleNameId(record.name)}>
                          {record.name}
                        </span>
                      </TableCell>
                      <TableCell className="max-w-[220px] truncate text-muted-foreground">
                        {record.description || "—"}
                      </TableCell>
                      <TableCell className="text-center">
                        <Checkbox
                          id={settingsRequestRuleEnabledId(record.id)}
                          size="large"
                          checked={record.enabled}
                          disabled={mutatingRuleSetId === record.id}
                          aria-label={`${t("label.enabled")}: ${record.name}`}
                          onCheckedChange={(checked) =>
                            setRuleEnabled(record, checked === true)
                          }
                        />
                      </TableCell>
                      <TableCell className="max-w-[220px] truncate text-muted-foreground">
                        {libraryNames}
                      </TableCell>
                      <TableCell className="max-w-[320px]">
                        {record.declaredActions ? (
                          <RequestDeclaredActionsSummary
                            declared={record.declaredActions}
                            idPrefix={selectorId(
                              "settings-request-rule-declared-action",
                              record.id,
                            )}
                          />
                        ) : null}
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
                    <TableCell colSpan={6} className="text-muted-foreground">
                      {t("settings.noRequestRulesFound")}
                    </TableCell>
                  </TableRow>
                ) : null}
              </TableBody>
            </Table>
          </div>
          <RuleTablePager
            idPrefix="settings-request-rules"
            page={rulePage}
            onPageChange={setRulePageIndex}
          />
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

                <RuleEditorSurface
                  idPrefix="settings-request-rule"
                  schema={builderSchema}
                  builderStatus={builderStatus}
                  variant={null}
                  mode={builderMode}
                  onModeChange={setBuilderMode}
                  available
                  onEditAsRego={editAsRego}
                  model={ruleSetDraft.builderModel}
                  onModelChange={setBuilderModel}
                  options={builderOptions}
                  generatedSource={ruleSetDraft.regoSource}
                  feedback={validationStatus}
                  starterSource={REQUEST_STARTER_SOURCE}
                  editor={(readOnly, source) => (
                    <LazyRegoEditor
                      ruleFamily="request"
                      id="settings-request-rule-rego-source"
                      value={source}
                      readOnly={readOnly}
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

                <RequestRuleSourceFindings
                  source={ruleSetDraft.regoSource}
                  checked={
                    !validating &&
                    validationResult !== null &&
                    !validationResult.unavailable
                  }
                  warnings={validationResult?.warnings ?? []}
                />

                <div>
                  <Label className="mb-2 block">
                    {t("settings.requestRuleLibraries")}
                  </Label>
                  <p className="mb-2 text-xs text-muted-foreground">
                    {t("settings.requestRuleLibrariesHelp")}
                  </p>
                  <LibrarySelect
                    idPrefix="settings-request-rule-library"
                    libraries={libraries}
                    selected={ruleSetDraft.libraryIds}
                    emptyLabel={t("settings.requestRuleLibrariesAll")}
                    onChange={(libraryIds) =>
                      setRuleSetDraft((prev) => ({ ...prev, libraryIds }))
                    }
                  />
                </div>

                {!canManagePermissions &&
                requestRuleReadsRequester(ruleSetDraft.regoSource) ? (
                  <p
                    id="settings-request-rule-requester-hint"
                    className="rounded border border-[var(--scry-warning-border)] bg-[var(--scry-warning-bg)] px-3 py-2 text-xs text-[var(--scry-warning-text)]"
                  >
                    {t("settings.requestRuleRequesterFieldsHint")}
                  </p>
                ) : null}

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
      </div>
      <GutterPanel
        id={REQUEST_TEMPLATES_PANEL_ID}
        label={t("settings.requestTemplateGallery")}
        {...sidePanelProps("templates")}
      >
        <RequestTemplateGallery
          onApply={(template) => {
            applyTemplate(template);
            closeSidePanel();
          }}
        />
      </GutterPanel>
      <GutterPanel
        id={REQUEST_REFERENCE_PANEL_ID}
        label={t("settings.refReqTitle")}
        {...sidePanelProps("reference")}
      >
        <RequestContextReference />
      </GutterPanel>
    </div>
  );
}
