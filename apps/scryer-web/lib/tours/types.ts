import type { usePermissions } from "@/lib/hooks/use-permissions";

/**
 * What a step or tour may be gated on. Gates read this and nothing else, so a
 * tour never shows a stop the viewer cannot use.
 */
export type TourGateContext = {
  permissions: ReturnType<typeof usePermissions>;
  /** The viewport is phone-sized and the sidebar is folded away. */
  isMobile: boolean;
};

export type TourGate = (context: TourGateContext) => boolean;

/**
 * An element a step points at, named by the parts of its `selectorId`, e.g.
 * `["root-sidebar-nav", "requests"]`. Only durable ids are targeted, never
 * text, roles or CSS paths, so a copy or layout change cannot silently move a
 * spotlight.
 */
export type TourTarget = readonly string[];

/** Illustrations a step may show in its card when it has no spotlight. */
export type TourIcon =
  | "sparkles"
  | "search"
  | "library"
  | "requests"
  | "activity"
  | "calendar"
  | "settings"
  | "rules"
  | "maintenance"
  | "lists"
  | "discover"
  | "system"
  | "users"
  | "notifications"
  | "plugins";

export type TourStep = {
  /** Unique within its tour; used for keys and diagnostics. */
  id: string;
  /** Translation key of the card's heading. */
  titleKey: string;
  /** Translation key of the card's text. */
  bodyKey: string;
  icon?: TourIcon;
  /**
   * Element to spotlight. When it is absent, or does not appear in time, the
   * card shows centered on its own; the step still reads correctly.
   */
  target?: TourTarget;
  /**
   * What to spotlight instead on a phone, where the same control is often a
   * different element (e.g. the search button that stands in for the bar).
   */
  mobileTarget?: TourTarget;
  /** Route to open before the step shows, when its target lives elsewhere. */
  route?: string;
  /**
   * A button that opens a route and ends the tour as completed, e.g. "Open
   * request rules" on the last stop of a highlight tour.
   */
  action?: { labelKey: string; route: string };
  /** The step is left out when this returns false. */
  when?: TourGate;
};

export type TourKind = "onboarding" | "highlights";

export type TourDefinition = {
  /**
   * Stored with the viewer once the tour is finished or skipped: 1 to 64
   * lowercase letters, digits, dots or hyphens. Never reuse or rename a
   * shipped id; a materially new tour gets a new one.
   */
  id: string;
  kind: TourKind;
  /**
   * For a highlights tour, the release it introduces, e.g. `0.23.0`. The
   * newest unseen highlights tour is the one offered.
   */
  release?: string;
  /** The tour is not offered when this returns false. */
  when?: TourGate;
  steps: readonly TourStep[];
};
