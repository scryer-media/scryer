import type {
  MediaDolbyVision,
  MediaHdrCapabilities,
  MediaStreamDisposition,
} from "../types/media-analysis";

type FormatPillStream = {
  kind: string;
  channels: number | null;
  metadata: {
    id: string | null;
    profile: string | null;
    channelLayout: string | null;
    disposition?: Pick<MediaStreamDisposition, "commentary" | "visualImpaired"> | null;
    hdr?:
      | (Pick<MediaHdrCapabilities, "dolbyVision" | "hdr10plus" | "hdr10" | "hlg" | "pq"> & {
          dovi?: Pick<MediaDolbyVision, "baseLayerCompatibilityId"> | null;
        })
      | null;
  };
};

export type MediaFormatPillSource = {
  analysis?: { selectedVideoId: string | null; streams: FormatPillStream[] } | null;
  videoHdrFormat: string | null;
  audioChannels: number | null;
  audioStreams: { profile?: string | null; channels: number | null }[];
};

type AudioPillStream = {
  channels: number | null;
  profile: string | null;
  channelLayout: string | null;
};

type SurroundLayout = { label: string; channels: number };

const CHANNEL_COUNT_LAYOUTS: Record<number, string> = {
  3: "2.1",
  4: "4.0",
  5: "5.0",
  6: "5.1",
  7: "6.1",
  8: "7.1",
};

function selectedVideoHdr(analysis: MediaFormatPillSource["analysis"]) {
  const videoStreams = analysis?.streams.filter((stream) => stream.kind === "VIDEO") ?? [];
  const selected =
    videoStreams.find(
      (stream) => stream.metadata.id != null && stream.metadata.id === analysis?.selectedVideoId,
    ) ?? videoStreams[0];
  return selected?.metadata.hdr ?? null;
}

function legacyHdrPills(format: string | null): string[] {
  const trimmed = format?.trim();
  if (!trimmed) return [];
  switch (trimmed.toLowerCase()) {
    case "dolby vision":
      return ["DV"];
    case "hdr10+":
      return ["HDR10+"];
    case "hdr10":
      return ["HDR"];
    case "hlg":
      return ["HLG"];
    default:
      return [trimmed];
  }
}

// Dolby Vision is shown with the layer a non-DV display falls back to, so a
// profile 8.1 file reads "DV" "HDR" while profile 5 (IPT-PQ base layer) and
// 8.2 (SDR base layer) read "DV" alone.
export function hdrFormatPills(source: MediaFormatPillSource): string[] {
  const hdr = selectedVideoHdr(source.analysis);
  const pills: string[] = [];
  if (hdr?.dolbyVision) {
    pills.push("DV");
    const compatibility = hdr.dovi?.baseLayerCompatibilityId;
    if (compatibility === 0 || compatibility === 2) return pills;
  }
  if (hdr?.hdr10plus) pills.push("HDR10+");
  else if (hdr?.hdr10) pills.push("HDR");
  else if (hdr?.hlg) pills.push("HLG");
  else if (hdr?.pq && !hdr.dolbyVision) pills.push("HDR");
  return pills.length > 0 ? pills : legacyHdrPills(source.videoHdrFormat);
}

function audioPillStreams(source: MediaFormatPillSource): AudioPillStream[] {
  if (source.analysis?.streams.length) {
    return source.analysis.streams
      .filter(
        (stream) =>
          stream.kind === "AUDIO" &&
          !stream.metadata.disposition?.commentary &&
          !stream.metadata.disposition?.visualImpaired,
      )
      .map((stream) => ({
        channels: stream.channels,
        profile: stream.metadata.profile,
        channelLayout: stream.metadata.channelLayout,
      }));
  }
  if (source.audioStreams.length > 0) {
    return source.audioStreams.map((stream) => ({
      channels: stream.channels,
      profile: stream.profile ?? null,
      channelLayout: null,
    }));
  }
  return [{ channels: source.audioChannels, profile: null, channelLayout: null }];
}

function surroundLayout(stream: AudioPillStream): SurroundLayout | null {
  const layout = stream.channelLayout?.match(/^\d+(?:\.\d+){1,2}/)?.[0];
  if (layout) {
    const channels = layout.split(".").reduce((sum, part) => sum + Number(part), 0);
    return channels > 2 ? { label: layout, channels } : null;
  }
  if (stream.channels == null || stream.channels <= 2) return null;
  return {
    label: CHANNEL_COUNT_LAYOUTS[stream.channels] ?? `${stream.channels}ch`,
    channels: stream.channels,
  };
}

// The pills describe the best the file offers: the widest main audio track
// (commentary and audio-description tracks don't count), plus Atmos or DTS:X
// when any main track carries it.
export function audioFormatPills(source: MediaFormatPillSource): string[] {
  const streams = audioPillStreams(source);
  const widest = streams
    .map(surroundLayout)
    .reduce<SurroundLayout | null>(
      (best, layout) => (layout && (!best || layout.channels > best.channels) ? layout : best),
      null,
    );
  const pills: string[] = widest ? [widest.label] : [];
  if (streams.some((stream) => /atmos/i.test(stream.profile ?? ""))) pills.push("Atmos");
  if (streams.some((stream) => /dts:x/i.test(stream.profile ?? ""))) pills.push("DTS:X");
  return pills;
}
