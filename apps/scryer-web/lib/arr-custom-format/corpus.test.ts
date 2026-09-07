import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { inspectCustomFormats } from "./inspection.ts";
import { translateCustomFormats } from "./translate.ts";
import type { ArrSource } from "./types.ts";

type Corpus = {
  revision: string;
  formats: Array<{ source: ArrSource; path: string; format: unknown }>;
};

const corpus = JSON.parse(
  readFileSync(new URL("./__fixtures__/trash-corpus.json", import.meta.url), "utf8"),
) as Corpus;

test("pinned TRaSH corpus translates at least 97% of complete formats", (context) => {
  assert.equal(corpus.revision, "56cb176ef4b59d734d3643287e66aafd7c809bd4");
  assert.equal(corpus.formats.length, 478, "Do not shrink the compatibility denominator");
  const totals = { sonarr: 0, radarr: 0 };
  const translated = { sonarr: 0, radarr: 0 };
  const failures: string[] = [];

  for (const entry of corpus.formats) {
    totals[entry.source]++;
    const inspection = inspectCustomFormats(JSON.stringify(entry.format), entry.source);
    assert.equal(inspection.fatal, false, entry.path);
    assert.equal(inspection.formats.length, 1, entry.path);
    const format = inspection.formats[0];
    const result = translateCustomFormats(inspection.formats, { [format.id]: 100 });
    assert.equal(result.formats.length, 1, entry.path);
    assert.ok(result.regoSource.startsWith("import rego.v1"), entry.path);
    if (result.formats[0].status === "translated") {
      translated[entry.source]++;
      assert.equal(result.formats[0].diagnostics.some((item) => item.level === "error"), false, entry.path);
    } else {
      assert.ok(result.formats[0].diagnostics.length > 0, entry.path);
      failures.push(`${entry.path}: ${result.formats[0].diagnostics.map((item) => item.code).join(", ")}`);
    }
  }

  assert.deepEqual(totals, { sonarr: 236, radarr: 242 });
  for (const source of ["sonarr", "radarr"] as const) {
    context.diagnostic(`${source}: ${translated[source]}/${totals[source]} complete translations (${(100 * translated[source] / totals[source]).toFixed(2)}%)`);
  }
  const totalTranslated = translated.sonarr + translated.radarr;
  context.diagnostic(`Combined: ${totalTranslated}/478 (${(100 * totalTranslated / 478).toFixed(2)}%). This measures supported complete formats, not end-to-end Arr parser equivalence.`);
  for (const failure of failures) context.diagnostic(failure);
  assert.ok(totalTranslated / 478 >= 0.97, `${totalTranslated}/478 formats translated; unsupported formats:\n${failures.join("\n")}`);
});
