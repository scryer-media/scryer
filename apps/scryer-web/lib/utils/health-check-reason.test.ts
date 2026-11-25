import assert from "node:assert/strict";
import test from "node:test";

import {
  healthCheckMessage,
  parseHealthCheckFacts,
} from "./health-check-reason.ts";

const translate = (
  key: string,
  values?: Record<string, string | number | boolean | null | undefined>,
) => `${key}:${JSON.stringify(values ?? {})}`;

test("a known reason is translated with its counts", () => {
  assert.equal(
    healthCheckMessage(
      {
        reason: "ARCHIVE_WORKSPACE_UNMARKED_ENTRIES",
        facts: { unmarked: 4 },
        message: "server sentence",
      },
      translate,
    ),
    'jobs.healthCheckReason.ARCHIVE_WORKSPACE_UNMARKED_ENTRIES:{"unmarked":4}',
  );
});

test("an unknown or missing reason shows the server sentence", () => {
  for (const reason of [undefined, null, "SOMETHING_NEW"]) {
    assert.equal(
      healthCheckMessage({ reason, message: "server sentence" }, translate),
      "server sentence",
    );
  }
});

test("facts keep only finite counts", () => {
  assert.deepEqual(
    parseHealthCheckFacts({ blocked: 2, label: "x", bad: Number.NaN }),
    { blocked: 2 },
  );
  assert.deepEqual(parseHealthCheckFacts(null), {});
  assert.deepEqual(parseHealthCheckFacts([1]), {});
});
