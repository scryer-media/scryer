import type { MediaAnalysisAttempt, MediaAnalysisDetails, MediaDiscTitle, MediaRational, MediaStreamDetail } from "@/lib/types/media-analysis";
import { MediaStructuralDiagnostics } from "./media-structural-diagnostics";
import { useTranslate } from "@/lib/context/translate-context";

type Translate = (key: string, values?: Record<string, string | number>) => string;

function rate(value: number | string | null | undefined, t: Translate) {
  return value == null ? t("label.unknown") : `${(Number(value) / 1000).toLocaleString(undefined, { maximumFractionDigits: 1 })} kbps`;
}
function rational(value: MediaRational | null, t: Translate) {
  return value ? `${value.numerator}/${value.denominator}` : t("label.unknown");
}
function time(value: number | null, t: Translate) {
  return value == null ? t("label.unknown") : `${value.toLocaleString(undefined, { maximumFractionDigits: 3 })} s`;
}

/** Every recovered fact about one stream, as a label/value grid. */
function Stream({ stream }: { stream: MediaStreamDetail }) {
  const t = useTranslate();
  const metadata = stream.metadata;
  const roles = Object.entries(metadata.disposition).filter(([, value]) => value === true).map(([key]) => key.replace(/([A-Z])/g, " $1").toLowerCase());
  const hdr = [["Dolby Vision", metadata.hdr.dolbyVision], ["HDR10+", metadata.hdr.hdr10plus], ["HDR10", metadata.hdr.hdr10], ["HLG", metadata.hdr.hlg], ["PQ", metadata.hdr.pq]].filter(([, value]) => value === true).map(([label]) => label);
  const unknown = t("label.unknown");
  return <div className="space-y-1 border-t py-2">
    <p className="font-medium">{stream.kind.toLowerCase()} {metadata.id ?? "?"} · {stream.codec ?? t("mediaFile.streamUnknownCodec")}{stream.name ? ` · ${stream.name}` : ""}</p>
    {roles.length > 0 ? <p>{roles.join(", ")}</p> : null}
    <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-muted-foreground">
      <dt>{t("mediaFile.streamLanguage")}</dt><dd>{stream.language ?? unknown}{metadata.originalLanguage && metadata.originalLanguage !== stream.language ? ` (${metadata.originalLanguage})` : ""}</dd>
      <dt>{t("mediaFile.streamProfileLevel")}</dt><dd>{metadata.profile ?? unknown} / {metadata.level ?? unknown}</dd>
      <dt>{t("mediaFile.streamBitrate")}</dt><dd>{rate(metadata.bitrateBps, t)} · {metadata.bitrateProvenance.toLowerCase()}</dd>
      {metadata.estimatedBitrateBps != null ? <><dt>{t("mediaFile.streamEstimatedBitrate")}</dt><dd>{rate(metadata.estimatedBitrateBps, t)}</dd></> : null}
      {stream.kind === "VIDEO" ? <>
        <dt>{t("mediaFile.streamDimensions")}</dt><dd>{stream.width ?? "?"} × {stream.height ?? "?"}</dd>
        <dt>{t("mediaFile.streamPixelFormat")}</dt><dd>{metadata.pixelFormat ?? unknown} / {metadata.bitDepth ?? "?"}-bit</dd>
        <dt>{t("mediaFile.streamFieldOrder")}</dt><dd>{metadata.fieldOrder ?? unknown}</dd>
        <dt>{t("mediaFile.streamAspectRotation")}</dt><dd>{rational(metadata.displayAspectRatio, t)} / {metadata.rotationDegrees == null ? unknown : `${metadata.rotationDegrees}°`}</dd>
        <dt>{t("mediaFile.streamDeclaredFrameRate")}</dt><dd>{rational(metadata.declaredFrameRate, t)}</dd>
        <dt>{t("mediaFile.streamObservedFrameRate")}</dt><dd>{rational(metadata.observedFrameRate, t)} · {t(metadata.variableFrameRate == null ? "mediaFile.streamVfrUnknown" : metadata.variableFrameRate ? "mediaFile.streamVfrObserved" : "mediaFile.streamVfrNotObserved")}</dd>
        <dt>{t("mediaFile.streamHdr")}</dt><dd>{hdr.length ? hdr.join(", ") : t("mediaFile.streamNoHdr")}</dd>
        <dt>{t("mediaFile.streamColor")}</dt><dd>{metadata.color.primaries ?? "?"} / {metadata.color.transfer ?? "?"} / {metadata.color.matrix ?? "?"}</dd>
        {metadata.color.contentLight ? <><dt>{t("mediaFile.streamContentLight")}</dt><dd>{metadata.color.contentLight.maxCll ?? "?"} / {metadata.color.contentLight.maxFall ?? "?"}</dd></> : null}
        {metadata.color.masteringDisplay ? <><dt>{t("mediaFile.streamMasteringLuminance")}</dt><dd>{metadata.color.masteringDisplay.minLuminance ?? "?"}–{metadata.color.masteringDisplay.maxLuminance ?? "?"} cd/m²</dd></> : null}
        {metadata.hdr.dovi ? <><dt>{t("mediaFile.streamDolbyVision")}</dt><dd>{metadata.hdr.dovi.profile ?? "?"} / {metadata.hdr.dovi.baseLayerCompatibilityId ?? "?"}</dd></> : null}
      </> : stream.kind === "AUDIO" ? <>
        <dt>{t("mediaFile.streamChannels")}</dt><dd>{stream.channels ?? "?"} / {metadata.channelLayout ?? t("mediaFile.streamUnknownLayout")}</dd>
        <dt>{t("mediaFile.streamSampleRate")}</dt><dd>{metadata.sampleRate ?? "?"} Hz / {metadata.sampleFormat ?? unknown}</dd>
        <dt>{t("mediaFile.streamSampleDepth")}</dt><dd>{metadata.sampleBitDepth == null ? unknown : `${metadata.sampleBitDepth}-bit`}</dd>
      </> : null}
    </dl>
  </div>;
}

/** One authored title's angles, warnings, streams, and chapters. */
export function DiscTitleDetailBody({ title }: { title: MediaDiscTitle }) {
  const t = useTranslate();
  return <div className="space-y-2 py-2">
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
      {time(chapter.startSeconds, t)}{chapter.endSeconds == null ? "" : `–${time(chapter.endSeconds, t)}`} · {chapter.title ?? t("mediaFile.discChapterIdentity", { id: chapter.id })}
    </li>)}</ol> : <p className="text-muted-foreground">{t("mediaFile.discTitleChaptersUnknown")}</p>}
  </div>;
}

/** A collapsed title: identity, duration, and status on the summary line. */
export function DiscTitleDetails({ title }: { title: MediaDiscTitle }) {
  const t = useTranslate();
  return <details className="border-t py-1">
    <summary className="cursor-pointer font-medium">
      {t("mediaFile.discTitleIdentity", { id: title.id })} · {title.durationSeconds == null ? t("label.unknown") : time(title.durationSeconds, t)} · {title.report.status.toLowerCase()}
    </summary>
    <DiscTitleDetailBody title={title} />
  </details>;
}

/**
 * The whole-file inspection record: report, every stream, chapters,
 * attachments, caption services, and the on-demand structural diagnostics.
 * Reference material, so it sits behind a disclosure wherever it is shown.
 */
export function MediaAnalysisTechnicalDetails({ analysis, attempt, videoBitrateKbps, fileId }: {
  analysis: MediaAnalysisDetails;
  attempt?: MediaAnalysisAttempt | null;
  videoBitrateKbps: number | null;
  fileId?: string;
}) {
  const t = useTranslate();
  const selectedVideo = analysis.streams.find((stream) => stream.kind === "VIDEO"
    && stream.metadata.id === analysis.selectedVideoId
    && (analysis.selectedProgramId == null || stream.metadata.programId === analysis.selectedProgramId));
  const videoBitrate = analysis.revision > 0 ? selectedVideo?.metadata.bitrateBps
    : videoBitrateKbps == null ? null : videoBitrateKbps * 1000;
  return <div className="space-y-2 text-xs">
    <p className="font-medium">{t("mediaFile.analysisHeading", { status: analysis.report.status.toLowerCase() })}</p>
    {attempt && !attempt.succeeded ? <div className="space-y-1 rounded border border-amber-500 p-2" role="status">
      <p>{t("mediaFile.analysisLatestInspection", { status: attempt.report.status.toLowerCase(), time: new Date(attempt.attemptedAt).toLocaleString() })}</p>
      <p>{t(analysis.revision > 0 ? "mediaFile.analysisPreviousRetained" : "mediaFile.analysisNoneSaved")}</p>
      {attempt.report.budgetExhausted ? <p>{t("mediaFile.analysisBudgetExhausted")}</p> : null}
      {attempt.report.warnings.map((warning, index) => <p key={`${warning.code}-${index}`}>{warning.message}</p>)}
    </div> : null}
    <p className="text-muted-foreground">{t("mediaFile.analysisSummary", { duration: time(analysis.durationSeconds, t), overall: rate(analysis.overallBitrateBps, t), video: rate(videoBitrate, t) })}</p>
    {analysis.report.budgetExhausted ? <p>{t("mediaFile.analysisBudgetStopped")}</p> : null}
    {analysis.report.warnings.map((warning, index) => <p key={`${warning.code}-${index}`} className="text-amber-500">{warning.message}</p>)}
    {analysis.programs.map((program) => <p key={program.id}>{t("mediaFile.analysisProgram", { id: program.id })}{program.name ? ` · ${program.name}` : ""} · {t("mediaFile.analysisProgramStreams", { ids: program.streamIds.join(", ") })}{analysis.selectedProgramId === program.id ? ` · ${t("mediaFile.analysisSelected")}` : ""}</p>)}
    {analysis.streams.map((stream, index) => <Stream key={`${stream.metadata.programId}-${stream.metadata.id}-${index}`} stream={stream} />)}
    {analysis.chapters.length > 0 ? <div className="border-t pt-2"><p className="font-medium">{t("mediaFile.analysisChapters")}</p>{analysis.chapters.map((chapter) => <p key={chapter.id}>{time(chapter.startSeconds, t)} · {chapter.title ?? t("mediaFile.discChapterIdentity", { id: chapter.id })}</p>)}</div> : null}
    {analysis.attachments.length > 0 ? <div className="border-t pt-2"><p className="font-medium">{t("mediaFile.analysisAttachments")}</p>{analysis.attachments.map((attachment) => <p key={attachment.id}>{attachment.name ?? attachment.id} · {attachment.mediaType ?? t("mediaFile.analysisUnknownType")} · {t("mediaFile.analysisBytes", { count: Number(attachment.sizeBytes).toLocaleString() })}</p>)}</div> : null}
    {analysis.captionServices.map((caption) => <p key={`${caption.streamId}-${caption.standard}-${caption.serviceNumber}`}>{t("mediaFile.analysisCaptionService", { standard: caption.standard, number: caption.serviceNumber ?? "?", language: caption.language ?? t("mediaFile.analysisUnknownLanguage") })}</p>)}
    {fileId ? <MediaStructuralDiagnostics key={fileId} fileId={fileId} /> : null}
  </div>;
}
