import * as React from "react";
import { useQuery } from "urql";
import { useActivityEventStream } from "@/lib/hooks/use-activity-event-stream";
import { TITLE_OVERVIEW_IMPORT_REFRESH_KINDS } from "@/lib/utils/title-overview-refresh-policy";
import type { CurrentReleaseScope } from "@/lib/utils/current-release";
import { currentReleaseFilesQuery } from "@/lib/graphql/queries";
import { currentReleaseNames, releaseIdentityName, type CurrentReleaseFile } from "@/lib/utils/current-release";
import {
  Ban,
  ArrowDown,
  ArrowUp,
  Check,
  ChevronDown,
  Database,
  Download,
  FilePlus2,
  Languages,
  Layers,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { MovieAvailabilityBadge } from "@/components/common/movie-availability-badge";
import { useTranslate } from "@/lib/context/translate-context";
import { cn } from "@/lib/utils";
import { ScoreLog, scoreToneClassName } from "@/components/common/score-log";
import { formatScore, toScoreLogEntries } from "@/lib/utils/score-log";
import { releaseCoversMultipleEpisodes } from "@/lib/utils/release-queue-scope";
import {
  releaseSearchResultQueueAdditionalId,
  releaseSearchResultQueueId,
  releaseSearchResultQueueReasonId,
  releaseSearchResultRowId,
} from "@/lib/utils/dom-ids";
import type { Release } from "@/lib/types";

import { sortReleaseSearchResults as sortBy, type ReleaseSearchSortKey, type ReleaseSearchSortDirection } from "@/lib/utils/release-search-sort";
export type { ReleaseSearchSortKey, ReleaseSearchSortDirection } from "@/lib/utils/release-search-sort";
type SortKey = ReleaseSearchSortKey;
type SortDirection = ReleaseSearchSortDirection;
type SearchResultPresentation = "default" | "selected-title";

const selectedTitleTagClassNames = [
  "bg-[var(--scry-facet-series-bg)] text-[var(--scry-facet-series-text)]",
  "bg-[var(--scry-facet-movie-bg)] text-[var(--scry-facet-movie-text)]",
  "bg-[var(--scry-facet-anime-bg)] text-[var(--scry-facet-anime-text)]",
  "bg-[var(--scry-chip)] text-[var(--scry-muted2)]",
];

function selectedTitleTagClassName(index: number) {
  return cn(
    "inline-flex items-center rounded-[6px] px-[9px] py-[3px] text-[10.5px] font-semibold",
    selectedTitleTagClassNames[index % selectedTitleTagClassNames.length],
  );
}

function bytesToWholeReadable(raw: number | null | undefined) {
  if (!raw || raw <= 0) {
    return "—";
  }
  const gb = 1024 * 1024 * 1024;
  const mb = 1024 * 1024;
  const kb = 1024;

  if (raw >= gb) {
    return `${(raw / gb).toFixed(2)} GiB`;
  }
  if (raw >= mb) {
    return `${(raw / mb).toFixed(1)} MiB`;
  }
  if (raw >= kb) {
    return `${(raw / kb).toFixed(1)} KiB`;
  }
  return `${Math.floor(raw)} B`;
}

function ScoringLogPanel({
  decision,
}: {
  decision: NonNullable<Release["qualityProfileDecision"]>;
}) {
  return (
    <ScoreLog
      entries={toScoreLogEntries(decision.scoringLog)}
      total={decision.releaseScore}
    />
  );
}

/**
 * Where the release stands against the title's audio language preferences.
 * Nothing is shown when the title has none, so rows look as they always did.
 */
function audioLanguageBadge(
  rank: NonNullable<Release["qualityProfileDecision"]>["audioLanguage"],
  t: ReturnType<typeof useTranslate>,
): React.ReactNode {
  if (!rank || (rank.preferredCount === 0 && rank.optionalHits === 0)) {
    return null;
  }
  const parts: string[] = [];
  let tone = "bg-[var(--scry-info-bg-strong)] text-[var(--scry-info-text)]";
  if (rank.preferredCount > 0) {
    if (rank.preferredRank >= rank.preferredCount) {
      parts.push(t("nzb.audioLanguageNotPreferred"));
      tone = "bg-[var(--scry-chip)] text-[var(--scry-muted3)]";
    } else if (rank.preferredCount === 1) {
      parts.push(t("nzb.audioLanguagePreferred"));
      tone = "bg-[var(--scry-success-bg-strong)] text-[var(--scry-success-text-soft)]";
    } else {
      parts.push(t("nzb.audioLanguagePreferredRank", { rank: rank.preferredRank + 1 }));
      if (rank.preferredRank === 0) {
        tone = "bg-[var(--scry-success-bg-strong)] text-[var(--scry-success-text-soft)]";
      }
    }
  }
  if (rank.optionalHits > 0) {
    parts.push(t("nzb.audioLanguageOptionalHits", { count: rank.optionalHits }));
  }
  return (
    <span
      data-ui="release-search-result-audio-language"
      title={t("nzb.audioLanguageHint")}
      className={cn(
        "inline-flex items-center gap-1 rounded-[6px] px-[7px] py-px text-[10px] font-bold",
        tone,
      )}
    >
      <Languages className="h-2.5 w-2.5 shrink-0" />
      {parts.join(" · ")}
    </span>
  );
}

function SearchResultRow({
  result,
  currentFile = false,
  onQueue,
  onQueueAdditional,
  disabled = false,
  requireCandidateToken = false,
  mobile = false,
  presentation = "default",
}: {
  result: Release;
  currentFile?: boolean;
  onQueue: (r: Release) => Promise<void> | void;
  onQueueAdditional?: (r: Release) => Promise<void> | void;
  disabled?: boolean;
  requireCandidateToken?: boolean;
  mobile?: boolean;
  presentation?: SearchResultPresentation;
}) {
  const t = useTranslate();
  const [expanded, setExpanded] = React.useState(false);
  const [queueRequested, setQueueRequested] = React.useState(false);
  const [additionalQueueRequested, setAdditionalQueueRequested] =
    React.useState(false);
  const decision = result.qualityProfileDecision;
  const hasLog = decision && decision.scoringLog.length > 0;
  const blockReason =
    decision && decision.blockCodes.length > 0
      ? decision.blockCodes.join(" · ")
      : null;
  const rejectionBadge = blockReason ? (
    <span className="inline-flex max-w-full items-center gap-1 rounded-[6px] bg-[var(--scry-danger-bg)] px-2 py-0.5 text-[10px] font-bold text-[var(--scry-danger-text-soft)]">
      <Ban className="h-2.5 w-2.5 shrink-0" />
      <span className="min-w-0 break-words">{blockReason}</span>
    </span>
  ) : null;
  const approvedBadge =
    decision?.allowed ? (
      <span className="inline-flex items-center gap-1 rounded-[6px] bg-[var(--scry-success-bg-strong)] px-[7px] py-px text-[10px] font-bold text-[var(--scry-success-text-soft)]">
        <Check className="h-2.5 w-2.5" />
        Approved
      </span>
    ) : null;
  const languageBadge = audioLanguageBadge(decision?.audioLanguage, t);
  const currentFileBadge = currentFile ? (
    <span
      data-ui="release-search-result-current-file"
      className="inline-flex items-center gap-1 rounded-[6px] bg-[var(--scry-success-bg-strong)] px-[7px] py-px text-[10px] font-bold text-[var(--scry-success-text-soft)]"
    >
      <Check className="h-2.5 w-2.5 shrink-0" />
      {t("nzb.currentFile")}
    </span>
  ) : null;
  // A pack answers an episode search by covering that episode among others.
  // The row says so, because "1 GB" and "15 GB" for the same episode is only
  // explicable once you know one of them is the whole season.
  const packBadge = releaseCoversMultipleEpisodes(result) ? (
    <span
      data-ui="release-search-result-pack"
      title={t("label.multiEpisodePackHint")}
      className="inline-flex items-center gap-1 rounded-[6px] bg-[var(--scry-info-bg-strong)] px-[7px] py-px text-[10px] font-bold text-[var(--scry-info-text)]"
    >
      <Layers className="h-2.5 w-2.5 shrink-0" />
      {t("label.multiEpisodePack")}
    </span>
  ) : null;
  const parsedBits = [
    result.parsedRelease?.quality,
    result.parsedRelease?.videoCodec,
    result.parsedRelease?.videoEncoding,
    result.parsedRelease?.audio,
  ]
    .filter((value) => value)
    .filter((value) => typeof value === "string" && value.trim().length > 0);
  const parsedMetadata = result.parsedRelease
    ? [
        result.parsedRelease.detectedHdr
          ? { label: "HDR", className: "bg-[var(--scry-info-bg-strong)] text-[var(--scry-info-text)]" }
          : null,
        result.parsedRelease.isDolbyVision
          ? {
              label: "Dolby Vision",
              className: "bg-[rgba(var(--scry-accent-rgb),0.2)] text-[var(--scry-accent-text)]",
            }
          : null,
        result.parsedRelease.isProperUpload
          ? { label: "Proper", className: "bg-[var(--scry-warning-bg-strong)] text-[var(--scry-warning-text)]" }
          : null,
        result.parsedRelease.isRemux
          ? { label: "Remux", className: "bg-[rgba(var(--scry-accent-rgb),0.2)] text-[var(--scry-accent-text)]" }
          : null,
        result.parsedRelease.isBdDisk
          ? { label: "BD", className: "bg-[var(--scry-danger-bg-strong)] text-[var(--scry-danger-text)]" }
          : null,
        result.parsedRelease.isAiEnhanced
          ? { label: "AI Enhanced", className: "bg-[var(--scry-danger-bg-strong)] text-[var(--scry-danger-text)]" }
          : null,
        result.parsedRelease.isAtmos
          ? { label: "Atmos", className: "bg-[var(--scry-info-bg-strong)] text-[var(--scry-info-text)]" }
          : null,
      ]
        .filter(Boolean)
        .filter(
          (value) =>
            value !== null && value !== undefined && typeof value === "object",
        )
        .map((entry) => entry as { label: string; className: string })
    : [];
  // A dead swarm blocks queueing for the same reason auto-search skipped it:
  // the grab cannot finish. The row still shows the release and its seeder
  // count so the operator can see why, rather than the release vanishing.
  const belowMinimumSeeders = result.autoDecisionCode === "minimum_seeders";
  // The code travels with the text so a reader (or a check) can tell the two
  // reasons apart without parsing the localized sentence.
  const queueUnavailableCode =
    requireCandidateToken && !result.candidateToken
      ? "manual_unavailable"
      : belowMinimumSeeders
        ? "minimum_seeders"
        : null;
  const queueUnavailableReason =
    queueUnavailableCode === "manual_unavailable"
      ? t("queue.manualUnavailableForResult")
      : queueUnavailableCode === "minimum_seeders"
        ? t("queue.belowMinimumSeeders")
        : null;
  const queueDisabled = disabled || queueRequested || queueUnavailableReason !== null;
  const queueButtonMuted = queueUnavailableReason !== null;
  const additionalQueueDisabled =
    disabled ||
    additionalQueueRequested ||
    queueUnavailableReason !== null ||
    !onQueueAdditional;
  const idVariant = mobile ? "mobile" : undefined;
  const rowId = releaseSearchResultRowId(result, idVariant);
  const queueButtonId = releaseSearchResultQueueId(result, idVariant);
  const queueReasonId = releaseSearchResultQueueReasonId(result, idVariant);
  const queueAdditionalButtonId = releaseSearchResultQueueAdditionalId(
    result,
    idVariant,
  );

  const handleQueueClick = React.useCallback(() => {
    if (queueDisabled) {
      return;
    }

    setQueueRequested(true);

    try {
      const maybePromise = onQueue(result);
      if (
        maybePromise &&
        typeof (maybePromise as Promise<void>).then === "function"
      ) {
        void (maybePromise as Promise<void>).catch(() => {
          setQueueRequested(false);
        });
      }
    } catch {
      setQueueRequested(false);
    }
  }, [onQueue, queueDisabled, result]);

  const handleQueueAdditionalClick = React.useCallback(() => {
    if (additionalQueueDisabled || !onQueueAdditional) {
      return;
    }

    setAdditionalQueueRequested(true);

    try {
      const maybePromise = onQueueAdditional(result);
      if (
        maybePromise &&
        typeof (maybePromise as Promise<void>).then === "function"
      ) {
        void (maybePromise as Promise<void>).catch(() => {
          setAdditionalQueueRequested(false);
        });
      }
    } catch {
      setAdditionalQueueRequested(false);
    }
  }, [additionalQueueDisabled, onQueueAdditional, result]);


  if (presentation === "selected-title") {
    const scoreClassName = scoreToneClassName(decision?.releaseScore);
    const selectedTitleTags = [
      ...parsedBits.map((metadataBit, index) => ({
        className: selectedTitleTagClassName(index),
        label: metadataBit,
      })),
      ...parsedMetadata.map((metadataBit) => ({
        className: cn(
          "inline-flex items-center rounded-[6px] px-[9px] py-[3px] text-[10.5px] font-semibold",
          metadataBit.className,
        ),
        label: metadataBit.label,
      })),
    ];

    return (
      <div className="border-b border-[var(--scry-line2)] last:border-b-0">
      <div
        id={rowId}
        data-ui="release-search-result-row"
        data-release-source={result.source ?? ""}
        data-release-title={result.title}
        data-release-link={result.link ?? ""}
        data-release-download-url={result.downloadUrl ?? ""}
        data-release-candidate-token={result.candidateToken ?? ""}
        className="flex items-center gap-4 px-4 py-3.5 transition-colors hover:bg-[var(--scry-hover)] max-md:flex-col max-md:items-stretch"
      >
        <div className="min-w-0 flex-1">
          <div className="mb-1.5 break-words text-[13px] font-semibold leading-[1.35] text-[var(--scry-ink2)]">
            {result.title}
          </div>
          <div className="mb-2 flex flex-wrap items-center gap-x-[7px] gap-y-1 text-[11px] text-[var(--scry-faint)]">
            <Database className="h-3 w-3 shrink-0" />
            <span>{result.source ?? t("label.unknown")}</span>
            {result.publishedAt ? (
              <>
                <span
                  aria-hidden="true"
                  className="h-[3px] w-[3px] rounded-full bg-[var(--scry-faint4)]"
                />
                <span>{result.publishedAt}</span>
              </>
            ) : null}
            {approvedBadge}
            {currentFileBadge}{packBadge}{languageBadge}
            {rejectionBadge}
          </div>
          {selectedTitleTags.length > 0 ? (
            <div className="flex flex-wrap gap-1.5">
              {selectedTitleTags.map((metadataBit, index) => (
                <span
                  key={`${metadataBit.label}-${index}`}
                  className={metadataBit.className}
                >
                  {metadataBit.label}
                </span>
              ))}
            </div>
          ) : null}
          <MovieAvailabilityBadge result={result} className="mt-2" />
          {queueUnavailableReason ? (
            <p
              id={queueReasonId}
              data-ui="release-search-result-queue-reason"
              data-reason={queueUnavailableCode ?? undefined}
              className="mt-2 text-[11px] text-[var(--scry-faint)]"
            >
              {queueUnavailableReason}
            </p>
          ) : null}
        </div>
        <div className="w-[74px] shrink-0 text-center max-md:w-auto max-md:text-left">
          {hasLog ? (
            <button
              type="button"
              onClick={() => setExpanded((prev) => !prev)}
              aria-expanded={expanded}
              aria-label={
                expanded ? t("nzb.hideScoringLog") : t("nzb.showScoringLog")
              }
              className={cn(
                "text-[14px] font-bold tabular-nums underline-offset-2 hover:underline",
                scoreClassName,
              )}
            >
              {formatScore(decision?.releaseScore)}
            </button>
          ) : (
            <div
              className={cn(
                "text-[14px] font-bold tabular-nums",
                scoreClassName,
              )}
            >
              {formatScore(decision?.releaseScore)}
            </div>
          )}
          <div className="mt-0.5 text-[9.5px] font-bold uppercase tracking-[0.06em] text-[var(--scry-faint3)]">
            {t("nzb.score")}
          </div>
        </div>
        <div className="w-16 shrink-0 text-right max-md:w-auto max-md:text-left">
          <div className="font-[var(--font-code)] text-[15px] font-bold text-[var(--scry-ink2)]">
            {bytesToWholeReadable(result.sizeBytes)}
          </div>
        </div>
        <div className="flex w-[166px] shrink-0 flex-col gap-[7px] max-md:w-full">
          <Button
            id={queueButtonId}
            data-ui="release-search-result-queue"
            size="sm"
            onClick={handleQueueClick}
            disabled={queueDisabled}
            className={cn(
              "h-[38px] justify-center gap-[7px] rounded-[10px] border-0 text-[12.5px] font-bold text-[var(--scry-success-on-solid)] shadow-[0_6px_16px_rgba(var(--scry-success-rgb),0.28)]",
              queueButtonMuted
                ? "border border-[var(--scry-border2)] bg-[var(--scry-soft)] text-[var(--scry-muted3)] shadow-none hover:bg-[var(--scry-soft)] hover:text-[var(--scry-muted3)]"
                : "bg-[var(--scry-success-solid)] hover:bg-[var(--scry-success-solid-hover)]",
            )}
          >
            {queueRequested ? (
              <>
                <Check className="h-[15px] w-[15px]" />
                {t("queue.state.queued")}
              </>
            ) : (
              <>
                <Download className="h-[15px] w-[15px]" />
                {t("nzb.queue")}
              </>
            )}
          </Button>
          {onQueueAdditional ? (
            <Button
              id={queueAdditionalButtonId}
              data-ui="release-search-result-queue-additional"
              type="button"
              size="sm"
              variant="outline"
              onClick={handleQueueAdditionalClick}
              disabled={additionalQueueDisabled}
              className={cn(
                "h-[34px] justify-center gap-[7px] rounded-[10px] border-[var(--scry-baccent)] bg-[rgba(var(--scry-accent-rgb),0.16)] text-[11.5px] font-semibold text-[var(--scry-accent-text)] shadow-none hover:border-[var(--scry-accent)] hover:bg-[rgba(var(--scry-accent-rgb),0.26)] hover:text-[var(--scry-accent-text)]",
                additionalQueueRequested &&
                  "border-[var(--scry-success-border)] bg-[var(--scry-success-bg)] text-[var(--scry-success-text)] hover:border-[var(--scry-success-border-strong)] hover:bg-[var(--scry-success-bg-strong)] hover:text-[var(--scry-success-text)]",
              )}
            >
              {additionalQueueRequested ? (
                <>
                  <Check className="h-3.5 w-3.5" />
                  {t("queue.state.queued")}
                </>
              ) : (
                <>
                  <FilePlus2 className="h-3.5 w-3.5" />
                  {t("nzb.queueAdditionalFile")}
                </>
              )}
            </Button>
          ) : null}
        </div>
      </div>
      {expanded && hasLog ? <ScoringLogPanel decision={decision} /> : null}
      </div>
    );
  }

  if (mobile) {
    return (
      <div
        id={rowId}
        data-ui="release-search-result-row"
        data-release-source={result.source ?? ""}
        data-release-title={result.title}
        data-release-link={result.link ?? ""}
        data-release-download-url={result.downloadUrl ?? ""}
        data-release-candidate-token={result.candidateToken ?? ""}
        className="rounded-lg border border-border bg-background/40 px-2.5 py-2"
      >
        <div className="space-y-1.5">
          <p className="min-w-0 whitespace-normal break-words text-sm font-semibold leading-snug text-foreground">
            {result.title}
          </p>
          <div className="flex flex-wrap items-center gap-x-2 gap-y-1 text-xs text-muted-foreground">
            <span>{result.source ?? t("label.unknown")}</span>
            {result.publishedAt ? (
              <>
                <span aria-hidden="true">•</span>
                <span>{result.publishedAt}</span>
              </>
            ) : null}
            <span aria-hidden="true">•</span>
            <span className="font-[var(--font-code)] font-medium text-foreground/80">
              {bytesToWholeReadable(result.sizeBytes)}
            </span>
            {decision ? (
              <>
                <span aria-hidden="true">•</span>
                {hasLog ? (
                  <button
                    type="button"
                    onClick={() => setExpanded((prev) => !prev)}
                    aria-expanded={expanded}
                    aria-label={
                      expanded ? t("nzb.hideScoringLog") : t("nzb.showScoringLog")
                    }
                    className="inline-flex items-center gap-1"
                  >
                    {t("nzb.score")}
                    <span
                      className={`font-[var(--font-code)] font-semibold underline decoration-dotted underline-offset-2 ${scoreToneClassName(decision.releaseScore)}`}
                    >
                      {formatScore(decision.releaseScore)}
                    </span>
                  </button>
                ) : (
                  <span className="inline-flex items-center gap-1">
                    {t("nzb.score")}
                    <span
                      className={`font-[var(--font-code)] font-semibold ${scoreToneClassName(decision.releaseScore)}`}
                    >
                      {formatScore(decision.releaseScore)}
                    </span>
                  </span>
                )}
              </>
            ) : null}
            {currentFileBadge}{packBadge}{languageBadge}
            {rejectionBadge}
          </div>
          {parsedBits.length > 0 ? (
            <div className="flex flex-wrap gap-1.5">
              {parsedBits.map((metadataBit) => (
                <span
                  key={metadataBit}
                  className="inline-flex items-center rounded-full border border-border bg-muted px-2 py-0.5 text-[11px] font-medium text-muted-foreground"
                >
                  {metadataBit}
                </span>
              ))}
            </div>
          ) : null}
          {parsedMetadata.length > 0 ? (
            <div className="flex flex-wrap gap-1.5">
              {parsedMetadata.map((metadataBit) => (
                <span
                  key={metadataBit.label}
                  className={`inline-flex items-center rounded-full border border-transparent px-2 py-0.5 text-[11px] font-medium ${metadataBit.className}`}
                >
                  {metadataBit.label}
                </span>
              ))}
            </div>
          ) : null}
          <MovieAvailabilityBadge result={result} className="self-start" />
          {queueUnavailableReason ? (
            <p
              id={queueReasonId}
              data-ui="release-search-result-queue-reason"
              data-reason={queueUnavailableCode ?? undefined}
              className="text-xs text-muted-foreground"
            >
              {queueUnavailableReason}
            </p>
          ) : null}
          {expanded && hasLog ? (
            <div className="overflow-x-auto rounded-md border border-border">
              <ScoringLogPanel decision={decision} />
            </div>
          ) : null}
          <div className="flex flex-row-reverse flex-wrap gap-2 pt-0.5">
            <Button
              id={queueButtonId}
              data-ui="release-search-result-queue"
              size="sm"
              onClick={handleQueueClick}
              disabled={queueDisabled}
              className={
                queueButtonMuted
                  ? "min-w-24 flex-1"
                  : queueRequested
                    ? "min-w-24 flex-1 border border-[var(--scry-success-border-strong)] bg-[var(--scry-success-bg-strong)] text-[var(--scry-success-text)]"
                    : "min-w-24 flex-1 border border-[var(--scry-success-border-strong)] bg-[var(--scry-success-solid)] text-[var(--scry-success-on-solid)] hover:bg-[var(--scry-success-solid-hover)] focus-visible:ring-[var(--scry-success-border-strong)]"
              }
              variant={queueButtonMuted ? "ghost" : "default"}
            >
              {queueRequested ? (
                <span className="inline-flex items-center gap-1.5">
                  <Check className="h-3.5 w-3.5" />
                  {t("queue.state.queued")}
                </span>
              ) : (
                t("nzb.queue")
              )}
            </Button>
            {onQueueAdditional ? (
              <Button
                id={queueAdditionalButtonId}
                data-ui="release-search-result-queue-additional"
                type="button"
                size="sm"
                variant="outline"
                onClick={handleQueueAdditionalClick}
                disabled={additionalQueueDisabled}
                className={cn(
                  "flex-1 border-[rgba(var(--scry-accent-rgb),0.30)] bg-[rgba(var(--scry-accent-rgb),0.08)] text-[var(--scry-accent-text)] shadow-none hover:bg-[rgba(var(--scry-accent-rgb),0.13)] hover:text-[var(--scry-accent-text)] focus-visible:ring-[rgba(var(--scry-accent-rgb),0.25)]",
                  additionalQueueRequested &&
                    "border-[var(--scry-success-border)] bg-[var(--scry-success-bg)] text-[var(--scry-success-text)] hover:border-[var(--scry-success-border-strong)] hover:bg-[var(--scry-success-bg-strong)] hover:text-[var(--scry-success-text)]",
                )}
              >
                {additionalQueueRequested ? (
                  <span className="inline-flex items-center gap-1.5">
                    <Check className="h-3.5 w-3.5" />
                    {t("queue.state.queued")}
                  </span>
                ) : (
                  <span className="inline-flex items-center gap-1.5">
                    <FilePlus2 className="h-3.5 w-3.5" />
                    {t("nzb.queueAdditionalFile")}
                  </span>
                )}
              </Button>
            ) : null}
          </div>
        </div>
      </div>
    );
  }

  return (
    <>
      <tr
        id={rowId}
        data-ui="release-search-result-row"
        data-release-source={result.source ?? ""}
        data-release-title={result.title}
        data-release-link={result.link ?? ""}
        data-release-download-url={result.downloadUrl ?? ""}
        data-release-candidate-token={result.candidateToken ?? ""}
      >
        <td className="rounded-l-lg border border-border border-r-0 px-4 py-2 align-middle">
          <div className="space-y-1">
            <p className="min-w-0 whitespace-normal break-words text-base font-semibold text-foreground">
              {result.title}
            </p>
            <div className="flex flex-wrap items-center gap-x-2 gap-y-1 text-xs text-muted-foreground">
              <span>{result.source ?? t("label.unknown")}</span>
              {result.publishedAt ? (
                <>
                  <span aria-hidden="true">•</span>
                  <span>{result.publishedAt}</span>
                </>
              ) : null}
              {currentFileBadge}{packBadge}{languageBadge}
              {rejectionBadge}
            </div>
            {parsedBits.length > 0 ? (
              <div className="mt-1 flex flex-wrap gap-1.5">
                {parsedBits.map((metadataBit) => (
                  <span
                    key={metadataBit}
                    className="inline-flex items-center rounded-full border border-border bg-muted px-2 py-0.5 text-[11px] font-medium text-muted-foreground"
                  >
                    {metadataBit}
                  </span>
                ))}
              </div>
            ) : null}
            {parsedMetadata.length > 0 ? (
              <div className="mt-1 flex flex-wrap gap-1.5">
                {parsedMetadata.map((metadataBit) => (
                  <span
                    key={metadataBit.label}
                    className={`inline-flex items-center rounded-full border border-transparent px-2 py-0.5 text-[11px] font-medium ${metadataBit.className}`}
                  >
                    {metadataBit.label}
                  </span>
                ))}
              </div>
            ) : null}
            <MovieAvailabilityBadge result={result} className="mt-1" />
            {queueUnavailableReason ? (
              <p
                id={queueReasonId}
                data-ui="release-search-result-queue-reason"
                data-reason={queueUnavailableCode ?? undefined}
                className="mt-1 text-xs text-muted-foreground"
              >
                {queueUnavailableReason}
              </p>
            ) : null}
          </div>
        </td>
        <td className="border border-border border-x-0 px-2 py-2 text-center align-middle">
          {decision ? (
            hasLog ? (
              <button
                type="button"
                className={`text-sm font-[var(--font-code)] underline-offset-2 hover:underline ${scoreToneClassName(decision.releaseScore)}`}
                onClick={() => setExpanded((prev) => !prev)}
                aria-label={
                  expanded ? t("nzb.hideScoringLog") : t("nzb.showScoringLog")
                }
              >
                {formatScore(decision.releaseScore)}
              </button>
            ) : (
              <span
                className={`text-sm font-[var(--font-code)] ${scoreToneClassName(decision.releaseScore)}`}
              >
                {formatScore(decision.releaseScore)}
              </span>
            )
          ) : (
            <span className="text-sm font-[var(--font-code)] text-muted-foreground">
              {formatScore(undefined)}
            </span>
          )}
        </td>
        <td className="whitespace-nowrap border border-border border-x-0 px-2 py-2 text-center align-middle font-[var(--font-code)] text-base font-semibold text-foreground">
          {bytesToWholeReadable(result.sizeBytes)}
        </td>
        <td className="rounded-r-lg border border-border border-l-0 px-3 py-2 text-center align-middle">
          <div className="flex flex-col items-stretch gap-1.5">
            <Button
              id={queueButtonId}
              data-ui="release-search-result-queue"
              size="default"
              onClick={handleQueueClick}
              disabled={queueDisabled}
              className={
                queueButtonMuted
                  ? "h-9 text-sm"
                  : queueRequested
                    ? "h-9 border border-[var(--scry-success-border-strong)] bg-[var(--scry-success-bg-strong)] text-sm text-[var(--scry-success-text)]"
                    : "h-9 border border-[var(--scry-success-border-strong)] bg-[var(--scry-success-solid)] text-sm text-[var(--scry-success-on-solid)] hover:bg-[var(--scry-success-solid-hover)] focus-visible:ring-[var(--scry-success-border-strong)]"
              }
              variant={queueButtonMuted ? "ghost" : "default"}
            >
              {queueRequested ? (
                <span className="inline-flex items-center gap-1.5">
                  <Check className="h-3.5 w-3.5" />
                  {t("queue.state.queued")}
                </span>
              ) : (
                t("nzb.queue")
              )}
            </Button>
            {onQueueAdditional ? (
              <Button
                id={queueAdditionalButtonId}
                data-ui="release-search-result-queue-additional"
                type="button"
                size="sm"
                variant="outline"
                onClick={handleQueueAdditionalClick}
                disabled={additionalQueueDisabled}
                className={cn(
                  "h-8 border-[rgba(var(--scry-accent-rgb),0.30)] bg-[rgba(var(--scry-accent-rgb),0.08)] text-xs text-[var(--scry-accent-text)] shadow-none hover:bg-[rgba(var(--scry-accent-rgb),0.13)] hover:text-[var(--scry-accent-text)] focus-visible:ring-[rgba(var(--scry-accent-rgb),0.25)]",
                  additionalQueueRequested &&
                    "border-[var(--scry-success-border)] bg-[var(--scry-success-bg)] text-[var(--scry-success-text)] hover:border-[var(--scry-success-border-strong)] hover:bg-[var(--scry-success-bg-strong)] hover:text-[var(--scry-success-text)]",
                )}
              >
                {additionalQueueRequested ? (
                  <span className="inline-flex items-center gap-1.5">
                    <Check className="h-3.5 w-3.5" />
                    {t("queue.state.queued")}
                  </span>
                ) : (
                  <span className="inline-flex items-center gap-1.5">
                    <FilePlus2 className="h-3.5 w-3.5" />
                    {t("nzb.queueAdditionalFile")}
                  </span>
                )}
              </Button>
            ) : null}
          </div>
        </td>
      </tr>
      {expanded && hasLog ? (
        <tr>
          <td
            colSpan={4}
            className="border border-x border-t-0 border-border p-0"
          >
            <ScoringLogPanel decision={decision} />
          </td>
        </tr>
      ) : null}
    </>
  );
}

const EMPTY_CURRENT_RELEASES = new Set<string>();
const OWNED_RELEASE_REFRESH_KINDS = new Set([...TITLE_OVERVIEW_IMPORT_REFRESH_KINDS, "system_notice", "file_analyzed"]);
type SearchResultBucketsProps = Omit<
  React.ComponentProps<typeof SearchResultBucketsContent>,
  "currentNames"
> & { ownedScope?: CurrentReleaseScope };

export function SearchResultBuckets({
  titleId,
  ...props
}: SearchResultBucketsProps & { titleId?: string }) {
  return titleId ? (
    <TitleSearchResultBuckets titleId={titleId} {...props} />
  ) : (
    <SearchResultBucketsContent {...props} />
  );
}

function TitleSearchResultBuckets({
  titleId,
  ownedScope,
  ...props
}: SearchResultBucketsProps & { titleId: string }) {
  const [{ data }, refresh] = useQuery<{ title: { id: string; mediaFiles: CurrentReleaseFile[] } | null }>({
    query: currentReleaseFilesQuery,
    variables: { id: titleId },
    requestPolicy: "cache-and-network",
  });
  useActivityEventStream({ titleId, kinds: OWNED_RELEASE_REFRESH_KINDS,
    onStart: () => refresh({ requestPolicy: "network-only" }),
    onEvent: () => refresh({ requestPolicy: "network-only" }) });
  const currentNames = React.useMemo(
    () => currentReleaseNames(data?.title?.id === titleId ? data.title.mediaFiles : [], ownedScope),
    [data, titleId, ownedScope],
  );
  return <SearchResultBucketsContent {...props} currentNames={currentNames} />;
}

function SearchResultBucketsContent({
  results,
  currentNames = EMPTY_CURRENT_RELEASES,
  onQueue,
  onQueueAdditional,
  canQueueAdditional,
  disabled = false,
  requireCandidateToken = false,
  compact = false,
  sortKey: controlledSortKey,
  sortDirection: controlledSortDirection,
  onSortChange,
  hideInlineSortControls = false,
  showBlockedInline = false,
  presentation = "default",
}: {
  results: Release[];
  currentNames?: Set<string>;
  onQueue: (r: Release) => Promise<void> | void;
  onQueueAdditional?: (r: Release) => Promise<void> | void;
  canQueueAdditional?: (r: Release) => boolean;
  disabled?: boolean;
  requireCandidateToken?: boolean;
  compact?: boolean;
  sortKey?: SortKey;
  sortDirection?: SortDirection;
  onSortChange?: (key: SortKey, direction: SortDirection) => void;
  hideInlineSortControls?: boolean;
  showBlockedInline?: boolean;
  presentation?: SearchResultPresentation;
}) {
  const t = useTranslate();
  const [localSortKey, setLocalSortKey] = React.useState<SortKey>("recommended");
  const [localSortDirection, setLocalSortDirection] =
    React.useState<SortDirection>("desc");
  const sortKey = controlledSortKey ?? localSortKey;
  const sortDirection = controlledSortDirection ?? localSortDirection;

  // Interactive releases are explicit operator choices. Score blocks remain
  // visible as diagnostics, but never prevent manual queueing.
  const sortedResults = React.useMemo(
    () => sortBy(results, sortKey, sortDirection),
    [results, sortDirection, sortKey],
  );

  const handleSort = React.useCallback(
    (next: SortKey) => {
      const nextDirection: SortDirection =
        sortKey === next
          ? sortDirection === "asc"
            ? "desc"
            : "asc"
          : "desc";

      if (controlledSortKey === undefined) {
        setLocalSortKey(next);
      }
      if (controlledSortDirection === undefined) {
        setLocalSortDirection(nextDirection);
      }
      onSortChange?.(next, nextDirection);
    },
    [controlledSortDirection, controlledSortKey, onSortChange, sortDirection, sortKey],
  );

  const renderSortIcon = React.useCallback(
    (key: SortKey) => {
      if (sortKey !== key) {
        return <ChevronDown className="h-3 w-3 opacity-30" />;
      }
      return sortDirection === "desc" ? (
        <ArrowDown className="h-3 w-3" />
      ) : (
        <ArrowUp className="h-3 w-3" />
      );
    },
    [sortDirection, sortKey],
  );

  const renderTable = React.useCallback(
    (entries: Release[]) => {
      return (
        <div className="space-y-3">
          {hideInlineSortControls ? null : (
            <div
              className={cn(
                "flex flex-wrap items-center gap-2",
              )}
            >
              <Button
                type="button"
                size="xs"
                variant={sortKey === "recommended" ? "secondary" : "outline"}
                onClick={() => handleSort("recommended")}
              >
                {t("nzb.recommended")}
              </Button>
              <Button
                type="button"
                size="xs"
                variant={sortKey === "score" ? "secondary" : "outline"}
                onClick={() => handleSort("score")}
              >
                {t("nzb.score")} {renderSortIcon("score")}
              </Button>
              <Button
                type="button"
                size="xs"
                variant={sortKey === "size" ? "secondary" : "outline"}
                onClick={() => handleSort("size")}
              >
                {t("nzb.size")} {renderSortIcon("size")}
              </Button>
            </div>
          )}

          <div className={cn("space-y-2", !compact && "md:hidden")}>
            {entries.map((result) => (
              <SearchResultRow
                key={`${result.source}-${result.title}-${result.link}`}
                result={result}
                currentFile={currentNames.has(releaseIdentityName(result.title))}
                onQueue={onQueue}
                onQueueAdditional={
                  onQueueAdditional &&
                  (!canQueueAdditional || canQueueAdditional(result))
                    ? onQueueAdditional
                    : undefined
                }
                disabled={disabled}
                requireCandidateToken={requireCandidateToken}
                mobile
              />
            ))}
          </div>

          <div
            className={cn(
              "hidden overflow-x-auto rounded-md border border-border bg-background/30",
              !compact && "md:block",
            )}
          >
            <table className="w-full table-fixed text-left">
              <colgroup>
                <col />
                <col className="w-20" />
                <col className="w-[6.5rem]" />
                <col className="w-56" />
              </colgroup>
              <thead className="bg-card/80">
                <tr>
                  <th className="px-4 py-3 text-base font-bold text-foreground">
                    Release
                  </th>
                  <th className="px-2 py-3 text-center text-base font-bold text-foreground">
                    <button
                      type="button"
                      className="inline-flex w-full items-center justify-center gap-1"
                      onClick={() => handleSort("score")}
                    >
                      {t("nzb.score")} {renderSortIcon("score")}
                    </button>
                  </th>
                  <th className="px-2 py-3 text-center text-base font-bold text-foreground">
                    <button
                      type="button"
                      className="inline-flex w-full items-center justify-center gap-1"
                      onClick={() => handleSort("size")}
                    >
                      {t("nzb.size")} {renderSortIcon("size")}
                    </button>
                  </th>
                  <th className="px-3 py-3 text-center text-base font-bold text-foreground">
                    Actions
                  </th>
                </tr>
              </thead>
              <tbody>
                {entries.map((result) => (
                  <SearchResultRow
                    key={`${result.source}-${result.title}-${result.link}`}
                    result={result}
                    currentFile={currentNames.has(releaseIdentityName(result.title))}
                    onQueue={onQueue}
                    onQueueAdditional={
                      onQueueAdditional &&
                      (!canQueueAdditional || canQueueAdditional(result))
                        ? onQueueAdditional
                        : undefined
                    }
                    disabled={disabled}
                    requireCandidateToken={requireCandidateToken}
                  />
                ))}
              </tbody>
            </table>
          </div>
        </div>
      );
    },
    [
      canQueueAdditional,
      currentNames,
      compact,
      disabled,
      handleSort,
      hideInlineSortControls,
      onQueue,
      onQueueAdditional,
      renderSortIcon,
      requireCandidateToken,
      sortKey,
      t,
    ],
  );

  const renderSelectedTitleList = React.useCallback(
    (entries: Release[]) => {
      return (
        <div>
          {entries.map((result) => (
            <SearchResultRow
              key={`${result.source}-${result.title}-${result.link}`}
              result={result}
              currentFile={currentNames.has(releaseIdentityName(result.title))}
              onQueue={onQueue}
              onQueueAdditional={
                onQueueAdditional &&
                (!canQueueAdditional || canQueueAdditional(result))
                  ? onQueueAdditional
                  : undefined
              }
              disabled={disabled}
              requireCandidateToken={requireCandidateToken}
              presentation="selected-title"
            />
          ))}
        </div>
      );
    },
    [
      canQueueAdditional,
      currentNames,
      disabled,
      onQueue,
      onQueueAdditional,
      requireCandidateToken,
    ],
  );

  if (presentation === "selected-title") {
    return (
      <div>
        {results.length === 0 ? (
          <p className="p-4 text-sm text-muted-foreground">
            {t("nzb.noConsideredResults")}
          </p>
        ) : (
          renderSelectedTitleList(sortedResults)
        )}
      </div>
    );
  }

  if (showBlockedInline) {
    return (
      <div className="space-y-3">
        {results.length === 0 ? (
          <p className="text-sm text-muted-foreground">
            {t("nzb.noConsideredResults")}
          </p>
        ) : (
          renderTable(sortedResults)
        )}
      </div>
    );
  }

  return (
    <div className="space-y-3">
      {results.length === 0 ? (
        <p className="text-sm text-muted-foreground">
          {t("nzb.noConsideredResults")}
        </p>
      ) : (
        renderTable(sortedResults)
      )}
    </div>
  );
}
