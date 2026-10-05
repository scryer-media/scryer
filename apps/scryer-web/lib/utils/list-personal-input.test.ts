import assert from "node:assert/strict";
import test from "node:test";
import { draftToSubscribeInput, emptyListDraft, isListModeSelectable, PUBLIC_LIST_MODES } from "./lists.ts";

test("personal subscriptions carry only the selected account id and source parameters", () => {
  const draft = { ...emptyListDraft("Member list", ["MOVIE"]), mode: "REQUEST" as const };
  const input = draftToSubscribeInput({ provider: "trakt", sourceType: "my_list", params: [{ key: "list_id", value: "synthetic-list" }], url: null, credentialId: "owned-account" }, draft);
  assert.equal(input.scope, "PERSONAL");
  assert.equal(input.credentialId, "owned-account");
  assert.equal(input.mode, "REQUEST");
  assert.deepEqual(input.params, [{ key: "list_id", value: "synthetic-list" }]);
  assert.ok(isListModeSelectable(input.mode));
  assert.equal(PUBLIC_LIST_MODES.includes("REQUEST"), false);
  const publicInput = draftToSubscribeInput({ provider: "tmdb", sourceType: "popular", params: [], url: null }, emptyListDraft("Public chart", ["MOVIE"]));
  assert.equal(publicInput.scope, "PUBLIC");
  assert.equal(Object.hasOwn(publicInput, "credentialId"), false);
});
