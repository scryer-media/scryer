import assert from "node:assert/strict";
import test from "node:test";
import { PALETTES, effectiveTheme, normalizePalette } from "./palettes.ts";
import { applyHighlightColor, deriveAccentCssVars } from "./theme.ts";

test("fixed palettes choose their own mode without changing the saved Classic mode", () => {
  for (const theme of ["LIGHT", "DARK", "SYSTEM"] as const) {
    for (const palette of PALETTES) {
      const settings = { theme, palette: palette.id };
      assert.equal(effectiveTheme(settings), palette.mode ?? theme.toLowerCase());
      assert.equal(settings.theme, theme);
      assert.equal(effectiveTheme({ ...settings, palette: "CLASSIC" }), theme.toLowerCase());
    }
  }
});

test("unknown or missing palettes fall back to Classic", () => {
  for (const value of [undefined, null, "CUSTOM", "", "ocean"]) {
    assert.equal(normalizePalette(value), "CLASSIC");
  }
  for (const palette of PALETTES) assert.equal(normalizePalette(palette.id), palette.id);
});

test("both saved default identifiers resolve to one appearance and retain the mode preference", () => {
  assert.equal(PALETTES.filter((palette) => palette.labelKey === "palette.scryerDefault").length, 1);
  for (const palette of ["CLASSIC", "SCRYER_DEFAULT"] as const) {
    assert.equal(normalizePalette(palette), "CLASSIC");
    for (const theme of ["LIGHT", "DARK", "SYSTEM"] as const) {
      assert.equal(effectiveTheme({ palette, theme }), theme.toLowerCase());
    }
  }
});

test("accent reset removes every inline override so the selected palette takes over", () => {
  const properties = new Map<string, string>();
  const root = { style: {
    setProperty: (name: string, value: string) => properties.set(name, value),
    removeProperty: (name: string) => properties.delete(name),
  } } as unknown as HTMLElement;
  applyHighlightColor(root, "#ffffff", true);
  assert.equal(properties.get("--primary-foreground"), "#000000");
  assert.equal(properties.get("--scry-accent"), "#ffffff");
  applyHighlightColor(root, "#000000", false);
  assert.equal(properties.get("--primary-foreground"), "#ffffff");
  applyHighlightColor(root, null, false);
  assert.equal(properties.size, 0);
  applyHighlightColor(root, "#ffffff", true);
  applyHighlightColor(root, "invalid", true);
  assert.equal(properties.size, 0);
});

test("preset accent button text has readable contrast", () => {
  const luminance = (hex: string) => {
    const channels = [1, 3, 5].map((offset) => {
      const value = parseInt(hex.slice(offset, offset + 2), 16) / 255;
      return value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
    });
    return channels[0] * 0.2126 + channels[1] * 0.7152 + channels[2] * 0.0722;
  };
  for (const palette of PALETTES) {
    const background = luminance(palette.accent);
    const foreground = luminance(deriveAccentCssVars(palette.accent, palette.mode !== "light")["--primary-foreground"]);
    assert.ok((Math.max(background, foreground) + 0.05) / (Math.min(background, foreground) + 0.05) >= 4.5, palette.id);
  }
});
