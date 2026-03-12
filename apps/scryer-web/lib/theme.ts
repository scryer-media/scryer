export const SELECTABLE_THEMES = ["light", "dark", "pride"] as const;

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
