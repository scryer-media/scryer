import assert from "node:assert/strict";
import test from "node:test";
import { parseServiceLogLine, prettyServiceLogLine } from "./service-log-lines.ts";

test("parses and humanizes a contextual JSON service log", () => {
  const parsed = parseServiceLogLine(JSON.stringify({
    timestamp: "2026-08-23T12:00:00.000Z",
    level: "INFO",
    target: "scryer::import",
    fields: { message: "import completed", count: 2 },
    context: {
      actor: { id: "user-1", display_name: "Sam" },
      workflow: { kind: "import", id: "import-1" },
      resource: { title_id: "title-1" },
    },
  }));

  assert.equal(parsed?.level, "info");
  assert.match(parsed?.human ?? "", /import completed/);
  assert.match(parsed?.human ?? "", /actor=Sam/);
  assert.match(parsed?.human ?? "", /title_id=title-1/);
  assert.equal(prettyServiceLogLine(parsed)?.includes("\n  \"context\""), true);
});

test("falls back when a line is not the Scryer JSON envelope", () => {
  assert.equal(parseServiceLogLine("2026-08-23T12:00:00Z INFO test: legacy"), null);
  assert.equal(parseServiceLogLine('{"level":"INFO"}'), null);
});
