import assert from "node:assert/strict";
import test from "node:test";
import { indexerCapsDiagnostic } from "./indexer-caps-diagnostic.ts";

test("capability diagnostics translate codes and preserve the typed retry delay", () => {
  const t = (key: string, values?: Record<string, string | number>) =>
    key + (values ? JSON.stringify(values) : "");
  assert.equal(indexerCapsDiagnostic({}, t), undefined);
  assert.equal(
    indexerCapsDiagnostic({ capsSource: "FRESH" }, t),
    "indexerSearch.caps.fresh",
  );
  assert.equal(
    indexerCapsDiagnostic(
      {
        capsSource: "PERMISSIVE",
        capsReason: "INDEXER_CAPS_REFRESH_DEFERRED",
        capsRetryAfterSeconds: 17,
      },
      t,
    ),
    'indexerSearch.caps.permissive · indexerSearch.caps.refreshDeferred · indexerSearch.caps.retryAfter{"seconds":17}',
  );
  assert.equal(
    indexerCapsDiagnostic(
      { capsSource: "toString", capsReason: "unknown provider prose" },
      t,
    ),
    "indexerSearch.caps.unknown · indexerSearch.caps.refreshUnavailable",
  );
});
