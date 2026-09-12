import * as React from "react";
import { AlertTriangle, CheckCircle2, Clock, Loader2, Send } from "lucide-react";
import { useClient } from "urql";

import { CatalogActionDialogSummary } from "@/components/root/catalog-action-dialog-summary";
import { MonitorSelectionPicker } from "@/components/common/monitor-selection-picker";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogFooter } from "@/components/ui/dialog";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { useTranslate } from "@/lib/context/translate-context";
import { defaultMonitorTypeForFacet } from "@/lib/facets/helpers";
import { previewMyRequestDecisionQuery } from "@/lib/graphql/queries";
import type { MetadataTvdbSearchItem } from "@/lib/graphql/smg-queries";
import {
  submitMediaRequestInput,
  type CatalogQualityProfileOption,
  type MetadataCatalogMonitorType,
  type MetadataCatalogRequestOptions,
} from "@/lib/hooks/use-global-search";
import type { Facet, LibraryRecord } from "@/lib/types";
import type { RequestPreflightResult } from "@/lib/types/request-rule-sets";
import {
  REQUEST_LEASE_DAYS_MAX,
  REQUEST_LEASE_DAYS_MIN,
  REQUEST_LEASE_DAY_CHOICES,
  clampRequestLeaseDays,
  requestFallbackReasonIsInformative,
  requestFallbackReasonLabelKey,
} from "@/lib/utils/request-rule-sets";
import type { MonitorSelectionDraft } from "@/lib/types/titles";
import {
  EMPTY_MONITOR_SELECTION,
  isMonitorSelectionEmpty,
  monitorSelectionInput,
} from "@/lib/utils/monitor-selection";
import {
  REQUEST_MEDIA_LEASE_DAYS_ID,
  REQUEST_MEDIA_LEASE_ID,
  REQUEST_PREFLIGHT_BANNER_ID,
  mediaRequestMonitorOptionId,
  mediaRequestProfileOptionId,
  requestMediaLeaseOptionId,
  selectorId,
} from "@/lib/utils/dom-ids";
import { Input, integerInputProps, sanitizeDigits } from "@/components/ui/input";

/// How long the dialog waits after the requester stops changing the request
/// before asking what would happen to it. Short enough to feel live, long
/// enough that dragging a lease slider is one question rather than twenty.
const PREFLIGHT_DEBOUNCE_MS = 400;

/// "Forever" is the default and is not a number: the API models it as an absent
/// `requestedLeaseDays`, so the picker keeps it as its own choice rather than
/// as a sentinel day count.
type LeaseChoice = "forever" | "custom" | `${number}`;

/// What would happen to this request if it were submitted right now.
///
/// It is a courtesy, not a gate: a "would be denied" answer still leaves submit
/// enabled, because the server denying it for real is what writes the decision
/// trace the requester can then read. A rule in shadow says so out loud, since
/// its verdict is recorded rather than acted on.
///
/// Renders nothing at all when the pre-flight has not answered or failed, so a
/// broken evaluation never blocks a request.
function RequestPreflightBanner({
  preflight,
}: {
  preflight: RequestPreflightResult | null;
}) {
  const t = useTranslate();
  if (!preflight) {
    return null;
  }

  const shadow = preflight.evaluationMode === "SHADOW";
  const reasonCodes = preflight.reasons.map((reason) => reason.code).join(", ");
  // The server's own word for why the verdict fell back to needing approval.
  // The first reason code is only an approximation of it, kept for a server
  // that predates `fallbackReason`.
  const fallbackReason =
    preflight.fallbackReason ?? preflight.reasons[0]?.code ?? null;
  const fallbackLabelKey = fallbackReason
    ? requestFallbackReasonLabelKey(fallbackReason)
    : null;
  // A known fallback code reads as plain English; anything else falls back to
  // the reason codes the rules emitted, which is what a requester can act on.
  const manualReviewReason = !requestFallbackReasonIsInformative(fallbackReason)
    ? null
    : fallbackLabelKey
      ? t(fallbackLabelKey)
      : reasonCodes || fallbackReason;

  const { Icon, className, title, body } =
    preflight.outcome === "AUTO_APPROVE"
      ? {
          Icon: CheckCircle2,
          className:
            "border-[var(--scry-success-border)] bg-[var(--scry-success-bg)] text-[var(--scry-success-text)]",
          title: t("requests.preflightAutoApprove"),
          body:
            preflight.tags.length > 0
              ? t("requests.preflightTags", { tags: preflight.tags.join(", ") })
              : null,
        }
      : preflight.outcome === "DENY"
        ? {
            Icon: AlertTriangle,
            className:
              "border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)] text-[var(--scry-danger-text)]",
            title: t("requests.preflightDeny", { reasons: reasonCodes }),
            body: null,
          }
        : {
            Icon: Clock,
            className:
              "border-[var(--scry-info-border)] bg-[var(--scry-info-bg)] text-[var(--scry-info-text)]",
            title: t("requests.preflightManualReview"),
            body: preflight.metadataPartial
              ? t("requests.preflightMetadataPartial")
              : manualReviewReason
                ? t("requests.preflightReason", { reason: manualReviewReason })
                : null,
          };

  return (
    <div
      id={REQUEST_PREFLIGHT_BANNER_ID}
      data-preflight-outcome={preflight.outcome}
      data-preflight-mode={preflight.evaluationMode}
      className={`flex items-start gap-2.5 rounded-[10px] border px-3 py-2.5 text-sm ${className}`}
    >
      <Icon className="mt-0.5 h-4 w-4 shrink-0" />
      <div className="space-y-1">
        <p className="font-semibold">
          {title}
          {shadow ? ` · ${t("requests.preflightPreviewOnly")}` : ""}
        </p>
        {body ? <p className="text-[13px] leading-5">{body}</p> : null}
      </div>
    </div>
  );
}

type RequestMediaDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  result: MetadataTvdbSearchItem;
  facet: Facet;
  requestableLibraries: LibraryRecord[];
  qualityProfileOptions: CatalogQualityProfileOption[];
  onRequest: (
    result: MetadataTvdbSearchItem,
    facet: Facet,
    options: MetadataCatalogRequestOptions,
  ) => Promise<boolean>;
};

export function RequestMediaDialog({
  open,
  onOpenChange,
  result,
  facet,
  requestableLibraries,
  qualityProfileOptions,
  onRequest,
}: RequestMediaDialogProps) {
  const t = useTranslate();
  const [libraryId, setLibraryId] = React.useState("");
  const [qualityProfileId, setQualityProfileId] = React.useState("");
  const [monitorType, setMonitorType] = React.useState<MetadataCatalogMonitorType>(
    () => defaultMonitorTypeForFacet(facet),
  );
  const [monitorSelection, setMonitorSelection] =
    React.useState<MonitorSelectionDraft>(EMPTY_MONITOR_SELECTION);
  const [monitorSelectionLoading, setMonitorSelectionLoading] =
    React.useState(false);
  const [isSubmitting, setIsSubmitting] = React.useState(false);
  const [leaseChoice, setLeaseChoice] = React.useState<LeaseChoice>("forever");
  const [customLeaseDays, setCustomLeaseDays] = React.useState(30);
  const [preflight, setPreflight] = React.useState<RequestPreflightResult | null>(
    null,
  );
  const client = useClient();

  React.useEffect(() => {
    if (!open) return;
    setLibraryId(
      requestableLibraries.find((library) => library.isDefault)?.id ||
        requestableLibraries[0]?.id ||
        "",
    );
    setQualityProfileId("");
    setMonitorType(defaultMonitorTypeForFacet(facet));
    setMonitorSelection(EMPTY_MONITOR_SELECTION);
    setMonitorSelectionLoading(false);
    setIsSubmitting(false);
    setLeaseChoice("forever");
    setCustomLeaseDays(30);
    setPreflight(null);
  }, [facet, open, requestableLibraries]);

  /// Null means forever, which is what the API models an absent lease as.
  const requestedLeaseDays =
    leaseChoice === "forever"
      ? null
      : leaseChoice === "custom"
        ? clampRequestLeaseDays(customLeaseDays)
        : Number(leaseChoice);

  const selectedLibrary = requestableLibraries.find((library) => library.id === libraryId) ?? null;
  const canRequestMonitorType = facet !== "MOVIE";
  const monitorOptions: Array<{ value: MetadataCatalogMonitorType; label: string }> = [
    { value: "FUTURE_EPISODES", label: t("search.monitorType.futureEpisodes") },
    {
      value: "MISSING_AND_FUTURE_EPISODES",
      label: t("search.monitorType.missingAndFutureEpisodes"),
    },
    { value: "ALL_EPISODES", label: t("search.monitorType.allEpisodes") },
    { value: "NONE", label: t("search.monitorType.none") },
    { value: "ADVANCED", label: t("search.monitorType.advanced") },
  ];
  // Same rule as the add dialog: the picker (and its one metadata query) only
  // exists while ADVANCED is the live choice.
  const advancedSelected = canRequestMonitorType && monitorType === "ADVANCED";
  const advancedTvdbId = String(result.tvdbId ?? "").trim();
  const advancedBlocksSubmit =
    advancedSelected &&
    (!advancedTvdbId ||
      monitorSelectionLoading ||
      isMonitorSelectionEmpty(monitorSelection));
  const requestProfileOptions = React.useMemo(() => {
    const requestProfileIds = selectedLibrary?.requestQualityProfileIds?.length
      ? selectedLibrary.requestQualityProfileIds
      : selectedLibrary?.requestQualityProfileDefaultId
        ? [selectedLibrary.requestQualityProfileDefaultId]
        : [];
    return requestProfileIds.map((profileId) => {
      const profile = qualityProfileOptions.find((option) => option.id === profileId);
      return {
        id: profileId,
        name: profile?.name ?? profileId,
      };
    });
  }, [qualityProfileOptions, selectedLibrary]);
  React.useEffect(() => {
    if (!open || !selectedLibrary) return;
    const defaultProfileId =
      selectedLibrary.requestQualityProfileDefaultId ||
      requestProfileOptions[0]?.id ||
      "";
    setQualityProfileId((current) =>
      current && requestProfileOptions.some((profile) => profile.id === current)
        ? current
        : defaultProfileId,
    );
  }, [open, requestProfileOptions, selectedLibrary]);

  /// Ask the server what would happen to this request, debounced on every
  /// choice that can change the answer. The query is the requester's own
  /// pre-flight: it never returns rule internals, and it never errors for an
  /// evaluation failure, so calling it on every edit is safe.
  ///
  /// A failure hides the banner entirely rather than blocking the dialog. The
  /// answer is a courtesy; the submit is the thing that matters.
  React.useEffect(() => {
    if (!open || !selectedLibrary || !qualityProfileId) {
      setPreflight(null);
      return;
    }
    let cancelled = false;
    const timer = setTimeout(() => {
      const input = submitMediaRequestInput(result, facet, {
        libraryId: selectedLibrary.id,
        requestedQualityProfileId: qualityProfileId,
        requestedMonitorType: canRequestMonitorType ? monitorType : undefined,
        requestedMonitorSelection: advancedSelected
          ? monitorSelectionInput(monitorSelection)
          : undefined,
        requestedLeaseDays: requestedLeaseDays ?? undefined,
      });
      void client
        .query(previewMyRequestDecisionQuery, { input }, {
          requestPolicy: "network-only",
        })
        .toPromise()
        .then(({ data, error }) => {
          if (cancelled) return;
          setPreflight(
            error
              ? null
              : ((data?.previewMyRequestDecision as RequestPreflightResult) ??
                  null),
          );
        })
        .catch(() => {
          if (!cancelled) setPreflight(null);
        });
    }, PREFLIGHT_DEBOUNCE_MS);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [
    advancedSelected,
    canRequestMonitorType,
    client,
    facet,
    monitorSelection,
    monitorType,
    open,
    qualityProfileId,
    requestedLeaseDays,
    result,
    selectedLibrary,
  ]);

  const handleSubmit = React.useCallback(async () => {
    const selectedLibraryId = selectedLibrary?.id.trim();
    const selectedQualityProfileId = qualityProfileId.trim();
    if (!selectedLibraryId || !selectedQualityProfileId) return;

    setIsSubmitting(true);
    try {
      const accepted = await onRequest(result, facet, {
        libraryId: selectedLibraryId,
        requestedQualityProfileId: selectedQualityProfileId,
        requestedMonitorType: canRequestMonitorType ? monitorType : undefined,
        requestedMonitorSelection: advancedSelected
          ? monitorSelectionInput(monitorSelection)
          : undefined,
        requestedLeaseDays: requestedLeaseDays ?? undefined,
      });
      if (accepted) {
        onOpenChange(false);
      }
    } finally {
      setIsSubmitting(false);
    }
  }, [
    advancedSelected,
    canRequestMonitorType,
    facet,
    monitorSelection,
    monitorType,
    onOpenChange,
    onRequest,
    qualityProfileId,
    requestedLeaseDays,
    result,
    selectedLibrary,
  ]);

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        id="request-media-dialog"
        className="max-h-[90vh] gap-0 overflow-y-auto p-0 sm:max-w-5xl"
      >
        <CatalogActionDialogSummary result={result} facet={facet} mode="request" />

        <div className="space-y-6 p-5 sm:p-7">
          <div className="grid gap-4 sm:grid-cols-2">
            <label className="space-y-1 sm:col-span-2">
              <span className="block text-xs font-medium text-card-foreground">
                {t("search.addConfigLibrary")}
              </span>
              <Select
                value={selectedLibrary?.id || ""}
                onValueChange={setLibraryId}
                disabled={isSubmitting || requestableLibraries.length <= 1}
              >
                <SelectTrigger id="request-media-library" className="h-12 w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {requestableLibraries.map((library) => (
                    <SelectItem
                      id={selectorId("request-media-library-option", library.id)}
                      key={library.id}
                      value={library.id}
                    >
                      {library.name}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </label>

            <label className="space-y-1">
              <span className="block text-xs font-medium text-card-foreground">
                {t("requests.requestedQualityProfile")}
              </span>
              <Select
                value={qualityProfileId}
                onValueChange={setQualityProfileId}
                disabled={isSubmitting || requestProfileOptions.length <= 1}
              >
                <SelectTrigger
                  id="request-media-quality-profile"
                  className="h-12 w-full"
                >
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {requestProfileOptions.map((profile) => (
                    <SelectItem
                      id={mediaRequestProfileOptionId("request", profile.id)}
                      key={profile.id}
                      value={profile.id}
                    >
                      {profile.name}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </label>

            {canRequestMonitorType ? (
              <label className="space-y-1">
                <span className="block text-xs font-medium text-card-foreground">
                  {t("requests.requestedMonitorType")}
                </span>
                <Select
                  value={monitorType}
                  onValueChange={(value) => {
                    const nextMonitorType = value as MetadataCatalogMonitorType;
                    setMonitorType(nextMonitorType);
                    if (nextMonitorType !== "ADVANCED") {
                      setMonitorSelection(EMPTY_MONITOR_SELECTION);
                    }
                  }}
                  disabled={isSubmitting}
                >
                  <SelectTrigger
                    id="request-media-monitor-type"
                    className="h-12 w-full"
                  >
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {monitorOptions.map((option) => (
                      <SelectItem
                        id={mediaRequestMonitorOptionId("request", option.value)}
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

            <label className="space-y-1">
              <span className="block text-xs font-medium text-card-foreground">
                {t("search.keepFor")}
              </span>
              <Select
                value={leaseChoice}
                onValueChange={(value) => setLeaseChoice(value as LeaseChoice)}
                disabled={isSubmitting}
              >
                <SelectTrigger id={REQUEST_MEDIA_LEASE_ID} className="h-12 w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem
                    id={requestMediaLeaseOptionId("forever")}
                    value="forever"
                  >
                    {t("search.keepForForever")}
                  </SelectItem>
                  {REQUEST_LEASE_DAY_CHOICES.map((days) => (
                    <SelectItem
                      id={requestMediaLeaseOptionId(String(days))}
                      key={days}
                      value={String(days)}
                    >
                      {t("search.keepForDays", { count: days })}
                    </SelectItem>
                  ))}
                  <SelectItem
                    id={requestMediaLeaseOptionId("custom")}
                    value="custom"
                  >
                    {t("search.keepForCustom")}
                  </SelectItem>
                </SelectContent>
              </Select>
              <span className="block text-[11px] text-muted-foreground">
                {t("search.keepForHelp")}
              </span>
            </label>

            {leaseChoice === "custom" ? (
              <label className="space-y-1">
                <span className="block text-xs font-medium text-card-foreground">
                  {t("search.keepForCustomLabel")}
                </span>
                <Input
                  id={REQUEST_MEDIA_LEASE_DAYS_ID}
                  {...integerInputProps}
                  min={REQUEST_LEASE_DAYS_MIN}
                  max={REQUEST_LEASE_DAYS_MAX}
                  className="h-12"
                  value={customLeaseDays}
                  disabled={isSubmitting}
                  onChange={(event) =>
                    setCustomLeaseDays(
                      Number(sanitizeDigits(event.target.value)) || 0,
                    )
                  }
                  onBlur={() =>
                    setCustomLeaseDays((days) => clampRequestLeaseDays(days))
                  }
                />
              </label>
            ) : null}
          </div>

          <RequestPreflightBanner preflight={preflight} />

          {advancedSelected ? (
            <MonitorSelectionPicker
              facet={facet}
              tvdbId={advancedTvdbId}
              value={monitorSelection}
              onChange={setMonitorSelection}
              onLoadingChange={setMonitorSelectionLoading}
              disabled={isSubmitting}
              idPrefix="request-media"
            />
          ) : null}

        <DialogFooter className="items-stretch gap-3 sm:items-center">
          <Button
            id="request-media-cancel"
            type="button"
            variant="outline"
            onClick={() => onOpenChange(false)}
            disabled={isSubmitting}
            className="h-12 px-8"
          >
            {t("label.cancel")}
          </Button>
          <Button
            id="request-media-submit"
            type="button"
            onClick={() => void handleSubmit()}
            disabled={
              isSubmitting ||
              !selectedLibrary ||
              !qualityProfileId ||
              (canRequestMonitorType && !monitorType) ||
              advancedBlocksSubmit
            }
            className="h-12 gap-2 bg-primary px-8 text-primary-foreground hover:bg-primary/90"
          >
            {isSubmitting ? (
              <Loader2 className="h-4 w-4 animate-spin" />
            ) : (
              <Send className="h-4 w-4" />
            )}
            {isSubmitting ? t("search.requesting") : t("search.request")}
          </Button>
        </DialogFooter>
        </div>
      </DialogContent>
    </Dialog>
  );
}
