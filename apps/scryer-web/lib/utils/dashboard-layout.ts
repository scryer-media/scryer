import type {
  UiDashboardBreakpoint,
  UiDashboardLayout,
  UiDashboardLayoutItem,
} from "../types/settings.ts";

/**
 * The dashboard grid's data: which panels exist, how small each may get, how
 * its content density follows its size, and the default arrangement at every
 * breakpoint. Icons and renderers are bound to these ids in
 * `components/views/dashboard/dashboard-panel-registry.tsx`; this half stays
 * free of React so it can be tested on its own.
 *
 * What stays outside the grid: the greeting header and the plugin-update strip.
 * Both are alert surfaces. A user must never be able to hide or bury the
 * notice that something needs attention, so they stay fixed above the grid.
 *
 * The three stat tiles form one panel. They read as a single summary strip, and
 * today's `1.25fr 1fr 1fr` split cannot be expressed in a column grid of 24
 * or fewer columns that also halves and thirds evenly, so the tiles keep their
 * own inner grid and the panel spans the full width by default.
 *
 * No panel is permission-gated on its own: the whole dashboard route requires
 * Manage System Settings, and the one gated control inside a panel (indexer
 * error history) keeps its own check in the panel.
 */

export type DashboardBreakpoint = UiDashboardBreakpoint;
export type DashboardTier = "compact" | "regular" | "expanded";
export type DashboardGridSize = { w: number; h: number };
export type DashboardLayoutItem = UiDashboardLayoutItem;
export type DashboardLayouts = Record<
  DashboardBreakpoint,
  DashboardLayoutItem[]
>;

/** Smallest to largest. */
export const DASHBOARD_BREAKPOINTS: readonly DashboardBreakpoint[] = [
  "XS",
  "SM",
  "MD",
  "LG",
];

/**
 * Smallest viewport width, in CSS pixels, at which each breakpoint applies.
 * These are the media queries the fixed dashboard used (`min-[701px]`,
 * `min-[1241px]`, `min-[1501px]`), read from the window exactly as they were,
 * so every viewport gets the arrangement it had before. The grid still sizes
 * its columns from the dashboard's own measured width.
 */
export const DASHBOARD_BREAKPOINT_MIN_WIDTH: Readonly<
  Record<DashboardBreakpoint, number>
> = {
  XS: 0,
  SM: 701,
  MD: 1241,
  LG: 1501,
};

/**
 * Columns per breakpoint. Twelve halves and thirds evenly for the two- and
 * three-across rows at MD and LG; SM stacks full-width panels but keeps six
 * columns so a user can still pair panels; XS is a single column.
 */
export const DASHBOARD_COLUMNS: Readonly<Record<DashboardBreakpoint, number>> =
  {
    XS: 1,
    SM: 6,
    MD: 12,
    LG: 12,
  };

/** Pixel height of one grid row; with the margin, one row step is 32px. */
export const DASHBOARD_ROW_HEIGHT = 20;
/** Gap between panels, matching the old `gap-3`. */
export const DASHBOARD_MARGIN: readonly [number, number] = [12, 12];

/** Server limits: the grid never builds a layout these refuse. */
export const DASHBOARD_MAX_COLUMNS = 24;
export const DASHBOARD_MAX_ITEMS = 64;
const PANEL_ID_PATTERN = /^[a-z0-9-]+$/;
const PANEL_ID_MAX_LENGTH = 64;

export const DASHBOARD_PANEL_IDS = [
  "stats",
  "requests",
  "manual-imports",
  "recently-imported",
  "indexers",
  "download-clients",
  "storage",
  "active-queue",
] as const;

export type DashboardPanelId = (typeof DASHBOARD_PANEL_IDS)[number];

type PerBreakpoint<T> = Readonly<Record<DashboardBreakpoint, T>>;

/**
 * Size thresholds for the two richer tiers at one breakpoint. A panel renders
 * `expanded` when it is at least `expanded` in both dimensions, otherwise
 * `regular` when at least `regular`, otherwise `compact`. Null means the tier
 * is out of reach at that breakpoint.
 */
export type DashboardTierThresholds = {
  regular: DashboardGridSize | null;
  expanded: DashboardGridSize | null;
};

export type DashboardPanelSpec = {
  titleKey: string;
  descriptionKey: string;
  /** Smallest size the content still reads at, per breakpoint. */
  min: PerBreakpoint<DashboardGridSize>;
  tiers: PerBreakpoint<DashboardTierThresholds>;
};

const size = (w: number, h: number): DashboardGridSize => ({ w, h });

/** Lists of rows (requests, imports, recent imports, queue). */
const LIST_TIERS: PerBreakpoint<DashboardTierThresholds> = {
  XS: { regular: size(1, 1), expanded: null },
  SM: { regular: size(4, 5), expanded: null },
  MD: { regular: size(5, 5), expanded: size(9, 7) },
  LG: { regular: size(4, 5), expanded: size(7, 7) },
};

/** Provider tables need more width before their extra columns fit. */
const TABLE_TIERS: PerBreakpoint<DashboardTierThresholds> = {
  XS: { regular: size(1, 1), expanded: null },
  SM: { regular: size(5, 4), expanded: null },
  MD: { regular: size(6, 4), expanded: size(9, 4) },
  LG: { regular: size(5, 4), expanded: size(8, 4) },
};

export const DASHBOARD_PANELS: Readonly<
  Record<DashboardPanelId, DashboardPanelSpec>
> = {
  // Compact stacks the tiles, regular sets them two across and expanded three
  // across. XS only ever stacks; SM tops out at two across, as before.
  stats: {
    titleKey: "dashboard.panel.stats",
    descriptionKey: "dashboard.panel.statsDescription",
    min: { XS: size(1, 2), SM: size(3, 2), MD: size(3, 2), LG: size(3, 2) },
    tiers: {
      XS: { regular: null, expanded: null },
      SM: { regular: size(4, 2), expanded: null },
      MD: { regular: size(5, 2), expanded: size(9, 2) },
      LG: { regular: size(5, 2), expanded: size(9, 2) },
    },
  },
  requests: {
    titleKey: "nav.requests",
    descriptionKey: "dashboard.panel.requestsDescription",
    min: { XS: size(1, 5), SM: size(3, 5), MD: size(4, 5), LG: size(3, 5) },
    tiers: LIST_TIERS,
  },
  "manual-imports": {
    titleKey: "dashboard.manualImports",
    descriptionKey: "dashboard.panel.manualImportsDescription",
    min: { XS: size(1, 5), SM: size(3, 5), MD: size(4, 5), LG: size(3, 5) },
    tiers: LIST_TIERS,
  },
  "recently-imported": {
    titleKey: "dashboard.recentlyImported",
    descriptionKey: "dashboard.panel.recentlyImportedDescription",
    min: { XS: size(1, 5), SM: size(3, 5), MD: size(4, 5), LG: size(3, 5) },
    tiers: LIST_TIERS,
  },
  indexers: {
    titleKey: "settings.indexers",
    descriptionKey: "dashboard.panel.indexersDescription",
    min: { XS: size(1, 5), SM: size(3, 5), MD: size(4, 5), LG: size(3, 5) },
    tiers: TABLE_TIERS,
  },
  "download-clients": {
    titleKey: "settings.downloadClients",
    descriptionKey: "dashboard.panel.downloadClientsDescription",
    min: { XS: size(1, 5), SM: size(3, 5), MD: size(4, 5), LG: size(3, 5) },
    tiers: TABLE_TIERS,
  },
  // Storage tiles wrap at about 196px, so the panel can be narrower than a list.
  storage: {
    titleKey: "dashboard.storage",
    descriptionKey: "dashboard.panel.storageDescription",
    min: { XS: size(1, 4), SM: size(2, 4), MD: size(3, 4), LG: size(2, 4) },
    tiers: {
      XS: { regular: size(1, 1), expanded: null },
      SM: { regular: size(3, 4), expanded: null },
      MD: { regular: size(5, 5), expanded: size(9, 8) },
      LG: { regular: size(4, 5), expanded: size(8, 8) },
    },
  },
  "active-queue": {
    titleKey: "dashboard.activeQueue",
    descriptionKey: "dashboard.panel.activeQueueDescription",
    min: { XS: size(1, 5), SM: size(3, 5), MD: size(4, 5), LG: size(3, 5) },
    tiers: LIST_TIERS,
  },
};

const item = (
  panelId: DashboardPanelId,
  x: number,
  y: number,
  w: number,
  h: number,
): DashboardLayoutItem => ({
  panelId,
  x,
  y,
  w,
  h,
});

/**
 * The default arrangement, transcribed from the fixed Tailwind grids it
 * replaces (the same viewport breakpoints, so each width keeps its tier):
 *
 * - Stat tiles: one full-width panel at every breakpoint. Its tiles were
 *   `grid-cols-1` below 701, `grid-cols-2` from 701 and three across from
 *   1081; the panel's tier reproduces that (stacked, two, three).
 * - Requests · Manual imports · Recently imported:
 *   `grid-cols-1 min-[1241px]:grid-cols-2 min-[1501px]:grid-cols-3`, so one
 *   per row at XS and SM, two across at MD (recently imported alone in the
 *   left half of the second row) and three across at LG.
 * - Indexers · Download clients, then Storage · Active queue:
 *   `grid-cols-1 min-[1241px]:grid-cols-2`, so stacked at XS and SM and side
 *   by side at MD and LG.
 *
 * Heights are in 32px row steps: 2 rows (52px) holds the one-line tiles, 7
 * (212px) the old 172px preview body plus header, 9 (276px) the 218px provider
 * tables and 10 (308px) five queue rows.
 */
export const DEFAULT_DASHBOARD_LAYOUTS: Readonly<DashboardLayouts> = {
  LG: [
    item("stats", 0, 0, 12, 2),
    item("requests", 0, 2, 4, 7),
    item("manual-imports", 4, 2, 4, 7),
    item("recently-imported", 8, 2, 4, 7),
    item("indexers", 0, 9, 6, 9),
    item("download-clients", 6, 9, 6, 9),
    item("storage", 0, 18, 6, 10),
    item("active-queue", 6, 18, 6, 10),
  ],
  MD: [
    item("stats", 0, 0, 12, 2),
    item("requests", 0, 2, 6, 7),
    item("manual-imports", 6, 2, 6, 7),
    item("recently-imported", 0, 9, 6, 7),
    item("indexers", 0, 16, 6, 9),
    item("download-clients", 6, 16, 6, 9),
    item("storage", 0, 25, 6, 10),
    item("active-queue", 6, 25, 6, 10),
  ],
  SM: [
    item("stats", 0, 0, 6, 4),
    item("requests", 0, 4, 6, 7),
    item("manual-imports", 0, 11, 6, 7),
    item("recently-imported", 0, 18, 6, 7),
    item("indexers", 0, 25, 6, 9),
    item("download-clients", 0, 34, 6, 9),
    item("storage", 0, 43, 6, 10),
    item("active-queue", 0, 53, 6, 10),
  ],
  XS: [
    item("stats", 0, 0, 1, 6),
    item("requests", 0, 6, 1, 7),
    item("manual-imports", 0, 13, 1, 7),
    item("recently-imported", 0, 20, 1, 7),
    item("indexers", 0, 27, 1, 9),
    item("download-clients", 0, 36, 1, 9),
    item("storage", 0, 45, 1, 10),
    item("active-queue", 0, 55, 1, 10),
  ],
};

export function isDashboardPanelId(value: string): value is DashboardPanelId {
  return Object.hasOwn(DASHBOARD_PANELS, value);
}

/** Whether a string is a panel id the server will accept at all. */
export function isWellFormedPanelId(value: string): boolean {
  return value.length <= PANEL_ID_MAX_LENGTH && PANEL_ID_PATTERN.test(value);
}

/** The breakpoint for a viewport width, as the old `min-[…px]` queries chose it. */
export function dashboardBreakpointForWidth(
  width: number,
): DashboardBreakpoint {
  let matching: DashboardBreakpoint = "XS";
  for (const breakpoint of DASHBOARD_BREAKPOINTS) {
    if (width >= DASHBOARD_BREAKPOINT_MIN_WIDTH[breakpoint]) {
      matching = breakpoint;
    }
  }
  return matching;
}

function fits(
  current: DashboardGridSize,
  threshold: DashboardGridSize | null,
): boolean {
  return (
    threshold !== null && current.w >= threshold.w && current.h >= threshold.h
  );
}

/** How much a panel shows at its current size. */
export function dashboardPanelTier(
  panelId: DashboardPanelId,
  breakpoint: DashboardBreakpoint,
  current: DashboardGridSize,
): DashboardTier {
  const thresholds = DASHBOARD_PANELS[panelId].tiers[breakpoint];
  if (fits(current, thresholds.expanded)) {
    return "expanded";
  }
  if (fits(current, thresholds.regular)) {
    return "regular";
  }
  return "compact";
}

/** Size a panel takes when added: its default at that breakpoint. */
export function dashboardPanelDefaultSize(
  panelId: DashboardPanelId,
  breakpoint: DashboardBreakpoint,
): DashboardGridSize {
  const placed = DEFAULT_DASHBOARD_LAYOUTS[breakpoint].find(
    (entry) => entry.panelId === panelId,
  );
  if (placed) {
    return { w: placed.w, h: placed.h };
  }
  return { ...DASHBOARD_PANELS[panelId].min[breakpoint] };
}

/** Smallest size, never wider than the breakpoint's grid. */
export function dashboardPanelMinSize(
  panelId: DashboardPanelId,
  breakpoint: DashboardBreakpoint,
): DashboardGridSize {
  const min = DASHBOARD_PANELS[panelId].min[breakpoint];
  return { w: Math.min(min.w, DASHBOARD_COLUMNS[breakpoint]), h: min.h };
}

function cloneItems(
  items: readonly DashboardLayoutItem[],
): DashboardLayoutItem[] {
  return items.map((entry) => ({ ...entry }));
}

export function defaultDashboardLayouts(): DashboardLayouts {
  return {
    XS: cloneItems(DEFAULT_DASHBOARD_LAYOUTS.XS),
    SM: cloneItems(DEFAULT_DASHBOARD_LAYOUTS.SM),
    MD: cloneItems(DEFAULT_DASHBOARD_LAYOUTS.MD),
    LG: cloneItems(DEFAULT_DASHBOARD_LAYOUTS.LG),
  };
}

function toInt(value: number, fallback: number): number {
  return Number.isFinite(value) ? Math.round(value) : fallback;
}

/**
 * Brings items into this build's grid for a breakpoint: unknown and repeated
 * panel ids are dropped, a layout saved against another column count is scaled
 * to this one, and every item is clamped inside the grid at no less than its
 * minimum size.
 */
export function normalizeDashboardItems(
  items: readonly DashboardLayoutItem[],
  breakpoint: DashboardBreakpoint,
  savedColumns: number = DASHBOARD_COLUMNS[breakpoint],
): DashboardLayoutItem[] {
  const columns = DASHBOARD_COLUMNS[breakpoint];
  const scale = savedColumns > 0 ? columns / savedColumns : 1;
  const seen = new Set<string>();
  const normalized: DashboardLayoutItem[] = [];
  for (const entry of items) {
    if (!isDashboardPanelId(entry.panelId) || seen.has(entry.panelId)) {
      continue;
    }
    seen.add(entry.panelId);
    const min = dashboardPanelMinSize(entry.panelId, breakpoint);
    const w = Math.min(columns, Math.max(min.w, toInt(entry.w * scale, min.w)));
    const h = Math.max(min.h, toInt(entry.h, min.h));
    const x = Math.min(columns - w, Math.max(0, toInt(entry.x * scale, 0)));
    const y = Math.max(0, toInt(entry.y, 0));
    normalized.push({ panelId: entry.panelId, x, y, w, h });
  }
  return normalized;
}

/**
 * What renders at each breakpoint: the saved layout where there is one, the
 * default everywhere else.
 */
export function effectiveDashboardLayouts(
  saved: readonly UiDashboardLayout[],
): DashboardLayouts {
  const layouts = defaultDashboardLayouts();
  for (const layout of saved) {
    if (!DASHBOARD_BREAKPOINTS.includes(layout.breakpoint)) {
      continue;
    }
    layouts[layout.breakpoint] = normalizeDashboardItems(
      layout.items,
      layout.breakpoint,
      layout.columns,
    );
  }
  return layouts;
}

/** Reading order: top to bottom, then left to right. */
function sortItems(
  items: readonly DashboardLayoutItem[],
): DashboardLayoutItem[] {
  return [...items].sort(
    (left, right) =>
      left.y - right.y ||
      left.x - right.x ||
      left.panelId.localeCompare(right.panelId),
  );
}

export function sameDashboardItems(
  left: readonly DashboardLayoutItem[],
  right: readonly DashboardLayoutItem[],
): boolean {
  if (left.length !== right.length) {
    return false;
  }
  const a = sortItems(left);
  const b = sortItems(right);
  return a.every((entry, index) => {
    const other = b[index];
    return (
      other !== undefined &&
      entry.panelId === other.panelId &&
      entry.x === other.x &&
      entry.y === other.y &&
      entry.w === other.w &&
      entry.h === other.h
    );
  });
}

/** Breakpoints whose draft differs from what is showing now. */
export function changedDashboardBreakpoints(
  draft: DashboardLayouts,
  current: DashboardLayouts,
): DashboardBreakpoint[] {
  return DASHBOARD_BREAKPOINTS.filter(
    (breakpoint) => !sameDashboardItems(draft[breakpoint], current[breakpoint]),
  );
}

/** The save payload for one breakpoint, in reading order and inside the grid. */
export function dashboardLayoutInput(
  breakpoint: DashboardBreakpoint,
  items: readonly DashboardLayoutItem[],
): UiDashboardLayout {
  return {
    breakpoint,
    columns: DASHBOARD_COLUMNS[breakpoint],
    items: sortItems(normalizeDashboardItems(items, breakpoint)).slice(
      0,
      DASHBOARD_MAX_ITEMS,
    ),
  };
}

/** Adds a panel below everything else, at its default size. */
export function appendDashboardPanel(
  items: readonly DashboardLayoutItem[],
  panelId: DashboardPanelId,
  breakpoint: DashboardBreakpoint,
): DashboardLayoutItem[] {
  if (items.some((entry) => entry.panelId === panelId)) {
    return cloneItems(items);
  }
  const bottom = items.reduce(
    (lowest, entry) => Math.max(lowest, entry.y + entry.h),
    0,
  );
  const { w, h } = dashboardPanelDefaultSize(panelId, breakpoint);
  return [...cloneItems(items), { panelId, x: 0, y: bottom, w, h }];
}

function overlaps(
  left: DashboardLayoutItem,
  right: DashboardLayoutItem,
): boolean {
  return (
    left.x < right.x + right.w &&
    right.x < left.x + left.w &&
    left.y < right.y + right.h &&
    right.y < left.y + left.h
  );
}

/**
 * Places a panel where it was dropped, clamped inside the grid. Nothing else
 * moves: when the drop lands on another panel, the new one goes down its
 * column to the first rows that are free.
 */
export function placeDashboardPanel(
  items: readonly DashboardLayoutItem[],
  panelId: DashboardPanelId,
  breakpoint: DashboardBreakpoint,
  position: { x: number; y: number; w?: number; h?: number },
): DashboardLayoutItem[] {
  const rest = normalizeDashboardItems(
    items.filter((entry) => entry.panelId !== panelId),
    breakpoint,
  );
  const fallback = dashboardPanelDefaultSize(panelId, breakpoint);
  const [placed] = normalizeDashboardItems(
    [
      {
        panelId,
        x: position.x,
        y: position.y,
        w: position.w ?? fallback.w,
        h: position.h ?? fallback.h,
      },
    ],
    breakpoint,
  );
  if (!placed) {
    return rest;
  }
  for (;;) {
    const blocker = rest.find((entry) => overlaps(entry, placed));
    if (!blocker) {
      break;
    }
    placed.y = blocker.y + blocker.h;
  }
  return [...rest, placed];
}

export function removeDashboardPanel(
  items: readonly DashboardLayoutItem[],
  panelId: string,
): DashboardLayoutItem[] {
  return cloneItems(items.filter((entry) => entry.panelId !== panelId));
}
