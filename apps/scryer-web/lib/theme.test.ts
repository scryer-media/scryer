import assert from "node:assert/strict";
import test from "node:test";
import { runInNewContext } from "node:vm";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { ThemeProvider } from "next-themes";
import {
  fromUiThemeValue,
  getNextTheme,
  isDarkTheme,
  migrateStoredTheme,
  SELECTABLE_THEMES,
  THEME_CLASS_NAMES,
  toUiThemeValue,
} from "./theme.ts";

test("active themes retain light, dark, and system behavior", () => {
  assert.deepEqual(SELECTABLE_THEMES, ["light", "dark"]);
  for (const theme of ["light", "dark", "system"] as const) {
    assert.equal(fromUiThemeValue(toUiThemeValue(theme)), theme);
  }
  assert.equal(getNextTheme("light"), "dark");
  assert.equal(getNextTheme("dark"), "system");
  assert.equal(getNextTheme("system"), "light");
  assert.equal(isDarkTheme("light"), false);
  assert.equal(isDarkTheme("dark"), true);
});

test("retired theme values normalize to dark and are never emitted", () => {
  assert.equal(fromUiThemeValue("PRIDE"), "dark");
  assert.equal(toUiThemeValue("pride"), "DARK");
  assert.equal(getNextTheme("pride"), "system");
  assert.equal(isDarkTheme("pride"), true);
  assert.equal(THEME_CLASS_NAMES.pride, "dark");
});

test("stored theme migration is idempotent and leaves other preferences alone", () => {
  for (const initial of ["pride", "light", "dark", "system", null]) {
    let value = initial;
    let writes = 0;
    const storage = {
      getItem(key: string) { assert.equal(key, "theme"); return value; },
      setItem(key: string, next: string) {
        assert.equal(key, "theme");
        value = next;
        writes += 1;
      },
    };
    migrateStoredTheme(storage);
    migrateStoredTheme(storage);
    assert.equal(value, initial === "pride" ? "dark" : initial);
    assert.equal(writes, initial === "pride" ? 1 : 0);
  }
});

test("provider bootstrap applies dark for legacy read-only storage and preserves system themes", () => {
  const markup = renderToStaticMarkup(createElement(ThemeProvider, {
    attribute: "class",
    defaultTheme: "dark",
    enableSystem: true,
    themes: [...SELECTABLE_THEMES],
    value: THEME_CLASS_NAMES,
  }));
  const script = markup.match(/<script[^>]*>([\s\S]*?)<\/script>/)?.[1];
  assert.ok(script, "provider must render its initial theme script");
  for (const [stored, systemDark, expected] of [
    ["pride", false, "dark"],
    ["light", true, "light"],
    ["dark", false, "dark"],
    ["system", true, "dark"],
    ["system", false, "light"],
    [null, false, "dark"],
  ] as const) {
    const classes = new Set(["light"]);
    runInNewContext(script, {
      localStorage: {
        getItem: () => stored,
        setItem: () => { throw new Error("read-only storage"); },
      },
      window: { matchMedia: () => ({ matches: systemDark }) },
      document: {
        documentElement: {
          classList: {
            remove: (...names: string[]) => names.forEach((name) => classes.delete(name)),
            add: (name: string) => classes.add(name),
          },
          style: {},
        },
      },
    });
    assert.deepEqual([...classes], [expected]);
  }
});
