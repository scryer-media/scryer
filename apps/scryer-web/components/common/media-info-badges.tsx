import { useTranslate } from "@/lib/context/translate-context";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { Badge as UiBadge } from "@/components/ui/badge";
import { ChevronDown } from "lucide-react";
import { audioFormatPills, hdrFormatPills } from "@/lib/utils/media-format-pills";
import {
  audioStreamsForFile,
  formatLanguage,
  formatSingleAudioTrack,
  formatSingleSubtitleTrack,
  resolveAudioChannels,
  resolveAudioCodec,
  resolveContainerFormat,
  resolveResolution,
  resolveSourceType,
  resolveSubtitleCodec,
  resolveVideoCodec,
  type AudioStreamDetail,
  type MediaInfoFile,
  type SubtitleStreamDetail,
} from "@/lib/utils/media-info-format";

export type { AudioStreamDetail, MediaInfoFile, SubtitleStreamDetail };

function Badge({
  children,
  tone = "info",
}: {
  children: React.ReactNode;
  tone?: "info" | "warning" | "negative";
}) {
  return (
    <UiBadge tone={tone} className="px-1.5 text-[11px]">
      {children}
    </UiBadge>
  );
}

function AudioRoles({ stream }: { stream: AudioStreamDetail }) {
  const t = useTranslate();
  const metadata = stream.metadata;
  return <>
    {metadata?.disposition.commentary ? <span>{t("mediaFile.commentary")}</span> : null}
    {metadata?.disposition.visualImpaired ? <span>{t("mediaFile.audioDescription")}</span> : null}
    {metadata?.disposition.hearingImpaired ? <span>{t("mediaFile.hearingImpaired")}</span> : null}
    {metadata?.programId != null ? <span>{t("mediaFile.program", { id: metadata.programId })}</span> : null}
  </>;
}

export function AudioTracksPopover({
  streams,
  presentation = "default",
}: {
  streams: AudioStreamDetail[];
  presentation?: "default" | "selected-title";
}) {
  const t = useTranslate();
  if (streams.length === 1 && presentation === "default") {
    return <Badge tone="info">{formatSingleAudioTrack(streams[0])}<AudioRoles stream={streams[0]} /></Badge>;
  }
  return (
    <Popover>
      <PopoverTrigger asChild>
        <button
          type="button"
          className={
            presentation === "selected-title"
              ? "inline-flex cursor-pointer items-center gap-1 rounded-[6px] bg-[var(--scry-chip)] px-[9px] py-[3px] text-[10.5px] font-semibold text-[var(--scry-muted2)] hover:bg-[var(--scry-hover)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--scry-focus)]"
              : "inline-flex cursor-pointer items-center gap-1 rounded border border-[var(--scry-info-border)] bg-[var(--scry-info-bg)] px-1.5 py-0.5 text-[11px] font-medium text-[var(--scry-info-text)] hover:bg-[var(--scry-info-bg-strong)]"
          }
        >
          {t("mediaFile.audioCount", { count: streams.length })}
          <ChevronDown className="h-3 w-3 opacity-70" />
        </button>
      </PopoverTrigger>
      <PopoverContent className="w-auto max-w-xs p-2" align="start">
        <div className="max-h-60 space-y-1 overflow-y-auto">
          {streams.map((stream, i) => (
            <div key={i} className="flex flex-wrap items-center gap-2 rounded px-2 py-1 text-xs even:bg-muted/50">
              <span className="min-w-[5rem] font-medium">{formatLanguage(stream.language)}</span>
              <span className="text-muted-foreground">{resolveAudioCodec(stream.codec) ?? "?"} {stream.profile}</span>
              <span className="text-muted-foreground">{resolveAudioChannels(stream.channels, stream.metadata?.channelLayout) ?? "?"}</span>
              <AudioRoles stream={stream} />
              {stream.name ? <span className="text-muted-foreground">{stream.name}</span> : null}
              {stream.bitrateKbps ? (
                <span className="text-muted-foreground/60">{stream.bitrateKbps} kbps</span>
              ) : null}
            </div>
          ))}
        </div>
      </PopoverContent>
    </Popover>
  );
}

export function SubtitleTracksPopover({
  streams,
  presentation = "default",
}: {
  streams: SubtitleStreamDetail[];
  presentation?: "default" | "selected-title";
}) {
  const t = useTranslate();
  if (streams.length === 1 && presentation === "default") {
    return <Badge tone="info">{formatSingleSubtitleTrack(streams[0])}</Badge>;
  }
  return (
    <Popover>
      <PopoverTrigger asChild>
        <button
          type="button"
          className={
            presentation === "selected-title"
              ? "inline-flex cursor-pointer items-center gap-1 rounded-[6px] bg-[var(--scry-chip)] px-[9px] py-[3px] text-[10.5px] font-semibold text-[var(--scry-muted2)] hover:bg-[var(--scry-hover)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--scry-focus)]"
              : "inline-flex cursor-pointer items-center gap-1 rounded border border-border bg-muted/50 px-1.5 py-0.5 text-[11px] font-medium text-muted-foreground hover:bg-muted dark:hover:bg-muted/80"
          }
        >
          {t("mediaFile.subtitleCount", { count: streams.length })}
          <ChevronDown className="h-3 w-3 opacity-70" />
        </button>
      </PopoverTrigger>
      <PopoverContent className="w-auto max-w-md p-2" align="start">
        <div className="max-h-60 space-y-1 overflow-y-auto">
          {streams.map((track, i) => (
            <div key={i} className="flex items-center gap-2 rounded px-2 py-1 text-xs even:bg-muted/50">
              <span className="min-w-[5rem] font-medium">{formatLanguage(track.language)}</span>
              <span className="text-muted-foreground">{resolveSubtitleCodec(track.codec)}</span>
              {track.forced ? (
                <span className="text-muted-foreground/60">[Forced]</span>
              ) : null}
              {track.default ? (
                <span className="text-muted-foreground/60">[Default]</span>
              ) : null}
              {track.name ? (
                <span className="truncate text-muted-foreground/60">{track.name}</span>
              ) : null}
            </div>
          ))}
        </div>
      </PopoverContent>
    </Popover>
  );
}

export function MediaInfoBadges({
  file,
  includeContainer = false,
}: {
  file: MediaInfoFile;
  includeContainer?: boolean;
}) {
  const t = useTranslate();

  const resolution = resolveResolution(file.videoWidth, file.videoHeight);
  const videoCodec = resolveVideoCodec(file.videoCodec);
  const containerFormat = includeContainer ? resolveContainerFormat(file.containerFormat) : null;

  const sourceType = file.sourceType ? resolveSourceType(file.sourceType) : null;
  const hasContainer = containerFormat != null;
  const hdrPills = hdrFormatPills(file);
  const audioPills = audioFormatPills(file);
  const hasVideo = !!(resolution || videoCodec || hdrPills.length > 0 || file.analysis?.is3D);
  const hasRelease = !!(sourceType || file.edition);
  const audioStreams = audioStreamsForFile(file);
  const hasAudioStreams = audioStreams.length > 0;
  const hasSubtitles = file.subtitleStreams.length > 0 || file.subtitleLanguages.length > 0;
  const isPendingScan = file.scanStatus === "imported";
  const isScanFailed = file.scanStatus === "scan_failed";
  const requiresReview = file.scanStatus === "review_required";

  if (!file.analysis && !hasContainer && !hasVideo && !hasRelease && !hasAudioStreams && audioPills.length === 0 && !hasSubtitles && !isPendingScan && !isScanFailed && !requiresReview) return null;

  return (
    <div className="flex flex-wrap items-center gap-1">
      {containerFormat ? <Badge tone="info">{containerFormat}</Badge> : null}
      {resolution ? <Badge tone="info">{resolution}</Badge> : null}
      {videoCodec ? <Badge tone="info">{videoCodec}</Badge> : null}
      {file.analysis?.is3D ? <Badge tone="info">3D</Badge> : null}
      {hdrPills.map((pill) => <Badge key={pill} tone="info">{pill}</Badge>)}
      {sourceType ? <Badge tone="info">{sourceType}</Badge> : null}
      {file.edition ? <Badge tone="info">{file.edition}</Badge> : null}
      {audioPills.map((pill) => <Badge key={pill} tone="info">{pill}</Badge>)}
      {hasAudioStreams ? <AudioTracksPopover streams={audioStreams} /> : null}
      {hasSubtitles ? (
        <SubtitleTracksPopover
          streams={file.subtitleStreams.length > 0
            ? file.subtitleStreams
            : file.subtitleLanguages.map((lang, i) => ({
                language: lang,
                codec: file.subtitleCodecs[i] ?? null,
                name: null,
                forced: false,
                default: false,
              }))}
        />
      ) : null}
      {isPendingScan ? <Badge tone="warning">{t("mediaFile.pendingScan")}</Badge> : null}
      {isScanFailed ? <Badge tone="negative">{t("mediaFile.scanFailed")}</Badge> : null}
      {requiresReview ? <Badge tone="warning">{t("mediaFile.reviewRequired")}</Badge> : null}
    </div>
  );
}
