import type { MediaAnalysisAttempt, MediaAnalysisDetails, MediaDiscTitle, MediaRational, MediaStreamDetail } from "@/lib/types/media-analysis";
import { useEffect, useState } from "react";
import { DiscTitleSelection } from "./disc-title-selection";
import { DiscEpisodeMapping } from "./disc-episode-mapping";
import { MediaStructuralDiagnostics } from "./media-structural-diagnostics";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { Badge } from "@/components/ui/badge";
import { useTranslate } from "@/lib/context/translate-context";
import { discReviewInventory } from "@/lib/utils/disc-review";

function rate(value: number | string | null | undefined) {
  return value == null ? "Unknown" : `${(Number(value) / 1000).toLocaleString(undefined, { maximumFractionDigits: 1 })} kbps`;
}
function rational(value: MediaRational | null) {
  return value ? `${value.numerator}/${value.denominator}` : "Unknown";
}
function time(value: number | null) {
  return value == null ? "Unknown" : `${value.toLocaleString(undefined, { maximumFractionDigits: 3 })} s`;
}
function Stream({ stream }: { stream: MediaStreamDetail }) {
  const metadata = stream.metadata;
  const roles = Object.entries(metadata.disposition).filter(([, value]) => value === true).map(([key]) => key.replace(/([A-Z])/g, " $1").toLowerCase());
  const hdr = [["Dolby Vision", metadata.hdr.dolbyVision], ["HDR10+", metadata.hdr.hdr10plus], ["HDR10", metadata.hdr.hdr10], ["HLG", metadata.hdr.hlg], ["PQ", metadata.hdr.pq]].filter(([, value]) => value === true).map(([label]) => label);
  return <div className="space-y-1 border-t py-2">
    <p className="font-medium">{stream.kind.toLowerCase()} {metadata.id ?? "?"} · {stream.codec ?? "Unknown codec"}{stream.name ? ` · ${stream.name}` : ""}</p>
    {roles.length > 0 ? <p>{roles.join(", ")}</p> : null}
    <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-muted-foreground">
      <dt>Language</dt><dd>{stream.language ?? "Unknown"}{metadata.originalLanguage && metadata.originalLanguage !== stream.language ? ` (${metadata.originalLanguage})` : ""}</dd>
      <dt>Profile / level</dt><dd>{metadata.profile ?? "Unknown"} / {metadata.level ?? "Unknown"}</dd>
      <dt>Bitrate</dt><dd>{rate(metadata.bitrateBps)} · {metadata.bitrateProvenance.toLowerCase()}</dd>
      {metadata.estimatedBitrateBps != null ? <><dt>Estimated bitrate</dt><dd>{rate(metadata.estimatedBitrateBps)}</dd></> : null}
      {stream.kind === "VIDEO" ? <>
        <dt>Dimensions</dt><dd>{stream.width ?? "?"} × {stream.height ?? "?"}</dd>
        <dt>Pixel format / depth</dt><dd>{metadata.pixelFormat ?? "Unknown"} / {metadata.bitDepth ?? "?"}-bit</dd>
        <dt>Field order</dt><dd>{metadata.fieldOrder ?? "Unknown"}</dd>
        <dt>Aspect / rotation</dt><dd>{rational(metadata.displayAspectRatio)} / {metadata.rotationDegrees == null ? "Unknown" : `${metadata.rotationDegrees}°`}</dd>
        <dt>Declared frame rate</dt><dd>{rational(metadata.declaredFrameRate)}</dd>
        <dt>Observed frame rate</dt><dd>{rational(metadata.observedFrameRate)} · VFR {metadata.variableFrameRate == null ? "unknown" : metadata.variableFrameRate ? "observed" : "not observed"}</dd>
        <dt>HDR signals</dt><dd>{hdr.length ? hdr.join(", ") : "No confirmed HDR signal"}</dd>
        <dt>Color primaries / transfer / matrix</dt><dd>{metadata.color.primaries ?? "?"} / {metadata.color.transfer ?? "?"} / {metadata.color.matrix ?? "?"}</dd>
        {metadata.color.contentLight ? <><dt>MaxCLL / MaxFALL</dt><dd>{metadata.color.contentLight.maxCll ?? "?"} / {metadata.color.contentLight.maxFall ?? "?"}</dd></> : null}
        {metadata.color.masteringDisplay ? <><dt>Mastering luminance</dt><dd>{metadata.color.masteringDisplay.minLuminance ?? "?"}–{metadata.color.masteringDisplay.maxLuminance ?? "?"} cd/m²</dd></> : null}
        {metadata.hdr.dovi ? <><dt>Dolby Vision profile / compatibility</dt><dd>{metadata.hdr.dovi.profile ?? "?"} / {metadata.hdr.dovi.baseLayerCompatibilityId ?? "?"}</dd></> : null}
      </> : stream.kind === "AUDIO" ? <>
        <dt>Channels / layout</dt><dd>{stream.channels ?? "?"} / {metadata.channelLayout ?? "Unknown layout"}</dd>
        <dt>Sample rate / format</dt><dd>{metadata.sampleRate ?? "?"} Hz / {metadata.sampleFormat ?? "Unknown"}</dd>
        <dt>Sample depth</dt><dd>{metadata.sampleBitDepth == null ? "Unknown" : `${metadata.sampleBitDepth}-bit`}</dd>
      </> : null}
    </dl>
  </div>;
}

export function DiscTitleDetails({ title }: { title: MediaDiscTitle }) {
  const t = useTranslate();
  return <details className="border-t py-1">
    <summary className="cursor-pointer font-medium">
      {t("mediaFile.discTitleIdentity", { id: title.id })} · {title.durationSeconds == null ? t("label.unknown") : time(title.durationSeconds)} · {title.report.status.toLowerCase()}
    </summary>
    <div className="space-y-2 py-2">
      <p>{t("mediaFile.discAngleInventory", { count: title.angleCount || t("label.unknown") })}</p>
      {title.aliases.length ? <p>{t("mediaFile.discTitleAliases", { ids: title.aliases.join(", ") })}</p> : null}
      {title.report.budgetExhausted ? <p className="text-amber-500">{t("mediaFile.discTitleBudgetExhausted")}</p> : null}
      {title.report.warnings.map((warning, index) => <p key={`${warning.code}-${index}`} className="text-amber-500">{warning.message}</p>)}
      <p className="font-medium">{t("mediaFile.discTitleStreams")}</p>
      {title.streams.length ? title.streams.map((stream, index) =>
        <Stream key={`${stream.metadata.programId}-${stream.metadata.id}-${index}`} stream={stream} />)
        : <p className="text-muted-foreground">{t("mediaFile.discTitleStreamsUnknown")}</p>}
      <p className="font-medium">{t("mediaFile.discTitleChapters")}</p>
      {title.chapters.length ? <ol className="space-y-1">{title.chapters.map((chapter, index) => <li key={`${chapter.id}-${index}`}>
        {time(chapter.startSeconds)}{chapter.endSeconds == null ? "" : `–${time(chapter.endSeconds)}`} · {chapter.title ?? t("mediaFile.discChapterIdentity", { id: chapter.id })}
      </li>)}</ol> : <p className="text-muted-foreground">{t("mediaFile.discTitleChaptersUnknown")}</p>}
    </div>
  </details>;
}

export function MediaAnalysisDetailsPopover({ analysis: storedAnalysis, attempt: storedAttempt, videoBitrateKbps, fileId }: { analysis: MediaAnalysisDetails; attempt?: MediaAnalysisAttempt | null; videoBitrateKbps: number | null; fileId?: string }) {
  const t = useTranslate();
  const [analysis, setAnalysis] = useState(storedAnalysis);
  const [attempt, setAttempt] = useState(storedAttempt);
  useEffect(() => { setAnalysis(storedAnalysis); }, [storedAnalysis]);
  useEffect(() => { setAttempt(storedAttempt); }, [storedAttempt]);
  const onSelectionChanged = (next: MediaAnalysisDetails) => { setAnalysis(next); setAttempt(null); };
  const disc = discReviewInventory(analysis.disc, attempt);
  const selectedVideo = analysis.streams.find((stream) => stream.kind === "VIDEO"
    && stream.metadata.id === analysis.selectedVideoId
    && (analysis.selectedProgramId == null || stream.metadata.programId === analysis.selectedProgramId));
  const videoBitrate = analysis.revision > 0 ? selectedVideo?.metadata.bitrateBps
    : videoBitrateKbps == null ? null : videoBitrateKbps * 1000;
  return <Popover>
    <PopoverTrigger asChild><button type="button" aria-label="Media analysis details"><Badge>Details{attempt && !attempt.succeeded ? " · review" : analysis.report.status === "INCOMPLETE" ? " · incomplete" : ""}</Badge></button></PopoverTrigger>
    <PopoverContent align="start" className="max-h-[70vh] w-[min(36rem,90vw)] overflow-y-auto text-xs">
      <p className="font-medium">Media analysis · {analysis.report.status.toLowerCase()}</p>
      {attempt && !attempt.succeeded ? <div className="my-2 space-y-1 rounded border border-amber-500 p-2" role="status">
        <p>Latest inspection: {attempt.report.status.toLowerCase()} · <time dateTime={attempt.attemptedAt}>{new Date(attempt.attemptedAt).toLocaleString()}</time></p>
        <p>{analysis.revision > 0 ? "Previous successful metadata is retained below." : "No successful analysis has been saved yet."}</p>
        {attempt.report.budgetExhausted ? <p>The inspection budget was exhausted.</p> : null}
        {attempt.report.warnings.map((warning, index) => <p key={`${warning.code}-${index}`}>{warning.message}</p>)}
      </div> : null}
      <p className="my-2 text-muted-foreground">Duration {time(analysis.durationSeconds)} · Overall {rate(analysis.overallBitrateBps)} · Video {rate(videoBitrate)}</p>
      {analysis.report.budgetExhausted ? <p>Inspection stopped at its read budget.</p> : null}
      {analysis.report.warnings.map((warning, index) => <p key={`${warning.code}-${index}`} className="my-1 text-amber-500">{warning.message}</p>)}
      {disc ? <div className="my-3 space-y-1">
        <p className="font-medium">{disc.discType.toUpperCase()} · {disc.filesystem} {disc.volumeLabel ?? ""}</p>
        {disc !== analysis.disc ? <p className="text-amber-500">{t("mediaFile.discReviewInventory")}</p> : null}
        <p>{analysis.disc?.automaticSelection ? "Automatic longest-title selection" : "Saved title selection"}: {analysis.disc?.selectedTitleId ?? "Requires review"}</p>
        {fileId ? <DiscTitleSelection fileId={fileId} analysis={analysis} inventory={disc} onChanged={onSelectionChanged} requiresReview={attempt != null && !attempt.succeeded} /> : null}
        {fileId ? <DiscEpisodeMapping key={fileId} fileId={fileId} analysis={analysis} inventory={disc} onChanged={onSelectionChanged} /> : null}
        {disc.titles.map((title) => <DiscTitleDetails key={title.id} title={title} />)}
      </div> : null}
      {analysis.programs.map((program) => <p key={program.id}>Program {program.id}{program.name ? ` · ${program.name}` : ""} · streams {program.streamIds.join(", ")}{analysis.selectedProgramId === program.id ? " · selected" : ""}</p>)}
      {analysis.streams.map((stream, index) => <Stream key={`${stream.metadata.programId}-${stream.metadata.id}-${index}`} stream={stream} />)}
      {analysis.chapters.length > 0 ? <div className="border-t pt-2"><p className="font-medium">Chapters</p>{analysis.chapters.map((chapter) => <p key={chapter.id}>{time(chapter.startSeconds)} · {chapter.title ?? `Chapter ${chapter.id}`}</p>)}</div> : null}
      {analysis.attachments.length > 0 ? <div className="border-t pt-2"><p className="font-medium">Attachments</p>{analysis.attachments.map((attachment) => <p key={attachment.id}>{attachment.name ?? attachment.id} · {attachment.mediaType ?? "Unknown type"} · {Number(attachment.sizeBytes).toLocaleString()} bytes</p>)}</div> : null}
      {analysis.captionServices.map((caption) => <p key={`${caption.streamId}-${caption.standard}-${caption.serviceNumber}`}>{caption.standard} service {caption.serviceNumber ?? "?"} · {caption.language ?? "Unknown language"}</p>)}
      {fileId ? <MediaStructuralDiagnostics key={fileId} fileId={fileId} /> : null}
    </PopoverContent>
  </Popover>;
}
