import assert from "node:assert/strict";
import test from "node:test";

import {
  audioFormatPills,
  hdrFormatPills,
  type MediaFormatPillSource,
} from "./media-format-pills.ts";

type Stream = NonNullable<MediaFormatPillSource["analysis"]>["streams"][number];
type Hdr = NonNullable<Stream["metadata"]["hdr"]>;

function videoStream(hdr: Partial<Hdr>, id = "0"): Stream {
  return {
    kind: "VIDEO",
    channels: null,
    metadata: {
      id,
      profile: "Main 10",
      channelLayout: null,
      hdr: { dolbyVision: null, hdr10plus: null, hdr10: null, hlg: null, pq: null, ...hdr },
    },
  };
}

function audioStream(
  channels: number,
  options: { layout?: string; profile?: string; commentary?: boolean } = {},
): Stream {
  return {
    kind: "AUDIO",
    channels,
    metadata: {
      id: null,
      profile: options.profile ?? null,
      channelLayout: options.layout ?? null,
      disposition: { commentary: options.commentary ?? false, visualImpaired: false },
    },
  };
}

function source(overrides: Partial<MediaFormatPillSource>): MediaFormatPillSource {
  return {
    analysis: null,
    videoHdrFormat: null,
    audioChannels: null,
    audioStreams: [],
    ...overrides,
  };
}

function analysis(streams: Stream[], selectedVideoId: string | null = null) {
  return { selectedVideoId, streams };
}

test("dolby vision with an HDR10 base layer shows both pills", () => {
  const file = source({
    analysis: analysis([
      videoStream({
        dolbyVision: true,
        hdr10: true,
        pq: true,
        dovi: { baseLayerCompatibilityId: 1 },
      }),
    ]),
    videoHdrFormat: "Dolby Vision",
  });
  assert.deepEqual(hdrFormatPills(file), ["DV", "HDR"]);
});

test("dolby vision without an HDR fallback layer shows DV alone", () => {
  const profile5 = source({
    analysis: analysis([
      videoStream({
        dolbyVision: true,
        hdr10: true,
        pq: true,
        dovi: { baseLayerCompatibilityId: 0 },
      }),
    ]),
  });
  assert.deepEqual(hdrFormatPills(profile5), ["DV"]);

  const unknownCompatibility = source({
    analysis: analysis([videoStream({ dolbyVision: true, pq: true })]),
  });
  assert.deepEqual(hdrFormatPills(unknownCompatibility), ["DV"]);
});

test("HDR10+ replaces the plain HDR pill", () => {
  const file = source({ analysis: analysis([videoStream({ hdr10plus: true, hdr10: true, pq: true })]) });
  assert.deepEqual(hdrFormatPills(file), ["HDR10+"]);
});

test("HLG and bare PQ streams are labelled", () => {
  assert.deepEqual(hdrFormatPills(source({ analysis: analysis([videoStream({ hlg: true })]) })), ["HLG"]);
  assert.deepEqual(hdrFormatPills(source({ analysis: analysis([videoStream({ pq: true })]) })), ["HDR"]);
});

test("SDR video has no HDR pills", () => {
  const file = source({ analysis: analysis([videoStream({ hdr10: false, hlg: false, pq: false })]) });
  assert.deepEqual(hdrFormatPills(file), []);
});

test("the selected video stream decides the HDR pills", () => {
  const file = source({
    analysis: analysis([videoStream({ pq: false }, "0"), videoStream({ hdr10: true }, "1")], "1"),
  });
  assert.deepEqual(hdrFormatPills(file), ["HDR"]);
});

test("files scanned before stream analysis fall back to the stored HDR format", () => {
  assert.deepEqual(hdrFormatPills(source({ videoHdrFormat: "Dolby Vision" })), ["DV"]);
  assert.deepEqual(hdrFormatPills(source({ videoHdrFormat: "HDR10+" })), ["HDR10+"]);
  assert.deepEqual(hdrFormatPills(source({ videoHdrFormat: "HDR10" })), ["HDR"]);
  assert.deepEqual(hdrFormatPills(source({ videoHdrFormat: "HLG" })), ["HLG"]);
});

test("the widest main audio track sets the surround pill and Atmos is flagged", () => {
  const file = source({
    analysis: analysis([
      videoStream({}),
      audioStream(8, { layout: "7.1", profile: "Dolby TrueHD + Dolby Atmos" }),
      audioStream(6, { layout: "5.1(side)", profile: "Dolby Digital" }),
      audioStream(2, { layout: "stereo" }),
    ]),
  });
  assert.deepEqual(audioFormatPills(file), ["7.1", "Atmos"]);
});

test("commentary tracks do not set the surround pill", () => {
  const file = source({
    analysis: analysis([
      audioStream(6, { layout: "5.1(side)" }),
      audioStream(8, { layout: "7.1", commentary: true }),
    ]),
  });
  assert.deepEqual(audioFormatPills(file), ["5.1"]);
});

test("DTS:X is flagged as object audio", () => {
  const file = source({
    analysis: analysis([audioStream(8, { layout: "7.1", profile: "DTS-HD MA + DTS:X" })]),
  });
  assert.deepEqual(audioFormatPills(file), ["7.1", "DTS:X"]);
});

test("stereo and mono audio have no surround pill", () => {
  const file = source({
    analysis: analysis([audioStream(2, { layout: "stereo" }), audioStream(1, { layout: "mono" })]),
  });
  assert.deepEqual(audioFormatPills(file), []);
});

test("channel count names the layout when the scan has none", () => {
  const file = source({
    audioStreams: [{ channels: 6, profile: "Dolby Digital Plus + Dolby Atmos" }],
  });
  assert.deepEqual(audioFormatPills(file), ["5.1", "Atmos"]);
  assert.deepEqual(audioFormatPills(source({ audioChannels: 8 })), ["7.1"]);
});
