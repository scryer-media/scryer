import type {
  SetMyCatalogViewInput,
  UiCatalogViewMode,
  UiDeviceClass,
  UiSettings,
} from "../types/settings.ts";

export type CatalogContentViewMode = "compact" | "poster-table" | "poster";
export type CatalogFacet = "MOVIES" | "SERIES" | "ANIME";
export type CatalogTableViewMode = "COMPACT" | "POSTER_TABLE";
export type CatalogColumnsInput = NonNullable<SetMyCatalogViewInput["columns"]>;

/** The column choices both table layouts share, in display order. */
const TABLE_VIEW_MODES: readonly CatalogTableViewMode[] = ["COMPACT", "POSTER_TABLE"];

export function catalogDeviceClass(isMobile: boolean): UiDeviceClass {
  return isMobile ? "MOBILE" : "DESKTOP";
}

export function catalogFacetForView(view: string): CatalogFacet | null {
  switch (view) {
    case "movies":
      return "MOVIES";
    case "series":
      return "SERIES";
    case "anime":
      return "ANIME";
    default:
      return null;
  }
}

export function catalogViewModeToProfile(mode: CatalogContentViewMode): UiCatalogViewMode {
  switch (mode) {
    case "compact":
      return "COMPACT";
    case "poster-table":
      return "POSTER_TABLE";
    default:
      return "POSTER";
  }
}

function catalogViewModeFromProfile(mode: string): CatalogContentViewMode | null {
  switch (mode) {
    case "COMPACT":
      return "compact";
    case "POSTER_TABLE":
      return "poster-table";
    case "POSTER":
      return "poster";
    default:
      return null;
  }
}

function tableViewModeFor(mode: CatalogContentViewMode): CatalogTableViewMode {
  return mode === "compact" ? "COMPACT" : "POSTER_TABLE";
}

/** The view mode saved on the profile for this device class and facet, if any. */
export function savedCatalogViewMode(
  settings: Pick<UiSettings, "catalogViews">,
  deviceClass: UiDeviceClass,
  facet: CatalogFacet,
): CatalogContentViewMode | null {
  const saved = settings.catalogViews.find(
    (entry) => entry.deviceClass === deviceClass && entry.facet === facet,
  );
  return saved ? catalogViewModeFromProfile(saved.viewMode) : null;
}

/**
 * The visible columns saved on the profile for this device class and facet, or
 * null when nothing is saved. Rows for the active table layout win; the other
 * layout fills in when only it was saved. Column ids this version does not
 * offer for the facet are ignored, and offered columns without a saved row
 * keep their default.
 */
export function savedCatalogVisibleColumns<K extends string>(
  settings: Pick<UiSettings, "tableColumns">,
  options: {
    deviceClass: UiDeviceClass;
    facet: CatalogFacet;
    viewMode: CatalogContentViewMode;
    columnKeys: readonly K[];
    defaults: Record<K, boolean>;
    isSupported: (key: K) => boolean;
  },
): Record<K, boolean> | null {
  const slice = settings.tableColumns.filter(
    (row) => row.deviceClass === options.deviceClass && row.facet === options.facet,
  );
  const active = tableViewModeFor(options.viewMode);
  const preferred = slice.filter((row) => row.tableViewMode === active);
  const rows = preferred.length > 0 ? preferred : slice;
  const known = new Set<string>(options.columnKeys);
  const visible: Record<K, boolean> = { ...options.defaults };
  let matched = false;
  for (const row of rows) {
    if (!known.has(row.columnId)) {
      continue;
    }
    const key = row.columnId as K;
    if (!options.isSupported(key)) {
      continue;
    }
    visible[key] = row.visible;
    matched = true;
  }
  return matched ? visible : null;
}

/**
 * The saved column slice for one facet: every column the facet offers, in
 * display order, for both table layouts so they stay in step.
 */
export function catalogColumnsInput<K extends string>(
  visible: Record<K, boolean>,
  columnKeys: readonly K[],
  isSupported: (key: K) => boolean,
): CatalogColumnsInput {
  const supported = columnKeys.filter(isSupported);
  return TABLE_VIEW_MODES.flatMap((tableViewMode) =>
    supported.map((columnId, columnOrder) => ({
      tableViewMode,
      columnId,
      columnOrder,
      visible: visible[columnId] === true,
    })),
  );
}
