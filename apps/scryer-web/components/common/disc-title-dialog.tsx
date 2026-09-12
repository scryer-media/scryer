import * as React from "react";
import { useClient } from "urql";
import { Disc3, Loader2, TriangleAlert } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { IconButton } from "@/components/ui/icon-button";
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group";
import {
  DiscTitleDetailBody,
  MediaAnalysisTechnicalDetails,
} from "@/components/common/media-analysis-details";
import { useTranslate } from "@/lib/context/translate-context";
import { TITLE_MEDIA_FILE_FIELDS } from "@/lib/graphql/queries";
import type {
  MediaAnalysisAttempt,
  MediaAnalysisDetails,
  MediaDiscTitle,
} from "@/lib/types/media-analysis";
import type { TitleMediaFileRecord } from "@/lib/types/titles";
import {
  discEpisodeSelections,
  discNeedsReview,
  discOutcome,
  discReviewInventory,
  discTitleAudioCount,
  discTitleIdentity,
  discTitleSelectable,
  discTitleVideoSummary,
  formatDiscDuration,
} from "@/lib/utils/disc-review";
import { cn } from "@/lib/utils";

const AUTOMATIC = "__automatic__";

const selectDiscTitleMutation = `
  mutation SelectMediaFileDiscTitle($fileId: ID!, $discTitleId: String) {
    selectMediaFileDiscTitle(fileId: $fileId, discTitleId: $discTitleId) {
      ${TITLE_MEDIA_FILE_FIELDS}
    }
  }
`;
const episodeTargetsMutation = `mutation DiscEpisodeTargets($fileId: ID!) {
  mediaFileDiscEpisodeTargets(fileId: $fileId) { episodeId label durationSeconds }
}`;
const mapEpisodesMutation = `mutation MapDiscEpisodes($fileId: ID!, $mappings: [MediaDiscEpisodeMappingInput!]!) {
  mapMediaFileDiscEpisodes(fileId: $fileId, mappings: $mappings) { ${TITLE_MEDIA_FILE_FIELDS} }
}`;
const REFRESHED_TYPENAMES = ["TitlePayload", "EpisodePayload", "CollectionPayload", "EpisodeMediaAvailabilityPayload"];

type EpisodeTarget = { episodeId: string; label: string; durationSeconds: number | string | null };

export type DiscTitleFile = {
  id: string;
  filePath?: string;
  analysis: MediaAnalysisDetails;
  analysisAttempt?: MediaAnalysisAttempt | null;
  videoBitrateKbps: number | null;
};

/**
 * The file row's call to action for a disc image: neutral when the disc plays
 * a title Scryer chose on its own, warning-toned when a person has to step in.
 * Renders nothing for a file that is not a disc image.
 */
export function DiscTitleButton({ file, id, className }: { file: DiscTitleFile; id?: string; className?: string }) {
  const t = useTranslate();
  const [open, setOpen] = React.useState(false);
  const outcome = discOutcome(file.analysis, file.analysisAttempt);
  if (!outcome) return null;
  const review = discNeedsReview(outcome);
  return <>
    <IconButton
      id={id}
      label={t(review ? "mediaFile.discReviewAction" : "mediaFile.discTitle")}
      tone={review ? "upgrade" : "neutral"}
      className={className}
      onClick={() => setOpen(true)}
    >
      <Disc3 className="h-4 w-4" />
    </IconButton>
    {open ? <DiscTitleDialog open onOpenChange={setOpen} file={file} /> : null}
  </>;
}

/**
 * One place to see what a disc image plays and to change it.
 *
 * The header states the outcome in words. A movie disc offers the authored
 * titles as a single choice, with "automatic" first; an episodic disc offers
 * one episode picker per authored title instead, because the disc holds several
 * episodes and nothing on it says which title is which. Per-title streams and
 * the whole-file inspection record stay reachable behind disclosures.
 */
export function DiscTitleDialog({ open, onOpenChange, file }: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  file: DiscTitleFile;
}) {
  const client = useClient();
  const t = useTranslate();
  const [analysis, setAnalysis] = React.useState(file.analysis);
  const [attempt, setAttempt] = React.useState(file.analysisAttempt ?? null);
  React.useEffect(() => { setAnalysis(file.analysis); }, [file.analysis]);
  React.useEffect(() => { setAttempt(file.analysisAttempt ?? null); }, [file.analysisAttempt]);

  const inventory = discReviewInventory(analysis.disc, attempt);
  const outcome = discOutcome(analysis, attempt);
  const review = discNeedsReview(outcome);
  const savedTitle = discTitleIdentity(analysis.disc?.selection.titleId ?? "", inventory);
  const [selected, setSelected] = React.useState(savedTitle || AUTOMATIC);
  React.useEffect(() => { setSelected(savedTitle || AUTOMATIC); }, [savedTitle]);

  const savedMappings = React.useMemo(() => discEpisodeSelections(analysis.disc, inventory), [analysis.disc, inventory]);
  const [mappings, setMappings] = React.useState<Record<string, string>>(savedMappings);
  React.useEffect(() => { setMappings(savedMappings); }, [savedMappings]);
  const [targets, setTargets] = React.useState<EpisodeTarget[] | null>(null);
  const [busy, setBusy] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const requestVersion = React.useRef(0);

  // Which kind of disc this is comes from the title it belongs to: an episodic
  // title has episode targets, a movie has none. Asked once per open.
  React.useEffect(() => {
    let active = true;
    const version = ++requestVersion.current;
    setTargets(null);
    void client.mutation<{ mediaFileDiscEpisodeTargets: EpisodeTarget[] }>(episodeTargetsMutation, { fileId: file.id })
      .toPromise()
      .then((result) => {
        if (!active || version !== requestVersion.current) return;
        if (result.error) throw result.error;
        setTargets(result.data?.mediaFileDiscEpisodeTargets ?? []);
      })
      .catch((cause: unknown) => {
        if (!active || version !== requestVersion.current) return;
        setError(cause instanceof Error ? cause.message : t("mediaFile.discMappingFailed"));
        setTargets([]);
      });
    return () => { active = false; };
  }, [client, file.id, t]);

  const onChanged = (next: MediaAnalysisDetails) => { setAnalysis(next); setAttempt(null); };

  async function saveTitle() {
    const version = ++requestVersion.current;
    setBusy(true);
    setError(null);
    try {
      const result = await client.mutation<{ selectMediaFileDiscTitle: TitleMediaFileRecord }>(
        selectDiscTitleMutation,
        { fileId: file.id, discTitleId: selected === AUTOMATIC ? null : selected },
        { additionalTypenames: REFRESHED_TYPENAMES },
      ).toPromise();
      if (version !== requestVersion.current) return;
      if (result.error) throw result.error;
      const next = result.data?.selectMediaFileDiscTitle.analysis;
      if (!next) throw new Error(t("mediaFile.discSelectionFailed"));
      onChanged(next);
    } catch (cause) {
      if (version !== requestVersion.current) return;
      setError(cause instanceof Error ? cause.message : t("mediaFile.discSelectionFailed"));
    } finally {
      if (version === requestVersion.current) setBusy(false);
    }
  }

  async function saveMappings() {
    const version = ++requestVersion.current;
    setBusy(true);
    setError(null);
    try {
      const result = await client.mutation<{ mapMediaFileDiscEpisodes: TitleMediaFileRecord }>(
        mapEpisodesMutation,
        {
          fileId: file.id,
          mappings: Object.entries(mappings)
            .filter(([, episodeId]) => episodeId)
            .map(([discTitleId, episodeId]) => ({ discTitleId, episodeId })),
        },
        { additionalTypenames: REFRESHED_TYPENAMES },
      ).toPromise();
      if (version !== requestVersion.current) return;
      if (result.error) throw result.error;
      const next = result.data?.mapMediaFileDiscEpisodes.analysis;
      if (!next) throw new Error(t("mediaFile.discMappingFailed"));
      onChanged(next);
    } catch (cause) {
      if (version !== requestVersion.current) return;
      setError(cause instanceof Error ? cause.message : t("mediaFile.discMappingFailed"));
    } finally {
      if (version === requestVersion.current) setBusy(false);
    }
  }

  if (!inventory) return null;
  const mode: "loading" | "movie" | "episodes" = targets == null ? "loading" : targets.length > 0 ? "episodes" : "movie";
  const titleById = new Map(inventory.titles.map((title) => [title.id, title]));
  const savedTitleMissing = savedTitle !== "" && !titleById.has(savedTitle);
  const titleChanged = (selected === AUTOMATIC ? "" : selected) !== savedTitle;
  const mappingsChanged = JSON.stringify(normalizeMappings(mappings)) !== JSON.stringify(normalizeMappings(savedMappings));
  const fileName = file.filePath?.split("/").pop() ?? null;
  const playing = outcome && (outcome.kind === "automatic" || outcome.kind === "manual") ? titleById.get(outcome.titleId) ?? null : null;

  return <Dialog open={open} onOpenChange={onOpenChange}>
    <DialogContent id="disc-title-dialog" className="sm:max-w-3xl">
      <DialogHeader>
        <DialogTitle>{t("mediaFile.discTitle")}</DialogTitle>
        <DialogDescription className="break-all">
          {[discTypeLabel(inventory.discType), inventory.filesystem, inventory.volumeLabel, fileName].filter(Boolean).join(" · ")}
        </DialogDescription>
      </DialogHeader>

      <div className="max-h-[65vh] space-y-4 overflow-y-auto pr-1 text-sm">
        <DiscOutcomeStatement outcome={outcome} playing={playing} t={t} />
        {inventory !== analysis.disc ? (
          <p id="disc-title-inventory-note" className="text-xs text-[var(--scry-warning-text)]">{t("mediaFile.discReviewInventory")}</p>
        ) : null}

        {mode === "loading" ? (
          <p className="flex items-center gap-2 text-muted-foreground"><Loader2 className="h-4 w-4 animate-spin" />{t("mediaFile.discMappingLoading")}</p>
        ) : mode === "movie" ? (
          <section className="space-y-2">
            <h3 className="text-xs font-medium text-muted-foreground">{t("mediaFile.discChooseTitle")}</h3>
            <RadioGroup value={selected} onValueChange={setSelected} disabled={busy} className="gap-2">
              <TitleChoice value={AUTOMATIC} checked={selected === AUTOMATIC} heading={t("mediaFile.discAutomatic")}
                summary={analysis.disc?.automaticSelection && playing ? t("mediaFile.discAutomaticCurrent", { id: playing.id }) : null} />
              {savedTitleMissing ? (
                <TitleChoice value={savedTitle} checked={selected === savedTitle} disabled
                  heading={t("mediaFile.discTitleIdentity", { id: savedTitle })} summary={t("mediaFile.discTitleMissing")} />
              ) : null}
              {inventory.titles.map((title) => (
                <TitleChoice key={title.id} value={title.id} checked={selected === title.id} disabled={!discTitleSelectable(title)}
                  heading={t("mediaFile.discTitleIdentity", { id: title.id })}
                  summary={titleSummary(title, t)}
                  badges={<TitleBadges title={title} playingId={playing?.id ?? null} t={t} />}
                  detail={title} />
              ))}
            </RadioGroup>
          </section>
        ) : (
          <section className="space-y-2">
            <h3 className="text-xs font-medium text-muted-foreground">{t("mediaFile.discEpisodeMapping")}</h3>
            <p className="text-xs text-muted-foreground">{t("mediaFile.discMappingScope")}</p>
            <ul className="space-y-2">
              {[...new Set([...inventory.titles.map((title) => title.id), ...Object.keys(mappings)])].map((id) => {
                const title = titleById.get(id) ?? null;
                const episodeId = mappings[id] ?? "";
                const selectable = title ? discTitleSelectable(title) : false;
                return <li key={id} id={`disc-title-mapping-${id}`} className="rounded-lg border border-border px-3 py-2">
                  <div className="flex flex-wrap items-start justify-between gap-2">
                    <div className="min-w-0 space-y-1">
                      <div className="flex flex-wrap items-center gap-2">
                        <span className="font-medium">{t("mediaFile.discTitleIdentity", { id })}</span>
                        {title ? <TitleBadges title={title} playingId={null} t={t} /> : <Badge tone="warning">{t("mediaFile.discTitleMissing")}</Badge>}
                      </div>
                      {title ? <p className="text-xs text-muted-foreground">{titleSummary(title, t)}</p> : null}
                    </div>
                    <select
                      aria-label={t("mediaFile.discTitleIdentity", { id })}
                      className="h-9 min-w-48 rounded-md border border-border bg-background px-2 text-sm"
                      value={episodeId}
                      disabled={busy || (!selectable && !episodeId)}
                      onChange={(event) => setMappings((previous) => ({ ...previous, [id]: event.target.value }))}
                    >
                      <option value="">{t("mediaFile.discMappingNone")}</option>
                      {episodeId && !targets?.some((target) => target.episodeId === episodeId) ? <option value={episodeId} disabled>{episodeId}</option> : null}
                      {targets?.map((target) => <option key={target.episodeId} value={target.episodeId}
                        disabled={Object.entries(mappings).some(([key, value]) => key !== id && value === target.episodeId)}>{target.label}</option>)}
                    </select>
                  </div>
                  {title ? <details className="mt-1 text-xs">
                    <summary className="cursor-pointer text-muted-foreground">{t("mediaFile.discStreamsAndChapters")}</summary>
                    <DiscTitleDetailBody title={title} />
                  </details> : null}
                </li>;
              })}
            </ul>
          </section>
        )}

        {error ? <p role="alert" className="text-sm text-[var(--scry-danger-text)]">{error}</p> : null}

        <details id="disc-title-technical" className="rounded-lg border border-border px-3 py-2">
          <summary className="cursor-pointer text-xs font-medium text-muted-foreground">{t("mediaFile.discTechnicalDetails")}</summary>
          <div className="mt-2">
            <MediaAnalysisTechnicalDetails analysis={analysis} attempt={attempt} videoBitrateKbps={file.videoBitrateKbps} fileId={file.id} />
          </div>
        </details>
      </div>

      <DialogFooter>
        <Button id="disc-title-dismiss" type="button" variant="outline" disabled={busy} onClick={() => onOpenChange(false)}>
          {t("label.close")}
        </Button>
        {mode === "movie" ? (
          <Button id="disc-title-save" type="button" variant="primary" disabled={busy || (!titleChanged && !review)} onClick={() => { void saveTitle(); }}>
            {busy ? <Loader2 className="mr-1 h-4 w-4 animate-spin" /> : null}
            {t(busy ? "mediaFile.discSaving" : "mediaFile.discUseTitle")}
          </Button>
        ) : mode === "episodes" ? (
          <Button id="disc-title-save" type="button" variant="primary" disabled={busy || !mappingsChanged} onClick={() => { void saveMappings(); }}>
            {busy ? <Loader2 className="mr-1 h-4 w-4 animate-spin" /> : null}
            {t(busy ? "mediaFile.discSaving" : "mediaFile.discMappingSave")}
          </Button>
        ) : null}
      </DialogFooter>
    </DialogContent>
  </Dialog>;
}

type Translate = (key: string, values?: Record<string, string | number>) => string;

const DISC_TYPE_LABELS: Record<string, string> = { dvd: "DVD", bluray: "Blu-ray", uhd_bluray: "UHD Blu-ray" };
function discTypeLabel(discType: string) {
  return DISC_TYPE_LABELS[discType] ?? discType;
}

function normalizeMappings(mappings: Record<string, string>) {
  return Object.entries(mappings).filter(([, episodeId]) => episodeId).sort(([a], [b]) => a.localeCompare(b));
}

function titleSummary(title: MediaDiscTitle, t: Translate) {
  const audio = discTitleAudioCount(title);
  return [
    formatDiscDuration(title.durationSeconds) ?? t("label.unknown"),
    title.chapters.length ? t("mediaFile.discTitleChapterCount", { count: title.chapters.length }) : null,
    discTitleVideoSummary(title),
    audio ? t("mediaFile.discTitleAudioCount", { count: audio }) : null,
  ].filter(Boolean).join(" · ");
}

/** The one sentence a person needs before anything else: what plays, and why or why not. */
function DiscOutcomeStatement({ outcome, playing, t }: { outcome: ReturnType<typeof discOutcome>; playing: MediaDiscTitle | null; t: Translate }) {
  if (!outcome) return null;
  const review = discNeedsReview(outcome);
  const duration = playing ? formatDiscDuration(playing.durationSeconds) ?? t("label.unknown") : t("label.unknown");
  const text = outcome.kind === "automatic" ? t("mediaFile.discOutcomeAutomatic", { id: outcome.titleId, duration })
    : outcome.kind === "manual" ? t("mediaFile.discOutcomeManual", { id: outcome.titleId, duration })
    : outcome.kind === "unselected" ? t("mediaFile.discOutcomeUnselected")
    : t("mediaFile.discOutcomeIncomplete", { status: outcome.status.toLowerCase() });
  return <p
    id="disc-title-outcome"
    className={cn(
      "flex items-start gap-2 rounded-lg border px-3 py-3",
      review
        ? "border-[var(--scry-warning-border)] bg-[var(--scry-warning-bg)] text-[var(--scry-warning-text)]"
        : "border-border bg-muted/20 text-foreground",
    )}
  >
    {review ? <TriangleAlert className="mt-0.5 h-4 w-4 shrink-0" /> : <Disc3 className="mt-0.5 h-4 w-4 shrink-0 text-muted-foreground" />}
    <span>{text}</span>
  </p>;
}

function TitleBadges({ title, playingId, t }: { title: MediaDiscTitle; playingId: string | null; t: Translate }) {
  return <>
    {title.id === playingId ? <Badge tone="info">{t("mediaFile.discPlaying")}</Badge> : null}
    {discTitleSelectable(title) ? null : <Badge tone="warning">{t("mediaFile.discTitleNotSelectable", { status: title.report.status.toLowerCase() })}</Badge>}
  </>;
}

/** One selectable row: the radio, a heading, a one-line summary, and an optional disclosure. */
function TitleChoice({ value, checked, disabled = false, heading, summary, badges, detail }: {
  value: string;
  checked: boolean;
  disabled?: boolean;
  heading: string;
  summary: string | null;
  badges?: React.ReactNode;
  detail?: MediaDiscTitle;
}) {
  const t = useTranslate();
  const inputId = `disc-title-choice-${value}`;
  return <div
    className={cn(
      "flex items-start gap-3 rounded-lg border px-3 py-2",
      checked ? "border-[var(--scry-accent-ring)]" : "border-border",
      disabled ? "opacity-60" : null,
    )}
  >
    <RadioGroupItem id={inputId} value={value} disabled={disabled} className="mt-1" />
    <div className="min-w-0 flex-1 space-y-1">
      <label htmlFor={inputId} className={cn("flex flex-wrap items-center gap-2", disabled ? null : "cursor-pointer")}>
        <span className="font-medium">{heading}</span>
        {badges}
      </label>
      {summary ? <p className="text-xs text-muted-foreground">{summary}</p> : null}
      {detail ? <details className="text-xs">
        <summary className="cursor-pointer text-muted-foreground">{t("mediaFile.discStreamsAndChapters")}</summary>
        <DiscTitleDetailBody title={detail} />
      </details> : null}
    </div>
  </div>;
}
