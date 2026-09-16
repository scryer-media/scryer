import assert from "node:assert/strict";
import test from "node:test";
import type {
  MediaAnalysisDetails,
  MediaProbeReport,
  MediaStreamDetail,
  MediaStreamMetadata,
} from "../types/media-analysis.ts";
import {
  attachmentRows,
  audioStreamsForFile,
  audioTrackRows,
  chapterRows,
  formatDurationSeconds,
  formatMediaFileSize,
  mediaFileBaseName,
  mediaInfoSections,
  resolveContainerFormat,
  resolveResolution,
  resolveSubtitleCodec,
  resolveVideoCodec,
  subtitleTrackRows,
  type MediaInfoFileDetails,
} from "./media-info-format.ts";

const LABELS = { added: null, grabbedAt: null, yes: "Yes", no: "No" };

const REPORT: MediaProbeReport = {
  status: "COMPLETE",
  bytesRead: 4096,
  seeks: 3,
  elapsedMs: 120,
  budgetExhausted: false,
  warnings: [],
};

function metadata(overrides: Partial<MediaStreamMetadata> = {}): MediaStreamMetadata {
  return {
    id: "1",
    programId: null,
    originalLanguage: null,
    languageProvenance: "CONTAINER",
    disposition: {
      default: false,
      forced: false,
      original: false,
      commentary: false,
      hearingImpaired: false,
      visualImpaired: false,
      attachedPicture: false,
      stillImage: false,
    },
    durationSeconds: null,
    bitrateBps: null,
    bitrateProvenance: "CONTAINER",
    estimatedBitrateBps: null,
    sampleRate: null,
    sampleFormat: null,
    sampleBitDepth: null,
    channelLayout: null,
    pixelFormat: null,
    profile: null,
    level: null,
    bitDepth: null,
    fieldOrder: null,
    sampleAspectRatio: null,
    displayAspectRatio: null,
    rotationDegrees: null,
    declaredFrameRate: null,
    observedFrameRate: null,
    variableFrameRate: null,
    color: {
      primaries: null,
      transfer: null,
      matrix: null,
      fullRange: null,
      provenance: "CONTAINER",
      masteringDisplay: null,
      contentLight: null,
    },
    hdr: { dolbyVision: null, hdr10plus: null, hdr10: null, hlg: null, pq: null, dovi: null },
    ...overrides,
  };
}

function stream(overrides: Partial<MediaStreamDetail> = {}): MediaStreamDetail {
  return {
    kind: "AUDIO",
    codec: null,
    width: null,
    height: null,
    channels: null,
    language: null,
    name: null,
    metadata: metadata(),
    ...overrides,
  };
}

function analysis(overrides: Partial<MediaAnalysisDetails> = {}): MediaAnalysisDetails {
  return {
    revision: 3,
    durationSeconds: null,
    durationProvenance: "CONTAINER",
    overallBitrateBps: null,
    selectedVideoId: null,
    selectedProgramId: null,
    streams: [],
    programs: [],
    chapters: [],
    attachments: [],
    captionServices: [],
    disc: null,
    report: REPORT,
    ...overrides,
  };
}

function mediaFile(overrides: Partial<MediaInfoFileDetails> = {}): MediaInfoFileDetails {
  return {
    id: "file-1",
    filePath: "/library/Synthetic Show Alpha/Season 01/Synthetic Show Alpha - S01E01.mkv",
    sizeBytes: 3 * 1024 ** 3,
    scanStatus: "scanned",
    videoCodec: null,
    videoWidth: null,
    videoHeight: null,
    videoBitrateKbps: null,
    videoBitDepth: null,
    videoHdrFormat: null,
    videoFrameRate: null,
    videoProfile: null,
    audioCodec: null,
    audioChannels: null,
    audioBitrateKbps: null,
    audioLanguages: [],
    audioStreams: [],
    subtitleLanguages: [],
    subtitleCodecs: [],
    subtitleStreams: [],
    hasMultiaudio: false,
    durationSeconds: null,
    numChapters: null,
    containerFormat: null,
    ...overrides,
  };
}

test("audioStreamsForFile prefers the native analysis audio streams", () => {
  const file = mediaFile({
    audioCodec: "ac3",
    audioLanguages: ["eng", "deu"],
    audioStreams: [{ codec: "aac", channels: 2, language: "eng", bitrateKbps: 128 }],
    analysis: analysis({
      streams: [
        stream({ kind: "VIDEO", codec: "hevc", metadata: metadata({ id: "0" }) }),
        stream({
          codec: "truehd",
          channels: 8,
          language: "eng",
          metadata: metadata({ id: "1", bitrateBps: 4_000_000, channelLayout: "7.1" }),
        }),
      ],
    }),
  });

  const streams = audioStreamsForFile(file);
  assert.equal(streams.length, 1);
  assert.equal(streams[0].codec, "truehd");
  assert.equal(streams[0].bitrateKbps, 4000);
});

test("audioStreamsForFile falls back to the stored stream rows", () => {
  const file = mediaFile({
    audioCodec: "ac3",
    audioStreams: [
      { codec: "dts", channels: 6, language: "eng", bitrateKbps: 768 },
      { codec: "aac", channels: 2, language: "jpn", bitrateKbps: 192 },
    ],
    analysis: analysis({ streams: [] }),
  });

  assert.deepEqual(
    audioStreamsForFile(file).map((entry) => entry.codec),
    ["dts", "aac"],
  );
});

test("audioStreamsForFile synthesizes one track per legacy audio language", () => {
  const file = mediaFile({
    audioCodec: "eac3",
    audioChannels: 6,
    audioBitrateKbps: 640,
    audioLanguages: ["eng", "spa"],
  });

  assert.deepEqual(audioStreamsForFile(file), [
    { codec: "eac3", channels: 6, bitrateKbps: 640, language: "eng" },
    { codec: "eac3", channels: 6, bitrateKbps: 640, language: "spa" },
  ]);
});

test("audioStreamsForFile synthesizes a single track from the legacy codec alone", () => {
  const file = mediaFile({ audioCodec: "aac" });
  assert.deepEqual(audioStreamsForFile(file), [
    { codec: "aac", channels: null, bitrateKbps: null, language: null },
  ]);
});

test("audioStreamsForFile reports nothing when the file carries no audio facts", () => {
  assert.deepEqual(audioStreamsForFile(mediaFile()), []);
});

test("formatters map codecs, containers and resolutions to display labels", () => {
  assert.equal(resolveResolution(3840, 2160), "4K");
  assert.equal(resolveResolution(null, 1080), "1080p");
  assert.equal(resolveResolution(null, 576), "576p");
  assert.equal(resolveResolution(null, null), null);
  assert.equal(resolveVideoCodec("hevc"), "HEVC");
  assert.equal(resolveVideoCodec("h264"), "H.264");
  assert.equal(resolveVideoCodec(null), null);
  assert.equal(resolveContainerFormat("matroska"), "MKV");
  assert.equal(resolveContainerFormat("mystery"), "MYSTERY");
  assert.equal(resolveContainerFormat(null), null);
  assert.equal(resolveSubtitleCodec("hdmv_pgs_subtitle"), "PGS");
  assert.equal(resolveSubtitleCodec(null), "?");
  assert.equal(formatDurationSeconds(3723), "1h 02m 03s");
  assert.equal(formatDurationSeconds(75), "1m 15s");
  assert.equal(formatDurationSeconds(null), null);
  assert.equal(formatMediaFileSize(1024 ** 3), "1.00 GB");
  assert.equal(formatMediaFileSize(0), "-");
  assert.equal(
    mediaFileBaseName("/library/Synthetic Feature Beta (2019)/Synthetic Feature Beta.mkv"),
    "Synthetic Feature Beta.mkv",
  );
  assert.equal(mediaFileBaseName(null), null);
});

test("mediaInfoSections drops empty rows and empty sections", () => {
  const sections = mediaInfoSections(mediaFile({ sizeBytes: null }), LABELS);
  assert.deepEqual(
    sections.map((entry) => entry.id),
    ["media-info-file"],
  );
  assert.deepEqual(
    sections[0].rows.map((entry) => entry.labelKey),
    ["mediaInfo.path", "mediaInfo.scanStatus"],
  );
});

test("mediaInfoSections builds the file, video, release and analysis tables", () => {
  const file = mediaFile({
    containerFormat: "matroska",
    durationSeconds: 3600,
    numChapters: 12,
    role: "primary",
    originalFilePath: "/downloads/synthetic.beta.2019.mkv",
    releaseHash: "abc123",
    videoCodec: "hevc",
    videoWidth: 3840,
    videoHeight: 2160,
    videoBitDepth: 10,
    videoHdrFormat: "HDR10",
    sceneName: "Synthetic.Feature.Beta.2019.2160p",
    releaseGroup: "SYNTHGRP",
    sourceType: "bluray",
    edition: "Extended",
    indexerSource: "Example Indexer",
    acquisitionScore: 1250,
    analysis: analysis({ overallBitrateBps: 24_000_000, revision: 4 }),
  });

  const byId = new Map(mediaInfoSections(file, { ...LABELS, added: "2026-01-02" }).map((s) => [s.id, s]));
  const value = (id: string, labelKey: string) =>
    byId.get(id)?.rows.find((entry) => entry.labelKey === labelKey)?.value ?? null;

  assert.equal(value("media-info-file", "mediaInfo.container"), "MKV");
  assert.equal(value("media-info-file", "mediaInfo.duration"), "1h 00m 00s");
  assert.equal(value("media-info-file", "mediaInfo.chapterCount"), "12");
  assert.equal(value("media-info-file", "mediaInfo.added"), "2026-01-02");
  assert.equal(value("media-info-file", "mediaInfo.releaseHash"), "abc123");
  assert.equal(value("media-info-video", "mediaInfo.resolution"), "3840 × 2160 (4K)");
  assert.equal(value("media-info-video", "mediaInfo.videoCodec"), "HEVC");
  assert.equal(value("media-info-video", "mediaInfo.bitDepth"), "10-bit");
  assert.equal(value("media-info-video", "mediaInfo.hdr"), "HDR");
  assert.equal(value("media-info-release", "mediaInfo.sourceType"), "BluRay");
  assert.equal(value("media-info-release", "mediaInfo.releaseGroup"), "SYNTHGRP");
  assert.equal(value("media-info-release", "mediaInfo.acquisitionScore"), "1250");
  assert.equal(value("media-info-analysis", "mediaInfo.revision"), "4");
  assert.equal(value("media-info-analysis", "mediaInfo.overallBitrate"), "24,000 kbps");
  assert.equal(value("media-info-analysis", "mediaInfo.budgetExhausted"), "No");
});

test("audioTrackRows describes each track, roles included", () => {
  const file = mediaFile({
    analysis: analysis({
      streams: [
        stream({
          codec: "truehd",
          channels: 8,
          language: "eng",
          name: "Main mix",
          metadata: metadata({
            id: "1",
            channelLayout: "7.1",
            profile: "TrueHD Atmos",
            sampleRate: 48000,
            sampleBitDepth: 24,
            bitrateBps: 4_500_000,
            disposition: { ...metadata().disposition, default: true },
          }),
        }),
        stream({
          codec: "ac3",
          channels: 2,
          language: "eng",
          metadata: metadata({
            id: "2",
            disposition: { ...metadata().disposition, commentary: true },
          }),
        }),
      ],
    }),
  });

  const rows = audioTrackRows(file);
  assert.equal(rows.length, 2);
  assert.deepEqual(
    { ...rows[0], language: undefined },
    {
      index: 1,
      language: undefined,
      codec: "TrueHD",
      profile: "TrueHD Atmos",
      channels: "7.1",
      bitrate: "4,500 kbps",
      sampleRate: "48,000 Hz",
      sampleDepth: "24-bit",
      roleKeys: ["mediaInfo.roleDefault"],
      name: "Main mix",
    },
  );
  assert.deepEqual(rows[1].roleKeys, ["mediaFile.commentary"]);
  assert.equal(rows[1].channels, "2ch");
});

test("subtitleTrackRows fills in from the legacy language and codec columns", () => {
  const file = mediaFile({ subtitleLanguages: ["eng", "fra"], subtitleCodecs: ["subrip"] });
  const rows = subtitleTrackRows(file);
  assert.deepEqual(
    rows.map((entry) => [entry.index, entry.codec, entry.forced]),
    [
      [1, "SRT", false],
      [2, "?", false],
    ],
  );
});

test("chapterRows and attachmentRows read straight from the analysis", () => {
  const file = mediaFile({
    analysis: analysis({
      chapters: [{ id: "c1", title: "Cold open", startSeconds: 0, endSeconds: 90 }],
      attachments: [{ id: "a1", name: "cover.jpg", mediaType: "image/jpeg", sizeBytes: 2048 }],
    }),
  });

  assert.deepEqual(chapterRows(file), [
    { index: 1, start: "0s", end: "1m 30s", title: "Cold open" },
  ]);
  assert.deepEqual(attachmentRows(file), [
    { id: "a1", name: "cover.jpg", mediaType: "image/jpeg", size: "2.00 KB" },
  ]);
});
