import type {
  MediaAnalysisDetails,
  MediaRational,
  MediaStreamDetail,
  MediaStreamMetadata,
} from "../types/media-analysis.ts";
import { hdrFormatPills } from "./media-format-pills.ts";

export type AudioStreamDetail = {
  metadata?: Pick<MediaStreamMetadata, "channelLayout" | "disposition" | "programId"> &
    Partial<Pick<MediaStreamMetadata, "id" | "sampleRate" | "sampleFormat" | "sampleBitDepth">>;
  profile?: string | null;
  name?: string | null;
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
  id?: string;
  analysis?: MediaAnalysisDetails;
  analysisAttempt?: import("../types/media-analysis.ts").MediaAnalysisAttempt | null;
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

/** A media file as the on-disk panel knows it: the badge facts plus its row identity. */
export type MediaInfoFileDetails = MediaInfoFile & {
  filePath?: string | null;
  sizeBytes?: number | null;
  role?: string | null;
  createdAt?: string | null;
};

export type MediaInfoRow = { labelKey: string; value: string };
export type MediaInfoSection = { id: string; titleKey: string; rows: MediaInfoRow[] };

export type MediaInfoAudioTrackRow = {
  index: number;
  language: string;
  codec: string | null;
  profile: string | null;
  channels: string | null;
  bitrate: string | null;
  sampleRate: string | null;
  sampleDepth: string | null;
  roleKeys: string[];
  name: string | null;
};

export type MediaInfoSubtitleTrackRow = {
  index: number;
  language: string;
  codec: string;
  forced: boolean;
  default: boolean;
  name: string | null;
};

export type MediaInfoChapterRow = {
  index: number;
  start: string;
  end: string | null;
  title: string | null;
};

export type MediaInfoAttachmentRow = {
  id: string;
  name: string;
  mediaType: string | null;
  size: string | null;
};

export type MediaInfoCaptionRow = {
  streamId: string;
  standard: string;
  serviceNumber: string | null;
  language: string | null;
};

export function resolveResolution(width: number | null, height: number | null): string | null {
  if (width == null && height == null) return null;
  if ((width != null && width >= 7680) || (height != null && height >= 4200)) return "8K";
  if ((width != null && width >= 3840) || (height != null && height >= 2100)) return "4K";
  if ((width != null && width >= 1920) || (height != null && height >= 1000)) return "1080p";
  if ((width != null && width >= 1280) || (height != null && height >= 700)) return "720p";
  return height != null ? `${height}p` : null;
}

export function resolveVideoCodec(codec: string | null): string | null {
  if (codec == null) return null;
  if (codec === "hevc") return "HEVC";
  if (codec === "h264") return "H.264";
  if (codec === "av1") return "AV1";
  if (codec === "vc1") return "VC-1";
  return codec.toUpperCase();
}

export function resolveAudioCodec(codec: string | null): string | null {
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

export function resolveAudioChannels(channels: number | null, layout?: string | null): string | null {
  if (layout) return layout;
  if (channels == null) return null;
  return `${channels}ch`;
}

let displayNamesCache: Intl.DisplayNames | null = null;

export function formatLanguage(code: string | null): string {
  if (!code) return "?";
  try {
    displayNamesCache ??= new Intl.DisplayNames(undefined, { type: "language" });
    return displayNamesCache.of(code) ?? code;
  } catch {
    return code;
  }
}

export function resolveSubtitleCodec(codec: string | null): string {
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

export function resolveContainerFormat(format: string | null): string | null {
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

export function resolveSourceType(source: string): string | null {
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

export function formatSingleAudioTrack(stream: AudioStreamDetail): string {
  const parts = [
    formatLanguage(stream.language),
    resolveAudioCodec(stream.codec),
    stream.profile,
    resolveAudioChannels(stream.channels, stream.metadata?.channelLayout),
  ].filter((value): value is string => Boolean(value && value !== "?"));
  return parts.length > 0 ? parts.join(" ") : "Audio";
}

export function formatSingleSubtitleTrack(track: SubtitleStreamDetail): string {
  const parts = [formatLanguage(track.language)];
  if (track.forced) parts.push("Forced");
  else if (track.default) parts.push("Default");
  return parts.filter(Boolean).join(" ");
}

export function formatMediaFileSize(sizeBytes: number | null | undefined): string {
  const bytes = sizeBytes ?? Number.NaN;
  if (!Number.isFinite(bytes) || bytes <= 0) {
    return "-";
  }
  if (bytes >= 1024 ** 3) {
    return `${(bytes / 1024 ** 3).toFixed(2)} GB`;
  }
  if (bytes >= 1024 ** 2) {
    return `${(bytes / 1024 ** 2).toFixed(2)} MB`;
  }
  if (bytes >= 1024) {
    return `${(bytes / 1024).toFixed(2)} KB`;
  }
  return `${bytes.toFixed(0)} B`;
}

/**
 * The per-track audio list every surface shows, in one place.
 *
 * Native analysis wins; a file scanned before analysis existed falls back to
 * its stored stream rows; a file older still carries only the flat legacy
 * columns, and those are synthesized into one track per declared language so
 * the audio pill and the modal say the same thing about it.
 */
export function audioStreamsForFile(file: MediaInfoFile): AudioStreamDetail[] {
  if (file.analysis?.streams.length) {
    const streams = file.analysis.streams
      .filter((stream) => stream.kind === "AUDIO")
      .map((stream) => audioStreamFromAnalysis(stream));
    if (streams.length > 0) return streams;
  }
  if (file.audioStreams.length > 0) return file.audioStreams;

  const languages = file.audioLanguages.filter((language) => language.trim() !== "");
  if (!file.audioCodec && file.audioChannels == null && languages.length === 0) return [];
  const template = {
    codec: file.audioCodec,
    channels: file.audioChannels,
    bitrateKbps: file.audioBitrateKbps,
  };
  if (languages.length === 0) return [{ ...template, language: null }];
  return languages.map((language) => ({ ...template, language }));
}

function audioStreamFromAnalysis(stream: MediaStreamDetail): AudioStreamDetail {
  return {
    codec: stream.codec,
    channels: stream.channels,
    language: stream.language,
    profile: stream.metadata.profile,
    name: stream.name,
    metadata: stream.metadata,
    bitrateKbps:
      stream.metadata.bitrateBps == null ? null : Number(stream.metadata.bitrateBps) / 1000,
  };
}

/** The per-track subtitle list, with the same fallback the file rows use. */
export function subtitleStreamsForFile(file: MediaInfoFile): SubtitleStreamDetail[] {
  if (file.subtitleStreams.length > 0) return file.subtitleStreams;
  const length = Math.max(file.subtitleLanguages.length, file.subtitleCodecs.length);
  return Array.from({ length }, (_, index) => ({
    language: file.subtitleLanguages[index] ?? null,
    codec: file.subtitleCodecs[index] ?? null,
    name: null,
    forced: false,
    default: false,
  }));
}

export function selectedVideoStream(
  analysis: MediaAnalysisDetails | null | undefined,
): MediaStreamDetail | null {
  const videos = analysis?.streams.filter((stream) => stream.kind === "VIDEO") ?? [];
  const selected = videos.find(
    (stream) => stream.metadata.id != null && stream.metadata.id === analysis?.selectedVideoId,
  );
  return selected ?? videos[0] ?? null;
}

export function formatDurationSeconds(seconds: number | null | undefined): string | null {
  if (seconds == null || !Number.isFinite(seconds) || seconds <= 0) return null;
  const whole = Math.round(seconds);
  const hours = Math.floor(whole / 3600);
  const minutes = Math.floor((whole % 3600) / 60);
  const rest = whole % 60;
  if (hours > 0) return `${hours}h ${String(minutes).padStart(2, "0")}m ${String(rest).padStart(2, "0")}s`;
  if (minutes > 0) return `${minutes}m ${String(rest).padStart(2, "0")}s`;
  return `${rest}s`;
}

export function formatBitrateBps(bps: number | string | null | undefined): string | null {
  if (bps == null) return null;
  const value = Number(bps);
  if (!Number.isFinite(value) || value <= 0) return null;
  return `${Math.round(value / 1000).toLocaleString()} kbps`;
}

function formatRational(value: MediaRational | null | undefined): string | null {
  if (!value) return null;
  const numerator = Number(value.numerator);
  const denominator = Number(value.denominator);
  if (!Number.isFinite(numerator) || !Number.isFinite(denominator) || denominator === 0) return null;
  return `${numerator}/${denominator}`;
}

function formatFrameRate(value: MediaRational | null | undefined): string | null {
  if (!value) return null;
  const numerator = Number(value.numerator);
  const denominator = Number(value.denominator);
  if (!Number.isFinite(numerator) || !Number.isFinite(denominator) || denominator === 0) return null;
  return `${(numerator / denominator).toLocaleString(undefined, { maximumFractionDigits: 3 })} fps`;
}

function row(labelKey: string, value: string | null | undefined): MediaInfoRow | null {
  const trimmed = typeof value === "string" ? value.trim() : value;
  return trimmed ? { labelKey, value: trimmed } : null;
}

function compactRows(rows: (MediaInfoRow | null)[]): MediaInfoRow[] {
  return rows.filter((entry): entry is MediaInfoRow => entry != null);
}

function section(id: string, titleKey: string, rows: (MediaInfoRow | null)[]): MediaInfoSection | null {
  const kept = compactRows(rows);
  return kept.length > 0 ? { id, titleKey, rows: kept } : null;
}

/** Values the caller has to format for the viewer's locale before the rows are built. */
export type MediaInfoLabels = {
  added?: string | null;
  grabbedAt?: string | null;
  yes: string;
  no: string;
};

export function mediaInfoFileSection(
  file: MediaInfoFileDetails,
  labels: MediaInfoLabels,
): MediaInfoSection | null {
  const size = formatMediaFileSize(file.sizeBytes);
  const chapters = file.numChapters ?? file.analysis?.chapters.length ?? null;
  return section("media-info-file", "mediaInfo.sectionFile", [
    row("mediaInfo.path", file.filePath),
    row("mediaInfo.size", size === "-" ? null : size),
    row("mediaInfo.container", resolveContainerFormat(file.containerFormat)),
    row(
      "mediaInfo.duration",
      formatDurationSeconds(file.analysis?.durationSeconds ?? file.durationSeconds),
    ),
    row("mediaInfo.chapterCount", chapters ? String(chapters) : null),
    row("mediaInfo.scanStatus", file.scanStatus),
    row("mediaInfo.role", file.role),
    row("mediaInfo.added", labels.added),
    row("mediaInfo.originalPath", file.originalFilePath),
    row("mediaInfo.releaseHash", file.releaseHash),
  ]);
}

export function mediaInfoVideoSection(
  file: MediaInfoFileDetails,
  labels: MediaInfoLabels,
): MediaInfoSection | null {
  const stream = selectedVideoStream(file.analysis);
  const metadata = stream?.metadata;
  const width = stream?.width ?? file.videoWidth;
  const height = stream?.height ?? file.videoHeight;
  const label = resolveResolution(width, height);
  const dimensions =
    width != null && height != null
      ? label
        ? `${width} × ${height} (${label})`
        : `${width} × ${height}`
      : label;
  const bitDepth = metadata?.bitDepth ?? file.videoBitDepth;
  const hdr = hdrFormatPills(file).join(", ");
  const contentLight = metadata?.color.contentLight;
  const mastering = metadata?.color.masteringDisplay;
  const dovi = metadata?.hdr.dovi;

  return section("media-info-video", "mediaInfo.sectionVideo", [
    row("mediaInfo.resolution", dimensions),
    row("mediaInfo.videoCodec", resolveVideoCodec(stream?.codec ?? file.videoCodec)),
    row("mediaInfo.profile", metadata?.profile ?? file.videoProfile),
    row("mediaInfo.bitDepth", bitDepth == null ? null : `${bitDepth}-bit`),
    row("mediaInfo.hdr", hdr),
    row("mediaInfo.threeD", file.analysis?.is3D ? labels.yes : null),
    row(
      "mediaInfo.frameRate",
      formatFrameRate(metadata?.observedFrameRate) ??
        formatFrameRate(metadata?.declaredFrameRate) ??
        file.videoFrameRate,
    ),
    row(
      "mediaInfo.videoBitrate",
      formatBitrateBps(metadata?.bitrateBps) ??
        (file.videoBitrateKbps == null ? null : `${file.videoBitrateKbps.toLocaleString()} kbps`),
    ),
    row("mediaInfo.pixelFormat", metadata?.pixelFormat),
    row("mediaInfo.fieldOrder", metadata?.fieldOrder),
    row("mediaInfo.aspectRatio", formatRational(metadata?.displayAspectRatio)),
    row(
      "mediaInfo.rotation",
      metadata?.rotationDegrees == null ? null : `${metadata.rotationDegrees}°`,
    ),
    row(
      "mediaInfo.color",
      metadata?.color &&
        (metadata.color.primaries != null ||
          metadata.color.transfer != null ||
          metadata.color.matrix != null)
        ? `${metadata.color.primaries ?? "?"} / ${metadata.color.transfer ?? "?"} / ${metadata.color.matrix ?? "?"}`
        : null,
    ),
    row(
      "mediaInfo.contentLight",
      contentLight && (contentLight.maxCll != null || contentLight.maxFall != null)
        ? `${contentLight.maxCll ?? "?"} / ${contentLight.maxFall ?? "?"}`
        : null,
    ),
    row(
      "mediaInfo.masteringDisplay",
      mastering && (mastering.minLuminance != null || mastering.maxLuminance != null)
        ? `${mastering.minLuminance ?? "?"}–${mastering.maxLuminance ?? "?"} cd/m²`
        : null,
    ),
    row(
      "mediaInfo.dolbyVision",
      dovi && dovi.profile != null
        ? `${dovi.profile}${dovi.baseLayerCompatibilityId == null ? "" : `.${dovi.baseLayerCompatibilityId}`}`
        : null,
    ),
  ]);
}

export function mediaInfoReleaseSection(
  file: MediaInfoFileDetails,
  labels: MediaInfoLabels,
): MediaInfoSection | null {
  return section("media-info-release", "mediaInfo.sectionRelease", [
    row("mediaInfo.sceneName", file.sceneName),
    row("mediaInfo.releaseGroup", file.releaseGroup),
    row("mediaInfo.sourceType", file.sourceType ? resolveSourceType(file.sourceType) : null),
    row("mediaInfo.parsedResolution", file.resolution),
    row("mediaInfo.parsedVideoCodec", file.videoCodecParsed),
    row("mediaInfo.parsedAudioCodec", file.audioCodecParsed),
    row("mediaInfo.edition", file.edition),
    row("mediaInfo.indexer", file.indexerSource),
    row("mediaInfo.grabbedTitle", file.grabbedReleaseTitle),
    row("mediaInfo.grabbedAt", labels.grabbedAt),
    row(
      "mediaInfo.acquisitionScore",
      file.acquisitionScore == null ? null : String(file.acquisitionScore),
    ),
  ]);
}

export function mediaInfoAnalysisSection(
  file: MediaInfoFileDetails,
  labels: MediaInfoLabels,
): MediaInfoSection | null {
  const analysis = file.analysis;
  if (!analysis) return null;
  const report = analysis.report;
  return section("media-info-analysis", "mediaInfo.sectionAnalysis", [
    row("mediaInfo.revision", String(analysis.revision)),
    row("mediaInfo.probeStatus", report.status),
    row("mediaInfo.bytesRead", Number(report.bytesRead).toLocaleString()),
    row("mediaInfo.seeks", Number(report.seeks).toLocaleString()),
    row("mediaInfo.elapsed", `${Number(report.elapsedMs).toLocaleString()} ms`),
    row("mediaInfo.budgetExhausted", report.budgetExhausted ? labels.yes : labels.no),
    row("mediaInfo.overallBitrate", formatBitrateBps(analysis.overallBitrateBps)),
    ...analysis.programs.map((program) =>
      row(
        "mediaInfo.program",
        [String(program.id), program.name, program.streamIds.join(", ")]
          .filter((part): part is string => Boolean(part))
          .join(" · "),
      ),
    ),
    ...report.warnings.map((warning) =>
      row("mediaInfo.warning", `${warning.code}: ${warning.message}`),
    ),
  ]);
}

export function audioTrackRows(file: MediaInfoFile): MediaInfoAudioTrackRow[] {
  return audioStreamsForFile(file).map((stream, index) => {
    const disposition = stream.metadata?.disposition;
    const roleKeys: string[] = [];
    if (disposition?.default) roleKeys.push("mediaInfo.roleDefault");
    if (disposition?.forced) roleKeys.push("mediaInfo.roleForced");
    if (disposition?.original) roleKeys.push("mediaInfo.roleOriginal");
    if (disposition?.commentary) roleKeys.push("mediaFile.commentary");
    if (disposition?.visualImpaired) roleKeys.push("mediaFile.audioDescription");
    if (disposition?.hearingImpaired) roleKeys.push("mediaFile.hearingImpaired");
    return {
      index: index + 1,
      language: formatLanguage(stream.language),
      codec: resolveAudioCodec(stream.codec),
      profile: stream.profile ?? null,
      channels: resolveAudioChannels(stream.channels, stream.metadata?.channelLayout),
      bitrate:
        stream.bitrateKbps == null
          ? null
          : `${Math.round(stream.bitrateKbps).toLocaleString()} kbps`,
      sampleRate:
        stream.metadata?.sampleRate == null
          ? null
          : `${stream.metadata.sampleRate.toLocaleString()} Hz`,
      sampleDepth:
        stream.metadata?.sampleBitDepth == null ? null : `${stream.metadata.sampleBitDepth}-bit`,
      roleKeys,
      name: stream.name ?? null,
    };
  });
}

export function subtitleTrackRows(file: MediaInfoFile): MediaInfoSubtitleTrackRow[] {
  return subtitleStreamsForFile(file).map((track, index) => ({
    index: index + 1,
    language: formatLanguage(track.language),
    codec: resolveSubtitleCodec(track.codec),
    forced: track.forced,
    default: track.default,
    name: track.name,
  }));
}

export function chapterRows(file: MediaInfoFile): MediaInfoChapterRow[] {
  return (file.analysis?.chapters ?? []).map((chapter, index) => ({
    index: index + 1,
    start: formatDurationSeconds(chapter.startSeconds) ?? "0s",
    end: formatDurationSeconds(chapter.endSeconds),
    title: chapter.title,
  }));
}

export function attachmentRows(file: MediaInfoFile): MediaInfoAttachmentRow[] {
  return (file.analysis?.attachments ?? []).map((attachment) => {
    const size = formatMediaFileSize(Number(attachment.sizeBytes));
    return {
      id: attachment.id,
      name: attachment.name ?? attachment.id,
      mediaType: attachment.mediaType,
      size: size === "-" ? null : size,
    };
  });
}

export function captionRows(file: MediaInfoFile): MediaInfoCaptionRow[] {
  return (file.analysis?.captionServices ?? []).map((caption) => ({
    streamId: caption.streamId,
    standard: caption.standard,
    serviceNumber: caption.serviceNumber == null ? null : String(caption.serviceNumber),
    language: caption.language,
  }));
}

/** The label/value sections of the modal, in reading order, empties dropped. */
export function mediaInfoSections(
  file: MediaInfoFileDetails,
  labels: MediaInfoLabels,
): MediaInfoSection[] {
  return [
    mediaInfoFileSection(file, labels),
    mediaInfoVideoSection(file, labels),
    mediaInfoReleaseSection(file, labels),
    mediaInfoAnalysisSection(file, labels),
  ].filter((entry): entry is MediaInfoSection => entry != null);
}

export function mediaFileBaseName(filePath: string | null | undefined): string | null {
  const trimmed = filePath?.trim();
  if (!trimmed) return null;
  return trimmed.split(/[\\/]/).filter(Boolean).pop() ?? trimmed;
}
