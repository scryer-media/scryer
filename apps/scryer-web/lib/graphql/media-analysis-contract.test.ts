import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { buildSchema, NoUnusedFragmentsRule, parse, specifiedRules, validate } from "graphql";
import { MEDIA_ANALYSIS_FIELDS, MEDIA_DISC_FIELDS } from "../types/media-analysis.ts";
import { TITLE_MEDIA_FILE_FIELDS } from "./queries.ts";

const schema = buildSchema(readFileSync(new URL("../../../../api/graphql/schema.graphql", import.meta.url), "utf8"));
const fragmentRules = specifiedRules.filter((rule) => rule !== NoUnusedFragmentsRule);

for (const [type, fields] of [
  ["MediaAnalysisDetailsPayload", MEDIA_ANALYSIS_FIELDS],
  ["MediaDiscMetadataPayload", MEDIA_DISC_FIELDS],
  ["TitleMediaFilePayload", TITLE_MEDIA_FILE_FIELDS],
]) {
  test(`${type} frontend fragment matches the exported API schema`, () => {
    const document = parse(`fragment MediaContract on ${type} { ${fields} }`);
    assert.deepEqual(validate(schema, document, fragmentRules).map((error) => error.message), []);
  });
}

test("HDR10+ queries retain the frontend property name through an explicit API alias", () => {
  for (const fields of [MEDIA_ANALYSIS_FIELDS, MEDIA_DISC_FIELDS]) {
    assert.match(fields, /hdr10plus:\s*hdr10Plus/);
  }
});
