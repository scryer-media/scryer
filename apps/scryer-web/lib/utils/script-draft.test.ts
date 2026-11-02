import assert from "node:assert/strict";
import test from "node:test";
import type { PostProcessingScriptDraft } from "@/lib/types/scripts";
import { withScriptType } from "./script-draft.ts";

const draft = (
  scriptType: string,
  scriptContent: string,
  parkedScriptContent = "",
) =>
  ({
    scriptType,
    scriptContent,
    parkedScriptContent,
  }) as PostProcessingScriptDraft;

test("inline code never reaches the file path", () => {
  const next = withScriptType(draft("inline", "echo hi"), "file");
  assert.equal(next.scriptType, "file");
  assert.equal(next.scriptContent, "");
});

test("a picked path never reaches the inline editor", () => {
  const next = withScriptType(draft("file", "/srv/run.sh"), "inline");
  assert.equal(next.scriptContent, "");
});

test("switching back restores each type's own text", () => {
  const start = draft("inline", "echo hi", "/srv/run.sh");
  const there = withScriptType(start, "file");
  assert.equal(there.scriptContent, "/srv/run.sh");
  assert.deepEqual(withScriptType(there, "inline"), start);
});

test("reselecting the current type changes nothing", () => {
  const start = draft("inline", "echo hi");
  assert.equal(withScriptType(start, "inline"), start);
});
