import assert from "node:assert/strict";
import { test } from "node:test";
import { parse } from "graphql";
import type { Operation } from "@urql/core";
import { isLongOperation, LONG_OPERATION_NAMES } from "./long-operations.ts";
import * as mutations from "./mutations.ts";

function operation(kind: Operation["kind"], document: string): Operation {
  return { kind, query: parse(document) } as unknown as Operation;
}

test("every long operation names a document the web sends", () => {
  const documents = Object.values(mutations).filter(
    (value): value is string => typeof value === "string",
  );
  for (const name of LONG_OPERATION_NAMES) {
    assert.ok(
      documents.some((document) => document.startsWith(`mutation ${name}(`)),
      `no mutation document is named ${name}`,
    );
  }
});

test("long mutations are recognised and ordinary requests are not", () => {
  assert.equal(
    isLongOperation(
      operation("mutation", "mutation RetryImport { retryImport }"),
    ),
    true,
  );
  assert.equal(
    isLongOperation(operation("mutation", "mutation SaveTag { saveTag }")),
    false,
  );
  assert.equal(
    isLongOperation(operation("query", "query Titles { titles }")),
    false,
  );
  assert.equal(
    isLongOperation(
      operation("subscription", "subscription RetryImport { retryImport }"),
    ),
    false,
  );
});
