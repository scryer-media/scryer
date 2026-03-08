import { useTranslate } from "@/lib/context/translate-context";

export type AudioStreamDetail = {
  codec: string | null;
  channels: number | null;
  language: string | null;
  bitrateKbps: number | null;
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
  hasMultiaudio: boolean;
  durationSeconds: number | null;
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
  if (width == null) return null;
  if (width >= 3840) return "4K";
  if (width >= 1920) return "1080p";
  if (width >= 1280) return "720p";
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
  color,
}: {
  children: React.ReactNode;
  color: "sky" | "blue" | "indigo" | "violet" | "cyan" | "teal" | "purple" | "amber" | "red";
}) {
  const colorClasses: Record<typeof color, string> = {
    sky: "border-sky-500/40 bg-sky-500/20 text-sky-700 dark:border-sky-500/30 dark:bg-sky-500/15 dark:text-sky-300",
    blue: "border-blue-500/40 bg-blue-500/20 text-blue-700 dark:border-blue-500/30 dark:bg-blue-500/15 dark:text-blue-300",
    indigo: "border-indigo-500/40 bg-indigo-500/20 text-indigo-700 dark:border-indigo-500/30 dark:bg-indigo-500/15 dark:text-indigo-300",
    violet: "border-violet-500/40 bg-violet-500/20 text-violet-700 dark:border-violet-500/30 dark:bg-violet-500/15 dark:text-violet-300",
    cyan: "border-cyan-500/40 bg-cyan-500/20 text-cyan-700 dark:border-cyan-500/30 dark:bg-cyan-500/15 dark:text-cyan-300",
    teal: "border-teal-500/40 bg-teal-500/20 text-teal-700 dark:border-teal-500/30 dark:bg-teal-500/15 dark:text-teal-300",
    purple: "border-purple-500/40 bg-purple-500/20 text-purple-700 dark:border-purple-500/30 dark:bg-purple-500/15 dark:text-purple-300",
    amber: "border-amber-500/40 bg-amber-500/20 text-amber-700 dark:border-amber-500/30 dark:bg-amber-500/15 dark:text-amber-300",
    red: "border-red-500/40 bg-red-500/20 text-red-700 dark:border-red-500/30 dark:bg-red-500/15 dark:text-red-300",
  };
  return (
    <span className={`rounded border px-1.5 py-0.5 text-[10px] font-medium ${colorClasses[color]}`}>
      {children}
    </span>
  );
}

export function MediaInfoBadges({ file }: { file: MediaInfoFile }) {
  const t = useTranslate();

  const resolution = resolveResolution(file.videoWidth, file.videoHeight);
  const videoCodec = resolveVideoCodec(file.videoCodec);
  const audioCodec = resolveAudioCodec(file.audioCodec);
  const audioChannels = resolveAudioChannels(file.audioChannels);

  const hdrColor = (): "indigo" | "violet" | "cyan" | "teal" => {
    if (file.videoHdrFormat === "Dolby Vision") return "indigo";
    if (file.videoHdrFormat === "HDR10+") return "violet";
    if (file.videoHdrFormat === "HLG") return "teal";
    return "cyan";
  };

  const sourceType = file.sourceType ? resolveSourceType(file.sourceType) : null;
  const hasTechInfo = resolution || videoCodec || file.videoHdrFormat || audioCodec || audioChannels || file.hasMultiaudio || sourceType || file.releaseGroup || file.edition;
  const isPendingScan = file.scanStatus === "imported";
  const isScanFailed = file.scanStatus === "scan_failed";

  if (!hasTechInfo && !isPendingScan && !isScanFailed) return null;

  return (
    <div className="flex flex-wrap gap-1">
      {resolution ? <Badge color="sky">{resolution}</Badge> : null}
      {videoCodec ? <Badge color="blue">{videoCodec}</Badge> : null}
      {file.videoHdrFormat ? <Badge color={hdrColor()}>{file.videoHdrFormat}</Badge> : null}
      {sourceType ? <Badge color="teal">{sourceType}</Badge> : null}
      {audioCodec ? <Badge color="violet">{audioCodec}</Badge> : null}
      {audioChannels ? <Badge color="purple">{audioChannels}</Badge> : null}
      {file.hasMultiaudio ? <Badge color="purple">Multi-Audio</Badge> : null}
      {file.edition ? <Badge color="cyan">{file.edition}</Badge> : null}
      {file.releaseGroup ? <Badge color="indigo">{file.releaseGroup}</Badge> : null}
      {isPendingScan ? <Badge color="amber">{t("mediaFile.pendingScan")}</Badge> : null}
      {isScanFailed ? <Badge color="red">{t("mediaFile.scanFailed")}</Badge> : null}
    </div>
  );
}
