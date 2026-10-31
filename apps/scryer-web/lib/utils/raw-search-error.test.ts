import assert from "node:assert/strict";
import test from "node:test";
import {
  rawSearchErrorMessage,
  rawSearchFailureMessage,
} from "./raw-search-error.ts";

const t = (key: string) => `translated:${key}`;

test("snapshot failures translate known codes and fall back to the server sentence", () => {
  assert.equal(
    rawSearchFailureMessage(
      {
        failureReasonCode: "RAW_SEARCH_HISTORY_EXPIRED",
        failureReason: "server",
      },
      t,
      "fallback",
    ),
    "translated:indexerSearch.error.expired",
  );
  assert.equal(
    rawSearchFailureMessage(
      { failureReasonCode: "FUTURE_REASON", failureReason: "server sentence" },
      t,
      "fallback",
    ),
    "server sentence",
  );
  assert.equal(
    rawSearchFailureMessage({ failureReasonCode: "toString" }, t, "fallback"),
    "fallback",
  );
});

test("refused continuations translate the reason extension, not the message", () => {
  const refusal = (reason: string, message: string) => ({
    graphQLErrors: [
      { message, extensions: { code: "VALIDATION_ERROR", reason } },
    ],
  });
  assert.equal(
    rawSearchErrorMessage(
      refusal("RAW_SEARCH_CURSOR_INVALID", "RAW_SEARCH_HISTORY_EXPIRED"),
      t,
      "fallback",
    ),
    "translated:indexerSearch.error.offset",
  );
  assert.equal(
    rawSearchErrorMessage(
      refusal("FUTURE_REASON", "validation: Synthetic refusal"),
      t,
      "fallback",
    ),
    "Synthetic refusal",
  );
});
