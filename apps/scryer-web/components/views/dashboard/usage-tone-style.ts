import type { UsageTag, UsageTone } from "@/lib/utils/dashboard";

/**
 * Maps a `UsageTone` onto the app's existing semantic tokens, the same way
 * `components/setup/import/facet-style.ts` maps a facet onto the facet tokens.
 *
 * Nothing here introduces a colour: `solid`/`rgb`/`text` name the
 * `--scry-success|warning|danger-*` variables already used by badges, progress
 * bars and status cells, so light mode and a re-themed accent follow along with
 * no edits here.
 */
export type UsageToneStyle = {
  /** Filled portion of a bar or ring. */
  solid: string;
  /** Comma-separated channels, for the translucent remainder of a ring. */
  rgb: string;
  /** Readable text in the same family. */
  text: string;
};

const USAGE_TONE_STYLES: Record<UsageTone["tone"], UsageToneStyle> = {
  success: {
    solid: "var(--scry-success-solid)",
    rgb: "var(--scry-success-rgb)",
    text: "var(--scry-success-text)",
  },
  warning: {
    solid: "var(--scry-warning-solid)",
    rgb: "var(--scry-warning-rgb)",
    text: "var(--scry-warning-text)",
  },
  danger: {
    solid: "var(--scry-danger-solid)",
    rgb: "var(--scry-danger-rgb)",
    text: "var(--scry-danger-text)",
  },
};

export function usageToneStyle(tone: UsageTone["tone"]): UsageToneStyle {
  return USAGE_TONE_STYLES[tone];
}

/** Badge tone for the threshold tag, or null when the tag is not shown. */
export function usageTagBadgeTone(
  tag: UsageTag,
): "warning" | "negative" | null {
  switch (tag) {
    case "low":
      return "warning";
    case "crit":
      return "negative";
    default:
      return null;
  }
}

/** i18n key for the threshold tag, or null when the tag is not shown. */
export function usageTagLabelKey(tag: UsageTag): string | null {
  switch (tag) {
    case "low":
      return "dashboard.tagLow";
    case "crit":
      return "dashboard.tagCrit";
    default:
      return null;
  }
}
