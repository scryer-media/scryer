import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { createElement, type ComponentType, type Context } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { createServer } from "vite";
import type { Translate } from "@/components/root/types";
import type { MediaDiscTitle, MediaStreamDetail } from "../types/media-analysis.ts";
import en from "../i18n/locales/en.ts";
import { interpolate } from "../i18n/types.ts";

const audio: MediaStreamDetail = {
  kind: "AUDIO", codec: "ac3", width: null, height: null, channels: 6,
  language: "eng", name: "Director commentary",
  metadata: {
    id: "1100", programId: 1, originalLanguage: "en", languageProvenance: "CONTAINER",
    disposition: { default: false, forced: null, original: null, commentary: true,
      hearingImpaired: null, visualImpaired: null, attachedPicture: null, stillImage: null },
    durationSeconds: 1800, bitrateBps: 640000, bitrateProvenance: "BITSTREAM", estimatedBitrateBps: null,
    sampleRate: 48000, sampleFormat: null, sampleBitDepth: null, channelLayout: "5.1",
    pixelFormat: null, profile: "Dolby Digital", level: null, bitDepth: null, fieldOrder: null,
    sampleAspectRatio: null, displayAspectRatio: null, rotationDegrees: null,
    declaredFrameRate: null, observedFrameRate: null, variableFrameRate: null,
    color: { primaries: null, transfer: null, matrix: null, fullRange: null,
      provenance: "UNKNOWN", masteringDisplay: null, contentLight: null },
    hdr: { dolbyVision: null, hdr10plus: null, hdr10: null, hlg: null, pq: null, dovi: null },
  },
};

test("disc title details render each title's own streams, chapters, and incomplete coverage", async () => {
  const server = await createServer({ root: fileURLToPath(new URL("../..", import.meta.url)),
    server: { middlewareMode: true }, appType: "custom", logLevel: "silent" });
  try {
    const [component, context, manual] = await Promise.all([
      server.ssrLoadModule("/components/common/media-analysis-details.tsx"),
      server.ssrLoadModule("/lib/context/translate-context.tsx"),
      server.ssrLoadModule("/components/dialogs/manual-disc-selection.tsx"),
    ]);
    const DiscTitleDetails = component.DiscTitleDetails as ComponentType<{ title: MediaDiscTitle }>;
    const ManualDiscSelectionControl = manual.ManualDiscSelectionControl as
      typeof import("../../components/dialogs/manual-disc-selection").ManualDiscSelectionControl;
    const TranslateContext = context.TranslateContext as Context<Translate | null>;
    const translate: Translate = (key, values) => interpolate(en[key] ?? key, values);
    const title: MediaDiscTitle = {
      id: "00002", aliases: ["00012"], durationSeconds: 1800, angleCount: 2, segments: [],
      streams: [audio], chapters: [{ id: "1", title: "Alternate opening", startSeconds: 12.5, endSeconds: 30 }],
      report: { status: "INCOMPLETE", bytesRead: 4096, seeks: 3, elapsedMs: 1, budgetExhausted: true,
        warnings: [{ code: "disc_segment_format_difference", message: "Stream formats differ between segments",
          streamId: null, offset: null }] },
    };
    const render = (value: MediaDiscTitle) => renderToStaticMarkup(createElement(TranslateContext.Provider,
      { value: translate }, createElement(DiscTitleDetails, { title: value })));
    const html = render(title);
    for (const text of ["Title 00002", "00012", "first angle", "probe budget", "formats differ",
      "Director commentary", "Dolby Digital", "5.1", "Alternate opening", "12.5 s", "30 s"]) {
      assert.ok(html.includes(text), text);
    }
    assert.match(html, /<details/);
    assert.match(html, /<summary/);
    const unknown = render({ ...title, id: "00003", durationSeconds: null, streams: [], chapters: [] });
    assert.match(unknown, /Title 00003.*Unknown/);
    assert.match(unknown, /No stream metadata was recovered/);
    assert.match(unknown, /No chapter metadata was recovered/);
    assert.doesNotMatch(unknown, /Director commentary|Alternate opening/);
    for (const isMovie of [true, false]) {
      const preview = renderToStaticMarkup(createElement(TranslateContext.Provider, { value: translate },
        createElement(ManualDiscSelectionControl, {
          disc: { discType: "bluray", selectedTitleId: title.id, automaticSelection: false, titles: [title] },
          report: title.report, isMovie, episodes: [{ id: "episode-2", label: "Episode 2" }],
          value: { titleId: title.id, episodeMappings: [{ discTitleId: title.id, episodeId: "episode-2" }] },
          onChange: () => {},
        })));
      assert.match(preview, /Inspect title streams and chapters/);
      assert.match(preview, /Director commentary/);
      assert.match(preview, /Alternate opening/);
      assert.match(preview, isMovie ? /value="00002" selected/ : /value="episode-2" selected/);
    }
  } finally {
    await server.close();
  }
});
