import assert from "node:assert/strict";
import { test } from "node:test";
import {
  listRatingInputText,
  parseListRatingInput,
} from "./list-rating-input.ts";

test("rating keystrokes preserve decimal points and trailing zeroes", () => {
  let text = "";
  for (const key of ["7", ".", "5", "0"]) {
    const next = parseListRatingInput(text + key);
    assert.ok(next);
    text = listRatingInputText(next.text, next.value);
  }
  assert.equal(text, "7.50");
  assert.equal(parseListRatingInput(text)?.value, 7.5);
});

test("ratings accept leading decimals, zero, clearing, and pasted decimals", () => {
  for (const [text, value] of [
    [".", null],
    [".5", 0.5],
    ["0", 0],
    ["", null],
    ["3.5", 3.5],
  ] as const) {
    assert.deepEqual(parseListRatingInput(text), { text, value });
    assert.equal(listRatingInputText(text, value), text);
  }
});

test("external rating changes replace stale drafts and invalid input is rejected", () => {
  assert.equal(listRatingInputText("7.", 8), "8");
  assert.equal(listRatingInputText("7.5", null), "");
  for (const text of ["7..5", "1e2", "-1", "words"])
    assert.equal(parseListRatingInput(text), null);
});
