import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { createElement, type ComponentType, type Context } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { createServer } from "vite";

import type { Translate } from "@/components/root/types";
import {
  parseDecisionExplanation,
  type ReleaseDecisionExplanationEntry,
} from "../utils/release-decision-explanation.ts";

const WEB_ROOT = fileURLToPath(new URL("../..", import.meta.url));
const translate: Translate = (key) => key;

test("expanded Wanted scoring breakdown renders decoded explanation entries", async () => {
  const server = await createServer({
    root: WEB_ROOT,
    server: { middlewareMode: true },
    appType: "custom",
    logLevel: "silent",
  });

  try {
    const [wantedModule, contextModule] = await Promise.all([
      server.ssrLoadModule("/components/views/wanted-scoring-breakdown.tsx"),
      server.ssrLoadModule("/lib/context/translate-context.tsx"),
    ]);
    const WantedScoringBreakdown = wantedModule.WantedScoringBreakdown as ComponentType<{
      entries: ReleaseDecisionExplanationEntry[];
    }>;
    const TranslateContext = contextModule.TranslateContext as Context<Translate | null>;
    const html = renderToStaticMarkup(
      createElement(
        TranslateContext.Provider,
        { value: translate },
        createElement(WantedScoringBreakdown, {
          entries: parseDecisionExplanation({
            quality_profile_decision: {
              scoring_log: [
                { code: "quality_tier", delta: 1000 },
                { code: "release_group", delta: -25 },
              ],
            },
          }),
        }),
      ),
    );

    assert.match(html, /wanted\.scoreCode/);
    assert.match(html, /quality_tier/);
    assert.match(html, /\+1000/);
    assert.match(html, /release_group/);
    assert.match(html, /-25/);
  } finally {
    await server.close();
  }
});
