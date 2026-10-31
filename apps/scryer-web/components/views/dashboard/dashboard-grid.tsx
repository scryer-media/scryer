import "react-grid-layout/css/styles.css";
import "./dashboard-grid.css";

import * as React from "react";
import { GripVertical, Plus, RotateCcw, X } from "lucide-react";
import {
  ReactGridLayout,
  noCompactor,
  useContainerWidth,
  verticalCompactor,
  type Compactor,
  type Layout,
  type LayoutItem,
} from "react-grid-layout";
import {
  useBeforeUnload,
  useBlocker,
  type BlockerFunction,
} from "react-router";
import { useClient } from "urql";

import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { DASHBOARD_PANEL_ICONS } from "@/components/views/dashboard/dashboard-panel-icons";
import { DASHBOARD_PANEL_RENDERERS } from "@/components/views/dashboard/dashboard-panel-registry";
import {
  DashboardGridFallback,
  type DashboardPanelData,
} from "@/components/views/dashboard-view";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { IconButton } from "@/components/ui/icon-button";
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useTranslate } from "@/lib/context/translate-context";
import { useUiSettings } from "@/lib/context/ui-settings-context";
import {
  resetMyDashboardLayoutMutation,
  setMyDashboardLayoutMutation,
} from "@/lib/graphql/mutations";
import type { UiSettings } from "@/lib/types";
import { cn } from "@/lib/utils";
import {
  DASHBOARD_BREAKPOINTS,
  DASHBOARD_BREAKPOINT_MIN_WIDTH,
  DASHBOARD_COLUMNS,
  DASHBOARD_MARGIN,
  DASHBOARD_PANELS,
  DASHBOARD_PANEL_IDS,
  DASHBOARD_ROW_HEIGHT,
  appendDashboardPanel,
  changedDashboardBreakpoints,
  dashboardBreakpointForWidth,
  dashboardLayoutInput,
  dashboardPanelDefaultSize,
  dashboardPanelMinSize,
  dashboardPanelTier,
  effectiveDashboardLayouts,
  isDashboardPanelId,
  placeDashboardPanel,
  removeDashboardPanel,
  sameDashboardItems,
  type DashboardBreakpoint,
  type DashboardLayoutItem,
  type DashboardLayouts,
  type DashboardPanelId,
} from "@/lib/utils/dashboard-layout";
import { dashboardLayoutErrorMessage } from "@/lib/utils/dashboard-layout-error-message";
import {
  dashboardDropRegions,
  nearestOpenSpot,
  type GridRect,
} from "@/lib/utils/dashboard-drop-targets";

/**
 * Behind the grid during a move, resize or tray drag: a crosshair at every
 * grid corner, so the panel being dragged can be seen snapping to them. One repeating SVG pattern
 * draws them all, so it costs the same however many rows there are.
 *
 * A corner sits in the middle of the gap between cells. The layer starts half
 * a gap before the grid, so the outer edges get corners too.
 */
const GRID_CROSSHAIR_ARM = 4;
const DASHBOARD_ROW_STEP = DASHBOARD_ROW_HEIGHT + DASHBOARD_MARGIN[1];

function DashboardGridCells({
  width,
  columns,
}: {
  width: number;
  columns: number;
}) {
  const patternId = `scry-grid-corners-${React.useId().replace(/[^a-zA-Z0-9]/g, "")}`;
  const [marginX, marginY] = DASHBOARD_MARGIN;
  const columnWidth = (width - marginX * (columns - 1)) / columns;
  const stepX = columnWidth + marginX;
  const stepY = DASHBOARD_ROW_HEIGHT + marginY;
  const cx = stepX / 2;
  const cy = stepY / 2;
  return (
    <svg
      className="scry-dashboard-grid-cells"
      aria-hidden="true"
      style={{
        left: -marginX / 2,
        top: -marginY / 2,
        width: width + marginX,
        height: `calc(100% + ${marginY / 2 + 3 * stepY}px)`,
      }}
    >
      <defs>
        <pattern
          id={patternId}
          x={-cx}
          y={-cy}
          width={stepX}
          height={stepY}
          patternUnits="userSpaceOnUse"
        >
          <path
            d={`M${cx - GRID_CROSSHAIR_ARM} ${cy}H${cx + GRID_CROSSHAIR_ARM}M${cx} ${cy - GRID_CROSSHAIR_ARM}V${cy + GRID_CROSSHAIR_ARM}`}
          />
        </pattern>
      </defs>
      <rect width="100%" height="100%" fill={`url(#${patternId})`} />
    </svg>
  );
}

/**
 * Behind the grid during a drag: every spot the dragged panel can land,
 * shaded, so it is plain where a drop will take before the placeholder gets
 * there. Cell geometry follows react-grid-layout's own.
 */
function DashboardDropTargets({
  regions,
  width,
  columns,
}: {
  regions: readonly GridRect[];
  width: number;
  columns: number;
}) {
  const [marginX, marginY] = DASHBOARD_MARGIN;
  const columnWidth = (width - marginX * (columns - 1)) / columns;
  // Each region spreads half a gap past its cells on every side, so regions
  // that touch meet without the grid gap showing as a seam between them.
  return (
    <div className="scry-dashboard-drop-targets" aria-hidden="true">
      {regions.map((region) => {
        const left = Math.max(
          0,
          region.x * (columnWidth + marginX) - marginX / 2,
        );
        const right = Math.min(
          width,
          (region.x + region.w) * (columnWidth + marginX) - marginX / 2,
        );
        const top = Math.max(
          0,
          region.y * (DASHBOARD_ROW_HEIGHT + marginY) - marginY / 2,
        );
        const bottom =
          (region.y + region.h) * (DASHBOARD_ROW_HEIGHT + marginY) -
          marginY / 2;
        return (
          <div
            key={`${region.x}:${region.y}`}
            className="scry-dashboard-drop-target"
            style={{ left, top, width: right - left, height: bottom - top }}
          />
        );
      })}
    </div>
  );
}

/**
 * The arrangeable dashboard grid. This module (and react-grid-layout with its
 * stylesheet) is loaded lazily from the dashboard container, so no other route
 * pays for it.
 *
 * Outside edit mode the grid is inert: nothing drags or resizes and no handles
 * render. In edit mode changes collect in a draft that only Save persists, one
 * mutation per breakpoint that changed; Cancel drops the draft.
 */

/**
 * Free placement keeps the saved arrangement exactly where the user put it,
 * but a free drag that pushes neighbours leaves them displaced and can stack
 * panels on top of each other. Refusing collisions instead means a panel only
 * lands on empty cells and nothing else moves; the stacked XS column reorders
 * with vertical compaction.
 */
const FREE_PLACEMENT_COMPACTOR = { ...noCompactor, preventCollision: true };

/**
 * While a panel moves or comes in from the tray it may pass over others
 * instead of being refused there: its outline goes to the nearest cells where
 * it fits, so the outline stays with the pointer rather than waiting at the
 * last free spot. Nothing else moves. `cellAspect` is a column step's width
 * over a row step's height, so "nearest" matches what is on screen.
 */
function snapToOpenCompactor(draggedId: string, cellAspect: number): Compactor {
  return {
    ...noCompactor,
    allowOverlap: true,
    compact(layout: Layout, cols: number): Layout {
      const dragged = layout.find((item) => item.i === draggedId);
      if (!dragged) {
        return layout;
      }
      const others = layout.filter((item) => item.i !== draggedId);
      const spot = nearestOpenSpot(others, dragged, cols, cellAspect);
      if (spot.x === dragged.x && spot.y === dragged.y) {
        return layout;
      }
      return layout.map((item) =>
        item === dragged ? { ...item, ...spot } : item,
      );
    },
  };
}

const DRAG_HANDLE_CLASS = "scry-dashboard-drag-handle";
const NO_DRAG_CLASS = "scry-dashboard-no-drag";
const GESTURE_CLASS = "scry-dashboard-gesture";
const RESIZE_AXES = ["s", "e", "se"] as const;
type ResizeAxis = (typeof RESIZE_AXES)[number];
/** A move or resize in progress, named for the cursor it pins. */
type GridGesture = ResizeAxis | "move";

/** Which handle a resize started from, read off the handle's class. */
function resizeAxisFromEvent(event: Event): ResizeAxis {
  const handle =
    event.target instanceof Element
      ? event.target.closest(".react-resizable-handle")
      : null;
  return (
    RESIZE_AXES.find((axis) =>
      handle?.classList.contains(`react-resizable-handle-${axis}`),
    ) ?? "se"
  );
}
/** react-grid-layout's id for the placeholder of an item dragged in from outside. */
const DROPPING_ITEM_ID = "__dropping-elem__";

const CONTAINER_PADDING: readonly [number, number] = [0, 0];

const BREAKPOINT_QUERIES = DASHBOARD_BREAKPOINTS.filter(
  (breakpoint) => DASHBOARD_BREAKPOINT_MIN_WIDTH[breakpoint] > 0,
).map(
  (breakpoint) =>
    `(min-width: ${DASHBOARD_BREAKPOINT_MIN_WIDTH[breakpoint]}px)`,
);

function subscribeToViewport(onChange: () => void): () => void {
  const lists = BREAKPOINT_QUERIES.map((query) => window.matchMedia(query));
  for (const list of lists) {
    list.addEventListener("change", onChange);
  }
  return () => {
    for (const list of lists) {
      list.removeEventListener("change", onChange);
    }
  };
}

/**
 * The breakpoint is chosen from the viewport, as the fixed dashboard's media
 * queries chose it, while the grid sizes its columns from its own measured
 * width. The media query listeners fire only when a boundary is crossed.
 */
function useDashboardBreakpoint(): DashboardBreakpoint {
  return React.useSyncExternalStore(
    subscribeToViewport,
    () => dashboardBreakpointForWidth(window.innerWidth),
    () => "LG",
  );
}

type DashboardGridProps = {
  data: DashboardPanelData;
  editing: boolean;
  onEditingChange: (editing: boolean) => void;
};

function toGridLayout(
  items: readonly DashboardLayoutItem[],
  breakpoint: DashboardBreakpoint,
): LayoutItem[] {
  return items.map((entry) => {
    const min = isDashboardPanelId(entry.panelId)
      ? dashboardPanelMinSize(entry.panelId, breakpoint)
      : { w: 1, h: 1 };
    return {
      i: entry.panelId,
      x: entry.x,
      y: entry.y,
      w: entry.w,
      h: entry.h,
      minW: min.w,
      minH: min.h,
    };
  });
}

function fromGridLayout(layout: Layout): DashboardLayoutItem[] {
  return layout
    .filter(
      (entry) => entry.i !== DROPPING_ITEM_ID && isDashboardPanelId(entry.i),
    )
    .map((entry) => ({
      panelId: entry.i,
      x: entry.x,
      y: entry.y,
      w: entry.w,
      h: entry.h,
    }));
}

export function DashboardGrid({
  data,
  editing,
  onEditingChange,
}: DashboardGridProps) {
  const t = useTranslate();
  const client = useClient();
  const setGlobalStatus = useGlobalStatus();
  const { uiSettings, uiSettingsLoaded, setUiSettings } = useUiSettings();
  const { width, containerRef, mounted } = useContainerWidth({
    measureBeforeMount: true,
  });

  const saved = React.useMemo(
    () => effectiveDashboardLayouts(uiSettings.dashboardLayouts),
    [uiSettings.dashboardLayouts],
  );
  const [draft, setDraft] = React.useState<DashboardLayouts | null>(null);
  const [trayOpen, setTrayOpen] = React.useState(false);
  const [dragPanelId, setDragPanelId] = React.useState<DashboardPanelId | null>(
    null,
  );
  const [saving, setSaving] = React.useState(false);
  const [confirmReset, setConfirmReset] = React.useState(false);

  const layouts = editing && draft ? draft : saved;
  const breakpoint = useDashboardBreakpoint();
  const stacked = breakpoint === "XS";

  const [gesture, setGesture] = React.useState<GridGesture | null>(null);

  // While a move or resize runs, the pointer leaves the panel or its handle as
  // the panel snaps to the grid and passes over other panels' text. Pin the
  // gesture's cursor and turn text selection off page-wide until release, so
  // the drag never turns into a text selection.
  React.useEffect(() => {
    if (!gesture) {
      return;
    }
    const classes = [GESTURE_CLASS, `${GESTURE_CLASS}-${gesture}`];
    document.body.classList.add(...classes);
    return () => document.body.classList.remove(...classes);
  }, [gesture]);

  // react-grid-layout keeps a moving or resizing panel's live position and
  // size in refs and redraws the panel only when its props or child element
  // change, so on its own the panel freezes between grid snaps, and over other
  // panels, while the outline moves on. Each step hands just that panel a
  // fresh element, so it follows the pointer; the other panels keep their
  // memoised elements and do not redraw.
  const [resizingPanelId, setResizingPanelId] = React.useState<string | null>(
    null,
  );
  const [gestureFrame, setGestureFrame] = React.useState(0);
  // While a panel moves, resizes or comes in from the tray, the grid area
  // reaches a full screen past the bottom of the window, so the drop zones and
  // crosshairs carry on below what is visible instead of stopping at the last
  // panel. Measured once when the gesture starts.
  const gridAreaRef = React.useRef<HTMLDivElement>(null);
  const rowsPastViewport = React.useCallback(() => {
    const top = gridAreaRef.current?.getBoundingClientRect().top ?? 0;
    return Math.max(
      0,
      Math.ceil((2 * window.innerHeight - top) / DASHBOARD_ROW_STEP),
    );
  }, []);
  const [gestureRows, setGestureRows] = React.useState<number | null>(null);
  const handleResizeStart = React.useCallback(
    (
      _layout: Layout,
      oldItem: LayoutItem | null,
      _next: unknown,
      _placeholder: unknown,
      event: Event,
    ) => {
      window.getSelection()?.removeAllRanges();
      setGesture(resizeAxisFromEvent(event));
      setGestureRows(rowsPastViewport());
      setResizingPanelId(oldItem?.i ?? null);
    },
    [rowsPastViewport],
  );
  const handleGestureStep = React.useCallback(
    () => setGestureFrame((frame) => frame + 1),
    [],
  );
  // The area a moved panel can land in. Other panels never move during a
  // drag, since collisions are refused, so it is worked out once at the start.
  const [moveTargets, setMoveTargets] = React.useState<GridRect[] | null>(null);
  const [movingPanelId, setMovingPanelId] = React.useState<string | null>(null);
  const handleDragStart = React.useCallback(
    (layout: Layout, oldItem: LayoutItem | null) => {
      window.getSelection()?.removeAllRanges();
      setGesture("move");
      setMovingPanelId(oldItem?.i ?? null);
      const rows = rowsPastViewport();
      setGestureRows(rows);
      if (oldItem && !stacked) {
        setMoveTargets(
          dashboardDropRegions(
            layout.filter((item) => item.i !== oldItem.i),
            { w: oldItem.w, h: oldItem.h },
            DASHBOARD_COLUMNS[breakpoint],
            rows,
          ),
        );
      }
    },
    [breakpoint, rowsPastViewport, stacked],
  );
  const handleGestureStop = React.useCallback(() => {
    setGesture(null);
    setGestureRows(null);
    setMoveTargets(null);
    setMovingPanelId(null);
    setResizingPanelId(null);
  }, []);
  const items = layouts[breakpoint];
  const placed = React.useMemo(
    () => new Set(items.map((entry) => entry.panelId)),
    [items],
  );

  const gridLayout = React.useMemo(
    () => toGridLayout(items, breakpoint),
    [items, breakpoint],
  );
  const gridConfig = React.useMemo(
    () => ({
      cols: DASHBOARD_COLUMNS[breakpoint],
      rowHeight: DASHBOARD_ROW_HEIGHT,
      margin: DASHBOARD_MARGIN,
      containerPadding: CONTAINER_PADDING,
    }),
    [breakpoint],
  );

  const updateDraft = React.useCallback(
    (change: (current: DashboardLayoutItem[]) => DashboardLayoutItem[]) => {
      setDraft((previous) => {
        const base = previous ?? saved;
        const next = change(base[breakpoint]);
        if (sameDashboardItems(next, base[breakpoint])) {
          return previous ?? base;
        }
        return { ...base, [breakpoint]: next };
      });
    },
    [breakpoint, saved],
  );

  const finish = React.useCallback(() => {
    setDraft(null);
    setTrayOpen(false);
    setDragPanelId(null);
    onEditingChange(false);
  }, [onEditingChange]);

  const handleLayoutChange = React.useCallback(
    (layout: Layout) => {
      if (!editing) {
        return;
      }
      const next = fromGridLayout(layout);
      updateDraft(() => next);
    },
    [editing, updateDraft],
  );

  const handleDrop = React.useCallback(
    (layout: Layout, item: LayoutItem | undefined) => {
      const panelId = dragPanelId;
      setDragPanelId(null);
      if (!editing || !panelId || !item) {
        return;
      }
      const rest = fromGridLayout(layout);
      updateDraft(() =>
        placeDashboardPanel(rest, panelId, breakpoint, {
          x: item.x,
          y: item.y,
          w: item.w,
          h: item.h,
        }),
      );
    },
    [breakpoint, dragPanelId, editing, updateDraft],
  );

  const dropItemSize = React.useMemo(
    () =>
      dragPanelId
        ? dashboardPanelDefaultSize(dragPanelId, breakpoint)
        : { w: 1, h: 1 },
    [breakpoint, dragPanelId],
  );

  const trayDragging = Boolean(
    dragPanelId && !placed.has(dragPanelId) && !stacked,
  );
  const [trayDragRows, setTrayDragRows] = React.useState<number | null>(null);
  const handleTrayDrag = React.useCallback(
    (panelId: DashboardPanelId | null) => {
      setDragPanelId(panelId);
      setTrayDragRows(panelId ? rowsPastViewport() : null);
    },
    [rowsPastViewport],
  );
  const trayRows = trayDragging ? trayDragRows : null;
  const trayTargets = React.useMemo(
    () =>
      trayRows !== null
        ? dashboardDropRegions(
            items,
            dropItemSize,
            DASHBOARD_COLUMNS[breakpoint],
            trayRows,
          )
        : null,
    [breakpoint, dropItemSize, items, trayRows],
  );
  const dropTargets = moveTargets ?? trayTargets;
  const extentRows = editing && !stacked ? (gestureRows ?? trayRows) : null;

  const columns = DASHBOARD_COLUMNS[breakpoint];
  const snapPanelId = stacked
    ? null
    : gesture === "move"
      ? movingPanelId
      : trayDragging
        ? DROPPING_ITEM_ID
        : null;
  const gridCompactor = React.useMemo(() => {
    if (stacked) {
      return verticalCompactor;
    }
    if (!snapPanelId) {
      return FREE_PLACEMENT_COMPACTOR;
    }
    const marginX = DASHBOARD_MARGIN[0];
    const stepX = (width - marginX * (columns - 1)) / columns + marginX;
    return snapToOpenCompactor(snapPanelId, stepX / DASHBOARD_ROW_STEP);
  }, [columns, snapPanelId, stacked, width]);

  const handleDropDragOver = React.useCallback(() => {
    if (!dragPanelId || placed.has(dragPanelId)) {
      return false;
    }
    return { w: dropItemSize.w, h: dropItemSize.h };
  }, [dragPanelId, dropItemSize, placed]);

  const reportFailure = React.useCallback(
    (error: unknown, fallbackKey: string) => {
      setGlobalStatus(dashboardLayoutErrorMessage(error, t, t(fallbackKey)), {
        level: "ERROR",
      });
    },
    [setGlobalStatus, t],
  );

  const save = React.useCallback(async () => {
    if (!draft) {
      finish();
      return;
    }
    const changed = changedDashboardBreakpoints(draft, saved);
    if (changed.length === 0) {
      finish();
      return;
    }
    setSaving(true);
    try {
      for (const entry of changed) {
        const result = await client
          .mutation<{ setMyDashboardLayout?: UiSettings }>(
            setMyDashboardLayoutMutation,
            {
              input: dashboardLayoutInput(entry, draft[entry]),
            },
          )
          .toPromise();
        if (result.error || !result.data?.setMyDashboardLayout) {
          // Breakpoints saved before this one stay saved; the draft is kept so
          // nothing is lost and Save can be tried again.
          reportFailure(result.error, "dashboard.layout.saveFailed");
          return;
        }
        setUiSettings(result.data.setMyDashboardLayout);
      }
      setGlobalStatus(t("dashboard.layout.saved"), { level: "SUCCESS" });
      finish();
    } catch (error) {
      reportFailure(error, "dashboard.layout.saveFailed");
    } finally {
      setSaving(false);
    }
  }, [
    client,
    draft,
    finish,
    reportFailure,
    saved,
    setGlobalStatus,
    setUiSettings,
    t,
  ]);

  // Leaving the page with an unsaved arrangement asks first: moves within the
  // app through the router, reloads and closing the tab through the browser.
  // A save in flight is not held up, since it ends edit mode itself.
  const hasUnsavedChanges = React.useMemo(
    () =>
      editing &&
      draft !== null &&
      changedDashboardBreakpoints(draft, saved).length > 0,
    [draft, editing, saved],
  );
  const guardNavigation = hasUnsavedChanges && !saving;
  const navigationBlocker = useBlocker(
    React.useCallback<BlockerFunction>(
      ({ currentLocation, nextLocation }) =>
        guardNavigation && currentLocation.pathname !== nextLocation.pathname,
      [guardNavigation],
    ),
  );
  useBeforeUnload(
    React.useCallback(
      (event: BeforeUnloadEvent) => {
        if (!guardNavigation) {
          return;
        }
        event.preventDefault();
        event.returnValue = "";
      },
      [guardNavigation],
    ),
  );
  const navigationHeld = navigationBlocker.state === "blocked";
  // A held move whose changes are gone (a save landed meanwhile) goes ahead.
  React.useEffect(() => {
    if (navigationHeld && !hasUnsavedChanges) {
      navigationBlocker.proceed?.();
    }
  }, [hasUnsavedChanges, navigationBlocker, navigationHeld]);
  const discardAndLeave = React.useCallback(() => {
    finish();
    navigationBlocker.proceed?.();
  }, [finish, navigationBlocker]);
  const stayOnPage = React.useCallback(
    () => navigationBlocker.reset?.(),
    [navigationBlocker],
  );

  const reset = React.useCallback(async () => {
    setSaving(true);
    try {
      const result = await client
        .mutation<{ resetMyDashboardLayout?: UiSettings }>(
          resetMyDashboardLayoutMutation,
          {
            breakpoint: null,
          },
        )
        .toPromise();
      if (result.error || !result.data?.resetMyDashboardLayout) {
        reportFailure(result.error, "dashboard.layout.resetFailed");
        return;
      }
      setUiSettings(result.data.resetMyDashboardLayout);
      setGlobalStatus(t("dashboard.layout.resetDone"), { level: "SUCCESS" });
      setConfirmReset(false);
      finish();
    } catch (error) {
      reportFailure(error, "dashboard.layout.resetFailed");
    } finally {
      setSaving(false);
    }
  }, [client, finish, reportFailure, setGlobalStatus, setUiSettings, t]);

  const panelElements = React.useMemo(
    () =>
      items
        .map((entry) => {
          if (!isDashboardPanelId(entry.panelId)) {
            return null;
          }
          const panelId = entry.panelId;
          const tier = dashboardPanelTier(panelId, breakpoint, entry);
          const title = t(DASHBOARD_PANELS[panelId].titleKey);
          return (
            <div
              key={panelId}
              data-panel-id={panelId}
              data-tier={tier}
              className="scry-dashboard-item"
            >
              {DASHBOARD_PANEL_RENDERERS[panelId]({ data, tier, breakpoint })}
              {editing ? (
                <div
                  className={DRAG_HANDLE_CLASS}
                  role="group"
                  aria-label={t("dashboard.layout.movePanel", { name: title })}
                >
                  <div className="scry-dashboard-drag-bar">
                    <GripVertical
                      className="h-4 w-4 shrink-0"
                      aria-hidden="true"
                    />
                    <span
                      className="scry-dashboard-drag-title"
                      aria-hidden="true"
                    >
                      {title}
                    </span>
                    <IconButton
                      appearance="boxed"
                      tone="delete"
                      className={cn(NO_DRAG_CLASS, "h-7 w-7")}
                      label={t("dashboard.layout.removePanel", { name: title })}
                      onClick={() =>
                        updateDraft((current) =>
                          removeDashboardPanel(current, panelId),
                        )
                      }
                    >
                      <X className="h-3.5 w-3.5" aria-hidden="true" />
                    </IconButton>
                  </div>
                </div>
              ) : null}
            </div>
          );
        })
        .filter(
          (element): element is React.ReactElement<Record<string, unknown>> =>
            element !== null,
        ),
    [breakpoint, data, editing, items, t, updateDraft],
  );

  return (
    <div ref={containerRef} className="flex min-w-0 flex-col gap-3">
      {editing ? (
        <div
          data-slot="dashboard-edit-toolbar"
          className="sticky top-0 z-20 flex min-w-0 flex-wrap items-center gap-2 rounded-xl border border-[var(--scry-accent)] bg-[var(--scry-bg)] px-3 py-2 shadow-sm"
        >
          <span className="flex min-w-0 flex-1 flex-col">
            <span className="text-[12px] font-semibold text-[var(--scry-ink2)]">
              {t("dashboard.layout.editing")}
            </span>
            <span className="text-[11px] text-[var(--scry-muted)]">
              {t(
                stacked
                  ? "dashboard.layout.editHintStacked"
                  : "dashboard.layout.editHint",
              )}
            </span>
          </span>
          <Button
            type="button"
            size="sm"
            variant="secondary"
            onClick={() => setTrayOpen(true)}
          >
            <Plus className="h-3.5 w-3.5" aria-hidden="true" />
            {t("dashboard.layout.panels")}
          </Button>
          <Button
            type="button"
            size="sm"
            variant="secondary"
            disabled={saving}
            onClick={() => setConfirmReset(true)}
          >
            <RotateCcw className="h-3.5 w-3.5" aria-hidden="true" />
            {t("dashboard.layout.reset")}
          </Button>
          <Button
            type="button"
            size="sm"
            variant="secondary"
            disabled={saving}
            onClick={finish}
          >
            {t("label.cancel")}
          </Button>
          <Button
            type="button"
            size="sm"
            disabled={saving}
            onClick={() => void save()}
          >
            {t("label.save")}
          </Button>
        </div>
      ) : null}

      {!uiSettingsLoaded || !mounted ? (
        <DashboardGridFallback />
      ) : (
        <>
          {items.length === 0 ? (
            <p className="rounded-xl border border-dashed border-border px-3 py-6 text-center text-[12px] text-[var(--scry-muted2)]">
              {t("dashboard.layout.empty")}
            </p>
          ) : null}
          {/*
            One grid per breakpoint. Each breakpoint shows its own panels in its
            own order and column count; remounting on a crossing starts the
            grid from that breakpoint's layout every time, so no positions
            carry over from the breakpoint just left.
          */}
          <div
            ref={gridAreaRef}
            className="relative"
            style={
              extentRows !== null
                ? { minHeight: extentRows * DASHBOARD_ROW_STEP }
                : undefined
            }
          >
            {extentRows !== null ? (
              <DashboardGridCells
                width={width}
                columns={DASHBOARD_COLUMNS[breakpoint]}
              />
            ) : null}
            {editing && dropTargets ? (
              <DashboardDropTargets
                regions={dropTargets}
                width={width}
                columns={DASHBOARD_COLUMNS[breakpoint]}
              />
            ) : null}
            <ReactGridLayout
              key={breakpoint}
              className={cn(
                "scry-dashboard-grid",
                editing && "scry-dashboard-grid-editing",
              )}
              width={width}
              layout={gridLayout}
              gridConfig={gridConfig}
              compactor={gridCompactor}
              dragConfig={{
                enabled: editing,
                handle: `.${DRAG_HANDLE_CLASS}`,
                cancel: `.${NO_DRAG_CLASS}`,
                allowMobileScroll: true,
              }}
              resizeConfig={{
                enabled: editing && !stacked,
                handles: [...RESIZE_AXES],
              }}
              dropConfig={{
                enabled: editing,
                defaultItem: dropItemSize,
                onDragOver: handleDropDragOver,
                touchEnabled: true,
              }}
              onResizeStart={handleResizeStart}
              onResize={handleGestureStep}
              onDrag={handleGestureStep}
              onResizeStop={handleGestureStop}
              onDragStart={handleDragStart}
              onDragStop={handleGestureStop}
              onLayoutChange={handleLayoutChange}
              onDrop={handleDrop}
            >
              {panelElements.map((element) =>
                element.key === (resizingPanelId ?? movingPanelId)
                  ? React.cloneElement(element, {
                      "data-gesture-frame": gestureFrame,
                    })
                  : element,
              )}
            </ReactGridLayout>
          </div>
        </>
      )}

      <DashboardPanelTray
        open={editing && trayOpen}
        onOpenChange={setTrayOpen}
        placed={placed}
        onDragPanel={handleTrayDrag}
        onAddPanel={(panelId) =>
          updateDraft((current) =>
            appendDashboardPanel(current, panelId, breakpoint),
          )
        }
      />

      <ConfirmDialog
        open={confirmReset}
        title={t("dashboard.layout.resetConfirmTitle")}
        description={t("dashboard.layout.resetConfirmDescription")}
        confirmLabel={t("dashboard.layout.reset")}
        cancelLabel={t("label.cancel")}
        isBusy={saving}
        onConfirm={() => void reset()}
        onCancel={() => setConfirmReset(false)}
      />

      <ConfirmDialog
        open={navigationHeld && hasUnsavedChanges}
        title={t("dashboard.layout.unsavedTitle")}
        description={t("dashboard.layout.unsavedDescription")}
        confirmLabel={t("label.discard")}
        cancelLabel={t("label.cancel")}
        onConfirm={discardAndLeave}
        onCancel={stayOnPage}
      />
    </div>
  );
}

/**
 * Every panel there is, from the right. Non-modal so a panel can be dragged
 * out of it onto the grid while it stays open.
 */
function DashboardPanelTray({
  open,
  onOpenChange,
  placed,
  onDragPanel,
  onAddPanel,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  placed: ReadonlySet<string>;
  onDragPanel: (panelId: DashboardPanelId | null) => void;
  onAddPanel: (panelId: DashboardPanelId) => void;
}) {
  const t = useTranslate();

  return (
    <Sheet open={open} onOpenChange={onOpenChange} modal={false}>
      <SheetContent
        side="right"
        className="w-80 gap-0 sm:max-w-sm"
        onInteractOutside={(event) => event.preventDefault()}
      >
        <SheetHeader>
          <SheetTitle>{t("dashboard.layout.trayTitle")}</SheetTitle>
          <SheetDescription>
            {t("dashboard.layout.trayDescription")}
          </SheetDescription>
        </SheetHeader>
        <ul className="flex min-h-0 flex-1 flex-col gap-2 overflow-y-auto px-4 pb-4">
          {DASHBOARD_PANEL_IDS.map((panelId) => {
            const Icon = DASHBOARD_PANEL_ICONS[panelId];
            const spec = DASHBOARD_PANELS[panelId];
            const title = t(spec.titleKey);
            const isPlaced = placed.has(panelId);
            return (
              <li
                key={panelId}
                draggable={!isPlaced}
                onDragStart={(event) => {
                  event.dataTransfer.setData("text/plain", panelId);
                  event.dataTransfer.effectAllowed = "copy";
                  onDragPanel(panelId);
                }}
                onDragEnd={() => onDragPanel(null)}
                className={cn(
                  "flex min-w-0 items-start gap-2 rounded-lg border border-border px-2 py-2",
                  isPlaced ? "opacity-70" : "cursor-grab",
                )}
              >
                {isPlaced ? (
                  <span
                    className="mt-0.5 h-4 w-4 shrink-0"
                    aria-hidden="true"
                  />
                ) : (
                  <span
                    data-rgl-draggable=""
                    className="scry-dashboard-tray-grip mt-0.5 shrink-0 text-[var(--scry-muted2)]"
                    aria-label={t("dashboard.layout.dragPanel", {
                      name: title,
                    })}
                    role="img"
                    onTouchStart={() => onDragPanel(panelId)}
                  >
                    <GripVertical className="h-4 w-4" aria-hidden="true" />
                  </span>
                )}
                <Icon
                  className="mt-0.5 h-4 w-4 shrink-0 text-[var(--scry-muted2)]"
                  aria-hidden="true"
                />
                <span className="flex min-w-0 flex-1 flex-col gap-0.5">
                  <span className="flex min-w-0 items-center gap-1.5">
                    <span className="truncate text-[12px] font-semibold text-[var(--scry-ink2)]">
                      {title}
                    </span>
                    {isPlaced ? (
                      <Badge
                        tone="positive"
                        className="shrink-0 px-1 py-0 text-[10px]"
                      >
                        {t("dashboard.layout.placed")}
                      </Badge>
                    ) : null}
                  </span>
                  <span className="text-[11px] text-[var(--scry-muted)]">
                    {t(spec.descriptionKey)}
                  </span>
                </span>
                {isPlaced ? null : (
                  <IconButton
                    appearance="boxed"
                    tone="accent"
                    label={t("dashboard.layout.addPanel", { name: title })}
                    onClick={() => onAddPanel(panelId)}
                  >
                    <Plus className="h-3.5 w-3.5" aria-hidden="true" />
                  </IconButton>
                )}
              </li>
            );
          })}
        </ul>
      </SheetContent>
    </Sheet>
  );
}
