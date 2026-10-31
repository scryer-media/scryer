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

test("score log renders decoded entries with rule badges, rejections and the total", async () => {
  const server = await createServer({
    root: WEB_ROOT,
    server: { middlewareMode: true },
    appType: "custom",
    logLevel: "silent",
  });

  try {
    const [scoreLogModule, contextModule] = await Promise.all([
      server.ssrLoadModule("/components/common/score-log.tsx"),
      server.ssrLoadModule("/lib/context/translate-context.tsx"),
    ]);
    const ScoreLog = scoreLogModule.ScoreLog as ComponentType<{
      entries: ReleaseDecisionExplanationEntry[];
      total?: number | null;
    }>;
    const TranslateContext =
      contextModule.TranslateContext as Context<Translate | null>;
    const html = renderToStaticMarkup(
      createElement(
        TranslateContext.Provider,
        { value: translate },
        createElement(ScoreLog, {
          entries: parseDecisionExplanation({
            quality_profile_decision: {
              scoring_log: [
                { code: "quality_tier", delta: 1000 },
                {
                  code: "release_group",
                  delta: -25,
                  source: "user:rule-1",
                  ruleSetName: "Synthetic groups",
                },
                {
                  code: "codec_required",
                  delta: 0,
                  kind: "mandatory_rejection",
                  source: { kind: "system_rule", name: "Synthetic codecs" },
                },
              ],
            },
          }),
          total: 975,
        }),
      ),
    );

    assert.match(html, /nzb\.scoringLog/);
    assert.match(html, /quality_tier/);
    assert.match(html, /\+1000/);
    assert.match(html, /release_group/);
    assert.match(html, /-25/);
    assert.match(html, /scoring\.customRule/);
    assert.match(html, /Synthetic groups/);
    assert.match(html, /scoring\.systemRule/);
    assert.match(html, /scoring\.mandatoryRejection/);
    assert.match(html, /nzb\.total/);
    assert.match(html, /\+975/);
  } finally {
    await server.close();
  }
});
