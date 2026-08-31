import { useTranslate } from "@/lib/context/translate-context";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { Badge as UiBadge } from "@/components/ui/badge";
import { ChevronDown } from "lucide-react";

export type AudioStreamDetail = {
  codec: string | null;
  channels: number | null;
  language: string | null;
  bitrateKbps: number | null;
};

export type SubtitleStreamDetail = {
  codec: string | null;
  language: string | null;
  name: string | null;
  forced: boolean;
  default: boolean;
};

export type MediaInfoFile = {
  scanStatus: string;
  videoCodec: string | null;
  videoWidth: number | null;
  videoHeight: number | null;
  videoBitrateKbps: number | null;
  videoBitDepth: number | null;
  videoHdrFormat: string | null;
  videoFrameRate: string | null;
  videoProfile: string | null;
  audioCodec: string | null;
  audioChannels: number | null;
  audioBitrateKbps: number | null;
  audioLanguages: string[];
  audioStreams: AudioStreamDetail[];
  subtitleLanguages: string[];
  subtitleCodecs: string[];
  subtitleStreams: SubtitleStreamDetail[];
  hasMultiaudio: boolean;
  durationSeconds: number | null;
  numChapters: number | null;
  containerFormat: string | null;
  sceneName?: string | null;
  releaseGroup?: string | null;
  sourceType?: string | null;
  resolution?: string | null;
  videoCodecParsed?: string | null;
  audioCodecParsed?: string | null;
  acquisitionScore?: number | null;
  scoringLog?: string | null;
  indexerSource?: string | null;
  grabbedReleaseTitle?: string | null;
  grabbedAt?: string | null;
  edition?: string | null;
  originalFilePath?: string | null;
  releaseHash?: string | null;
};

function resolveResolution(width: number | null, height: number | null): string | null {
  if (width == null && height == null) return null;
  if ((width != null && width >= 7680) || (height != null && height >= 4200)) return "8K";
  if ((width != null && width >= 3840) || (height != null && height >= 2100)) return "4K";
  if ((width != null && width >= 1920) || (height != null && height >= 1000)) return "1080p";
  if ((width != null && width >= 1280) || (height != null && height >= 700)) return "720p";
  return height != null ? `${height}p` : null;
}

function resolveVideoCodec(codec: string | null): string | null {
  if (codec == null) return null;
  if (codec === "hevc") return "HEVC";
  if (codec === "h264") return "H.264";
  if (codec === "av1") return "AV1";
  if (codec === "vc1") return "VC-1";
  return codec.toUpperCase();
}

function resolveAudioCodec(codec: string | null): string | null {
  if (codec == null) return null;
  if (codec === "truehd") return "TrueHD";
  if (codec === "eac3") return "EAC3";
  if (codec === "ac3") return "AC3";
  if (codec === "flac") return "FLAC";
  if (codec === "aac") return "AAC";
  if (codec === "dts") return "DTS";
  if (codec === "opus") return "Opus";
  return codec.toUpperCase();
}

function resolveAudioChannels(channels: number | null): string | null {
  if (channels == null) return null;
  if (channels === 8) return "7.1";
  if (channels === 6) return "5.1";
  if (channels === 2) return "2.0";
  if (channels === 1) return "1.0";
  return `${channels}ch`;
}

let displayNamesCache: Intl.DisplayNames | null = null;

function formatLanguage(code: string | null): string {
  if (!code) return "?";
  try {
    displayNamesCache ??= new Intl.DisplayNames(undefined, { type: "language" });
    return displayNamesCache.of(code) ?? code;
  } catch {
    return code;
  }
}

function resolveSubtitleCodec(codec: string | null): string {
  if (!codec) return "?";
  const c = codec.toLowerCase();
  if (c === "subrip" || c === "srt") return "SRT";
  if (c === "ass" || c === "ssa") return "ASS";
  if (c === "hdmv_pgs_subtitle" || c === "pgs" || c === "pgssub") return "PGS";
  if (c === "dvd_subtitle" || c === "dvdsub" || c === "vobsub") return "VobSub";
  if (c === "webvtt" || c === "vtt") return "WebVTT";
  if (c === "mov_text") return "TX3G";
  return codec.toUpperCase();
}

function formatSingleAudioTrack(stream: AudioStreamDetail): string {
  const parts = [
    formatLanguage(stream.language),
    resolveAudioCodec(stream.codec),
    resolveAudioChannels(stream.channels),
  ].filter((value): value is string => Boolean(value && value !== "?"));
  return parts.length > 0 ? parts.join(" ") : "Audio";
}

function formatSingleSubtitleTrack(track: SubtitleStreamDetail): string {
  const parts = [formatLanguage(track.language)];
  if (track.forced) parts.push("Forced");
  else if (track.default) parts.push("Default");
  return parts.filter(Boolean).join(" ");
}

function resolveSourceType(source: string): string | null {
  const s = source.toLowerCase();
  if (s === "bluray" || s === "blu-ray") return "BluRay";
  if (s === "webdl" || s === "web-dl") return "WEB-DL";
  if (s === "webrip" || s === "web-rip") return "WEBRip";
  if (s === "hdtv") return "HDTV";
  if (s === "dvd" || s === "dvdrip") return "DVD";
  if (s === "remux") return "Remux";
  if (s === "bdremux") return "BD Remux";
  return source;
}

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

function AudioTracksPopover({ streams }: { streams: AudioStreamDetail[] }) {
  const t = useTranslate();
  if (streams.length === 1) {
    return <Badge tone="info">{formatSingleAudioTrack(streams[0])}</Badge>;
  }
  return (
    <Popover>
      <PopoverTrigger asChild>
        <button
          type="button"
          className="inline-flex cursor-pointer items-center gap-1 rounded border border-[var(--scry-info-border)] bg-[var(--scry-info-bg)] px-1.5 py-0.5 text-[11px] font-medium text-[var(--scry-info-text)] hover:bg-[var(--scry-info-bg-strong)]"
        >
          {t("mediaFile.audioCount", { count: streams.length })}
          <ChevronDown className="h-3 w-3 opacity-70" />
        </button>
      </PopoverTrigger>
      <PopoverContent className="w-auto max-w-xs p-2" align="start">
        <div className="max-h-60 space-y-1 overflow-y-auto">
          {streams.map((stream, i) => (
            <div key={i} className="flex items-center gap-2 rounded px-2 py-1 text-xs even:bg-muted/50">
              <span className="min-w-[5rem] font-medium">{formatLanguage(stream.language)}</span>
              <span className="text-muted-foreground">{resolveAudioCodec(stream.codec) ?? "?"}</span>
              <span className="text-muted-foreground">{resolveAudioChannels(stream.channels) ?? "?"}</span>
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

function resolveContainerFormat(format: string | null): string | null {
  if (format == null) return null;

  switch (format.trim().toLowerCase()) {
    case "matroska":
      return "MKV";
    case "webm":
      return "WebM";
    case "mp4":
      return "MP4";
    case "avi":
      return "AVI";
    case "mpegts":
      return "MPEG-TS";
    case "asf":
      return "ASF";
    case "ogg":
      return "OGG";
    case "flv":
      return "FLV";
    default:
      return format.toUpperCase();
  }
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
  const hasVideo = !!(resolution || videoCodec || file.videoHdrFormat);
  const hasRelease = !!(sourceType || file.edition);
  const hasAudioStreams = file.audioStreams.length > 0;
  const hasSubtitles = file.subtitleStreams.length > 0 || file.subtitleLanguages.length > 0;
  const isPendingScan = file.scanStatus === "imported";
  const isScanFailed = file.scanStatus === "scan_failed";

  if (!hasContainer && !hasVideo && !hasRelease && !hasAudioStreams && !hasSubtitles && !isPendingScan && !isScanFailed) return null;

  return (
    <div className="flex flex-wrap items-center gap-1">
      {containerFormat ? <Badge tone="info">{containerFormat}</Badge> : null}
      {resolution ? <Badge tone="info">{resolution}</Badge> : null}
      {videoCodec ? <Badge tone="info">{videoCodec}</Badge> : null}
      {file.videoHdrFormat ? <Badge tone="info">{file.videoHdrFormat}</Badge> : null}
      {sourceType ? <Badge tone="info">{sourceType}</Badge> : null}
      {file.edition ? <Badge tone="info">{file.edition}</Badge> : null}
      {hasAudioStreams ? <AudioTracksPopover streams={file.audioStreams} /> : null}
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
    </div>
  );
}
