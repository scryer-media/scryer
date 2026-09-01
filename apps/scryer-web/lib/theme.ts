export const SELECTABLE_THEMES = ["light", "dark"] as const;

export const THEME_CYCLE_ORDER = [...SELECTABLE_THEMES, "system"] as const;

export type ThemePreference = (typeof THEME_CYCLE_ORDER)[number];

export function getNextTheme(theme?: string): ThemePreference {
  const currentTheme = THEME_CYCLE_ORDER.includes(theme as ThemePreference)
    ? (theme as ThemePreference)
    : "dark";
  const currentIndex = THEME_CYCLE_ORDER.indexOf(currentTheme);
  return THEME_CYCLE_ORDER[(currentIndex + 1) % THEME_CYCLE_ORDER.length];
}

export function getThemeLabel(theme?: string): string {
  switch (theme) {
    case "light":
      return "Light";
    case "dark":
      return "Dark";
    case "pride":
      return "Pride";
    default:
      return "System";
  }
}

export function isDarkTheme(theme?: string): boolean {
  return theme === "dark" || theme === "pride";
}

/** Server enum wire value for the UI theme (mirrors GraphQL `UiThemeValue`). */
export type UiThemeValue = "LIGHT" | "DARK" | "PRIDE" | "SYSTEM";

/**
 * Map a next-themes lowercase preference (the client theme-provider / CSS-class
 * identifier) to the server `UiThemeValue` enum sent over GraphQL.
 */
export function toUiThemeValue(theme?: string): UiThemeValue {
  switch (theme) {
    case "light":
      return "LIGHT";
    case "pride":
      return "PRIDE";
    case "system":
      return "SYSTEM";
    default:
      return "DARK";
  }
}

/**
 * Map a server `UiThemeValue` enum to the next-themes lowercase preference used
 * by the client theme provider and CSS classes.
 */
export function fromUiThemeValue(value?: string): "light" | "dark" | "pride" | "system" {
  switch (value) {
    case "LIGHT":
      return "light";
    case "PRIDE":
      return "pride";
    case "SYSTEM":
      return "system";
    default:
      return "dark";
  }
}

export type HighlightColorPreset = {
  /** Canonical #rrggbb value persisted to `highlightColor`. */
  value: string;
  /** i18n key for the human-readable color name. */
  labelKey: string;
};

/**
 * Preset accent colors offered in the Settings > Profile appearance picker,
 * mirroring the design handoff's "Highlight color" swatch row. The first entry
 * (`#5b64ff`) matches the built-in default accent.
 */
export const HIGHLIGHT_COLOR_PRESETS: readonly HighlightColorPreset[] = [
  { value: "#5b64ff", labelKey: "color.indigo" },
  { value: "#8b5cf6", labelKey: "color.violet" },
  { value: "#3b82f6", labelKey: "color.sky" },
  { value: "#14b8a6", labelKey: "color.teal" },
  { value: "#10b981", labelKey: "color.emerald" },
  { value: "#16a34a", labelKey: "color.green" },
  { value: "#06b6d4", labelKey: "color.cyan" },
  { value: "#f43f5e", labelKey: "color.rose" },
  { value: "#dc2626", labelKey: "color.red" },
  { value: "#c71684", labelKey: "color.fuchsia" },
  { value: "#d60270", labelKey: "color.magenta" },
  { value: "#e8512f", labelKey: "color.crab" },
] as const;

const HEX_COLOR_PATTERN = /^#[0-9a-f]{6}$/i;

/** CSS custom properties derived from a single highlight color. */
const ACCENT_CSS_VARS = [
  "--scry-accent",
  "--scry-accent-rgb",
  "--scry-accent2",
  "--scry-accent-dark",
  "--scry-accent-ring",
  "--scry-accent-text",
  "--scry-accent-grad",
  "--scry-baccent",
] as const;

export function isHighlightColor(value: string | null | undefined): value is string {
  return typeof value === "string" && HEX_COLOR_PATTERN.test(value);
}

type Rgb = { r: number; g: number; b: number };
type Hsl = { h: number; s: number; l: number };

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

function hexToRgb(hex: string): Rgb {
  const value = hex.replace("#", "");
  return {
    r: parseInt(value.slice(0, 2), 16),
    g: parseInt(value.slice(2, 4), 16),
    b: parseInt(value.slice(4, 6), 16),
  };
}

function rgbToHsl({ r, g, b }: Rgb): Hsl {
  const rn = r / 255;
  const gn = g / 255;
  const bn = b / 255;
  const max = Math.max(rn, gn, bn);
  const min = Math.min(rn, gn, bn);
  const delta = max - min;
  let h = 0;
  if (delta !== 0) {
    if (max === rn) h = ((gn - bn) / delta) % 6;
    else if (max === gn) h = (bn - rn) / delta + 2;
    else h = (rn - gn) / delta + 4;
    h = h * 60;
    if (h < 0) h += 360;
  }
  const l = (max + min) / 2;
  const s = delta === 0 ? 0 : delta / (1 - Math.abs(2 * l - 1));
  return { h, s: s * 100, l: l * 100 };
}

function hslToHex({ h, s, l }: Hsl): string {
  const sn = clamp(s, 0, 100) / 100;
  const ln = clamp(l, 0, 100) / 100;
  const c = (1 - Math.abs(2 * ln - 1)) * sn;
  const hp = ((((h % 360) + 360) % 360)) / 60;
  const x = c * (1 - Math.abs((hp % 2) - 1));
  let r: number;
  let g: number;
  let b: number;
  if (hp < 1) [r, g, b] = [c, x, 0];
  else if (hp < 2) [r, g, b] = [x, c, 0];
  else if (hp < 3) [r, g, b] = [0, c, x];
  else if (hp < 4) [r, g, b] = [0, x, c];
  else if (hp < 5) [r, g, b] = [x, 0, c];
  else [r, g, b] = [c, 0, x];
  const m = ln - c / 2;
  const toHex = (channel: number) =>
    Math.round((channel + m) * 255)
      .toString(16)
      .padStart(2, "0");
  return `#${toHex(r)}${toHex(g)}${toHex(b)}`;
}

type HslAdjustment = {
  /** Hue rotation in degrees. */
  rotate?: number;
  /** Relative lightness delta (percentage points). */
  lighten?: number;
  /** Relative saturation delta (percentage points). */
  saturate?: number;
  /** Absolute lightness (percentage); overrides `lighten`. */
  lightness?: number;
  /** Absolute saturation (percentage); overrides `saturate`. */
  saturation?: number;
};

function adjust(hsl: Hsl, change: HslAdjustment): Hsl {
  return {
    h: hsl.h + (change.rotate ?? 0),
    s:
      change.saturation !== undefined
        ? change.saturation
        : clamp(hsl.s + (change.saturate ?? 0), 0, 100),
    l:
      change.lightness !== undefined
        ? change.lightness
        : clamp(hsl.l + (change.lighten ?? 0), 0, 100),
  };
}

/**
 * Derive the full `--scry-accent*` token family from a single highlight color.
 * `isDark` selects on-accent text/border shades that read well against the
 * active theme background (mirroring the hand-tuned light vs dark defaults in
 * globals.css). The primary `--scry-accent` keeps the exact chosen hex; the
 * variants are computed in HSL space so any hue stays internally consistent.
 */
export function deriveAccentCssVars(
  highlightColor: string,
  isDark: boolean,
): Record<(typeof ACCENT_CSS_VARS)[number], string> {
  const rgb = hexToRgb(highlightColor);
  const hsl = rgbToHsl(rgb);
  const accent = `#${highlightColor.replace("#", "").toLowerCase()}`;
  const accent2 = hslToHex(adjust(hsl, { rotate: 14, lighten: 2 }));
  return {
    "--scry-accent": accent,
    "--scry-accent-rgb": `${rgb.r}, ${rgb.g}, ${rgb.b}`,
    "--scry-accent2": accent2,
    "--scry-accent-dark": hslToHex(adjust(hsl, { lighten: -9, saturate: -6 })),
    "--scry-accent-ring": hslToHex(adjust(hsl, { lighten: 8, saturate: -4 })),
    "--scry-accent-text": isDark
      ? hslToHex(adjust(hsl, { lightness: 82, saturate: -4 }))
      : hslToHex(adjust(hsl, { lighten: -10, saturate: -18 })),
    "--scry-accent-grad": `linear-gradient(135deg, ${accent}, ${accent2})`,
    "--scry-baccent": isDark
      ? hslToHex(adjust(hsl, { lightness: 30, saturation: 42 }))
      : hslToHex(adjust(hsl, { lightness: 85, saturation: 42 })),
  };
}

/**
 * Apply or clear the user's highlight color on the document root. A null or
 * invalid color removes the overrides so the active theme's built-in accent
 * (from globals.css) takes over again.
 */
export function applyHighlightColor(
  root: HTMLElement,
  highlightColor: string | null | undefined,
  isDark: boolean,
): void {
  if (!isHighlightColor(highlightColor)) {
    for (const cssVar of ACCENT_CSS_VARS) {
      root.style.removeProperty(cssVar);
    }
    return;
  }
  const vars = deriveAccentCssVars(highlightColor, isDark);
  for (const cssVar of ACCENT_CSS_VARS) {
    root.style.setProperty(cssVar, vars[cssVar]);
  }
}
