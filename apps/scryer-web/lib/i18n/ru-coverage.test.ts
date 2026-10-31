import assert from "node:assert/strict";
import test from "node:test";
import en from "./locales/en.ts";
import ru from "./locales/ru.ts";

const placeholders = (value: string) =>
  [...value.matchAll(/\{\{([^{}]+)\}\}/g)].map((match) => match[1]).sort();

test("Russian covers every English fallback key with a nonempty translation", () => {
  const missing = Object.keys(en).filter((key) => !ru[key]?.trim());
  assert.deepEqual(
    missing,
    [],
    `${missing.length} Russian translations missing`,
  );
});

test("Russian translations preserve interpolation parameters", () => {
  const mismatches = Object.keys(en).filter(
    (key) =>
      ru[key] !== undefined &&
      JSON.stringify(placeholders(en[key]!)) !==
        JSON.stringify(placeholders(ru[key]!)),
  );
  assert.deepEqual(mismatches, []);
});
