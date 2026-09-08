import assert from "node:assert/strict";
import test from "node:test";
import { resolveRuleDetailRequest } from "./rule-detail-request.ts";

function deferred<T>() {
  let resolve!: (value: T) => void;
  return { promise: new Promise<T>((complete) => { resolve = complete; }), resolve };
}

test("detail request uses the live dirty state after loading", async () => {
  const detail = deferred<string | null>();
  let editorState = { isOpen: true, isDirty: false };
  const result = resolveRuleDetailRequest(1, () => 1, () => detail.promise, () => editorState);
  editorState = { isOpen: true, isDirty: true };
  detail.resolve("rule");
  assert.deepEqual(await result, { type: "confirm", detail: "rule" });
});

test("older detail responses are ignored after supersession and pending intents", async () => {
  const first = deferred<string | null>();
  const second = deferred<string | null>();
  let current = 1;
  const old = resolveRuleDetailRequest(1, () => current, () => first.promise, () => ({ isOpen: false, isDirty: false }));
  current = 2;
  const latest = resolveRuleDetailRequest(2, () => current, () => second.promise, () => ({ isOpen: false, isDirty: false }));
  second.resolve("new");
  first.resolve("old");
  assert.deepEqual(await latest, { type: "open", detail: "new" });
  assert.deepEqual(await old, { type: "ignore" });
  for (const intent of ["close", "create", "template", "import"]) {
    const pending = deferred<string | null>();
    const request = current;
    const result = resolveRuleDetailRequest(request, () => current, () => pending.promise, () => ({ isOpen: false, isDirty: false }));
    current += 1;
    pending.resolve("old");
    assert.equal((await result).type, "ignore", intent);
  }
});
