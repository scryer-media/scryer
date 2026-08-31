import assert from "node:assert/strict";
import test from "node:test";
import {
  MAX_INDEXER_ERROR_BODY_PREVIEW_BYTES,
  decodedBase64Length,
  isSensitiveIndexerErrorHeader,
  presentIndexerErrorBody,
} from "./indexer-error-response.ts";

function encoded(value: string): string {
  return Buffer.from(value).toString("base64");
}

test("pretty prints valid JSON and preserves raw text", () => {
  const body = presentIndexerErrorBody(encoded('{"error":{"code":429}}'), "application/json");
  assert.equal(body.format, "json");
  assert.equal(body.formattedText, '{\n  "error": {\n    "code": 429\n  }\n}');
  assert.equal(body.rawText, '{"error":{"code":429}}');
});

test("pretty prints valid XML and falls back for malformed XML", () => {
  const valid = presentIndexerErrorBody(encoded("<error><code>100</code></error>"), "application/xml");
  assert.equal(valid.formattedText, "<error>\n  <code>100</code>\n</error>");
  const invalid = presentIndexerErrorBody(encoded("<error><code></error>"), "application/xml");
  assert.equal(invalid.formattedText, null);
  assert.equal(invalid.rawText, "<error><code></error>");
});

test("rejects multiple XML roots and mixed content instead of rewriting them", () => {
  const multipleRoots = presentIndexerErrorBody(encoded("<first/><second/>"), "application/xml");
  assert.equal(multipleRoots.formattedText, null);
  assert.equal(multipleRoots.rawText, "<first/><second/>");

  const mixedContent = presentIndexerErrorBody(
    encoded("<message>left <strong>important</strong> right</message>"),
    "application/xml",
  );
  assert.equal(mixedContent.formattedText, null);
  assert.equal(mixedContent.rawText, "<message>left <strong>important</strong> right</message>");
});

test("caps previews at one MiB and reports the full decoded length", () => {
  const source = Buffer.alloc(MAX_INDEXER_ERROR_BODY_PREVIEW_BYTES + 17, 97);
  const body = presentIndexerErrorBody(source.toString("base64"), "text/plain");
  assert.equal(body.byteLength, source.length);
  assert.equal(body.rawText?.length, MAX_INDEXER_ERROR_BODY_PREVIEW_BYTES);
  assert.equal(body.truncated, true);
});

test("keeps a UTF-8 preview when the byte cap splits a multibyte character", () => {
  const source = Buffer.concat([
    Buffer.alloc(MAX_INDEXER_ERROR_BODY_PREVIEW_BYTES - 1, 97),
    Buffer.from("€x"),
  ]);
  const body = presentIndexerErrorBody(source.toString("base64"), "text/plain");
  assert.equal(body.format, "text");
  assert.equal(body.rawText?.length, MAX_INDEXER_ERROR_BODY_PREVIEW_BYTES - 1);
  assert.equal(body.truncated, true);
});

test("treats invalid UTF-8 as binary", () => {
  const body = presentIndexerErrorBody(Buffer.from([0xff, 0xfe]).toString("base64"));
  assert.equal(body.format, "binary");
  assert.equal(body.rawText, null);
});

test("calculates padded and unpadded decoded sizes", () => {
  assert.equal(decodedBase64Length("YQ=="), 1);
  assert.equal(decodedBase64Length("YWI="), 2);
  assert.equal(decodedBase64Length("YWJj"), 3);
});

test("recognizes sensitive response header names", () => {
  for (const name of ["Authorization", "set-cookie", "X-Api-Key", "x-auth-token", "client-secret"]) {
    assert.equal(isSensitiveIndexerErrorHeader(name), true, name);
  }
  assert.equal(isSensitiveIndexerErrorHeader("content-type"), false);
});
