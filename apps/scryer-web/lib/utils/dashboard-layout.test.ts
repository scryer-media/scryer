import assert from "node:assert/strict";
import test from "node:test";

import {
  DASHBOARD_BREAKPOINTS,
  DASHBOARD_COLUMNS,
  DASHBOARD_MAX_COLUMNS,
  DASHBOARD_MAX_ITEMS,
  DASHBOARD_PANEL_IDS,
  DASHBOARD_PANELS,
  DEFAULT_DASHBOARD_LAYOUTS,
  appendDashboardPanel,
  changedDashboardBreakpoints,
  dashboardBreakpointForWidth,
  dashboardLayoutInput,
  dashboardPanelTier,
  defaultDashboardLayouts,
  effectiveDashboardLayouts,
  isDashboardPanelId,
  isWellFormedPanelId,
  normalizeDashboardItems,
  placeDashboardPanel,
  removeDashboardPanel,
  type DashboardBreakpoint,
  type DashboardLayoutItem,
} from "./dashboard-layout.ts";

/** Today's DOM order of the panels, top to bottom and left to right. */
const PAGE_ORDER = [
  "stats",
  "requests",
  "manual-imports",
  "recently-imported",
  "indexers",
  "download-clients",
  "storage",
  "active-queue",
];

const row = (panelId: string, x: number, y: number, w: number, h: number) => ({
  panelId,
  x,
  y,
  w,
  h,
});

test("LG default: three across, then two pairs, as min-[1501px] laid it out", () => {
  assert.deepEqual(DEFAULT_DASHBOARD_LAYOUTS.LG, [
    row("stats", 0, 0, 12, 2),
    row("requests", 0, 2, 4, 7),
    row("manual-imports", 4, 2, 4, 7),
    row("recently-imported", 8, 2, 4, 7),
    row("indexers", 0, 9, 6, 9),
    row("download-clients", 6, 9, 6, 9),
    row("storage", 0, 18, 6, 10),
    row("active-queue", 6, 18, 6, 10),
  ]);
});

test("MD default: two across, recently imported alone in the left half, as min-[1241px] did", () => {
  assert.deepEqual(DEFAULT_DASHBOARD_LAYOUTS.MD, [
    row("stats", 0, 0, 12, 2),
    row("requests", 0, 2, 6, 7),
    row("manual-imports", 6, 2, 6, 7),
    row("recently-imported", 0, 9, 6, 7),
    row("indexers", 0, 16, 6, 9),
    row("download-clients", 6, 16, 6, 9),
    row("storage", 0, 25, 6, 10),
    row("active-queue", 6, 25, 6, 10),
  ]);
});

test("SM default: every panel full width and stacked, as below 1241", () => {
  assert.deepEqual(DEFAULT_DASHBOARD_LAYOUTS.SM, [
    row("stats", 0, 0, 6, 4),
    row("requests", 0, 4, 6, 7),
    row("manual-imports", 0, 11, 6, 7),
    row("recently-imported", 0, 18, 6, 7),
    row("indexers", 0, 25, 6, 9),
    row("download-clients", 0, 34, 6, 9),
    row("storage", 0, 43, 6, 10),
    row("active-queue", 0, 53, 6, 10),
  ]);
});

test("XS default: one column, page order", () => {
  assert.deepEqual(DEFAULT_DASHBOARD_LAYOUTS.XS, [
    row("stats", 0, 0, 1, 6),
    row("requests", 0, 6, 1, 7),
    row("manual-imports", 0, 13, 1, 7),
    row("recently-imported", 0, 20, 1, 7),
    row("indexers", 0, 27, 1, 9),
    row("download-clients", 0, 36, 1, 9),
    row("storage", 0, 45, 1, 10),
    row("active-queue", 0, 55, 1, 10),
  ]);
});

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

test("every default holds every panel once, in page order, inside the grid, without overlap", () => {
  for (const breakpoint of DASHBOARD_BREAKPOINTS) {
    const items = DEFAULT_DASHBOARD_LAYOUTS[breakpoint];
    const columns = DASHBOARD_COLUMNS[breakpoint];
    assert.deepEqual(
      items.map((entry) => entry.panelId),
      PAGE_ORDER,
      `${breakpoint} order`,
    );
    for (const entry of items) {
      assert.ok(
        entry.x >= 0 && entry.y >= 0,
        `${breakpoint} ${entry.panelId} position`,
      );
      assert.ok(
        entry.x + entry.w <= columns,
        `${breakpoint} ${entry.panelId} fits`,
      );
    }
    items.forEach((left, index) => {
      for (const right of items.slice(index + 1)) {
        assert.equal(
          overlaps(left, right),
          false,
          `${breakpoint} ${left.panelId} / ${right.panelId}`,
        );
      }
    });
  }
});

test("defaults render the content the fixed page showed", () => {
  const expected: Record<DashboardBreakpoint, Record<string, string>> = {
    // Stat tiles: stacked below 701, two across from 701, three across wider.
    XS: { stats: "compact" },
    SM: { stats: "regular" },
    MD: { stats: "expanded" },
    LG: { stats: "expanded" },
  };
  for (const breakpoint of DASHBOARD_BREAKPOINTS) {
    for (const entry of DEFAULT_DASHBOARD_LAYOUTS[breakpoint]) {
      assert.ok(isDashboardPanelId(entry.panelId), entry.panelId);
      const tier = dashboardPanelTier(entry.panelId, breakpoint, entry);
      assert.equal(
        tier,
        expected[breakpoint][entry.panelId] ?? "regular",
        `${breakpoint} ${entry.panelId}`,
      );
    }
  }
});

test("panel ids are stable kebab-case the server accepts, and every default is at least the minimum", () => {
  assert.equal(new Set(DASHBOARD_PANEL_IDS).size, DASHBOARD_PANEL_IDS.length);
  for (const panelId of DASHBOARD_PANEL_IDS) {
    assert.ok(isWellFormedPanelId(panelId), panelId);
    const spec = DASHBOARD_PANELS[panelId];
    assert.ok(
      spec.titleKey.length > 0 && spec.descriptionKey.length > 0,
      panelId,
    );
    for (const breakpoint of DASHBOARD_BREAKPOINTS) {
      const placed = DEFAULT_DASHBOARD_LAYOUTS[breakpoint].find(
        (entry) => entry.panelId === panelId,
      );
      assert.ok(placed, `${breakpoint} ${panelId} placed`);
      const min = spec.min[breakpoint];
      assert.ok(
        min.w <= DASHBOARD_COLUMNS[breakpoint],
        `${breakpoint} ${panelId} min fits`,
      );
      assert.ok(
        placed.w >= min.w && placed.h >= min.h,
        `${breakpoint} ${panelId} default ≥ min`,
      );
    }
  }
  for (const breakpoint of DASHBOARD_BREAKPOINTS) {
    assert.ok(
      DASHBOARD_COLUMNS[breakpoint] >= 1 &&
        DASHBOARD_COLUMNS[breakpoint] <= DASHBOARD_MAX_COLUMNS,
    );
  }
  assert.ok(DASHBOARD_PANEL_IDS.length <= DASHBOARD_MAX_ITEMS);
});

test("tiers follow size: smaller is compact, wider and taller is expanded", () => {
  assert.equal(dashboardPanelTier("requests", "LG", { w: 3, h: 7 }), "compact");
  assert.equal(dashboardPanelTier("requests", "LG", { w: 4, h: 7 }), "regular");
  assert.equal(
    dashboardPanelTier("requests", "LG", { w: 8, h: 9 }),
    "expanded",
  );
  assert.equal(dashboardPanelTier("requests", "LG", { w: 8, h: 4 }), "compact");
  assert.equal(
    dashboardPanelTier("indexers", "MD", { w: 12, h: 9 }),
    "expanded",
  );
  assert.equal(dashboardPanelTier("stats", "LG", { w: 6, h: 2 }), "regular");
  assert.equal(dashboardPanelTier("stats", "LG", { w: 4, h: 2 }), "compact");
});

test("breakpoints follow the viewport at the old 701, 1241 and 1501 media queries", () => {
  assert.equal(dashboardBreakpointForWidth(0), "XS");
  assert.equal(dashboardBreakpointForWidth(700), "XS");
  assert.equal(dashboardBreakpointForWidth(701), "SM");
  assert.equal(dashboardBreakpointForWidth(1240), "SM");
  assert.equal(dashboardBreakpointForWidth(1241), "MD");
  assert.equal(dashboardBreakpointForWidth(1500), "MD");
  assert.equal(dashboardBreakpointForWidth(1501), "LG");
  // Laptop widths the container-measured version moved down a tier.
  assert.equal(dashboardBreakpointForWidth(1300), "MD");
  assert.equal(dashboardBreakpointForWidth(1550), "LG");
});

test("a saved layout replaces the default for its breakpoint only", () => {
  const layouts = effectiveDashboardLayouts([
    {
      breakpoint: "LG",
      columns: 12,
      items: [row("storage", 0, 0, 12, 10), row("stats", 0, 10, 12, 2)],
    },
  ]);
  assert.deepEqual(layouts.LG, [
    row("storage", 0, 0, 12, 10),
    row("stats", 0, 10, 12, 2),
  ]);
  assert.deepEqual(layouts.MD, DEFAULT_DASHBOARD_LAYOUTS.MD);
  assert.deepEqual(layouts.SM, DEFAULT_DASHBOARD_LAYOUTS.SM);
  assert.deepEqual(layouts.XS, DEFAULT_DASHBOARD_LAYOUTS.XS);
});

test("loading drops unknown and repeated panels and leaves missing ones off", () => {
  const items = normalizeDashboardItems(
    [
      row("retired-panel", 0, 0, 4, 4),
      row("requests", 0, 0, 4, 7),
      row("requests", 4, 0, 4, 7),
    ],
    "LG",
  );
  assert.deepEqual(items, [row("requests", 0, 0, 4, 7)]);
});

test("loading clamps into the grid and up to the minimum, and rescales another column count", () => {
  assert.deepEqual(
    normalizeDashboardItems([row("requests", 10, -2, 6, 1)], "LG"),
    [row("requests", 6, 0, 6, 5)],
  );
  assert.deepEqual(
    normalizeDashboardItems([row("storage", 12, 0, 12, 10)], "LG", 24),
    [row("storage", 6, 0, 6, 10)],
  );
  assert.deepEqual(
    normalizeDashboardItems([row("requests", 0, 0, 4, 7)], "XS"),
    [row("requests", 0, 0, 1, 7)],
  );
});

test("only breakpoints the draft changed are saved", () => {
  const current = defaultDashboardLayouts();
  const draft = defaultDashboardLayouts();
  assert.deepEqual(changedDashboardBreakpoints(draft, current), []);
  // Same items in another order are the same layout.
  draft.LG = [...draft.LG].reverse();
  assert.deepEqual(changedDashboardBreakpoints(draft, current), []);
  draft.MD = removeDashboardPanel(draft.MD, "storage");
  assert.deepEqual(changedDashboardBreakpoints(draft, current), ["MD"]);
});

test("the save payload stays inside every server limit", () => {
  for (const breakpoint of DASHBOARD_BREAKPOINTS) {
    const input = dashboardLayoutInput(breakpoint, [
      ...DEFAULT_DASHBOARD_LAYOUTS[breakpoint],
      row("requests", 99, 3, 99, 0),
    ]);
    assert.equal(input.columns, DASHBOARD_COLUMNS[breakpoint]);
    assert.equal(
      new Set(input.items.map((entry) => entry.panelId)).size,
      input.items.length,
    );
    for (const entry of input.items) {
      assert.ok(Number.isInteger(entry.x) && Number.isInteger(entry.y));
      assert.ok(entry.x >= 0 && entry.y >= 0 && entry.w >= 1 && entry.h >= 1);
      assert.ok(
        entry.x + entry.w <= input.columns,
        `${breakpoint} ${entry.panelId}`,
      );
    }
  }
});

test("adding puts the panel below everything at its default size; dropping puts it where it fell", () => {
  const without = removeDashboardPanel(DEFAULT_DASHBOARD_LAYOUTS.LG, "storage");
  const added = appendDashboardPanel(without, "storage", "LG");
  assert.deepEqual(added.at(-1), row("storage", 0, 28, 6, 10));
  assert.equal(
    appendDashboardPanel(added, "storage", "LG").length,
    added.length,
  );

  const dropped = placeDashboardPanel(without, "storage", "LG", {
    x: 0,
    y: 18,
  });
  assert.deepEqual(dropped.at(-1), row("storage", 0, 18, 6, 10));
});

test("a drop onto other panels moves only the dropped one, down to the first free rows", () => {
  const without = removeDashboardPanel(DEFAULT_DASHBOARD_LAYOUTS.LG, "storage");
  const dropped = placeDashboardPanel(without, "storage", "LG", {
    x: 11,
    y: 3,
  });
  // Clamped to x 6, then past manual imports, download clients and active queue.
  assert.deepEqual(dropped.at(-1), row("storage", 6, 28, 6, 10));
  assert.deepEqual(dropped.slice(0, -1), without);
  for (const [index, left] of dropped.entries()) {
    for (const right of dropped.slice(index + 1)) {
      const apart =
        left.x + left.w <= right.x ||
        right.x + right.w <= left.x ||
        left.y + left.h <= right.y ||
        right.y + right.h <= left.y;
      assert.ok(apart, `${left.panelId} overlaps ${right.panelId}`);
    }
  }
});
