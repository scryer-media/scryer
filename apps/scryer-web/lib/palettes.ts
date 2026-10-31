import type { UiPalette, UiSettings } from "./types/settings.ts";
import { fromUiThemeValue, type ThemePreference } from "./theme.ts";

export const PALETTES = [
  {
    id: "CLASSIC",
    labelKey: "palette.scryerDefault",
    mode: null,
    background: "#070d1c",
    surface: "#0a1120",
    accent: "#5b64ff",
    ink: "#dbe5ff",
  },
  {
    id: "SLATE",
    labelKey: "palette.slate",
    mode: "light",
    background: "#edf0f4",
    surface: "#ffffff",
    accent: "#315f9c",
    ink: "#202b3a",
  },
  {
    id: "OCEAN",
    labelKey: "palette.ocean",
    mode: "dark",
    background: "#061b26",
    surface: "#0b2937",
    accent: "#087e98",
    ink: "#e1f5fa",
  },
  {
    id: "FOREST",
    labelKey: "palette.forest",
    mode: "dark",
    background: "#101e19",
    surface: "#1a2c23",
    accent: "#25835a",
    ink: "#e7f3e9",
  },
  {
    id: "PLUM",
    labelKey: "palette.plum",
    mode: "dark",
    background: "#211426",
    surface: "#302039",
    accent: "#9254bb",
    ink: "#f7eafa",
  },
  {
    id: "EMBER",
    labelKey: "palette.ember",
    mode: "light",
    background: "#f7efe4",
    surface: "#fffaf3",
    accent: "#b64b1e",
    ink: "#38271e",
  },
  {
    id: "GRAPHITE",
    labelKey: "palette.graphite",
    mode: "dark",
    background: "#1a1b1e",
    surface: "#1f2024",
    accent: "#3fb39c",
    ink: "#ecebe6",
  },
  {
    id: "PAPYRUS",
    labelKey: "palette.papyrus",
    mode: "light",
    background: "#d8d0c4",
    surface: "#ece6dc",
    accent: "#80634b",
    ink: "#37312b",
  },
] as const;

export function normalizePalette(value: unknown): UiPalette {
  // Both saved identifiers refer to the single default appearance.
  if (value === "SCRYER_DEFAULT") return "CLASSIC";
  return PALETTES.find((palette) => palette.id === value)?.id ?? "CLASSIC";
}

export function getPalette(value: unknown) {
  return PALETTES.find((palette) => palette.id === normalizePalette(value))!;
}

export function effectiveTheme(
  settings: Pick<UiSettings, "palette" | "theme">,
): ThemePreference {
  return getPalette(settings.palette).mode ?? fromUiThemeValue(settings.theme);
}
