import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import { buildSchema, Kind, parse, validate, type DocumentNode } from "graphql";

import * as indexerClients from "./indexer-clients.ts";
import * as mutations from "./mutations.ts";
import * as queries from "./queries.ts";

const schema = buildSchema(
  readFileSync(
    new URL("../../../../api/graphql/schema.graphql", import.meta.url),
    "utf8",
  ),
);

function collectDocuments(
  modules: Record<string, Record<string, unknown>>,
): Array<{ name: string; document: DocumentNode }> {
  const documents: Array<{ name: string; document: DocumentNode }> = [];
  for (const [moduleName, moduleExports] of Object.entries(modules)) {
    for (const [exportName, exported] of Object.entries(moduleExports)) {
      if (typeof exported !== "string") {
        continue;
      }
      let document: DocumentNode;
      try {
        document = parse(exported);
      } catch {
        // Shared field-selection constants are document fragments, not
        // standalone documents; they are validated where they are embedded.
        continue;
      }
      if (
        !document.definitions.some(
          (definition) => definition.kind === Kind.OPERATION_DEFINITION,
        )
      ) {
        continue;
      }
      documents.push({ name: `${moduleName}.${exportName}`, document });
    }
  }
  return documents;
}

const documents = collectDocuments({ queries, mutations, indexerClients });

test("collects the hand-written operations", () => {
  assert.ok(
    documents.length >= 300,
    `only found ${documents.length} exported documents; the collection logic no longer matches the export style`,
  );
});

test("every exported GraphQL document validates against the API schema", () => {
  const failures: string[] = [];
  for (const { name, document } of documents) {
    for (const error of validate(schema, document)) {
      failures.push(`${name}: ${error.message}`);
    }
  }
  assert.deepEqual(failures, []);
});

test("background restore start and polling documents validate against the API schema", () => {
  const source = readFileSync(
    new URL("../../components/setup/setup-restore-view.tsx", import.meta.url),
    "utf8",
  );
  const operations =
    source.match(
      /(?:mutation (?:InspectRestoreBundle|ApplyRestoreBundle)|query RestoreInspectionStatus)\b[^`]+/g,
    ) ?? [];
  assert.equal(operations.length, 3);
  for (const operation of operations) {
    assert.deepEqual(validate(schema, parse(operation)), []);
  }
});
