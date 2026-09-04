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
  RefreshCw,
  ShieldX,
  SlidersVertical,
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
import { UnderlineFilterButton } from "@/components/common/underline-filter-button";
import { TitlePoster } from "@/components/title-poster";
import { Button } from "@/components/ui/button";
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
import { formatUiDateTime } from "@/lib/utils/date-format";
import {
  mediaRequestApproveId,
  mediaRequestCancelId,
  mediaRequestDismissId,
  mediaRequestEditId,
  mediaRequestMonitorOptionId,
  mediaRequestProfileOptionId,
  mediaRequestRowId,
  mediaRequestStatusId,
} from "@/lib/utils/dom-ids";
import { selectPosterVariantUrl } from "@/lib/utils/poster-images";
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
  | "NONE";

type UpdateRequestValues = {
  requestedQualityProfileId: string;
  requestedMonitorType?: RequestMonitorType;
};

type ApproveRequestValues = {
  qualityProfileId: string;
  monitorType?: RequestMonitorType;
};

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
  const [editRequest, setEditRequest] =
    React.useState<MediaRequestRecord | null>(null);
  const [editProfileId, setEditProfileId] = React.useState("");
  const [editMonitorType, setEditMonitorType] =
    React.useState<RequestMonitorType>("FUTURE_EPISODES");
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
  }, [editProfileOptions, editRequest]);

  const openApprovalDialog = (request: MediaRequestRecord) => {
    onLoadQualityProfileOptions();
    setApprovalRequest(request);
  };

  const closeApprovalDialog = () => {
    setApprovalRequest(null);
    setApprovalProfileId("");
    setApprovalMonitorType("FUTURE_EPISODES");
  };

  const confirmApproval = () => {
    if (!approvalRequest || !approvalProfileId) return;
    onApprove(approvalRequest, {
      qualityProfileId: approvalProfileId,
      monitorType:
        approvalRequest.facet === "MOVIE" ? undefined : approvalMonitorType,
    });
    closeApprovalDialog();
  };

  const openEditDialog = (request: MediaRequestRecord) => {
    onLoadQualityProfileOptions();
    setEditRequest(request);
  };

  const closeEditDialog = () => {
    setEditRequest(null);
    setEditProfileId("");
    setEditMonitorType("FUTURE_EPISODES");
  };

  const confirmUpdate = () => {
    if (!editRequest || !editProfileId) return;
    onUpdateRequest(editRequest, {
      requestedQualityProfileId: editProfileId,
      requestedMonitorType: editRequest.facet === "MOVIE" ? undefined : editMonitorType,
    });
    closeEditDialog();
  };

  const renderRequestCard = (request: MediaRequestRecord) => {
    const posterUrl = selectPosterVariantUrl(request.posterUrl, "w250");
    const backgroundPosterUrl =
      selectPosterVariantUrl(request.posterUrl, "original") ?? posterUrl;
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
    const actionsDisabled = loading || actionRequestId !== null;
    const approveDisabled = loading || actionRequestId !== null;
    const statusMeta = requestStatusTone(t, request.status);
    const StatusIcon = statusMeta.Icon;
    const canResolveRequest = mode === "admin" && request.status === "PENDING";
    const canEditOwnRequest = mode === "mine" && request.status === "PENDING";
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
        {backgroundPosterUrl ? (
          <div
            aria-hidden="true"
            className="absolute inset-0 scale-105 bg-cover bg-center opacity-60"
            style={{ backgroundImage: `url(${backgroundPosterUrl})` }}
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
          <div className="w-full shrink-0 bg-[var(--scry-inset)] sm:w-[150px]">
            <div className="aspect-[2/3] w-full overflow-hidden sm:h-full sm:min-h-[225px]">
              {posterUrl ? (
                <TitlePoster
                  src={posterUrl}
                  alt={t("media.posterAlt", { name: request.title })}
                  className="h-full w-full object-cover"
                  loading="lazy"
                />
              ) : (
                <div className="flex h-full min-h-[180px] w-full items-center justify-center text-xs text-[var(--scry-muted3)]">
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
                </div>
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
              </div>
            </div>
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
        <DialogContent id="approve-media-request-dialog" className="sm:max-w-sm">
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
              disabled={loading || actionRequestId !== null}
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
                onValueChange={(value) =>
                  setApprovalMonitorType(value as RequestMonitorType)
                }
                disabled={loading || actionRequestId !== null}
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
          <DialogFooter>
            <Button id="approve-media-request-cancel" type="button" variant="outline" onClick={closeApprovalDialog}>
              {t("label.cancel")}
            </Button>
            <Button
              id="approve-media-request-confirm"
              type="button"
              onClick={confirmApproval}
              disabled={!approvalProfileId || loading || actionRequestId !== null}
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
              disabled={loading || actionRequestId !== null || editProfileOptions.length === 0}
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
                onValueChange={(value) => setEditMonitorType(value as RequestMonitorType)}
                disabled={loading || actionRequestId !== null}
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
          <DialogFooter>
            <Button id="edit-media-request-cancel" type="button" variant="outline" onClick={closeEditDialog}>
              {t("label.cancel")}
            </Button>
            <Button
              id="edit-media-request-confirm"
              type="button"
              onClick={confirmUpdate}
              disabled={!editProfileId || loading || actionRequestId !== null}
            >
              <Check className="h-4 w-4" />
              {t("requests.saveChanges")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </section>
  );
}
