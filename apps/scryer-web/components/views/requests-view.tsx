import * as React from "react";
import {
  BookOpen,
  Check,
  CircleCheck,
  Clock,
  History,
  Inbox,
  Loader2,
  Pencil,
  Plus,
  RefreshCw,
  ScrollText,
  ShieldX,
  SlidersVertical,
  Timer,
  User,
  X,
  type LucideIcon,
} from "lucide-react";

import {
  AnidbExternalLink,
  AnilistExternalLink,
  ImdbExternalLink,
  MalExternalLink,
  TmdbExternalLink,
  TvdbMovieExternalLink,
  TvdbSeriesExternalLink,
} from "@/components/common/external-media-links";
import { TitleRatingsDisplay } from "@/components/common/title-ratings-display";
import { LibraryMultiSelect } from "@/components/common/library-multi-select";
import { MonitorSelectionPicker } from "@/components/common/monitor-selection-picker";
import { UnderlineFilterButton } from "@/components/common/underline-filter-button";
import { TitlePoster } from "@/components/title-poster";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input, integerInputProps, sanitizeDigits } from "@/components/ui/input";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { useTranslate } from "@/lib/context/translate-context";
import { useUiDateTimeFormat } from "@/lib/context/ui-settings-context";
import type { LibraryRecord, MediaRequestRecord } from "@/lib/types";
import type {
  RequestRuleDecisionRecord,
  TitleClaimRecord,
} from "@/lib/types/request-rule-sets";
import type { MonitorSelectionDraft } from "@/lib/types/titles";
import {
  REQUEST_LEASE_DAYS_MAX,
  REQUEST_LEASE_DAYS_MIN,
  REQUEST_LEASE_DAY_CHOICES,
  clampRequestLeaseDays,
  requestDecisionOutcomeBadgeTone,
  requestDecisionOutcomeLabelKey,
  requestFallbackReasonLabelKey,
  requestLeaseBadge,
  requestLeaseBadgeTone,
  requestVoteBadgeTone,
  requestVoteLabelKey,
  titleClaimStateBadgeTone,
  titleClaimStateLabelKey,
} from "@/lib/utils/request-rule-sets";
import {
  EMPTY_MONITOR_SELECTION,
  isMonitorSelectionEmpty,
  monitorSelectionFromRecord,
  monitorSelectionInput,
  monitorSelectionSummaryParts,
} from "@/lib/utils/monitor-selection";
import type { UiDateTimeFormat } from "@/lib/types/settings";
import { formatUiDateTime } from "@/lib/utils/date-format";
import {
  APPROVE_MEDIA_REQUEST_LEASE_DAYS_ID,
  APPROVE_MEDIA_REQUEST_LEASE_ID,
  APPROVE_MEDIA_REQUEST_TAG_ADD_ID,
  APPROVE_MEDIA_REQUEST_TAG_INPUT_ID,
  TITLE_CLAIM_EXTEND_DATE_ID,
  TITLE_CLAIM_RELEASE_REASON_ID,
  approveMediaRequestLeaseOptionId,
  approveMediaRequestTagRemoveId,
  mediaRequestApproveId,
  mediaRequestCancelId,
  mediaRequestClaimsPanelId,
  mediaRequestClaimsToggleId,
  mediaRequestDecisionId,
  mediaRequestDecisionPopoverId,
  mediaRequestDenyReasonId,
  mediaRequestDismissId,
  mediaRequestEditId,
  mediaRequestLeaseId,
  mediaRequestMonitorOptionId,
  mediaRequestMonitorSelectionId,
  mediaRequestPolicyTagsId,
  mediaRequestProfileOptionId,
  mediaRequestRowId,
  mediaRequestStatusId,
  titleClaimExtendId,
  titleClaimPermanentId,
  titleClaimReleaseId,
  titleClaimRowId,
} from "@/lib/utils/dom-ids";
import {
  selectBackdropVariantUrl,
  selectPosterVariantUrl,
} from "@/lib/utils/poster-images";
import { normalizeTitleExternalRating } from "@/lib/utils/title-ratings";
import { cn } from "@/lib/utils";

type QualityProfileOption = {
  id: string;
  name: string;
};

type RequestsMode = "admin" | "mine";
type RequestStatusFilter = "all" | MediaRequestRecord["status"];
type RequestFacetFilter = MediaRequestRecord["facet"];

type RequestMonitorType =
  | "MONITORED"
  | "UNMONITORED"
  | "FUTURE_EPISODES"
  | "MISSING_AND_FUTURE_EPISODES"
  | "ALL_EPISODES"
  | "NONE"
  | "ADVANCED";

type UpdateRequestValues = {
  requestedQualityProfileId: string;
  requestedMonitorType?: RequestMonitorType;
  requestedMonitorSelection?: MonitorSelectionDraft;
};

type ApproveRequestValues = {
  qualityProfileId: string;
  monitorType?: RequestMonitorType;
  monitorSelection?: MonitorSelectionDraft;
  /// The lease the approver granted. `leaseDays` and `leaseForever` are
  /// mutually exclusive — the API refuses both — and sending neither keeps
  /// whatever the requester asked for.
  leaseDays?: number;
  leaseForever?: boolean;
  /// Tags to stamp on the created title. Prefilled from what the policy
  /// emitted, and editable, because the approver is the one who decides what
  /// the title ends up carrying.
  tags?: string[];
};

/// What the approver does with the lease the requester asked for.
type ApprovalLeaseChoice = "requested" | "forever" | "custom" | `${number}`;

/// A claim action waiting on the operator to fill in its one detail.
type PendingClaimAction =
  | { kind: "extend"; claim: TitleClaimRecord; expiresAt: string }
  | { kind: "release"; claim: TitleClaimRecord; reason: string }
  | null;

type RequestsViewProps = {
  mode: RequestsMode;
  canShowAdminMode: boolean;
  canShowRequesterMode: boolean;
  onModeChange: (mode: RequestsMode) => void;
  statusFilter: RequestStatusFilter;
  onStatusFilterChange: (status: RequestStatusFilter) => void;
  libraries: LibraryRecord[];
  selectedLibraryIds: string[];
  onSelectedLibraryIdsChange: (libraryIds: string[]) => void;
  requests: MediaRequestRecord[];
  qualityProfileOptions: QualityProfileOption[];
  loading: boolean;
  actionRequestId: string | null;
  onRefresh: () => void;
  onLoadQualityProfileOptions: () => void;
  onApprove: (request: MediaRequestRecord, values: ApproveRequestValues) => void;
  onDismiss: (request: MediaRequestRecord) => void;
  onUpdateRequest: (request: MediaRequestRecord, values: UpdateRequestValues) => void;
  onCancelRequest: (request: MediaRequestRecord) => void;
  /// Retention claims for the titles requests created, keyed by request id and
  /// loaded only when the operator opens a row's claims panel. A row whose id
  /// is absent has not been asked for yet.
  claimsByRequestId: Record<string, TitleClaimRecord[]>;
  claimsLoadingRequestId: string | null;
  onLoadClaims: (request: MediaRequestRecord) => void;
  onExtendClaim: (claim: TitleClaimRecord, expiresAt: string) => void;
  onConvertClaim: (claim: TitleClaimRecord) => void;
  onReleaseClaim: (claim: TitleClaimRecord, reason: string) => void;
  claimActionId: string | null;
};

function requesterLabel(request: MediaRequestRecord): string {
  return request.requesters
    .map((requester) => requester.username)
    .filter(Boolean)
    .join(", ");
}

function requestExternalIdValue(
  request: MediaRequestRecord,
  source: string,
): string | undefined {
  return request.externalIds.find(
    (externalId) => externalId.source.toLowerCase() === source,
  )?.value;
}

function RequesterAvatarStack({ request }: { request: MediaRequestRecord }) {
  const avatarRequesters = request.requesters.filter((requester) =>
    requester.avatarUrl?.trim(),
  );
  if (avatarRequesters.length === 0) {
    return null;
  }
  return (
    <span className="inline-flex -space-x-2 align-middle">
      {avatarRequesters.map((requester) => (
        <img
          key={requester.userId}
          src={requester.avatarUrl ?? ""}
          alt=""
          title={requester.username}
          className="h-6 w-6 rounded-full border border-background bg-muted object-cover ring-1 ring-border"
          loading="lazy"
        />
      ))}
    </span>
  );
}

function profileLabel(
  profileId: string | null | undefined,
  profileName: string | null | undefined,
  qualityProfileOptions: QualityProfileOption[],
): string | null {
  if (profileName?.trim()) {
    return profileName.trim();
  }
  const normalizedId = profileId?.trim();
  if (!normalizedId) {
    return null;
  }
  return (
    qualityProfileOptions.find((profile) => profile.id === normalizedId)?.name ??
    normalizedId
  );
}

function monitorTypeLabel(t: ReturnType<typeof useTranslate>, value: string | null | undefined): string | null {
  const normalized = value?.replace(/[-_\s]/g, "").toLowerCase();
  switch (normalized) {
    case "monitored":
      return t("search.monitorType.monitored");
    case "unmonitored":
      return t("search.monitorType.unmonitored");
    case "futureepisodes":
      return t("search.monitorType.futureEpisodes");
    case "missingandfutureepisodes":
      return t("search.monitorType.missingAndFutureEpisodes");
    case "allepisodes":
      return t("search.monitorType.allEpisodes");
    case "none":
      return t("search.monitorType.none");
    case "advanced":
      return t("search.monitorType.advanced");
    default:
      return value?.trim() || null;
  }
}

function monitorTypeSelectValue(
  facet: MediaRequestRecord["facet"],
  value: string | null | undefined,
): RequestMonitorType {
  const normalized = value?.replace(/[-_\s]/g, "").toLowerCase();
  switch (normalized) {
    case "monitored":
      return "MONITORED";
    case "unmonitored":
      return "UNMONITORED";
    case "missingandfutureepisodes":
      return "MISSING_AND_FUTURE_EPISODES";
    case "allepisodes":
      return "ALL_EPISODES";
    case "none":
      return "NONE";
    case "advanced":
      return "ADVANCED";
    case "futureepisodes":
    default:
      return facet === "MOVIE" ? "MONITORED" : "FUTURE_EPISODES";
  }
}

function monitorOptions(t: ReturnType<typeof useTranslate>): Array<{ value: RequestMonitorType; label: string }> {
  return [
    { value: "FUTURE_EPISODES", label: t("search.monitorType.futureEpisodes") },
    {
      value: "MISSING_AND_FUTURE_EPISODES",
      label: t("search.monitorType.missingAndFutureEpisodes"),
    },
    { value: "ALL_EPISODES", label: t("search.monitorType.allEpisodes") },
    { value: "NONE", label: t("search.monitorType.none") },
    { value: "ADVANCED", label: t("search.monitorType.advanced") },
  ];
}

function requestProfileOptionsForLibrary(
  libraries: LibraryRecord[],
  libraryId: string,
  qualityProfileOptions: QualityProfileOption[],
): QualityProfileOption[] {
  const library = libraries.find((library) => library.id === libraryId);
  const requestProfileIds = library?.requestQualityProfileIds?.length
    ? library.requestQualityProfileIds
    : library?.requestQualityProfileDefaultId
      ? [library.requestQualityProfileDefaultId]
      : [];
  return requestProfileIds.map((profileId) => {
    const profile = qualityProfileOptions.find((option) => option.id === profileId);
    return {
      id: profileId,
      name: profile?.name ?? profileId,
    };
  });
}

function requestStatusLabel(t: ReturnType<typeof useTranslate>, status: MediaRequestRecord["status"]): string {
  switch (status) {
    case "PENDING":
      return t("requests.status.pending");
    case "APPROVED":
      return t("requests.status.approved");
    case "REJECTED":
      return "Dismissed";
    case "CANCELED":
      return t("requests.status.canceled");
    default:
      return status;
  }
}

function requestStatusTone(
  t: ReturnType<typeof useTranslate>,
  status: MediaRequestRecord["status"],
): { label: string; Icon: LucideIcon; className: string } {
  switch (status) {
    case "PENDING":
      return {
        label: requestStatusLabel(t, status),
        Icon: Clock,
        className: "border-[var(--scry-warning-border)] bg-[var(--scry-warning-bg)] text-[var(--scry-warning-text)]",
      };
    case "APPROVED":
      return {
        label: requestStatusLabel(t, status),
        Icon: Check,
        className: "border-[var(--scry-success-border)] bg-[var(--scry-success-bg)] text-[var(--scry-success-text)]",
      };
    case "REJECTED":
      return {
        label: requestStatusLabel(t, status),
        Icon: X,
        className: "border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)] text-[var(--scry-danger-text)]",
      };
    case "CANCELED":
    default:
      return {
        label: requestStatusLabel(t, status),
        Icon: ShieldX,
        className: "border-border bg-background text-muted-foreground",
      };
  }
}

function statusFilterOptions(mode: RequestsMode): Array<{
  value: RequestStatusFilter;
  label: string;
}> {
  if (mode === "admin") {
    return [
      { value: "PENDING", label: "Pending" },
      { value: "APPROVED", label: "Approved" },
      { value: "REJECTED", label: "Dismissed" },
    ];
  }

  return [
    { value: "all", label: "All" },
    { value: "PENDING", label: "Pending" },
    { value: "APPROVED", label: "Approved" },
    { value: "REJECTED", label: "Dismissed" },
    { value: "CANCELED", label: "Canceled" },
  ];
}

function requestCountByFacet(
  requests: MediaRequestRecord[],
  facet: RequestFacetFilter,
): number {
  return requests.filter((request) => request.facet === facet).length;
}

function requestCountByStatus(
  requests: MediaRequestRecord[],
  status: RequestStatusFilter,
): number {
  if (status === "all") {
    return requests.length;
  }
  return requests.filter((request) => request.status === status).length;
}


/// How long the media is held for, in one badge. "Requested" is a window nobody
/// has granted yet; "dormant" is a granted one still waiting for the title's
/// first import, so it says "N days from first import" rather than pretending
/// to know when it runs out.
function RequestLeaseBadge({
  request,
  dateTimeFormat,
}: {
  request: MediaRequestRecord;
  dateTimeFormat: UiDateTimeFormat;
}) {
  const t = useTranslate();
  const badge = requestLeaseBadge(request);
  const label =
    badge.variant === "forever"
      ? t("requests.leaseForever")
      : badge.variant === "requested"
        ? t("requests.leaseRequested", { count: badge.days })
        : badge.variant === "dormant"
          ? t("requests.leaseDormant", { count: badge.days })
          : badge.variant === "active"
            ? t("requests.leaseActive", {
                date: formatUiDateTime(badge.expiresAt, dateTimeFormat),
              })
            : badge.variant === "expired"
              ? t("requests.leaseExpired")
              : t("requests.leaseReleased");

  return (
    <Badge
      id={mediaRequestLeaseId(request.id)}
      data-request-lease={badge.variant}
      tone={requestLeaseBadgeTone(badge)}
      className="gap-1"
    >
      <Timer className="h-3 w-3" />
      {label}
    </Badge>
  );
}

/// The decision, and behind a click the whole of what the reader is permitted
/// to see of it.
///
/// `votes` arrives empty for a requester reading their own request — the
/// redaction *is* the empty list, and there is no flag saying which you got —
/// so the popover renders whatever it was handed: votes when there are any,
/// reasons always.
function RequestDecisionChip({
  request,
  decision,
}: {
  request: MediaRequestRecord;
  decision: RequestRuleDecisionRecord;
}) {
  const t = useTranslate();
  const outcomeKey = requestDecisionOutcomeLabelKey(decision.effectiveOutcome);
  const fallbackKey = decision.fallbackReason
    ? requestFallbackReasonLabelKey(decision.fallbackReason)
    : null;

  return (
    <Popover>
      <PopoverTrigger asChild>
        <button
          id={mediaRequestDecisionId(request.id)}
          type="button"
          data-request-decision-outcome={decision.effectiveOutcome}
          data-request-decision-mode={decision.mode}
          className="cursor-pointer"
        >
          <Badge
            tone={requestDecisionOutcomeBadgeTone(decision.effectiveOutcome)}
            className="gap-1"
          >
            <ScrollText className="h-3 w-3" />
            {outcomeKey ? t(outcomeKey) : decision.effectiveOutcome}
            {decision.mode === "SHADOW"
              ? ` · ${t("requests.decisionShadow")}`
              : ""}
          </Badge>
        </button>
      </PopoverTrigger>
      <PopoverContent
        id={mediaRequestDecisionPopoverId(request.id)}
        className="w-[340px] space-y-2 text-xs"
      >
        <p className="text-sm font-semibold">{t("requests.decisionTitle")}</p>
        {decision.policyOutcome !== decision.effectiveOutcome ? (
          <p className="text-muted-foreground">
            {t("requests.decisionPolicySaid", {
              outcome: decision.policyOutcome,
            })}
          </p>
        ) : null}
        {decision.fallbackReason ? (
          <p className="text-muted-foreground">
            {fallbackKey ? t(fallbackKey) : decision.fallbackReason}
          </p>
        ) : null}
        {decision.reasons.length > 0 ? (
          <ul className="list-disc space-y-0.5 pl-4 text-muted-foreground">
            {decision.reasons.map((reason) => (
              <li key={`${reason.ruleName}:${reason.code}`}>
                <code data-code-font>{reason.code}</code> — {reason.ruleName}
              </li>
            ))}
          </ul>
        ) : null}
        {decision.votes.length > 0 ? (
          <div className="space-y-1">
            <p className="font-semibold">{t("requests.decisionVotes")}</p>
            <div className="flex flex-wrap gap-1">
              {decision.votes.map((vote) => {
                const labelKey = requestVoteLabelKey(vote.vote);
                return (
                  <Badge
                    key={`${vote.ruleSetId}:${vote.revisionNumber}`}
                    tone={requestVoteBadgeTone(vote.vote)}
                    className="text-[10px]"
                    title={vote.error ?? undefined}
                  >
                    {vote.ruleSetName}: {labelKey ? t(labelKey) : vote.vote}
                  </Badge>
                );
              })}
            </div>
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
        {decision.reasons.length === 0 &&
        decision.votes.length === 0 &&
        !decision.fallbackReason ? (
          <p className="text-muted-foreground">
            {t("requests.decisionNothingToShow")}
          </p>
        ) : null}
      </PopoverContent>
    </Popover>
  );
}

/// The holds on a created title, and the three things a manager can do to one.
/// Releasing always asks for a reason: the API refuses a blank one, and the
/// trail is the point of the operation.
function RequestClaimsPanel({
  request,
  claims,
  loading,
  claimActionId,
  dateTimeFormat,
  onConvertClaim,
  onStartExtend,
  onStartRelease,
}: {
  request: MediaRequestRecord;
  claims: TitleClaimRecord[] | undefined;
  loading: boolean;
  claimActionId: string | null;
  dateTimeFormat: UiDateTimeFormat;
  onConvertClaim: (claim: TitleClaimRecord) => void;
  onStartExtend: (claim: TitleClaimRecord) => void;
  onStartRelease: (claim: TitleClaimRecord) => void;
}) {
  const t = useTranslate();

  if (loading && !claims) {
    return (
      <p
        id={mediaRequestClaimsPanelId(request.id)}
        className="text-xs text-[var(--scry-muted3)]"
      >
        {t("label.loading")}
      </p>
    );
  }

  if (!claims || claims.length === 0) {
    return (
      <p
        id={mediaRequestClaimsPanelId(request.id)}
        className="text-xs text-[var(--scry-muted3)]"
      >
        {t("requests.claimsEmpty")}
      </p>
    );
  }

  return (
    <div
      id={mediaRequestClaimsPanelId(request.id)}
      className="space-y-2 rounded-[10px] border border-[var(--scry-border3)] bg-[var(--scry-inset)] px-3 py-2"
    >
      {claims.map((claim) => {
        const stateKey = titleClaimStateLabelKey(claim.state);
        // Only a live hold can be extended, made permanent or released; a
        // released or converted one is history, and acting on it would write a
        // second claim rather than change the first.
        const live = claim.state === "ACTIVE" || claim.state === "DORMANT";
        const busy = claimActionId === claim.id;
        return (
          <div
            key={claim.id}
            id={titleClaimRowId(claim.id)}
            data-title-claim-state={claim.state}
            data-title-claim-kind={claim.kind}
            className="flex flex-wrap items-center gap-2 text-xs"
          >
            <Badge tone={titleClaimStateBadgeTone(claim.state)}>
              {stateKey ? t(stateKey) : claim.state}
            </Badge>
            <span className="text-[var(--scry-muted3)]">
              {claim.kind === "KEEP"
                ? t("requests.claimKindKeep")
                : claim.expiresAt
                  ? t("requests.claimExpiresAt", {
                      date: formatUiDateTime(claim.expiresAt, dateTimeFormat),
                    })
                  : t("requests.claimDurationDays", {
                      count: claim.durationDays ?? 0,
                    })}
            </span>
            {claim.releasedReason ? (
              <span className="text-[var(--scry-muted3)]">
                {t("requests.claimReleasedReason", {
                  reason: claim.releasedReason,
                })}
              </span>
            ) : null}
            {live ? (
              <div className="ml-auto flex flex-wrap gap-1">
                <Button
                  id={titleClaimExtendId(claim.id)}
                  type="button"
                  size="sm"
                  variant="secondary"
                  disabled={busy}
                  onClick={() => onStartExtend(claim)}
                >
                  {t("requests.claimExtend")}
                </Button>
                {claim.kind === "RETAIN_UNTIL" ? (
                  <Button
                    id={titleClaimPermanentId(claim.id)}
                    type="button"
                    size="sm"
                    variant="secondary"
                    disabled={busy}
                    onClick={() => onConvertClaim(claim)}
                  >
                    {t("requests.claimMakePermanent")}
                  </Button>
                ) : null}
                <Button
                  id={titleClaimReleaseId(claim.id)}
                  type="button"
                  size="sm"
                  variant="secondary"
                  disabled={busy}
                  onClick={() => onStartRelease(claim)}
                >
                  {t("requests.claimRelease")}
                </Button>
              </div>
            ) : null}
          </div>
        );
      })}
    </div>
  );
}

export function RequestsView({
  mode,
  canShowAdminMode,
  canShowRequesterMode,
  onModeChange,
  statusFilter,
  onStatusFilterChange,
  libraries,
  selectedLibraryIds,
  onSelectedLibraryIdsChange,
  requests,
  qualityProfileOptions,
  loading,
  actionRequestId,
  onRefresh,
  onLoadQualityProfileOptions,
  onApprove,
  onDismiss,
  onUpdateRequest,
  onCancelRequest,
  claimsByRequestId,
  claimsLoadingRequestId,
  onLoadClaims,
  onExtendClaim,
  onConvertClaim,
  onReleaseClaim,
  claimActionId,
}: RequestsViewProps) {
  const t = useTranslate();
  const dateTimeFormat = useUiDateTimeFormat();
  const showModeSwitch = canShowAdminMode && canShowRequesterMode;
  const HeadingIcon = mode === "admin" ? Inbox : Clock;
  const headingTitle = mode === "admin" ? "Request Queue" : "My Requests";
  const headingCopy =
    mode === "admin"
      ? null
      : "Track the titles you've asked Scryer to grab. You'll be notified when they're available.";
  const filters = statusFilterOptions(mode);
  const [adminFacetFilters, setAdminFacetFilters] = React.useState<
    Record<RequestFacetFilter, boolean>
  >({ MOVIE: true, SERIES: true, ANIME: true });
  const displayedRequests = React.useMemo(
    () =>
      mode === "admin"
        ? requests.filter((request) => adminFacetFilters[request.facet])
        : requests,
    [adminFacetFilters, mode, requests],
  );
  const [approvalRequest, setApprovalRequest] =
    React.useState<MediaRequestRecord | null>(null);
  const [approvalProfileId, setApprovalProfileId] = React.useState("");
  const [approvalMonitorType, setApprovalMonitorType] =
    React.useState<RequestMonitorType>("FUTURE_EPISODES");
  const [approvalMonitorSelection, setApprovalMonitorSelection] =
    React.useState<MonitorSelectionDraft>(EMPTY_MONITOR_SELECTION);
  const [approvalSelectionLoading, setApprovalSelectionLoading] =
    React.useState(false);
  const [approvalLeaseChoice, setApprovalLeaseChoice] =
    React.useState<ApprovalLeaseChoice>("requested");
  const [approvalCustomLeaseDays, setApprovalCustomLeaseDays] =
    React.useState(30);
  const [approvalTags, setApprovalTags] = React.useState<string[]>([]);
  const [approvalTagDraft, setApprovalTagDraft] = React.useState("");
  const [openClaimsRequestId, setOpenClaimsRequestId] = React.useState<
    string | null
  >(null);
  const [pendingClaimAction, setPendingClaimAction] =
    React.useState<PendingClaimAction>(null);
  const [editRequest, setEditRequest] =
    React.useState<MediaRequestRecord | null>(null);
  const [editProfileId, setEditProfileId] = React.useState("");
  const [editMonitorType, setEditMonitorType] =
    React.useState<RequestMonitorType>("FUTURE_EPISODES");
  const [editMonitorSelection, setEditMonitorSelection] =
    React.useState<MonitorSelectionDraft>(EMPTY_MONITOR_SELECTION);
  const [editSelectionLoading, setEditSelectionLoading] = React.useState(false);
  const editProfileOptions = React.useMemo(
    () =>
      editRequest
        ? requestProfileOptionsForLibrary(
            libraries,
            editRequest.libraryId,
            qualityProfileOptions,
          )
        : [],
    [editRequest, libraries, qualityProfileOptions],
  );

  React.useEffect(() => {
    if (!approvalRequest) return;
    const requestedProfileId = approvalRequest.requestedQualityProfileId?.trim() ?? "";
    const requestedStillValid = qualityProfileOptions.some(
      (profile) => profile.id === requestedProfileId,
    );
    const library = libraries.find(
      (library) => library.id === approvalRequest.libraryId,
    );
    const libraryDefaultProfileId =
      library?.qualityProfileId?.trim() ??
      library?.requestQualityProfileDefaultId?.trim() ??
      "";
    const libraryDefaultStillValid = qualityProfileOptions.some(
      (profile) => profile.id === libraryDefaultProfileId,
    );
    setApprovalProfileId(
      requestedStillValid
        ? requestedProfileId
        : libraryDefaultStillValid
          ? libraryDefaultProfileId
          : qualityProfileOptions[0]?.id ?? "",
    );
    setApprovalMonitorType(
      monitorTypeSelectValue(
        approvalRequest.facet,
        approvalRequest.requestedMonitorType,
      ),
    );
    setApprovalMonitorSelection(
      monitorSelectionFromRecord(approvalRequest.requestedMonitorSelection) ??
        EMPTY_MONITOR_SELECTION,
    );
    setApprovalSelectionLoading(false);
    /// "Keep what they asked for" is the default: an approver who has not
    /// touched the lease has not overridden it, and sending neither field is
    /// how the API reads that.
    setApprovalLeaseChoice("requested");
    setApprovalCustomLeaseDays(approvalRequest.requestedLeaseDays ?? 30);
    /// Prefilled from what the policy emitted, and editable: the approver is
    /// the one who decides what the created title actually carries.
    setApprovalTags([...(approvalRequest.policyTags ?? [])]);
    setApprovalTagDraft("");
  }, [approvalRequest, libraries, qualityProfileOptions]);

  React.useEffect(() => {
    if (!editRequest) return;
    const requestedProfileId = editRequest.requestedQualityProfileId?.trim() ?? "";
    const requestedStillAllowed = editProfileOptions.some(
      (profile) => profile.id === requestedProfileId,
    );
    setEditProfileId(
      requestedStillAllowed
        ? requestedProfileId
        : editProfileOptions[0]?.id ?? "",
    );
    setEditMonitorType(
      monitorTypeSelectValue(editRequest.facet, editRequest.requestedMonitorType),
    );
    setEditMonitorSelection(
      monitorSelectionFromRecord(editRequest.requestedMonitorSelection) ??
        EMPTY_MONITOR_SELECTION,
    );
    setEditSelectionLoading(false);
  }, [editProfileOptions, editRequest]);

  // The picker is mounted (and its metadata query issued) only while this
  // dialog's own monitor-type select says ADVANCED. Reading a request's stored
  // selection for the card summary never fetches anything.
  const approvalAdvancedSelected =
    approvalRequest !== null &&
    approvalRequest.facet !== "MOVIE" &&
    approvalMonitorType === "ADVANCED";
  const approvalTvdbId = approvalRequest
    ? requestExternalIdValue(approvalRequest, "tvdb")?.trim() ?? ""
    : "";
  const approvalBlocksConfirm =
    approvalAdvancedSelected &&
    (!approvalTvdbId ||
      approvalSelectionLoading ||
      isMonitorSelectionEmpty(approvalMonitorSelection));
  const editAdvancedSelected =
    editRequest !== null &&
    editRequest.facet !== "MOVIE" &&
    editMonitorType === "ADVANCED";
  const editTvdbId = editRequest
    ? requestExternalIdValue(editRequest, "tvdb")?.trim() ?? ""
    : "";
  const editBlocksConfirm =
    editAdvancedSelected &&
    (!editTvdbId ||
      editSelectionLoading ||
      isMonitorSelectionEmpty(editMonitorSelection));

  const openApprovalDialog = (request: MediaRequestRecord) => {
    onLoadQualityProfileOptions();
    setApprovalRequest(request);
  };

  const closeApprovalDialog = () => {
    setApprovalRequest(null);
    setApprovalProfileId("");
    setApprovalMonitorType("FUTURE_EPISODES");
    setApprovalMonitorSelection(EMPTY_MONITOR_SELECTION);
    setApprovalSelectionLoading(false);
    setApprovalLeaseChoice("requested");
    setApprovalTags([]);
    setApprovalTagDraft("");
  };

  const confirmApproval = () => {
    if (!approvalRequest || !approvalProfileId) return;
    /// `leaseDays` and `leaseForever` are mutually exclusive on the wire — the
    /// API refuses both — and "keep what they asked for" sends neither.
    const leaseDays =
      approvalLeaseChoice === "requested" || approvalLeaseChoice === "forever"
        ? undefined
        : approvalLeaseChoice === "custom"
          ? clampRequestLeaseDays(approvalCustomLeaseDays)
          : Number(approvalLeaseChoice);
    onApprove(approvalRequest, {
      qualityProfileId: approvalProfileId,
      monitorType:
        approvalRequest.facet === "MOVIE" ? undefined : approvalMonitorType,
      monitorSelection: approvalAdvancedSelected
        ? monitorSelectionInput(approvalMonitorSelection)
        : undefined,
      leaseDays,
      leaseForever: approvalLeaseChoice === "forever" ? true : undefined,
      tags: approvalTags,
    });
    closeApprovalDialog();
  };

  const addApprovalTag = () => {
    const tag = approvalTagDraft.trim();
    if (!tag) return;
    setApprovalTags((prev) => (prev.includes(tag) ? prev : [...prev, tag]));
    setApprovalTagDraft("");
  };

  const toggleClaimsPanel = (request: MediaRequestRecord) => {
    setOpenClaimsRequestId((current) => {
      if (current === request.id) {
        return null;
      }
      // Claims are read only when a row asks for them: a page of approved
      // requests would otherwise be one query per row for a panel nobody opened.
      onLoadClaims(request);
      return request.id;
    });
  };

  const openEditDialog = (request: MediaRequestRecord) => {
    onLoadQualityProfileOptions();
    setEditRequest(request);
  };

  const closeEditDialog = () => {
    setEditRequest(null);
    setEditProfileId("");
    setEditMonitorType("FUTURE_EPISODES");
    setEditMonitorSelection(EMPTY_MONITOR_SELECTION);
    setEditSelectionLoading(false);
  };

  const confirmUpdate = () => {
    if (!editRequest || !editProfileId) return;
    onUpdateRequest(editRequest, {
      requestedQualityProfileId: editProfileId,
      requestedMonitorType: editRequest.facet === "MOVIE" ? undefined : editMonitorType,
      requestedMonitorSelection: editAdvancedSelected
        ? monitorSelectionInput(editMonitorSelection)
        : undefined,
    });
    closeEditDialog();
  };

  const renderRequestCard = (request: MediaRequestRecord) => {
    const posterUrl = selectPosterVariantUrl(request.posterUrl, "w250");
    // Background art when the provider had it; the poster is only a fallback.
    const backgroundArtUrl =
      selectBackdropVariantUrl(request.backgroundUrl, "w1280") ??
      selectPosterVariantUrl(request.posterUrl, "original") ??
      posterUrl;
    const requesters = requesterLabel(request);
    const imdbId = requestExternalIdValue(request, "imdb");
    const tvdbId = requestExternalIdValue(request, "tvdb");
    const tmdbId = requestExternalIdValue(request, "tmdb");
    const malId = requestExternalIdValue(request, "mal");
    const anilistId = requestExternalIdValue(request, "anilist");
    const anidbId = requestExternalIdValue(request, "anidb");
    const externalRatings = request.externalRatings.map(normalizeTitleExternalRating);
    const hasExternalLink =
      Boolean(imdbId) ||
      Boolean(tvdbId) ||
      Boolean(tmdbId) ||
      Boolean(malId) ||
      Boolean(anilistId) ||
      Boolean(anidbId);
    const isResolving = actionRequestId === request.id;
    // Only an in-flight action locks the buttons. Background refreshes (focus
    // and poll pulses) must not, or a click that lands while one is running
    // hits a disabled button and is silently dropped.
    const actionsDisabled = actionRequestId !== null;
    const approveDisabled = actionRequestId !== null;
    const statusMeta = requestStatusTone(t, request.status);
    const StatusIcon = statusMeta.Icon;
    const canResolveRequest = mode === "admin" && request.status === "PENDING";
    const canEditOwnRequest = mode === "mine" && request.status === "PENDING";
    const decision = request.decision ?? null;
    const policyTags = request.policyTags ?? [];
    // Claim actions belong to whoever manages the library; admin mode only ever
    // lists requests in libraries the reader manages, so the mode is the check.
    const canManageClaims =
      mode === "admin" &&
      request.status === "APPROVED" &&
      Boolean(request.createdTitleId);
    const claimsOpen = openClaimsRequestId === request.id;
    // A denial's "why" is the decision's reasons; there is no separate field,
    // and a requester is permitted exactly this much of it.
    const denyReasons =
      mode === "mine" && request.status === "REJECTED"
        ? (decision?.reasons ?? [])
        : [];
    const libraryLabel =
      libraries.find((library) => library.id === request.libraryId)?.name ??
      request.libraryId;
    const requestedProfile =
      profileLabel(
        request.requestedQualityProfileId,
        request.requestedQualityProfileName,
        qualityProfileOptions,
      ) ?? t("requests.libraryDefaultProfile");
    const requestedMonitorType = request.requestedMonitorType
      ? monitorTypeLabel(t, request.requestedMonitorType)
      : null;
    const requestedSelection = monitorSelectionSummaryParts(
      request.requestedMonitorSelection,
      {
        specials: t("monitorSelection.specials"),
        season: (seasonNumber) =>
          t("monitorSelection.season", { number: seasonNumber }),
      },
    );
    const requestedSelectionSummary = [
      requestedSelection.seasons.join(", "),
      requestedSelection.movies.length > 0
        ? t("monitorSelection.summaryMovies", {
            movies: requestedSelection.movies.join(", "),
          })
        : "",
    ]
      .filter(Boolean)
      .join(" · ");

    return (
      <article
        key={request.id}
        id={mediaRequestRowId(request.id)}
        data-request-status={request.status}
        data-request-title={request.title}
        data-request-facet={request.facet.toLowerCase()}
        data-request-imdb-id={requestExternalIdValue(request, "imdb")}
        data-request-tvdb-id={requestExternalIdValue(request, "tvdb")}
        data-request-tmdb-id={requestExternalIdValue(request, "tmdb")}
        className="relative overflow-hidden rounded-[14px] border border-[var(--scry-border)] bg-[var(--scry-surf)] shadow-[0_10px_24px_rgba(0,0,0,0.16)]"
      >
        {backgroundArtUrl ? (
          <div
            aria-hidden="true"
            className="absolute inset-0 bg-cover bg-center opacity-60"
            style={{ backgroundImage: `url(${backgroundArtUrl})` }}
          />
        ) : null}
        <div
          aria-hidden="true"
          className="absolute inset-0 bg-[linear-gradient(90deg,var(--scry-surf)_0%,rgba(7,11,24,0.84)_38%,rgba(7,11,24,0.52)_100%)]"
        />
        <div
          aria-hidden="true"
          className="absolute inset-0 bg-[linear-gradient(0deg,rgba(4,7,16,0.76)_0%,rgba(4,7,16,0.22)_58%,rgba(255,255,255,0.04)_100%)]"
        />
        <div className="relative z-10 flex flex-col sm:flex-row">
          {/* The poster floats over the full-bleed art: a fixed 2:3 frame,
              vertically centred against the details column. */}
          <div className="flex w-full shrink-0 items-center justify-center p-4 sm:w-[236px] sm:pr-0">
            <div className="aspect-[2/3] w-[200px] shrink-0 overflow-hidden rounded-[9px] border border-[#2a3556] bg-[var(--scry-inset)] shadow-[0_8px_22px_rgba(0,0,0,0.5)]">
              {posterUrl ? (
                <TitlePoster
                  src={posterUrl}
                  alt={t("media.posterAlt", { name: request.title })}
                  className="h-full w-full object-cover"
                  loading="lazy"
                />
              ) : (
                <div className="flex h-full w-full items-center justify-center text-xs text-[var(--scry-muted3)]">
                  {t("label.noArt")}
                </div>
              )}
            </div>
          </div>
          <div className="flex min-w-0 flex-1 flex-col gap-4 p-4 sm:p-5">
            <div className="flex flex-col gap-3 xl:flex-row xl:items-start xl:justify-between">
              <div className="min-w-0">
                <div className="flex flex-wrap items-center gap-2">
                  <h2 className="text-[19px] font-semibold leading-tight text-[var(--scry-ink2)]">
                    {request.title}
                  </h2>
                  <span
                    id={mediaRequestStatusId(request.id)}
                    data-request-status={request.status}
                    className={cn(
                      "inline-flex items-center gap-1.5 rounded-[7px] border px-2 py-1 text-[11px] font-bold uppercase",
                      statusMeta.className,
                    )}
                  >
                    <StatusIcon className="h-3 w-3" />
                    {statusMeta.label}
                  </span>
                  <RequestLeaseBadge
                    request={request}
                    dateTimeFormat={dateTimeFormat}
                  />
                  {decision ? (
                    <RequestDecisionChip
                      request={request}
                      decision={decision}
                    />
                  ) : null}
                </div>
                {request.status === "PENDING" && policyTags.length > 0 ? (
                  <div
                    id={mediaRequestPolicyTagsId(request.id)}
                    className="mt-2 flex flex-wrap items-center gap-1"
                  >
                    <span className="text-[11px] text-[var(--scry-faint)]">
                      {t("requests.policyTagsLabel")}
                    </span>
                    {policyTags.map((tag) => (
                      <Badge key={tag} tone="info" className="text-[10px]">
                        {tag}
                      </Badge>
                    ))}
                  </div>
                ) : null}
                {denyReasons.length > 0 ? (
                  <p
                    id={mediaRequestDenyReasonId(request.id)}
                    className="mt-2 text-xs text-[var(--scry-danger-text)]"
                  >
                    {t("requests.denyReason", {
                      reasons: denyReasons
                        .map((reason) => reason.code)
                        .join(", "),
                    })}
                  </p>
                ) : null}
                <p className="mt-1 text-xs text-[var(--scry-muted3)]">
                  {request.year ?? t("label.yearUnknown")}
                </p>
              </div>
              <div className="flex flex-wrap gap-2 xl:justify-end">
                {canResolveRequest ? (
                  <>
                    <Button
                      id={mediaRequestApproveId(request.id)}
                      type="button"
                      size="sm"
                      variant="primary"
                      onClick={() => openApprovalDialog(request)}
                      disabled={approveDisabled}
                    >
                      {isResolving ? (
                        <Loader2 className="h-4 w-4 animate-spin" />
                      ) : (
                        <Check className="h-4 w-4" />
                      )}
                      {t("requests.approve")}
                    </Button>
                    <Button
                      id={mediaRequestDismissId(request.id)}
                      type="button"
                      size="sm"
                      variant="secondary"
                      onClick={() => onDismiss(request)}
                      disabled={actionsDisabled}
                    >
                      <X className="h-4 w-4" />
                      {t("requests.dismiss")}
                    </Button>
                  </>
                ) : null}
                {canEditOwnRequest ? (
                  <>
                    <Button
                      id={mediaRequestEditId(request.id)}
                      type="button"
                      size="sm"
                      variant="secondary"
                      onClick={() => openEditDialog(request)}
                      disabled={actionsDisabled}
                    >
                      <Pencil className="h-4 w-4" />
                      {t("requests.modify")}
                    </Button>
                    <Button
                      id={mediaRequestCancelId(request.id)}
                      type="button"
                      size="sm"
                      variant="secondary"
                      onClick={() => onCancelRequest(request)}
                      disabled={actionsDisabled}
                    >
                      {isResolving ? (
                        <Loader2 className="h-4 w-4 animate-spin" />
                      ) : (
                        <X className="h-4 w-4" />
                      )}
                      {t("requests.cancelRequest")}
                    </Button>
                  </>
                ) : null}
                {canManageClaims ? (
                  <Button
                    id={mediaRequestClaimsToggleId(request.id)}
                    type="button"
                    size="sm"
                    variant="secondary"
                    onClick={() => toggleClaimsPanel(request)}
                  >
                    <Timer className="h-4 w-4" />
                    {claimsOpen
                      ? t("requests.claimsHide")
                      : t("requests.claimsShow")}
                  </Button>
                ) : null}
              </div>
            </div>
            {canManageClaims && claimsOpen ? (
              <RequestClaimsPanel
                request={request}
                claims={claimsByRequestId[request.id]}
                loading={claimsLoadingRequestId === request.id}
                claimActionId={claimActionId}
                dateTimeFormat={dateTimeFormat}
                onConvertClaim={onConvertClaim}
                onStartExtend={(claim) =>
                  setPendingClaimAction({
                    kind: "extend",
                    claim,
                    expiresAt: "",
                  })
                }
                onStartRelease={(claim) =>
                  setPendingClaimAction({ kind: "release", claim, reason: "" })
                }
              />
            ) : null}
            <p className="line-clamp-4 max-w-[80ch] text-sm leading-6 text-[var(--scry-muted2)]">
              {request.overview || t("title.descriptionUnavailable")}
            </p>
            <TitleRatingsDisplay
              externalRatings={externalRatings}
              fallbackRating={request.rating ?? null}
              fallbackSources={request.ratingSources}
            />
            {hasExternalLink ? (
              <div className="flex flex-wrap items-center gap-2">
                <ImdbExternalLink imdbId={imdbId} size="compact" />
                {request.facet === "MOVIE" ? (
                  <TvdbMovieExternalLink
                    tvdbId={tvdbId}
                    slug={request.slug}
                    size="compact"
                  />
                ) : (
                  <TvdbSeriesExternalLink
                    tvdbId={tvdbId}
                    slug={request.slug}
                    size="compact"
                  />
                )}
                <TmdbExternalLink
                  mediaType={request.facet === "MOVIE" ? "movie" : "tv"}
                  tmdbId={tmdbId}
                  size="compact"
                />
                <MalExternalLink malId={malId} size="compact" />
                <AnilistExternalLink anilistId={anilistId} size="compact" />
                <AnidbExternalLink anidbId={anidbId} size="compact" />
              </div>
            ) : null}
            <div className="grid gap-2 text-xs text-[var(--scry-muted3)] sm:grid-cols-2 xl:grid-cols-4">
              <div className="rounded-[10px] border border-[var(--scry-border3)] bg-[var(--scry-inset)] px-3 py-2">
                <div className="mb-1 flex items-center gap-1.5 text-[var(--scry-faint)]">
                  <User className="h-3.5 w-3.5" />
                  {t("requests.requesters")}
                </div>
                <div className="flex items-center gap-1.5 text-[var(--scry-ink2)]">
                  <RequesterAvatarStack request={request} />
                  <span>{requesters || t("label.unknown")}</span>
                </div>
              </div>
              <div className="rounded-[10px] border border-[var(--scry-border3)] bg-[var(--scry-inset)] px-3 py-2">
                <div className="mb-1 flex items-center gap-1.5 text-[var(--scry-faint)]">
                  <BookOpen className="h-3.5 w-3.5" />
                  Library
                </div>
                <div className="text-[var(--scry-ink2)]">{libraryLabel}</div>
              </div>
              <div className="rounded-[10px] border border-[var(--scry-border3)] bg-[var(--scry-inset)] px-3 py-2">
                <div className="mb-1 flex items-center gap-1.5 text-[var(--scry-faint)]">
                  <SlidersVertical className="h-3.5 w-3.5" />
                  {t("requests.requestedQualityProfile")}
                </div>
                <div className="text-[var(--scry-ink2)]">{requestedProfile}</div>
              </div>
              <div className="rounded-[10px] border border-[var(--scry-border3)] bg-[var(--scry-inset)] px-3 py-2">
                <div className="mb-1 flex items-center gap-1.5 text-[var(--scry-faint)]">
                  <History className="h-3.5 w-3.5" />
                  {t("requests.updated")}
                </div>
                <div className="text-[var(--scry-ink2)]">
                  {formatUiDateTime(request.updatedAt, dateTimeFormat)}
                </div>
              </div>
              {requestedMonitorType ? (
                <div className="rounded-[10px] border border-[var(--scry-border3)] bg-[var(--scry-inset)] px-3 py-2 sm:col-span-2 xl:col-span-4">
                  <div className="mb-1 flex items-center gap-1.5 text-[var(--scry-faint)]">
                    <CircleCheck className="h-3.5 w-3.5" />
                    {t("requests.requestedMonitorType")}
                  </div>
                  <div className="text-[var(--scry-ink2)]">{requestedMonitorType}</div>
                  {requestedSelectionSummary ? (
                    <div
                      id={mediaRequestMonitorSelectionId(request.id)}
                      className="mt-1 text-[var(--scry-ink2)]"
                    >
                      {t("monitorSelection.summaryPrefix", {
                        selection: requestedSelectionSummary,
                      })}
                    </div>
                  ) : null}
                </div>
              ) : null}
            </div>
          </div>
        </div>
      </article>
    );
  };

  return (
    <section
      id="requests-view"
      className="scry-scroll flex min-h-0 flex-1 overflow-y-auto bg-[var(--scry-surfE)]"
    >
      <div className="mx-auto flex w-full max-w-[1240px] flex-col gap-4 px-4 py-6 sm:px-6 lg:px-8">
        <div className="flex items-start gap-4">
          <div className="flex h-11 w-11 flex-none items-center justify-center rounded-[13px] border border-[var(--scry-baccent)] bg-[rgba(var(--scry-accent-rgb),0.16)] text-[var(--scry-accent-text)]">
            <HeadingIcon className="h-5 w-5" />
          </div>
          <div className="min-w-0 flex-1">
            <h1 className="font-display text-[25px] font-bold leading-tight text-[var(--scry-ink)]">
              {headingTitle}
            </h1>
            {headingCopy ? (
              <p className="mt-1 max-w-2xl text-[13.5px] text-[var(--scry-muted)]">
                {headingCopy}
              </p>
            ) : null}
          </div>
        </div>

        <div className="flex flex-wrap items-end justify-between gap-3 border-b border-[var(--scry-border3)]">
          <div
            role="group"
            aria-label="Request filters"
            className="relative top-px flex min-h-10 min-w-0 max-w-full flex-1 flex-wrap items-center justify-start gap-x-5 gap-y-1 border-0 bg-transparent p-0 shadow-none"
          >
            {showModeSwitch ? (
              <>
                <UnderlineFilterButton
                  id="requests-mode-admin"
                  selected={mode === "admin"}
                  label={t("requests.mode.admin")}
                  onClick={() => onModeChange("admin")}
                />
                <UnderlineFilterButton
                  id="requests-mode-mine"
                  selected={mode === "mine"}
                  label={t("requests.mode.mine")}
                  onClick={() => onModeChange("mine")}
                />
              </>
            ) : null}
            {mode === "admin"
              ? ([
                  ["MOVIE", "Movie"],
                  ["SERIES", t("search.facetSeries")],
                  ["ANIME", t("search.facetAnime")],
                ] as Array<[RequestFacetFilter, string]>).map(([facet, label]) => (
                  <UnderlineFilterButton
                    key={facet}
                    selected={adminFacetFilters[facet]}
                    label={label}
                    count={requestCountByFacet(requests, facet)}
                    aria-pressed={adminFacetFilters[facet]}
                    onClick={() =>
                      setAdminFacetFilters((current) => ({
                        ...current,
                        [facet]: !current[facet],
                      }))
                    }
                  />
                ))
              : null}
            {filters.map((filter) => (
              <UnderlineFilterButton
                key={filter.value}
                selected={statusFilter === filter.value}
                label={filter.label}
                count={requestCountByStatus(requests, filter.value)}
                onClick={() => onStatusFilterChange(filter.value)}
              />
            ))}
          </div>
          <div className="mb-3 flex w-full shrink-0 items-center justify-end gap-2 sm:w-auto">
            <LibraryMultiSelect
              libraries={libraries}
              selectedLibraryIds={selectedLibraryIds}
              onSelectedLibraryIdsChange={onSelectedLibraryIdsChange}
              triggerClassName="h-10 min-w-56 rounded-[11px]"
            />
            {mode === "mine" ? (
              <Button
                type="button"
                variant="outline"
                className="h-10 w-10 rounded-[11px] p-0"
                onClick={onRefresh}
                disabled={loading}
                aria-label="Refresh requests"
              >
                <RefreshCw className={cn("h-4 w-4", loading && "animate-spin")} />
              </Button>
            ) : null}
          </div>
        </div>

        <div className="grid gap-4">
          {displayedRequests.length === 0 && !loading ? (
            <div
              id={mode === "admin" ? "requests-empty-admin" : "requests-empty-mine"}
              className="rounded-[14px] border border-dashed border-[var(--scry-border2)] bg-[var(--scry-surf)] px-4 py-8 text-center text-sm text-[var(--scry-muted3)]"
            >
              {mode === "admin" ? t("requests.empty") : t("requests.emptyMine")}
            </div>
          ) : null}

          {displayedRequests.map(renderRequestCard)}
        </div>
      </div>
      <Dialog open={approvalRequest !== null} onOpenChange={(open) => { if (!open) closeApprovalDialog(); }}>
        <DialogContent
          id="approve-media-request-dialog"
          className="max-h-[85vh] overflow-y-auto sm:max-w-md"
        >
          <DialogHeader>
            <DialogTitle>{t("requests.approveTitle")}</DialogTitle>
          </DialogHeader>
          <label className="space-y-2">
            <span className="block text-sm font-medium text-card-foreground">
              {t("requests.approvedQualityProfile")}
            </span>
            <Select
              value={approvalProfileId}
              onValueChange={setApprovalProfileId}
              disabled={actionRequestId !== null}
            >
              <SelectTrigger id="approve-media-request-quality-profile">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {qualityProfileOptions.map((profile) => (
                  <SelectItem
                    id={mediaRequestProfileOptionId("approve", profile.id)}
                    key={profile.id}
                    value={profile.id}
                  >
                    {profile.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </label>
          {approvalRequest && approvalRequest.facet !== "MOVIE" ? (
            <label className="space-y-2">
              <span className="block text-sm font-medium text-card-foreground">
                {t("requests.approvedMonitorType")}
              </span>
              <Select
                value={approvalMonitorType}
                onValueChange={(value) => {
                  const nextMonitorType = value as RequestMonitorType;
                  setApprovalMonitorType(nextMonitorType);
                  if (nextMonitorType !== "ADVANCED") {
                    setApprovalMonitorSelection(EMPTY_MONITOR_SELECTION);
                  }
                }}
                disabled={actionRequestId !== null}
              >
                <SelectTrigger
                  id="approve-media-request-monitor-type"
                  className="w-full"
                >
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {monitorOptions(t).map((option) => (
                    <SelectItem
                      id={mediaRequestMonitorOptionId("approve", option.value)}
                      key={option.value}
                      value={option.value}
                    >
                      {option.label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </label>
          ) : null}
          {approvalRequest && approvalAdvancedSelected ? (
            <MonitorSelectionPicker
              facet={approvalRequest.facet}
              tvdbId={approvalTvdbId}
              value={approvalMonitorSelection}
              onChange={setApprovalMonitorSelection}
              onLoadingChange={setApprovalSelectionLoading}
              disabled={actionRequestId !== null}
              idPrefix="approve-media-request"
            />
          ) : null}
          {approvalRequest ? (
            <label className="space-y-2">
              <span className="block text-sm font-medium text-card-foreground">
                {t("requests.approvedLease")}
              </span>
              <Select
                value={approvalLeaseChoice}
                onValueChange={(value) =>
                  setApprovalLeaseChoice(value as ApprovalLeaseChoice)
                }
                disabled={actionRequestId !== null}
              >
                <SelectTrigger id={APPROVE_MEDIA_REQUEST_LEASE_ID}>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem
                    id={approveMediaRequestLeaseOptionId("requested")}
                    value="requested"
                  >
                    {approvalRequest.requestedLeaseDays == null
                      ? t("requests.approvedLeaseKeepForever")
                      : t("requests.approvedLeaseKeepRequested", {
                          count: approvalRequest.requestedLeaseDays,
                        })}
                  </SelectItem>
                  <SelectItem
                    id={approveMediaRequestLeaseOptionId("forever")}
                    value="forever"
                  >
                    {t("search.keepForForever")}
                  </SelectItem>
                  {REQUEST_LEASE_DAY_CHOICES.map((days) => (
                    <SelectItem
                      id={approveMediaRequestLeaseOptionId(String(days))}
                      key={days}
                      value={String(days)}
                    >
                      {t("search.keepForDays", { count: days })}
                    </SelectItem>
                  ))}
                  <SelectItem
                    id={approveMediaRequestLeaseOptionId("custom")}
                    value="custom"
                  >
                    {t("search.keepForCustom")}
                  </SelectItem>
                </SelectContent>
              </Select>
              {approvalLeaseChoice === "custom" ? (
                <Input
                  id={APPROVE_MEDIA_REQUEST_LEASE_DAYS_ID}
                  {...integerInputProps}
                  min={REQUEST_LEASE_DAYS_MIN}
                  max={REQUEST_LEASE_DAYS_MAX}
                  value={approvalCustomLeaseDays}
                  disabled={actionRequestId !== null}
                  onChange={(event) =>
                    setApprovalCustomLeaseDays(
                      Number(sanitizeDigits(event.target.value)) || 0,
                    )
                  }
                  onBlur={() =>
                    setApprovalCustomLeaseDays((days) =>
                      clampRequestLeaseDays(days),
                    )
                  }
                />
              ) : null}
            </label>
          ) : null}
          {approvalRequest ? (
            <div className="space-y-2">
              <span className="block text-sm font-medium text-card-foreground">
                {t("requests.approvedTags")}
              </span>
              <p className="text-xs text-muted-foreground">
                {t("requests.approvedTagsHelp")}
              </p>
              {approvalTags.length > 0 ? (
                <div className="flex flex-wrap gap-1">
                  {approvalTags.map((tag) => (
                    <Badge key={tag} tone="info" className="gap-1">
                      {tag}
                      <button
                        id={approveMediaRequestTagRemoveId(tag)}
                        type="button"
                        aria-label={t("label.delete")}
                        onClick={() =>
                          setApprovalTags((prev) =>
                            prev.filter((value) => value !== tag),
                          )
                        }
                      >
                        <X className="h-3 w-3" />
                      </button>
                    </Badge>
                  ))}
                </div>
              ) : null}
              <div className="flex gap-2">
                <Input
                  id={APPROVE_MEDIA_REQUEST_TAG_INPUT_ID}
                  value={approvalTagDraft}
                  placeholder={t("requests.approvedTagsPlaceholder")}
                  disabled={actionRequestId !== null}
                  onChange={(event) => setApprovalTagDraft(event.target.value)}
                  onKeyDown={(event) => {
                    if (event.key === "Enter") {
                      event.preventDefault();
                      addApprovalTag();
                    }
                  }}
                />
                <Button
                  id={APPROVE_MEDIA_REQUEST_TAG_ADD_ID}
                  type="button"
                  variant="secondary"
                  disabled={
                    actionRequestId !== null || !approvalTagDraft.trim()
                  }
                  onClick={addApprovalTag}
                >
                  <Plus className="h-4 w-4" />
                </Button>
              </div>
            </div>
          ) : null}
          <DialogFooter>
            <Button id="approve-media-request-cancel" type="button" variant="outline" onClick={closeApprovalDialog}>
              {t("label.cancel")}
            </Button>
            <Button
              id="approve-media-request-confirm"
              type="button"
              onClick={confirmApproval}
              disabled={
                !approvalProfileId ||
                actionRequestId !== null ||
                approvalBlocksConfirm
              }
            >
              <Check className="h-4 w-4" />
              {t("requests.approve")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
      <Dialog open={editRequest !== null} onOpenChange={(open) => { if (!open) closeEditDialog(); }}>
        <DialogContent id="edit-media-request-dialog" className="sm:max-w-sm">
          <DialogHeader>
            <DialogTitle>{t("requests.modifyTitle")}</DialogTitle>
          </DialogHeader>
          <label className="space-y-2">
            <span className="block text-sm font-medium text-card-foreground">
              {t("requests.requestedQualityProfile")}
            </span>
            <Select
              value={editProfileId}
              onValueChange={setEditProfileId}
              disabled={actionRequestId !== null || editProfileOptions.length === 0}
            >
              <SelectTrigger id="edit-media-request-quality-profile">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {editProfileOptions.map((profile) => (
                  <SelectItem
                    id={mediaRequestProfileOptionId("edit", profile.id)}
                    key={profile.id}
                    value={profile.id}
                  >
                    {profile.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </label>
          {editRequest && editRequest.facet !== "MOVIE" ? (
            <label className="space-y-2">
              <span className="block text-sm font-medium text-card-foreground">
                {t("requests.requestedMonitorType")}
              </span>
              <Select
                value={editMonitorType}
                onValueChange={(value) => {
                  const nextMonitorType = value as RequestMonitorType;
                  setEditMonitorType(nextMonitorType);
                  if (nextMonitorType !== "ADVANCED") {
                    setEditMonitorSelection(EMPTY_MONITOR_SELECTION);
                  }
                }}
                disabled={actionRequestId !== null}
              >
                <SelectTrigger id="edit-media-request-monitor-type">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {monitorOptions(t).map((option) => (
                    <SelectItem
                      id={mediaRequestMonitorOptionId("edit", option.value)}
                      key={option.value}
                      value={option.value}
                    >
                      {option.label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </label>
          ) : null}
          {editRequest && editAdvancedSelected ? (
            <MonitorSelectionPicker
              facet={editRequest.facet}
              tvdbId={editTvdbId}
              value={editMonitorSelection}
              onChange={setEditMonitorSelection}
              onLoadingChange={setEditSelectionLoading}
              disabled={actionRequestId !== null}
              idPrefix="edit-media-request"
            />
          ) : null}
          <DialogFooter>
            <Button id="edit-media-request-cancel" type="button" variant="outline" onClick={closeEditDialog}>
              {t("label.cancel")}
            </Button>
            <Button
              id="edit-media-request-confirm"
              type="button"
              onClick={confirmUpdate}
              disabled={
                !editProfileId ||
                actionRequestId !== null ||
                editBlocksConfirm
              }
            >
              <Check className="h-4 w-4" />
              {t("requests.saveChanges")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
      <Dialog
        open={pendingClaimAction !== null}
        onOpenChange={(open) => {
          if (!open) setPendingClaimAction(null);
        }}
      >
        <DialogContent id="title-claim-dialog" className="sm:max-w-sm">
          <DialogHeader>
            <DialogTitle>
              {pendingClaimAction?.kind === "release"
                ? t("requests.claimReleaseTitle")
                : t("requests.claimExtendTitle")}
            </DialogTitle>
          </DialogHeader>
          {pendingClaimAction?.kind === "extend" ? (
            <label className="space-y-2">
              <span className="block text-sm font-medium text-card-foreground">
                {t("requests.claimExtendUntil")}
              </span>
              <Input
                id={TITLE_CLAIM_EXTEND_DATE_ID}
                type="date"
                value={pendingClaimAction.expiresAt}
                onChange={(event) =>
                  setPendingClaimAction((prev) =>
                    prev && prev.kind === "extend"
                      ? { ...prev, expiresAt: event.target.value }
                      : prev,
                  )
                }
              />
            </label>
          ) : null}
          {pendingClaimAction?.kind === "release" ? (
            <label className="space-y-2">
              <span className="block text-sm font-medium text-card-foreground">
                {t("requests.claimReleaseReason")}
              </span>
              <Input
                id={TITLE_CLAIM_RELEASE_REASON_ID}
                value={pendingClaimAction.reason}
                placeholder={t("requests.claimReleaseReasonPlaceholder")}
                onChange={(event) =>
                  setPendingClaimAction((prev) =>
                    prev && prev.kind === "release"
                      ? { ...prev, reason: event.target.value }
                      : prev,
                  )
                }
              />
              <span className="block text-xs text-muted-foreground">
                {t("requests.claimReleaseReasonHelp")}
              </span>
            </label>
          ) : null}
          <DialogFooter>
            <Button
              id="title-claim-cancel"
              type="button"
              variant="outline"
              onClick={() => setPendingClaimAction(null)}
            >
              {t("label.cancel")}
            </Button>
            <Button
              id="title-claim-confirm"
              type="button"
              disabled={
                claimActionId !== null ||
                pendingClaimAction === null ||
                (pendingClaimAction.kind === "extend"
                  ? !pendingClaimAction.expiresAt
                  : !pendingClaimAction.reason.trim())
              }
              onClick={() => {
                if (!pendingClaimAction) return;
                if (pendingClaimAction.kind === "extend") {
                  // The picker gives a day; the API takes an instant, and the
                  // hold should survive the whole of the day the operator named.
                  onExtendClaim(
                    pendingClaimAction.claim,
                    `${pendingClaimAction.expiresAt}T23:59:59Z`,
                  );
                } else {
                  onReleaseClaim(
                    pendingClaimAction.claim,
                    pendingClaimAction.reason.trim(),
                  );
                }
                setPendingClaimAction(null);
              }}
            >
              <Check className="h-4 w-4" />
              {t("label.yes")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </section>
  );
}
