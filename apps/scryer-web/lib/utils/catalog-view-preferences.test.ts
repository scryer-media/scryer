import assert from "node:assert/strict";
import test from "node:test";
import type { UiSettings, UiTableColumnSetting } from "../types/settings.ts";
import {
  catalogColumnsInput,
  catalogDeviceClass,
  catalogFacetForView,
  catalogViewModeToProfile,
  savedCatalogViewMode,
  savedCatalogVisibleColumns,
} from "./catalog-view-preferences.ts";

type Key = "alpha" | "beta" | "gamma";
const KEYS: readonly Key[] = ["alpha", "beta", "gamma"];
const DEFAULTS: Record<Key, boolean> = { alpha: true, beta: false, gamma: true };
const ALL_SUPPORTED = () => true;

function column(
  overrides: Partial<UiTableColumnSetting> & Pick<UiTableColumnSetting, "columnId" | "visible">,
): UiTableColumnSetting {
  return {
    deviceClass: "DESKTOP",
    facet: "SERIES",
    tableViewMode: "COMPACT",
    columnOrder: 0,
    ...overrides,
  };
}

function columnsFor(tableColumns: UiTableColumnSetting[]): Pick<UiSettings, "tableColumns"> {
  return { tableColumns };
}

test("view modes are kept per device class and facet", () => {
  const settings: Pick<UiSettings, "catalogViews"> = {
    catalogViews: [
      { deviceClass: "DESKTOP", facet: "SERIES", viewMode: "COMPACT" },
      { deviceClass: "MOBILE", facet: "SERIES", viewMode: "POSTER" },
      { deviceClass: "DESKTOP", facet: "MOVIES", viewMode: "POSTER_TABLE" },
    ],
  };
  assert.equal(savedCatalogViewMode(settings, "DESKTOP", "SERIES"), "compact");
  assert.equal(savedCatalogViewMode(settings, "MOBILE", "SERIES"), "poster");
  assert.equal(savedCatalogViewMode(settings, "DESKTOP", "MOVIES"), "poster-table");
  // Nothing saved: the caller falls back to the browser's stored view mode.
  assert.equal(savedCatalogViewMode(settings, "MOBILE", "ANIME"), null);
});

test("view mode and facet mappings round trip", () => {
  assert.equal(catalogDeviceClass(true), "MOBILE");
  assert.equal(catalogDeviceClass(false), "DESKTOP");
  assert.equal(catalogFacetForView("anime"), "ANIME");
  assert.equal(catalogFacetForView("dashboard"), null);
  for (const mode of ["compact", "poster-table", "poster"] as const) {
    const saved = savedCatalogViewMode(
      {
        catalogViews: [
          { deviceClass: "DESKTOP", facet: "MOVIES", viewMode: catalogViewModeToProfile(mode) },
        ],
      },
      "DESKTOP",
      "MOVIES",
    );
    assert.equal(saved, mode);
  }
});

test("saved columns apply over defaults and ignore unknown or unsupported ids", () => {
  const visible = savedCatalogVisibleColumns(
    columnsFor([
      column({ columnId: "alpha", visible: false }),
      column({ columnId: "beta", visible: true }),
      column({ columnId: "retiredColumn", visible: true }),
      column({ columnId: "gamma", visible: false }),
    ]),
    {
      deviceClass: "DESKTOP",
      facet: "SERIES",
      viewMode: "compact",
      columnKeys: KEYS,
      defaults: DEFAULTS,
      isSupported: (key) => key !== "gamma",
    },
  );
  assert.deepEqual(visible, { alpha: false, beta: true, gamma: true });
});

test("columns come from the matching device class and facet only", () => {
  const settings = columnsFor([
    column({ deviceClass: "MOBILE", columnId: "alpha", visible: false }),
    column({ facet: "MOVIES", columnId: "beta", visible: true }),
  ]);
  const options = {
    columnKeys: KEYS,
    defaults: DEFAULTS,
    isSupported: ALL_SUPPORTED,
    viewMode: "compact" as const,
  };
  assert.equal(
    savedCatalogVisibleColumns(settings, { ...options, deviceClass: "DESKTOP", facet: "SERIES" }),
    null,
  );
  assert.deepEqual(
    savedCatalogVisibleColumns(settings, { ...options, deviceClass: "MOBILE", facet: "SERIES" }),
    { alpha: false, beta: false, gamma: true },
  );
});

test("the active table layout wins, and the other layout fills in when it alone is saved", () => {
  const settings = columnsFor([
    column({ tableViewMode: "COMPACT", columnId: "beta", visible: true }),
    column({ tableViewMode: "POSTER_TABLE", columnId: "beta", visible: false }),
  ]);
  const options = {
    deviceClass: "DESKTOP" as const,
    facet: "SERIES" as const,
    columnKeys: KEYS,
    defaults: DEFAULTS,
    isSupported: ALL_SUPPORTED,
  };
  assert.equal(savedCatalogVisibleColumns(settings, { ...options, viewMode: "compact" })?.beta, true);
  assert.equal(
    savedCatalogVisibleColumns(settings, { ...options, viewMode: "poster-table" })?.beta,
    false,
  );
  const compactOnly = columnsFor([column({ columnId: "beta", visible: true })]);
  assert.equal(
    savedCatalogVisibleColumns(compactOnly, { ...options, viewMode: "poster" })?.beta,
    true,
  );
});

test("the saved slice lists every supported column for both table layouts", () => {
  const input = catalogColumnsInput(
    { alpha: true, beta: false, gamma: true },
    KEYS,
    (key) => key !== "beta",
  );
  assert.deepEqual(input, [
    { tableViewMode: "COMPACT", columnId: "alpha", columnOrder: 0, visible: true },
    { tableViewMode: "COMPACT", columnId: "gamma", columnOrder: 1, visible: true },
    { tableViewMode: "POSTER_TABLE", columnId: "alpha", columnOrder: 0, visible: true },
    { tableViewMode: "POSTER_TABLE", columnId: "gamma", columnOrder: 1, visible: true },
  ]);
});
