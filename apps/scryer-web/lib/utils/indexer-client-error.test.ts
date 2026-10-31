import assert from "node:assert/strict";
import test from "node:test";
import {
  indexerClientAccessRefusal,
  indexerClientErrorMessage,
} from "./indexer-client-error.ts";

const t = (key: string) => `translated:${key}`;
const refusal = (code: string, reason: string, message = "server detail") => ({
  graphQLErrors: [{ message, extensions: { code, reason } }],
});

test("client management refusals use typed reasons for each error family", () => {
  for (const [code, reason, key] of [
    ["UNAUTHORIZED", "INDEXER_CLIENTS_INTERACTIVE_SESSION_REQUIRED", "session"],
    ["UNAUTHORIZED", "INDEXER_CLIENTS_ALL_PERMISSIONS_REQUIRED", "permissions"],
    ["VALIDATION_ERROR", "INDEXER_CLIENTS_CONFIGURATION_CHANGED", "changed"],
    ["TEMPORARY_UNAVAILABLE", "INDEXER_CLIENTS_UNAVAILABLE", "unavailable"],
    ["NOT_FOUND", "INDEXER_CLIENTS_NOT_FOUND", "missing"],
  ]) {
    assert.equal(
      indexerClientErrorMessage(refusal(code!, reason!), t),
      `translated:indexerClients.error.${key}`,
    );
  }
});

test("unknown reasons use the server explanation without interpreting its text", () => {
  assert.equal(
    indexerClientErrorMessage(
      refusal(
        "VALIDATION_ERROR",
        "FUTURE_REASON",
        "validation: Custom source refused",
      ),
      t,
    ),
    "Custom source refused",
  );
  assert.equal(
    indexerClientErrorMessage(
      refusal(
        "VALIDATION_ERROR",
        "FUTURE_REASON",
        "INDEXER_CLIENTS_CONFIGURATION_CHANGED",
      ),
      t,
    ),
    "INDEXER_CLIENTS_CONFIGURATION_CHANGED",
  );
});

test("only refusals that close client management to the session count as access refusals", () => {
  for (const [code, reason, key] of [
    ["UNAUTHORIZED", "INDEXER_CLIENTS_INTERACTIVE_SESSION_REQUIRED", "session"],
    ["UNAUTHORIZED", "INDEXER_CLIENTS_ALL_PERMISSIONS_REQUIRED", "permissions"],
  ]) {
    assert.equal(
      indexerClientAccessRefusal(refusal(code!, reason!), t),
      `translated:indexerClients.error.${key}`,
    );
  }
  assert.equal(
    indexerClientAccessRefusal(
      refusal("TEMPORARY_UNAVAILABLE", "INDEXER_CLIENTS_UNAVAILABLE"),
      t,
    ),
    null,
  );
  assert.equal(
    indexerClientAccessRefusal(refusal("VALIDATION_ERROR", "FUTURE_REASON"), t),
    null,
  );
  assert.equal(indexerClientAccessRefusal(new Error("network"), t), null);
});
