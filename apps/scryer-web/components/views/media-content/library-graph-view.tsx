import * as React from "react";
import { useLibraryGraphHistory } from "@/lib/hooks/use-library-graph-history";
import Graph from "graphology";
import Sigma, { DEFAULT_STYLES } from "sigma";
import { sdfRectangle } from "sigma/rendering";
import { useClient } from "urql";
import { ArrowUpRight, Undo2 } from "lucide-react";

import { Button } from "@/components/ui/button";
import { LibraryGraphSelectionCard } from "./library-graph-selection-card";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useTranslate } from "@/lib/context/translate-context";
import { discoveryNeighborsQuery } from "@/lib/graphql/queries";
import { useAuth } from "@/lib/hooks/use-auth";
import { useTitleMoreLikeThisActions } from "@/lib/hooks/use-title-more-like-this-actions";
import type { DiscoveryItem, DiscoveryNeighbors } from "@/lib/types/discovery";
import { discoveryItemFacet } from "@/lib/utils/discovery-actions";
import { discoveryErrorMessage } from "@/lib/utils/discovery-error-message";
import { discoveryItemDisplayTitle } from "@/lib/utils/discovery-display";
import {
  createRectIndex,
  showOverviewTitleLabels,
} from "@/lib/utils/library-graph-labels";
import { graphSelectionCatalogTitle } from "@/lib/utils/library-graph-selection";
import {
  LIBRARY_PERMISSIONS,
  hasAnyLibraryPermission,
} from "@/lib/utils/permissions";
import { selectPosterVariantUrl } from "@/lib/utils/poster-images";
import {
  GRAPH_POSTER_ASPECT_RATIO,
  GRAPH_POSTER_CORNER_RADIUS_RATIO,
  createGraphPosterSprites,
  drawGraphPoster,
  type GraphPosterSprites,
  graphPosterArtworkOpacity,
  graphPosterUrls,
  graphPosterVariantUrls,
  isOriginalGraphPosterUrl,
  isThumbnailGraphPosterUrl,
  createGraphPosterAdmission,
  prepareGraphPosterImages,
  graphPosterBounds,
  graphPosterInViewport,
  graphPosterHoverable,
  graphRelationOpacity,
  changedGraphViewportEdges,
  traceGraphPoster,
  type GraphPosterBounds,
} from "@/lib/utils/library-graph-posters";
import {
  drawGraphRelations,
  hitGraphRelation,
  type GraphRelation,
  type GraphRelationHit,
  type GraphRelationNode,
} from "@/lib/utils/library-graph-relations";
import { createGraphHoverIntent } from "@/lib/utils/library-graph-hover-intent";
import type { Facet, TitleRecord } from "@/lib/types/titles";
import {
  UNPLACED_CLUSTER_ID,
  type LibraryGraphResult,
} from "@/lib/types/library-graph";
import {
  edgePairKey,
  egoGraphState,
  egoRingGroups,
  toggleClusterExpanded,
  type GraphNode,
} from "@/lib/utils/library-graph-merge";
import { GraphAdjacency } from "@/lib/utils/library-graph-adjacency";
import {
  LIBRARY_GRAPH_COLLAPSE_THRESHOLD,
  LIBRARY_GRAPH_MAX_NODE_SIZE,
  LIBRARY_GRAPH_MIN_NODE_SIZE,
  displayedLibraryGraph,
  graphDetailClusters,
  graphDetailPosterSize,
  graphDetailOpacity,
  graphNeighborhoodBounds,
  largeLibraryGraphLayout,
  libraryGraphOverviewPreview,
  joinLibraryGraphToTitles,
  sameTitleRecords,
  libraryGraphEdgeKindLabels,
  libraryGraphGroupName,
  libraryGraphNamedClusterIds,
  neighborReason,
  neighborReasonLines,
  nodeSizesByTitleId,
  RECOMMENDATION_SECTION_LIMIT,
  recommendationSectionName,
  recommendationSections,
  titleIndexByMatchKey,
  titleMatchKeys,
  type LibraryGraphSizeMetric,
  type NeighborReason,
  type RecommendationSection,
} from "@/lib/utils/library-graph-model";
import {
  convexHull,
  createLayout,
  egoLayout,
  layoutLoopDecision,
  layoutPositions,
  stepLayout,
  tweenPositions,
  easeOutCubic,
  type LayoutPoint,
  type LayoutState,
} from "@/lib/utils/library-graph-layout";
import { libraryGraphSourceForCatalog } from "@/lib/utils/library-graph-source";
import {
  beginExpansion,
  collapseTraversalToCrumb,
  emptyTraversal,
  expandTraversal,
  firstNodeInCluster,
  ghostSubjectKey,
  invalidateExpansions,
  isCurrentExpansion,
  isGhostSubjectKey,
  LIBRARY_GRAPH_GHOST_CAP,
  isLibraryItem,
  moreLikeThisEdgeKinds,
  neighborTargetKey,
  nextNodeInCluster,
  ownedNodeIndex,
  ownedSubjectForItem,
  stampGhostTitle,
  seedGhostPositions,
  traversalGraphState,
  type KeyboardNode,
  type OwnedNodeIndex,
  type TraversalState,
} from "@/lib/utils/library-graph-traversal";

const CLUSTER_COLORS = [
  "#5b64ff",
  "#14b8a6",
  "#f59e0b",
  "#ec4899",
  "#22c55e",
  "#8b5cf6",
  "#06b6d4",
  "#ef4444",
];
const UNPLACED_COLOR = "#7a8499";
/** How long the zoom must rest before posters load a sharper variant. */
const POSTER_UPGRADE_REST_MS = 150;
/** A reload of an already drawn graph that finishes sooner shows nothing. */
const REFRESH_INDICATOR_DELAY_MS = 600;
const LAYOUT_STEPS_PER_FRAME = 6;
const MIN_POSTER_RADIUS_PX = 18;
const HULL_PADDING_PX = 18;
// How far a neighbourhood name may wander from its titles to find a spot
// that covers none of them: rings this far apart, this many rings.
const HULL_LABEL_SEARCH_STEP_PX = 14;
const HULL_LABEL_SEARCH_RINGS = 10;
/** How many neighbours one expansion asks for. */
const NEIGHBOR_LIMIT = 24;
const GHOST_NODE_SIZE = LIBRARY_GRAPH_MIN_NODE_SIZE + 1;
/** How long nodes take to move between the overview and a focused node. */
const FOCUS_TWEEN_MS = 450;
// How long past the glide a picked title may wait to open if no glide ends.
const SELECT_AFTER_GLIDE_SLACK_MS = 80;
/** The outer ring of a focused node's neighbours, as a share of the frame. */
const FOCUS_OUTER_RADIUS_SHARE = 0.78;
/** The least on-screen gap between neighbours on one ring. */
const FOCUS_RING_SPACING_PX = 68;
/** Room left beside the outer ring for its labels. */
const FOCUS_LABEL_ROOM_PX = 130;
const FOCUS_MIN_OUTER_RADIUS_PX = 90;
// Focused-view node radii in screen pixels, large enough that every title on
// the rings shows its poster.
const FOCUS_CENTER_RADIUS_PX = 28;
const FOCUS_NEIGHBOR_MIN_RADIUS_PX = 17;
const FOCUS_NEIGHBOR_MAX_RADIUS_PX = 22;
// Recommendations on the outer rings.
const FOCUS_GHOST_RADIUS_PX = 26;
/** A focused neighbour's radius: its overview size, spread over the range. */
function focusNeighborRadiusPx(metricSize: number) {
  const share = Math.min(
    1,
    Math.max(
      0,
      (metricSize - LIBRARY_GRAPH_MIN_NODE_SIZE) /
        (LIBRARY_GRAPH_MAX_NODE_SIZE - LIBRARY_GRAPH_MIN_NODE_SIZE),
    ),
  );
  return (
    FOCUS_NEIGHBOR_MIN_RADIUS_PX +
    share * (FOCUS_NEIGHBOR_MAX_RADIUS_PX - FOCUS_NEIGHBOR_MIN_RADIUS_PX)
  );
}
/** Focused-view labels longer than this are cut short. */
const FOCUS_LABEL_MAX_CHARS = 28;

type NodeAttributes = {
  x: number;
  y: number;
  size: number;
  color: string;
  baseSize: number;
  baseColor: string;
  unselectedSize?: number;
  unselectedColor?: string;
  label: string;
  titleId?: string;
  posterUrl?: string | null;
  collapsedClusterId?: number;
  detailOpacity?: number;
  hull?: readonly LayoutPoint[];
  dots?: readonly LayoutPoint[];
  clusterId: number;
  /** A title reached by exploring, not in the library. */
  ghost?: boolean;
};

// Layout frames only move nodes, so sigma is told only positions changed.
const POSITION_HINTS: { attributes: (keyof NodeAttributes)[] } = {
  attributes: ["x", "y"],
};

type GraphLoadStatus = "pending" | "loaded" | "notBuilt" | "failed";

type ExpandNotice = { key: string; kind: "empty" };

type EdgeAttributes = {
  size: number;
  color: string;
  kinds: string[];
};

type LibraryGraphStore = Graph<NodeAttributes, EdgeAttributes>;

type Hover =
  | { kind: "node"; key: string; x: number; y: number; lines: string[] }
  | {
      kind: "edge";
      from: string;
      to: string;
      x: number;
      y: number;
      lines: string[];
    };

/**
 * A named neighbourhood's colour, by its place in `order`. Any other title,
 * in no named neighbourhood, takes the plain unplaced colour.
 */
function clusterColor(clusterId: number, order: readonly number[]): string {
  const position = order.indexOf(clusterId);
  if (position < 0) {
    return UNPLACED_COLOR;
  }
  return CLUSTER_COLORS[position % CLUSTER_COLORS.length];
}

function graphTransitionBlobs(
  groups: readonly { id: number; count: number; name: string; color: string }[],
  layout: ReturnType<typeof largeLibraryGraphLayout> | null,
  expanded: ReadonlySet<number>,
  spacing: number,
  sizeScale: number,
): NodeAttributes[] {
  const opacity = 1 - graphDetailOpacity(spacing);
  if (!layout || opacity === 0) return [];
  return groups.flatMap((group) => {
    const center = layout.positions.get(`cluster:${group.id}`);
    if (!center || !expanded.has(group.id)) return [];
    const baseSize = 24 * Math.sqrt(group.count);
    return [
      {
        ...center,
        label: group.name,
        clusterId: group.id,
        collapsedClusterId: group.id,
        hull: layout.hulls.get(group.id),
        dots: layout.dots.get(group.id),
        baseSize,
        size: (baseSize * spacing) / 34 / sizeScale,
        color: group.color,
        baseColor: group.color,
        detailOpacity: opacity,
      },
    ];
  });
}

// The camera at rest: unzoomed and centred on the layout's origin, where
// cluster rings and focused views are centred.
/** How far titles away from the hovered one fade. */
const HOVER_FADE_ALPHA = 0.18;

/** Room around the titles when the overview is fitted to them. */
const OVERVIEW_FIT_PADDING = 1.25;

// Zoom stops where a typical (median) title fills this share of the view's
// height, and a little past the framing of the whole graph (ratio 1).
const MAX_POSTER_VIEW_SHARE = 0.85;
const MAX_ZOOM_OUT_RATIO = 1.5;

function applyZoomLimits(
  sigma: Sigma<NodeAttributes, EdgeAttributes>,
  graph: LibraryGraphStore,
) {
  // Titles only: a folded neighbourhood is drawn far larger than a title
  // and would stop the zoom long before titles are readable. The median,
  // not the largest: the overview draws its best-rated titles several times
  // the size of the rest.
  const sizes: number[] = [];
  graph.forEachNode((_key, attributes) => {
    if (attributes.collapsedClusterId !== undefined) return;
    sizes.push(attributes.size);
  });
  sizes.sort((a, b) => a - b);
  const typical = sizes.length > 0 ? sizes[sizes.length >> 1] : 0;
  const height = sigma.getContainer().offsetHeight;
  // A title's size is its on-screen half-height at ratio 1.
  const minRatio =
    typical > 0 && height > 0
      ? Math.min(
          MAX_ZOOM_OUT_RATIO,
          (sigma.scaleSize(typical, 1) * 2) / (MAX_POSTER_VIEW_SHARE * height),
        )
      : null;
  // Set on the camera itself: `setSetting` refreshes every node, and this
  // runs each time a neighbourhood unfolds during a zoom.
  const camera = sigma.getCamera();
  camera.minRatio = minRatio;
  camera.maxRatio = MAX_ZOOM_OUT_RATIO;
}

// Panning keeps the view's centre over the titles' extent, widened by this
// share of its larger side (and at least the minimum, in graph units), so
// the graph cannot be dragged away into empty space.
const PAN_MARGIN_EXTENT_SHARE = 0.1;
const PAN_MARGIN_MIN = 200;

/**
 * Bounds the camera's centre to the graph's own extent. Sigma's pan
 * boundaries clamp to its node extent, which with `autoRescale: false` is the
 * viewport around the origin rather than the graph, so the camera is
 * constrained here instead. The extent is measured again only after the
 * graph changes. Returns the cleanup.
 */
function boundCameraToGraph(
  sigma: Sigma<NodeAttributes, EdgeAttributes>,
  graph: LibraryGraphStore,
) {
  let extent: {
    minX: number;
    minY: number;
    maxX: number;
    maxY: number;
  } | null = null;
  let stale = true;
  const markStale = () => {
    stale = true;
  };
  const events = [
    "nodeAdded",
    "nodeDropped",
    "nodeAttributesUpdated",
    "eachNodeAttributesUpdated",
    "cleared",
  ] as const;
  for (const event of events) graph.on(event, markStale);
  const camera = sigma.getCamera();
  const validate = camera.validateState.bind(camera);
  camera.validateState = (state) => {
    const valid = validate(state);
    if (stale) {
      stale = false;
      extent = null;
      graph.forEachNode((_key, { x, y }) => {
        if (!extent) extent = { minX: x, minY: y, maxX: x, maxY: y };
        else {
          extent.minX = Math.min(extent.minX, x);
          extent.minY = Math.min(extent.minY, y);
          extent.maxX = Math.max(extent.maxX, x);
          extent.maxY = Math.max(extent.maxY, y);
        }
      });
    }
    const bounds = extent as typeof extent;
    if (!bounds) return valid;
    const margin = Math.max(
      PAN_MARGIN_MIN,
      Math.max(bounds.maxX - bounds.minX, bounds.maxY - bounds.minY) *
        PAN_MARGIN_EXTENT_SHARE,
    );
    const a = sigma.graphToFramedGraph({
      x: bounds.minX - margin,
      y: bounds.minY - margin,
    });
    const b = sigma.graphToFramedGraph({
      x: bounds.maxX + margin,
      y: bounds.maxY + margin,
    });
    // A view with no size yet has no projection to bound against.
    if (![a.x, a.y, b.x, b.y].every(Number.isFinite)) return valid;
    valid.x = Math.min(
      Math.max(valid.x, Math.min(a.x, b.x)),
      Math.max(a.x, b.x),
    );
    valid.y = Math.min(
      Math.max(valid.y, Math.min(a.y, b.y)),
      Math.max(a.y, b.y),
    );
    return valid;
  };
  return () => {
    for (const event of events) graph.off(event, markStale);
    camera.validateState = validate;
  };
}

function restCamera(sigma: Sigma<NodeAttributes, EdgeAttributes>) {
  const origin = sigma.graphToFramedGraph({ x: 0, y: 0 });
  return { x: origin.x, y: origin.y, ratio: 1, angle: 0 };
}

// The camera framing the nodes `include` keeps, with `padding` around them,
// or null when it keeps none.
function framingCamera(
  sigma: Sigma<NodeAttributes, EdgeAttributes>,
  graph: LibraryGraphStore,
  include: (attributes: NodeAttributes) => boolean,
  padding: number,
) {
  let minX = Number.POSITIVE_INFINITY;
  let minY = Number.POSITIVE_INFINITY;
  let maxX = Number.NEGATIVE_INFINITY;
  let maxY = Number.NEGATIVE_INFINITY;
  graph.forEachNode((_key, attributes) => {
    if (!include(attributes)) {
      return;
    }
    const point = sigma.graphToFramedGraph({
      x: attributes.x as number,
      y: attributes.y as number,
    });
    minX = Math.min(minX, point.x);
    minY = Math.min(minY, point.y);
    maxX = Math.max(maxX, point.x);
    maxY = Math.max(maxY, point.y);
  });
  if (!Number.isFinite(minX)) {
    return null;
  }
  const span = Math.max(maxX - minX, maxY - minY);
  return {
    x: (minX + maxX) / 2,
    y: (minY + maxY) / 2,
    ratio: Math.min(1, Math.max(0.08, span * padding)),
  };
}

/**
 * Fade every drawn title together. The renderer's node layer holds only
 * titles (folded neighbourhoods are transparent there and painted on the
 * overlay), so the layer's opacity stands for each title's.
 */
const SELECTION_HINTS: { attributes: (keyof NodeAttributes)[] } = {
  attributes: ["color", "size"],
};

function applyDetailOpacity(
  sigma: Sigma<NodeAttributes, EdgeAttributes>,
  ref: { current: number },
  opacity: number,
) {
  ref.current = opacity;
  sigma.getStageCanvas().style.opacity = opacity >= 1 ? "" : String(opacity);
}

/**
 * The overview's poster geometry for one frame: graph coordinates to
 * viewport pixels, and graph sizes to pixel radii. Sigma's projection is
 * affine, so three projected points fix it exactly, and each title then
 * costs a few multiplications instead of the per-call matrix work and
 * allocations of `graphToViewport`.
 */
function viewportProjection(sigma: Sigma<NodeAttributes, EdgeAttributes>) {
  const origin = sigma.graphToViewport({ x: 0, y: 0 });
  const alongX = sigma.graphToViewport({ x: 1, y: 0 });
  const alongY = sigma.graphToViewport({ x: 0, y: 1 });
  return {
    x0: origin.x,
    y0: origin.y,
    xx: alongX.x - origin.x,
    xy: alongX.y - origin.y,
    yx: alongY.x - origin.x,
    yy: alongY.y - origin.y,
    // Sizes scale linearly with the camera; the node reducer leaves them
    // as stored, so the stored size is the drawn one.
    sizeScale: sigma.scaleSize(1),
  };
}

function withAlpha(hex: string, alpha: number): string {
  const value = Number.parseInt(hex.slice(1), 16);
  return `rgba(${(value >> 16) & 255}, ${(value >> 8) & 255}, ${value & 255}, ${alpha})`;
}

/**
 * The shortest arc that covers every angle, as a start and an end at or
 * after it. A band's titles can sit either side of wherever angles wrap.
 */
function coveringArc(angles: readonly number[]): {
  start: number;
  end: number;
} {
  const sorted = [...angles].sort((left, right) => left - right);
  let start = sorted[0] ?? 0;
  let largestGap = -1;
  for (let index = 0; index < sorted.length; index += 1) {
    const current = sorted[index];
    const next =
      index + 1 < sorted.length ? sorted[index + 1] : sorted[0] + Math.PI * 2;
    if (next - current > largestGap) {
      largestGap = next - current;
      start = next;
    }
  }
  return { start, end: start + Math.max(0, Math.PI * 2 - largestGap) };
}

function readCssColor(element: Element, name: string, fallback: string) {
  const value = getComputedStyle(element).getPropertyValue(name).trim();
  return value || fallback;
}

/**
 * A theme colour read once and kept until the theme changes. Reading one
 * every frame would recalculate styles after the frame's own DOM writes.
 */
function cachedCssColor(
  cache: Map<string, string>,
  element: Element,
  name: string,
  fallback: string,
) {
  let value = cache.get(name);
  if (value === undefined) {
    value = readCssColor(element, name, fallback);
    cache.set(name, value);
  }
  return value;
}

export type LibraryGraphViewProps = {
  facet: Facet;
  titles: readonly TitleRecord[];
  /**
   * Whether `titles` is the whole filtered catalog. While pages are still
   * loading the graph keeps showing the last complete set instead of
   * rebuilding on every page.
   */
  titlesComplete: boolean;
  allowOverviewPreview: boolean;
  totalTitles: number;
  loadedTitles?: number;
  libraryIds: readonly string[];
  selectedTitleId: string | null;
  contextPanelId?: string;
  controlsRef?: React.Ref<{ fit: () => void }>;
  onSelectTitle: (title: TitleRecord) => void;
  onOpenOverview: (title: TitleRecord) => void;
  /** Refreshes the catalog listing after a title is added or requested. */
  onCatalogChanged?: () => Promise<void> | void;
};

type NeighborsData = {
  discoveryNeighbors?: DiscoveryNeighbors | null;
};

/**
 * The catalog as a graph: one node per title, sized by a chosen metric, linked
 * by weighted edges and grouped into labelled cluster hulls. The node set is
 * the catalog's current filtered titles; filtering prunes nodes and the layout
 * settles from where it was.
 */
export default function LibraryGraphView({
  facet,
  titles: currentTitles,
  titlesComplete,
  allowOverviewPreview,
  totalTitles,
  loadedTitles = currentTitles.length,
  libraryIds,
  selectedTitleId,
  contextPanelId,
  controlsRef,
  onSelectTitle,
  onOpenOverview,
  onCatalogChanged,
}: LibraryGraphViewProps) {
  const t = useTranslate();
  const containerRef = React.useRef<HTMLDivElement | null>(null);
  const overlayRef = React.useRef<HTMLCanvasElement | null>(null);
  const hullLabelRefs = React.useRef(new Map<number, HTMLButtonElement>());
  const sigmaRef = React.useRef<Sigma<NodeAttributes, EdgeAttributes> | null>(
    null,
  );
  const graphRef = React.useRef<LibraryGraphStore | null>(null);
  const relationsStoreRef =
    React.useRef<GraphAdjacency<EdgeAttributes> | null>(null);
  const layoutRef = React.useRef<LayoutState | null>(null);
  const frameRef = React.useRef<number | null>(null);
  const tweenFrameRef = React.useRef<number | null>(null);
  // The overview frame's half-width in layout units; focused rings share it.
  const extentRef = React.useRef(220);
  // The focused node the camera last came to rest for; undefined before the
  // first drawing.
  const cameraCenterRef = React.useRef<string | null | undefined>(undefined);
  // Whether the overview fits its titles once its layout settles.
  const fitWhenSettledRef = React.useRef(false);
  // Bumped when a focused view's frame changes size, so its rings refit.
  const [focusFrameVersion, setFocusFrameVersion] = React.useState(0);
  // Restarts the layout loop after it paused for a hidden container.
  const resumeLayoutRef = React.useRef<() => void>(() => {});
  const positionsRef = React.useRef(new Map<string, LayoutPoint>());
  const posterCacheRef = React.useRef(new Map<string, HTMLImageElement>());
  const posterZoomRef = React.useRef({
    ratio: Number.NaN,
    changedAt: Number.NEGATIVE_INFINITY,
    timer: 0,
  });
  const viewportNodesRef = React.useRef(new Set<string>());
  const viewportRelationsRef = React.useRef(new Map<string, GraphRelation>());
  const relationHitsRef = React.useRef<GraphRelationHit[]>([]);
  const largeLayoutRef = React.useRef<ReturnType<
    typeof largeLibraryGraphLayout
  > | null>(null);
  const hoverableNodesRef = React.useRef(new Set<string>());
  const hoverIntentRef = React.useRef<ReturnType<
    typeof createGraphHoverIntent
  > | null>(null);
  const hoveredEdgeRef = React.useRef<{ from: string; to: string } | null>(
    null,
  );
  const transitionBlobsRef = React.useRef<NodeAttributes[]>([]);
  // How far the titles of unfolded neighbourhoods have faded in. One value
  // for every title: the renderer's node layer fades as a whole through its
  // canvas opacity, so a zoom never rewrites thousands of node colours.
  const detailOpacityRef = React.useRef(1);
  const posterSpritesRef = React.useRef<GraphPosterSprites | null>(null);
  // The title under the pointer and the titles one link from it, which stay
  // bright while the rest of the graph fades.
  const hoverFocusRef = React.useRef<{
    key: string;
    near: ReadonlySet<string>;
    accent: string;
  } | null>(null);
  const selectedTitleIdRef = React.useRef(selectedTitleId);
  const titlesByIdRef = React.useRef(new Map<string, TitleRecord>());
  const onSelectTitleRef = React.useRef(onSelectTitle);
  // A title picked in the graph opens once the graph has glided to it, so
  // the title's panel does not render over the glide.
  const pendingSelectRef = React.useRef<{
    title: TitleRecord;
    timer: number;
  } | null>(null);
  // Theme colours the overlay draws with; emptied when the theme changes.
  const themeColorsRef = React.useRef(new Map<string, string>());
  // Each neighbourhood name's size, measured once per name rather than every
  // frame between writes to the other names' positions.
  const hullLabelSizesRef = React.useRef(
    new WeakMap<HTMLElement, { text: string; w: number; h: number }>(),
  );
  // The node picked in the graph (owned or ghost) and the node keyboard focus
  // rests on; the overlay reads both while it draws.
  const selectedKeyRef = React.useRef<string | null>(null);
  const focusedKeyRef = React.useRef<string | null>(null);
  // The node a focused view is centred on, while the view is focused.
  const focusCenterRef = React.useRef<string | null>(null);
  const activateNodeRef = React.useRef<
    (key: string, openTitle: boolean) => void
  >(() => {});
  const ownedIndexRef = React.useRef<OwnedNodeIndex>(ownedNodeIndex([]));
  const expandTokenRef = React.useRef(0);
  const client = useClient();
  const auth = useAuth();
  const setGlobalStatus = useGlobalStatus();
  // After an Add, the acted-on ghost is marked as the title it became, and
  // the catalog listing is refreshed either way.
  const catalogChangedRef = React.useRef<
    (change: { addedTitleId: string | null }) => void
  >(() => {});
  const handleCatalogChanged = React.useCallback(
    (change: { addedTitleId: string | null }) => {
      catalogChangedRef.current(change);
    },
    [],
  );
  const moreLikeThisActions = useTitleMoreLikeThisActions({
    onCatalogChanged: handleCatalogChanged,
    canAddItems: hasAnyLibraryPermission(
      auth.user,
      LIBRARY_PERMISSIONS.manageTitles,
    ),
    canRequestItems: hasAnyLibraryPermission(
      auth.user,
      LIBRARY_PERMISSIONS.request,
    ),
  });

  const [titles, setTitles] = React.useState<readonly TitleRecord[] | null>(
    titlesComplete ? currentTitles : null,
  );
  // The last listing compared, so an unchanged refetch is compared once.
  const [seenTitles, setSeenTitles] = React.useState<
    readonly TitleRecord[] | null
  >(titlesComplete ? currentTitles : null);
  if (
    titlesComplete &&
    titles !== currentTitles &&
    seenTitles !== currentTitles
  ) {
    setSeenTitles(currentTitles);
    if (!titles || !sameTitleRecords(titles, currentTitles))
      setTitles(currentTitles);
  }
  const [result, setResult] = React.useState<LibraryGraphResult | null>(null);
  const [loadStatus, setLoadStatus] =
    React.useState<GraphLoadStatus>("pending");
  // What has been explored from the library this session; never persisted.
  const [traversal, setTraversal] = React.useState<
    TraversalState<DiscoveryItem>
  >(() => emptyTraversal<DiscoveryItem>());
  const [selectedKey, setSelectedKey] = React.useState<string | null>(null);
  const [selectionCardOpen, setSelectionCardOpen] = React.useState(true);
  const [focusedKey, setFocusedKey] = React.useState<string | null>(null);
  // Why each explored result sits next to the title it was found from, by
  // the pair of nodes their link joins.
  const [reasonsByPair, setReasonsByPair] = React.useState<
    ReadonlyMap<string, NeighborReason>
  >(() => new Map());
  // Replies already read this session, by facet and target key, so going
  // back to a title does not ask again.
  const neighborCacheRef = React.useRef(new Map<string, DiscoveryNeighbors>());
  const [expandingKey, setExpandingKey] = React.useState<string | null>(null);
  const [expandNotice, setExpandNotice] = React.useState<ExpandNotice | null>(
    null,
  );
  const [metric] = React.useState<LibraryGraphSizeMetric>("ratingEvidence");
  const [expandedClusterIds, setExpandedClusterIds] = React.useState<
    ReadonlySet<number>
  >(() => new Set());
  const [selectedNeighborhoods, setSelectedNeighborhoods] = React.useState<
    ReadonlySet<number>
  >(() => new Set());
  const [hover, setHover] = React.useState<Hover | null>(null);

  const applySelection = React.useCallback(() => {
    const graph = graphRef.current;
    const container = containerRef.current;
    if (!graph || !container) {
      return;
    }
    const accent = readCssColor(container, "--scry-accent", "#5b64ff");
    const selected = selectedTitleIdRef.current;
    const selectedKey = selectedKeyRef.current;
    graph.updateEachNodeAttributes((key, attributes) => {
      const isSelected =
        selectedKey !== null
          ? key === selectedKey
          : attributes.titleId !== undefined && attributes.titleId === selected;
      return {
        ...attributes,
        color: isSelected
          ? accent
          : (attributes.unselectedColor ?? attributes.baseColor),
        size:
          (attributes.unselectedSize ?? attributes.baseSize) *
          (isSelected ? 1.35 : 1),
      };
    }, SELECTION_HINTS);
  }, []);

  React.useEffect(() => {
    selectedTitleIdRef.current = selectedTitleId;
    applySelection();
  }, [applySelection, selectedTitleId]);
  React.useEffect(() => {
    onSelectTitleRef.current = onSelectTitle;
  }, [onSelectTitle]);
  const flushPendingSelect = React.useCallback(() => {
    const pending = pendingSelectRef.current;
    if (!pending) {
      return;
    }
    pendingSelectRef.current = null;
    window.clearTimeout(pending.timer);
    onSelectTitleRef.current(pending.title);
  }, []);
  React.useEffect(
    () => () => {
      const pending = pendingSelectRef.current;
      if (pending) {
        window.clearTimeout(pending.timer);
        pendingSelectRef.current = null;
      }
    },
    [],
  );
  // A theme switch changes the colours the overlay drew with.
  React.useEffect(() => {
    const forget = () => {
      themeColorsRef.current.clear();
      sigmaRef.current?.scheduleRender();
    };
    const observer = new MutationObserver(forget);
    observer.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ["class", "data-theme", "style"],
    });
    const scheme = window.matchMedia("(prefers-color-scheme: dark)");
    scheme.addEventListener("change", forget);
    return () => {
      observer.disconnect();
      scheme.removeEventListener("change", forget);
    };
  }, []);

  const libraryIdSignature = [...libraryIds].sort().join(",");

  // The graph covers the facet's selected libraries; filters only prune it.
  // It loads once per facet and library set, so a filter change keeps every
  // node where it was, and a filter that leaves no titles and is then cleared
  // does not load it again. A new facet or library set starts exploring
  // afresh.
  const sourceKey = `${facet}|${libraryIdSignature}`;
  // A new facet or library set drops what was explored from the old one,
  // including a "More like this" still waiting on its reply.
  const [exploredSourceKey, setExploredSourceKey] = React.useState(sourceKey);
  // Laid-out positions belong to one source. This runs before the layout
  // effects below in the commit that switches source.
  React.useLayoutEffect(() => {
    positionsRef.current.clear();
  }, [sourceKey]);
  if (exploredSourceKey !== sourceKey) {
    setExploredSourceKey(sourceKey);
    setSelectedNeighborhoods(new Set());
    // The old graph is not joined to the new listing while the new one loads.
    setResult(null);
    setLoadStatus("pending");
    // Cluster ids belong to one graph.
    setExpandedClusterIds(new Set());
    setTraversal(emptyTraversal<DiscoveryItem>());
    setReasonsByPair(new Map());
    setSelectedKey(null);
    setFocusedKey(null);
    setExpandingKey(null);
    setExpandNotice(null);
  }
  // Starts the exploration over: drops the explored titles, the pick and
  // focus, and retires any "More like this" reply still on its way.
  const resetExploration = React.useCallback(() => {
    invalidateExpansions(expandTokenRef);
    neighborCacheRef.current.clear();
    setReasonsByPair(new Map());
    setExpandedClusterIds(new Set());
    setTraversal(emptyTraversal<DiscoveryItem>());
    setSelectedKey(null);
    setFocusedKey(null);
    setExpandingKey(null);
    setExpandNotice(null);
  }, []);
  const loadedSourceKeyRef = React.useRef<string | null>(null);
  React.useEffect(() => {
    if (loadedSourceKeyRef.current === sourceKey) {
      return;
    }
    loadedSourceKeyRef.current = sourceKey;
    invalidateExpansions(expandTokenRef);
    let cancelled = false;
    let settled = false;
    void libraryGraphSourceForCatalog(client)
      .load({
        facet,
        libraryIds: libraryIdSignature ? libraryIdSignature.split(",") : [],
      })
      .then((loaded) => {
        settled = true;
        if (!cancelled) {
          setResult(loaded);
          setLoadStatus(loaded ? "loaded" : "notBuilt");
          resetExploration();
        }
      })
      .catch(() => {
        settled = true;
        if (!cancelled) {
          setResult(null);
          setLoadStatus("failed");
          resetExploration();
        }
      });
    return () => {
      cancelled = true;
      // A load cut short is not a load: the next run starts it again.
      if (!settled && loadedSourceKeyRef.current === sourceKey) {
        loadedSourceKeyRef.current = null;
      }
    };
  }, [client, facet, libraryIdSignature, resetExploration, sourceKey]);

  const previewing =
    !titlesComplete &&
    allowOverviewPreview &&
    (result?.nodes.length ?? 0) > LIBRARY_GRAPH_COLLAPSE_THRESHOLD;
  const overviewPreview = React.useMemo(
    () => (result && previewing ? libraryGraphOverviewPreview(result) : null),
    [result, previewing],
  );
  const join = React.useMemo(
    () =>
      !result
        ? null
        : previewing
          ? overviewPreview
          : titlesComplete && titles
            ? joinLibraryGraphToTitles(result, titles)
            : null,
    [result, titles, titlesComplete, previewing, overviewPreview],
  );
  const [paintedJoin, setPaintedJoin] = React.useState<typeof join>(null);
  React.useEffect(() => {
    if (!join) return;
    let frame = requestAnimationFrame(() => {
      frame = requestAnimationFrame(() => setPaintedJoin(join));
    });
    return () => cancelAnimationFrame(frame);
  }, [join]);
  const titlesById = React.useMemo(() => {
    const byId = new Map<string, TitleRecord>();
    for (const title of join?.titleBySubjectKey.values() ?? []) {
      byId.set(title.id, title);
    }
    return byId;
  }, [join]);
  // Every key a title of the catalog listing is known by.
  const catalogIndex = React.useMemo(
    () => titleIndexByMatchKey(titles ?? []),
    [titles],
  );
  // Every node of the built graph by its target key, and each node whose
  // title is in the current listing by every key that title is known by: an
  // explored result that matches links to the node instead of joining as a
  // ghost. A node filtered out of the listing is known only by its target
  // key, so a result naming it another way, or a title added after the graph
  // was built, still joins as a ghost; such a ghost carries the local title it
  // resolved to and the card treats it as owned.
  const ownedIndex = React.useMemo(
    () =>
      ownedNodeIndex(
        (result?.nodes ?? []).map((node) => {
          const title = join?.titleBySubjectKey.get(node.subjectKey);
          return {
            subjectKey: node.subjectKey,
            targetKey: node.targetKey,
            titleId: title?.id ?? null,
            matchKeys: title ? titleMatchKeys(title) : [],
          };
        }),
      ),
    [join, result],
  );
  React.useEffect(() => {
    ownedIndexRef.current = ownedIndex;
  }, [ownedIndex]);
  // A ghost dropped from the exploration (cut back on the trail, or pruned at
  // the cap) is no longer picked or focused, so reaching it again later does
  // not reopen its card.
  if (
    selectedKey !== null &&
    isGhostSubjectKey(selectedKey) &&
    !traversal.ghostItems.has(selectedKey)
  ) {
    setSelectedKey(null);
  }
  if (
    focusedKey !== null &&
    isGhostSubjectKey(focusedKey) &&
    !traversal.ghostItems.has(focusedKey)
  ) {
    setFocusedKey(null);
  }
  const explored = React.useMemo(
    () => (join ? traversalGraphState(join.graph, traversal) : null),
    [join, traversal],
  );
  const exploredNodesByKey = React.useMemo(
    () =>
      new Map((explored?.nodes ?? []).map((node) => [node.subjectKey, node])),
    [explored],
  );
  // The picked node, while it is in the graph, is the one the view focuses
  // on: only it and what it links to are drawn.
  const focusCenterKey =
    selectedKey !== null && exploredNodesByKey.has(selectedKey)
      ? selectedKey
      : null;
  // Collapsing follows the library's own size; explored ghosts never tip it.
  const largeLayout = React.useMemo(
    () =>
      join &&
      explored &&
      join.graph.nodes.length > LIBRARY_GRAPH_COLLAPSE_THRESHOLD
        ? largeLibraryGraphLayout(explored)
        : null,
    [join, explored],
  );
  React.useEffect(() => {
    largeLayoutRef.current = largeLayout;
  }, [largeLayout]);
  const overview = React.useMemo(
    () =>
      explored && join
        ? displayedLibraryGraph(
            explored,
            previewing ? new Set() : expandedClusterIds,
            LIBRARY_GRAPH_COLLAPSE_THRESHOLD,
            join.graph,
            selectedNeighborhoods,
          )
        : null,
    [expandedClusterIds, explored, join, selectedNeighborhoods, previewing],
  );
  const displayed = React.useMemo(
    () =>
      explored && focusCenterKey !== null
        ? (egoGraphState(explored, focusCenterKey) ?? overview)
        : overview,
    [explored, focusCenterKey, overview],
  );
  // A focused view's rings, inner first, each as sections around the circle:
  // library titles as one, recommendations by the neighbourhoods the server
  // found among the focused title's results, else by a shared genre or theme.
  const focusRings = React.useMemo(() => {
    if (focusCenterKey === null || !displayed) {
      return [];
    }
    const groups =
      traversal.expansions.find(
        (expansion) => expansion.anchor === focusCenterKey,
      )?.groups ?? [];
    const ghostKeys = new Set(
      displayed.nodes
        .filter((node) => node.ghost)
        .map((node) => node.subjectKey),
    );
    return egoRingGroups(displayed, focusCenterKey).map(
      (ring): RecommendationSection[] =>
        ring.length > 0 && ghostKeys.has(ring[0])
          ? recommendationSections(
              ring.map((key) => ({
                key,
                tags: traversal.ghostItems.get(key)?.canonicalTags,
              })),
              RECOMMENDATION_SECTION_LIMIT,
              groups,
            )
          : [{ tagKey: null, name: null, keys: ring }],
    );
  }, [displayed, focusCenterKey, traversal.expansions, traversal.ghostItems]);
  const recommendationBands = React.useMemo(
    () =>
      focusRings
        .flat()
        .filter((section) => section.name !== null)
        .map((section, index) => ({
          // Two neighbourhoods can carry the same label key.
          key: `${index}:${section.tagKey ?? ""}`,
          name: recommendationSectionName(section, t) ?? "",
          keys: section.keys,
          color: CLUSTER_COLORS[index % CLUSTER_COLORS.length],
        })),
    [focusRings, t],
  );
  const recommendationBandsRef = React.useRef(recommendationBands);
  React.useEffect(() => {
    recommendationBandsRef.current = recommendationBands;
  }, [recommendationBands]);
  // The neighbourhoods drawn as such: named, and big enough. Every other
  // cluster's titles, and unplaced ones, are only titles: no ring spot,
  // outline, name or colour of their own.
  const namedClusterIds = React.useMemo(
    () => libraryGraphNamedClusterIds(result?.clusters ?? []),
    [result],
  );
  const namedClusterIdsRef = React.useRef(namedClusterIds);
  React.useEffect(() => {
    namedClusterIdsRef.current = namedClusterIds;
  }, [namedClusterIds]);
  // Every named neighbourhood of the whole graph, before filtering, so layout
  // centres hold still when a filter empties one.
  const allClusterIds = React.useMemo(
    () => Array.from(namedClusterIds),
    [namedClusterIds],
  );
  const clusterOrder = React.useMemo(
    () =>
      (join?.graph.clusters ?? [])
        .map((cluster) => cluster.id)
        .filter((id) => namedClusterIds.has(id))
        .sort((a, b) => a - b),
    [join, namedClusterIds],
  );
  const groups = React.useMemo(() => {
    if (!join) {
      return [];
    }
    const counts = new Map<number, number>();
    for (const node of join.graph.nodes) {
      const id =
        join.graph.nodes.length > LIBRARY_GRAPH_COLLAPSE_THRESHOLD &&
        !namedClusterIds.has(node.clusterId)
          ? UNPLACED_CLUSTER_ID
          : node.clusterId;
      counts.set(id, (counts.get(id) ?? 0) + 1);
    }
    return Array.from(counts, ([id, count]) => ({
      id,
      count,
      name: libraryGraphGroupName(
        id,
        join.graph.clusters,
        join.titleBySubjectKey,
        t,
      ),
      color: clusterColor(id, clusterOrder),
      named: namedClusterIds.has(id),
    })).sort((a, b) =>
      a.id === UNPLACED_CLUSTER_ID
        ? 1
        : b.id === UNPLACED_CLUSTER_ID
          ? -1
          : a.id - b.id,
    );
  }, [clusterOrder, join, namedClusterIds, t]);
  // The groups shown by name: the named neighbourhoods, and the unplaced
  // titles when there are any.
  const shownGroups = React.useMemo(
    () =>
      groups.filter((group) => group.named || group.id === UNPLACED_CLUSTER_ID),
    [groups],
  );
  // Neighbourhoods are named and outlined only when there are several; one
  // neighbourhood is the whole library.
  const showsNeighbourhoods = groups.filter((group) => group.named).length > 1;
  const neighborhoodLegend =
    focusCenterKey !== null
      ? recommendationBands.map((band) => ({
          key: band.key,
          name: band.name,
          color: band.color,
          count: band.keys.length,
        }))
      : shownGroups.map((group) => ({
          key: String(group.id),
          name: group.name,
          color: group.color,
          count: group.count,
        }));
  const showsNeighbourhoodsRef = React.useRef(showsNeighbourhoods);
  React.useEffect(() => {
    showsNeighbourhoodsRef.current = showsNeighbourhoods;
  }, [showsNeighbourhoods]);
  const groupsRef = React.useRef(groups);
  React.useEffect(() => {
    groupsRef.current = groups;
  }, [groups]);

  const nodeLabel = React.useCallback(
    (node: GraphNode): string => {
      if (node.collapsedClusterId !== undefined) {
        return t("libraryGraph.collapsedCluster", {
          name: libraryGraphGroupName(
            node.collapsedClusterId,
            join?.graph.clusters ?? [],
            join?.titleBySubjectKey ?? new Map(),
            t,
          ),
          count: node.memberCount ?? 1,
        });
      }
      const item = node.ghost
        ? traversal.ghostItems.get(node.subjectKey)
        : undefined;
      if (item) {
        return discoveryItemDisplayTitle(item);
      }
      const title = join?.titleBySubjectKey.get(node.subjectKey);
      if (title) {
        return title.name;
      }
      return node.targetKey;
    },
    [join, t, traversal.ghostItems],
  );
  const keyboardNodes = React.useMemo<KeyboardNode[]>(
    () =>
      (displayed?.nodes ?? []).map((node) => ({
        key: node.subjectKey,
        // A focused view's arrow keys cycle through everything it shows.
        clusterId: focusCenterKey !== null ? 0 : node.clusterId,
        label: nodeLabel(node),
      })),
    [displayed, focusCenterKey, nodeLabel],
  );

  const drawOverlay = React.useCallback(() => {
    const sigma = sigmaRef.current;
    const graph = graphRef.current;
    const relations = relationsStoreRef.current;
    const canvas = overlayRef.current;
    if (!sigma || !graph || !relations || !canvas) {
      return;
    }
    // setGraph renders synchronously while graph replacement is detached.
    // Preserve the overlay and its viewport cache until this store is back,
    // so temporary camera matrices cannot be mixed with the previous scene.
    if (sigma.getGraph() !== graph) return;
    const { width, height } = sigma.getDimensions();
    const ratio = window.devicePixelRatio || 1;
    if (
      canvas.width !== Math.round(width * ratio) ||
      canvas.height !== Math.round(height * ratio)
    ) {
      canvas.width = Math.round(width * ratio);
      canvas.height = Math.round(height * ratio);
      canvas.style.width = `${width}px`;
      canvas.style.height = `${height}px`;
    }
    const context = canvas.getContext("2d");
    if (!context) {
      return;
    }
    // Sharper poster variants wait until the zoom has rested briefly.
    const zoom = sigma.getCamera().ratio;
    const posterZoom = posterZoomRef.current;
    const now = performance.now();
    if (zoom !== posterZoom.ratio) {
      if (Number.isFinite(posterZoom.ratio)) {
        posterZoom.changedAt = now;
        window.clearTimeout(posterZoom.timer);
        posterZoom.timer = window.setTimeout(
          () => sigmaRef.current?.scheduleRender(),
          POSTER_UPGRADE_REST_MS,
        );
      }
      posterZoom.ratio = zoom;
    }
    const zoomResting = now - posterZoom.changedAt >= POSTER_UPGRADE_REST_MS;
    context.setTransform(ratio, 0, 0, ratio, 0, 0);
    context.clearRect(0, 0, width, height);

    const { x0, y0, xx, xy, yx, yy, sizeScale } = viewportProjection(sigma);
    const project = (point: LayoutPoint): LayoutPoint => ({
      x: x0 + xx * point.x + yx * point.y,
      y: y0 + xy * point.x + yy * point.y,
    });
    const detailOpacity = detailOpacityRef.current;
    // Scale the margin with the title cloud instead of leaving a wide
    // screen-space border around tiny overview clusters.
    const hullPadding = Math.min(
      HULL_PADDING_PX,
      Math.hypot(xx * HULL_PADDING_PX, xy * HULL_PADDING_PX),
    );
    const pointsByCluster = new Map<number, LayoutPoint[]>();
    const viewportByNode = new Map<string, LayoutPoint>();
    const sizesByNode = new Map<string, number>();
    const viewportNodes = new Set<string>();
    const relationNodes = new Map<string, GraphRelationNode>();
    const drawZones: (() => void)[] = [];
    // Every drawn title, so neighbourhood names can keep off them.
    const nodeDiscs: (GraphPosterBounds & {
      x: number;
      y: number;
      r: number;
    })[] = [];
    // Built once the titles are all drawn, on the first name that needs it.
    let titleIndex: ReturnType<typeof createRectIndex> | null = null;
    const drawNode = (key: string, attributes: NodeAttributes) => {
      const point = project(attributes);
      viewportByNode.set(key, point);
      const size = attributes.size * sizeScale;
      sizesByNode.set(key, size);
      if (
        graph.hasNode(key) &&
        graphPosterInViewport(point, size, width, height)
      )
        viewportNodes.add(key);
      if (viewportNodes.has(key)) {
        const opacity =
          attributes.collapsedClusterId !== undefined
            ? 1
            : graphRelationOpacity(size, detailOpacity);
        relationNodes.set(key, { point, size, opacity });
        nodeDiscs.push({
          x: point.x,
          y: point.y,
          r: size,
          ...graphPosterBounds(point, size),
        });
      }
      // Ghosts are not in the library, so they never stretch a cluster hull.
      if (attributes.collapsedClusterId !== undefined) {
        if (
          point.x + size < 0 ||
          point.y + size < 0 ||
          point.x - size > width ||
          point.y - size > height
        )
          return;
        drawZones.push(() => {
          const color = attributes.baseColor;
          const hull = attributes.hull?.map(project);
          context.save();
          context.globalAlpha = attributes.detailOpacity ?? 1;
          context.beginPath();
          if (hull && hull.length > 2) {
            hull.forEach((point, index) => {
              if (index === 0) context.moveTo(point.x, point.y);
              else context.lineTo(point.x, point.y);
            });
          } else
            for (let i = 0; i < 24; i++) {
              const angle = (i * Math.PI) / 12;
              const radius = size * (1.12 + 0.04 * Math.sin(i * 2.3));
              const x = point.x + Math.cos(angle) * radius;
              const y = point.y + Math.sin(angle) * radius;
              if (i === 0) context.moveTo(x, y);
              else context.lineTo(x, y);
            }
          context.closePath();
          // The zone occludes aggregate connections beneath it; its mask
          // fades with the overview so detail links emerge continuously.
          context.save();
          context.globalCompositeOperation = "destination-out";
          context.fillStyle = "#000";
          context.strokeStyle = "#000";
          context.lineJoin = "round";
          context.lineWidth = hullPadding * 2;
          context.fill();
          context.stroke();
          context.restore();
          context.fillStyle = withAlpha(color, 0.1);
          context.fill();
          context.lineJoin = "round";
          context.lineWidth = hullPadding * 2;
          context.strokeStyle = withAlpha(color, 0.1);
          context.stroke();
          const count = Math.min(
            600,
            Math.max(24, Math.round((size * size) / 30)),
          );
          context.fillStyle = color;
          if (attributes.dots) {
            for (const dot of attributes.dots) {
              context.fillRect(
                x0 + xx * dot.x + yx * dot.y,
                y0 + xy * dot.x + yy * dot.y,
                1.5,
                2,
              );
            }
          } else
            for (let i = 0; i < count; i++) {
              const angle = i * 2.399963229728653;
              const radius = size * Math.sqrt((i + 0.5) / count);
              context.fillRect(
                point.x + Math.cos(angle) * radius,
                point.y + Math.sin(angle) * radius,
                1.5,
                2,
              );
            }
          const group = groupsRef.current.find(
            (group) => group.id === attributes.collapsedClusterId,
          );
          const name = group?.name ?? attributes.label;
          context.font = "600 12px system-ui, sans-serif";
          const halfWidth = context.measureText(name).width / 2 + 12;
          const y =
            (hull?.length
              ? Math.min(...hull.map((point) => point.y))
              : point.y - size * 1.15) - 16;
          context.beginPath();
          context.roundRect(point.x - halfWidth, y - 13, halfWidth * 2, 26, 13);
          context.fillStyle = cachedCssColor(
            themeColorsRef.current,
            canvas,
            "--scry-surfD",
            "#0b0e15",
          );
          context.fill();
          context.strokeStyle = color;
          context.lineWidth = 2;
          context.stroke();
          context.fillStyle = cachedCssColor(
            themeColorsRef.current,
            canvas,
            "--scry-ink2",
            "#e6e8ef",
          );
          context.textAlign = "center";
          context.textBaseline = "middle";
          context.fillText(name, point.x, y);
          context.restore();
        });
      }
      if (attributes.collapsedClusterId !== undefined || attributes.ghost) {
        return;
      }
      const clusterId = attributes.clusterId as number;
      if (clusterId === UNPLACED_CLUSTER_ID) {
        return;
      }
      const list = pointsByCluster.get(clusterId);
      if (list) {
        list.push(point);
      } else {
        pointsByCluster.set(clusterId, [point]);
      }
    };
    graph.forEachNode(drawNode);
    const changedEdges = changedGraphViewportEdges(
      viewportNodesRef.current,
      viewportNodes,
      (key) => (relations.hasNode(key) ? relations.edges(key) : []),
    );
    viewportNodesRef.current = viewportNodes;
    for (const key of changedEdges) {
      if (!relations.hasEdge(key)) {
        viewportRelationsRef.current.delete(key);
        continue;
      }
      const [from, to] = relations.extremities(key);
      if (!viewportNodes.has(from) || !viewportNodes.has(to))
        viewportRelationsRef.current.delete(key);
      else
        viewportRelationsRef.current.set(key, {
          ...relations.getEdgeAttributes(key),
          key,
          from,
          to,
        });
    }
    const hoverFocus = hoverFocusRef.current;
    relationHitsRef.current = hoverFocus
      ? []
      : drawGraphRelations(
          context,
          viewportRelationsRef.current.values(),
          relationNodes,
          null,
        );
    if (focusCenterRef.current === null) {
      transitionBlobsRef.current.forEach((attributes, index) =>
        drawNode(`transition:${index}`, attributes),
      );
    }

    // Aggregate relations sit below every overview zone, including during
    // the detail crossfade. Posters are painted after all zones and bands.
    for (const draw of drawZones) draw();

    // Cluster hulls, padded by stroking the hull with a wide round pen.
    context.lineJoin = "round";
    context.lineCap = "round";
    const focusCenter = focusCenterRef.current;
    // Neighbourhood names already placed this frame, so later ones step aside.
    const placedHullLabels: { x: number; y: number; w: number; h: number }[] =
      [];
    for (const group of groupsRef.current) {
      const label = hullLabelRefs.current.get(group.id);
      const points = pointsByCluster.get(group.id);
      // A focused view draws no cluster hulls, and neither does a graph
      // with one neighbourhood, which would only outline everything.
      if (
        focusCenter !== null ||
        !showsNeighbourhoodsRef.current ||
        !group.named ||
        !points ||
        points.length === 0
      ) {
        if (label) {
          label.style.visibility = "hidden";
        }
        continue;
      }
      const cachedHull = largeLayoutRef.current?.hulls.get(group.id);
      const hull = cachedHull ? cachedHull.map(project) : convexHull(points);
      const opacity =
        1 -
        (transitionBlobsRef.current.find(
          (blob) => blob.collapsedClusterId === group.id,
        )?.detailOpacity ?? 0);
      if (label) label.style.opacity = String(opacity);
      context.save();
      context.globalAlpha = opacity;
      context.beginPath();
      if (hull.length === 1) {
        // One node, or several on one spot: a zero-length path strokes
        // nothing, so the hull is the padding circle around that point.
        context.arc(hull[0].x, hull[0].y, hullPadding, 0, Math.PI * 2);
      } else {
        hull.forEach((point, index) => {
          if (index === 0) {
            context.moveTo(point.x, point.y);
          } else {
            context.lineTo(point.x, point.y);
          }
        });
      }
      context.closePath();
      context.fillStyle = withAlpha(group.color, 0.1);
      context.fill();
      if (hull.length > 1) {
        // Use the same zoom-scaled margin as the overview zone.
        context.strokeStyle = withAlpha(group.color, 0.1);
        context.lineWidth = hullPadding * 2;
        context.stroke();
      }
      if (label) {
        // The name sits where the neighbourhood's titles are, at their
        // median, not on the hull's rim: hulls stretch far past their
        // core and overlap each other.
        const median = (values: number[]) => {
          const sorted = [...values].sort((a, b) => a - b);
          return sorted[sorted.length >> 1];
        };
        const text = label.textContent ?? "";
        let size = hullLabelSizesRef.current.get(label);
        if (!size || size.text !== text) {
          size = {
            text,
            w: label.offsetWidth || 80,
            h: label.offsetHeight || 22,
          };
          hullLabelSizesRef.current.set(label, size);
        }
        const { w, h } = size;
        const clampX = (value: number) =>
          Math.min(Math.max(value, w / 2 + 4), width - w / 2 - 4);
        const clampY = (value: number) =>
          Math.min(Math.max(value, h / 2 + 4), height - h / 2 - 4);
        const baseX = median(points.map((point) => point.x));
        const baseY = median(points.map((point) => point.y));
        // A spot is clear when the name covers no title and no other name.
        const coversTitle = (cx: number, cy: number) =>
          (titleIndex ??= createRectIndex(nodeDiscs, width, height)).overlaps(
            cx - w / 2 - 3,
            cy - h / 2 - 3,
            cx + w / 2 + 3,
            cy + h / 2 + 3,
          );
        const coversName = (cx: number, cy: number) =>
          placedHullLabels.some(
            (other) =>
              Math.abs(other.x - cx) < (other.w + w) / 2 + 4 &&
              Math.abs(other.y - cy) < (other.h + h) / 2 + 4,
          );
        // Search outward from the middle of the titles, rings of growing
        // radius, trying above first, for the nearest clear spot.
        let x = clampX(baseX);
        let y = clampY(baseY);
        let fallback: { x: number; y: number } | null = null;
        search: for (let ring = 0; ring <= HULL_LABEL_SEARCH_RINGS; ring += 1) {
          const spots = ring === 0 ? 1 : 16;
          for (let spot = 0; spot < spots; spot += 1) {
            const angle = -Math.PI / 2 + (spot * Math.PI * 2) / spots;
            const cx = clampX(
              baseX + Math.cos(angle) * ring * HULL_LABEL_SEARCH_STEP_PX,
            );
            const cy = clampY(
              baseY + Math.sin(angle) * ring * HULL_LABEL_SEARCH_STEP_PX,
            );
            if (coversName(cx, cy)) {
              continue;
            }
            fallback ??= { x: cx, y: cy };
            if (!coversTitle(cx, cy)) {
              x = cx;
              y = cy;
              break search;
            }
          }
          if (ring === HULL_LABEL_SEARCH_RINGS && fallback) {
            // Nowhere clear nearby: at least keep off the other names.
            x = fallback.x;
            y = fallback.y;
          }
        }
        x = Math.round(x);
        y = Math.round(y);
        placedHullLabels.push({ x, y, w, h });
        label.style.borderColor = withAlpha(group.color, 0.85);
        label.style.visibility = "visible";
        label.style.transform = `translate(${x}px, ${y}px) translate(-50%, -50%)`;
      }
      context.restore();
    }

    // In a focused view, a tinted band behind each group of recommendations
    // that share a genre or theme.
    const bandCenter =
      focusCenter !== null ? viewportByNode.get(focusCenter) : undefined;
    // Where each band's name goes: just outside the band, midway along it.
    const bandLabels: { text: string; x: number; y: number; color: string }[] =
      [];
    if (bandCenter) {
      for (const band of recommendationBandsRef.current) {
        let inner = Infinity;
        let outer = 0;
        let pad = 0;
        const angles: number[] = [];
        for (const key of band.keys) {
          const point = viewportByNode.get(key);
          if (!point || !graph.hasNode(key)) {
            continue;
          }
          const radius = Math.hypot(
            point.x - bandCenter.x,
            point.y - bandCenter.y,
          );
          angles.push(
            Math.atan2(point.y - bandCenter.y, point.x - bandCenter.x),
          );
          const size = sizesByNode.get(key) ?? 0;
          inner = Math.min(inner, radius);
          outer = Math.max(outer, radius);
          pad = Math.max(
            pad,
            Math.hypot(size * GRAPH_POSTER_ASPECT_RATIO, size) + 6,
          );
        }
        if (outer === 0) {
          continue;
        }
        const { start, end } = coveringArc(angles);
        const spread = pad / Math.max(outer, 1);
        context.beginPath();
        context.arc(
          bandCenter.x,
          bandCenter.y,
          outer + pad,
          start - spread,
          end + spread,
        );
        context.arc(
          bandCenter.x,
          bandCenter.y,
          Math.max(0, inner - pad),
          end + spread,
          start - spread,
          true,
        );
        context.closePath();
        context.fillStyle = withAlpha(band.color, 0.14);
        context.fill();
        context.strokeStyle = withAlpha(band.color, 0.45);
        context.lineWidth = 1;
        context.stroke();
        const middle = (start + end) / 2;
        const labelRadius = outer + pad + 12;
        bandLabels.push({
          text: band.name,
          x: bandCenter.x + Math.cos(middle) * labelRadius,
          y: bandCenter.y + Math.sin(middle) * labelRadius,
          color: band.color,
        });
      }
    }

    // Active connections stay above zone fills and masks. Poster artwork
    // still covers crossing lines, keeping the title cards legible.
    if (hoverFocus)
      relationHitsRef.current = drawGraphRelations(
        context,
        viewportRelationsRef.current.values(),
        relationNodes,
        hoverFocus,
      );

    // Posters over the nodes large enough on screen to show one; a focused
    // view shows every title on its rings with its poster.
    const minPosterRadius =
      cameraCenterRef.current != null ? 0 : MIN_POSTER_RADIUS_PX;
    const admitPoster = createGraphPosterAdmission(
      width,
      height,
      0,
      Number.POSITIVE_INFINITY,
    );
    const visiblePosters = new Set<string>();
    const hoverableNodes = new Set<string>();
    viewportNodes.forEach((key) => {
      const attributes = graph.getNodeAttributes(key);
      const url = attributes.posterUrl as string | null | undefined;
      if (!url) {
        return;
      }
      const radius = sizesByNode.get(key) ?? 0;
      const point = viewportByNode.get(key);
      const threshold = minPosterRadius * 0.7;
      const posterOpacity = graphPosterArtworkOpacity(radius, minPosterRadius);
      if (radius < threshold || !point || !admitPoster(point, radius)) {
        return;
      }
      // Load the variant this size needs once the zoom rests; until then
      // a loaded smaller or larger one stands in, so a zoom never fetches,
      // decodes and cuts each poster at every size it passes through.
      const urls = graphPosterUrls(url, radius, ratio);
      const ready = (image: HTMLImageElement | undefined) =>
        image?.complete && image.naturalWidth > 0 ? image : undefined;
      const sprites = (posterSpritesRef.current ??= createGraphPosterSprites(
        () => sigmaRef.current?.scheduleRender(),
      ));
      let image: HTMLImageElement | undefined;
      for (const candidate of urls) {
        const loaded = ready(posterCacheRef.current.get(candidate));
        if (loaded && sprites.artwork(loaded)) {
          image = loaded;
          visiblePosters.add(candidate);
          break;
        }
      }
      // A poster's thumbnail stays in the working set while it is in view,
      // drawn or not, so the next zoom always has it to stand in.
      const thumbnail = graphPosterVariantUrls(url).w70;
      if (!image || posterCacheRef.current.has(thumbnail)) {
        visiblePosters.add(thumbnail);
      }
      if (zoomResting) visiblePosters.add(urls[0]);
      if (!image) {
        return;
      }
      const alpha = context.globalAlpha;
      if (attributes.ghost) {
        context.globalAlpha = 0.55;
      }
      const hoverFocus = hoverFocusRef.current;
      if (hoverFocus && !hoverFocus.near.has(key)) {
        context.globalAlpha = HOVER_FADE_ALPHA;
      }
      context.globalAlpha *= posterOpacity;
      const drawn = drawGraphPoster(
        context,
        sprites,
        image,
        point,
        radius,
        cachedCssColor(
          themeColorsRef.current,
          canvas,
          "--scry-surfD",
          "#0b0e15",
        ),
      );
      if (graphPosterHoverable(radius, posterOpacity, drawn)) {
        hoverableNodes.add(key);
      }
      context.globalAlpha = alpha;
      if (
        selectedKeyRef.current !== null
          ? key === selectedKeyRef.current
          : attributes.titleId &&
            attributes.titleId === selectedTitleIdRef.current
      ) {
        traceGraphPoster(context, point, radius, 2);
        context.lineWidth = 3;
        context.strokeStyle = cachedCssColor(
          themeColorsRef.current,
          canvas,
          "--scry-accent",
          "#5b64ff",
        );
        context.stroke();
      }
    });

    prepareGraphPosterImages(
      posterCacheRef.current,
      visiblePosters,
      () => sigmaRef.current?.scheduleRender(),
      undefined,
      undefined,
      undefined,
      {
        isLarge: isOriginalGraphPosterUrl,
        isThumbnail: isThumbnailGraphPosterUrl,
      },
    );
    hoverableNodesRef.current = hoverableNodes;
    if (
      hoverFocusRef.current &&
      !hoverableNodes.has(hoverFocusRef.current.key)
    ) {
      hoverFocusRef.current = null;
      setHover(null);
      sigma.getContainer().style.cursor = "";
      sigma.scheduleRefresh({
        partialGraph: { nodes: graph.nodes() },
        skipIndexation: true,
      });
    }
    const hoveredEdge = hoveredEdgeRef.current;
    if (
      hoveredEdge &&
      (!hoverableNodes.has(hoveredEdge.from) ||
        !hoverableNodes.has(hoveredEdge.to))
    ) {
      hoveredEdgeRef.current = null;
      setHover(null);
    }

    // Ghosts wear a dashed ring so they read apart from library titles.
    const muted = cachedCssColor(
      themeColorsRef.current,
      canvas,
      "--scry-muted2",
      UNPLACED_COLOR,
    );
    context.save();
    context.setLineDash([3, 3]);
    context.lineWidth = 1.5;
    context.strokeStyle = muted;
    graph.forEachNode((key, attributes) => {
      if (!attributes.ghost) {
        return;
      }
      const point = viewportByNode.get(key);
      const radius = sizesByNode.get(key) ?? 0;
      if (
        !point ||
        point.x < -radius ||
        point.y < -radius ||
        point.x > width + radius ||
        point.y > height + radius
      ) {
        return;
      }
      traceGraphPoster(context, point, radius, 2);
      context.stroke();
    });
    context.restore();

    // Title labels require a close zoom as well as an uncrowded view.
    // A small library at overview scale must not reveal labels just because
    // there are few nodes. Require most visible posters to be readable too.
    const visibleDiscs = nodeDiscs.filter(
      ({ x, y, r }) =>
        x + r >= 0 && y + r >= 0 && x - r <= width && y - r <= height,
    );
    const showOverviewLabels = showOverviewTitleLabels(
      visibleDiscs,
      width,
      height,
    );
    // A focused view names the nodes it shows, each label pointing away
    // from the centre so the ring stays clear. Labels are placed centre
    // first, then the keyboard-focused node, then outward; one that would
    // overlap a placed label is left off, and the hover card names it.
    const centerPoint = (focusCenter !== null
      ? viewportByNode.get(focusCenter)
      : undefined) ?? {
      x: width / 2,
      y: height / 2,
    };
    if (graph.order > 0) {
      context.save();
      context.textBaseline = "middle";
      context.lineJoin = "round";
      context.lineWidth = 3;
      context.strokeStyle = cachedCssColor(
        themeColorsRef.current,
        canvas,
        "--scry-surfD",
        "#0b0e15",
      );
      context.fillStyle = cachedCssColor(
        themeColorsRef.current,
        canvas,
        "--scry-ink2",
        "#e6e8ef",
      );
      const keyboardKey = focusedKeyRef.current;
      const order: string[] = [];
      graph.forEachNode((key) => {
        if (graph.getNodeAttribute(key, "collapsedClusterId") !== undefined)
          return;
        if (
          focusCenter === null &&
          !showOverviewLabels &&
          graph.getNodeAttribute(key, "collapsedClusterId") === undefined
        )
          return;
        const point = viewportByNode.get(key);
        if (
          point &&
          (focusCenter !== null ||
            (point.x >= 0 &&
              point.y >= 0 &&
              point.x <= width &&
              point.y <= height))
        ) {
          order.push(key);
        }
      });
      const rank = (key: string) =>
        key === focusCenter ? 0 : key === keyboardKey ? 1 : 2;
      const distance = (key: string) => {
        const point = viewportByNode.get(key);
        return point
          ? Math.hypot(point.x - centerPoint.x, point.y - centerPoint.y)
          : 0;
      };
      order.sort(
        (a, b) =>
          rank(a) - rank(b) ||
          // Small posters need their names most in the overview.
          (focusCenter === null
            ? (graph.getNodeAttribute(a, "size") as number) -
              (graph.getNodeAttribute(b, "size") as number)
            : 0) ||
          distance(a) - distance(b) ||
          (a < b ? -1 : a > b ? 1 : 0),
      );
      const placed: {
        left: number;
        top: number;
        right: number;
        bottom: number;
      }[] = placedHullLabels.map(({ x, y, w, h }) => ({
        left: x - w / 2,
        top: y - h / 2,
        right: x + w / 2,
        bottom: y + h / 2,
      }));
      // Band names first, as pills kept on the canvas; node labels give way
      // to them.
      context.font = "700 12px system-ui, sans-serif";
      context.textAlign = "center";
      for (const label of bandLabels) {
        const half = context.measureText(label.text).width / 2 + 8;
        const x = Math.min(Math.max(label.x, half + 4), width - half - 4);
        const y = Math.min(Math.max(label.y, 14), height - 14);
        const box = {
          left: x - half,
          top: y - 10,
          right: x + half,
          bottom: y + 10,
        };
        context.beginPath();
        context.roundRect(box.left, box.top, half * 2, 20, 10);
        context.fillStyle = cachedCssColor(
          themeColorsRef.current,
          canvas,
          "--scry-surfD",
          "#0b0e15",
        );
        context.fill();
        context.fillStyle = withAlpha(label.color, 0.25);
        context.fill();
        context.lineWidth = 1;
        context.strokeStyle = label.color;
        context.stroke();
        context.fillStyle = cachedCssColor(
          themeColorsRef.current,
          canvas,
          "--scry-ink2",
          "#e6e8ef",
        );
        context.fillText(label.text, x, y);
        placed.push(box);
      }
      context.lineWidth = 3;
      context.strokeStyle = cachedCssColor(
        themeColorsRef.current,
        canvas,
        "--scry-surfD",
        "#0b0e15",
      );
      context.fillStyle = cachedCssColor(
        themeColorsRef.current,
        canvas,
        "--scry-ink2",
        "#e6e8ef",
      );
      for (const key of order) {
        context.globalAlpha = detailOpacity;
        const point = viewportByNode.get(key);
        const raw = graph.getNodeAttribute(key, "label");
        if (!point || !raw) {
          continue;
        }
        const text =
          raw.length > FOCUS_LABEL_MAX_CHARS
            ? `${raw.slice(0, FOCUS_LABEL_MAX_CHARS - 1).trimEnd()}\u2026`
            : raw;
        const isCenter = key === focusCenter;
        context.font = isCenter
          ? "600 13px system-ui, sans-serif"
          : "500 11px system-ui, sans-serif";
        const lineHeight = isCenter ? 15 : 13;
        const textWidth = context.measureText(text).width;
        const radius = sizesByNode.get(key) ?? 0;
        const dx = point.x - centerPoint.x;
        const dy = point.y - centerPoint.y;
        let x = point.x;
        let y = point.y;
        let left: number;
        if (isCenter || Math.abs(dx) < radius) {
          // The centre, and nodes straight above or below it, carry their
          // label under (or over) them.
          context.textAlign = "center";
          y += (dy < 0 ? -1 : 1) * (radius + 10);
          left = x - textWidth / 2;
        } else {
          context.textAlign = dx > 0 ? "left" : "right";
          x += (dx > 0 ? 1 : -1) * (radius + 5);
          left = dx > 0 ? x : x - textWidth;
        }
        const box = {
          left: left - 2,
          top: y - lineHeight / 2,
          right: left + textWidth + 2,
          bottom: y + lineHeight / 2,
        };
        if (
          !isCenter &&
          (box.left < 0 ||
            box.right > width ||
            box.top < 0 ||
            box.bottom > height ||
            placed.some(
              (other) =>
                box.left < other.right &&
                box.right > other.left &&
                box.top < other.bottom &&
                box.bottom > other.top,
            ))
        ) {
          continue;
        }
        placed.push(box);
        context.strokeText(text, x, y);
        context.fillText(text, x, y);
      }
      context.restore();
    }

    // The keyboard focus ring.
    const focused = focusedKeyRef.current;
    const focusPoint = focused ? viewportByNode.get(focused) : undefined;
    if (focused && focusPoint && graph.hasNode(focused)) {
      const radius = sizesByNode.get(focused) ?? 0;
      traceGraphPoster(context, focusPoint, radius, 5);
      context.lineWidth = 2;
      context.strokeStyle = cachedCssColor(
        themeColorsRef.current,
        canvas,
        "--scry-ink2",
        "#e6e8ef",
      );
      context.stroke();
    }
  }, []);

  // One renderer for the life of the view.
  React.useEffect(() => {
    const container = containerRef.current;
    if (!container) {
      return;
    }
    const posterZoom = posterZoomRef.current;
    const graph: LibraryGraphStore = new Graph<NodeAttributes, EdgeAttributes>({
      type: "undirected",
      multi: false,
    });
    const sigma = new Sigma<NodeAttributes, EdgeAttributes>(graph, container, {
      primitives: {
        nodes: {
          shapes: [
            sdfRectangle({
              aspectRatio: GRAPH_POSTER_ASPECT_RATIO,
              cornerRadius: GRAPH_POSTER_CORNER_RADIUS_RATIO,
            }),
          ],
        },
      },
      styles: {
        nodes: {
          ...DEFAULT_STYLES.nodes,
          shape: "rectangle",
          backdropVisibility: "hidden",
          // The delayed application hover owns highlighting; native hover
          // must not reorder the node buffers during a passing swipe.
          depth: "nodes",
          size: { attribute: "size" },
        },
        // Relations are painted from the visible adjacency cache on the
        // overlay. Their fades must not re-upload the full WebGL edge buffer.
        edges: { ...DEFAULT_STYLES.edges, visibility: "hidden" },
      },
      settings: {
        // The view can sit in a hidden drawer (narrow screens with a title
        // open), where the container has no size; sigma must not throw there.
        allowInvalidContainer: true,
        renderLabels: false,
        renderEdgeLabels: false,
        enableEdgeEvents: false,
        autoRescale: false,
        gestureTarget: "graph",
        // Small, quick steps per wheel or trackpad event, so zooming reads
        // as one smooth motion rather than a jump between fixed levels.
        zoomingRatio: 1.1,
        zoomDuration: 90,
        // Scale the fixed geometry together; moving titles during zoom
        // breaks the camera's cursor anchor and forces graph reindexing.
        zoomToSizeRatioFunction: (ratio) => ratio,
      },
      nodeReducer: (key, attributes, node) => {
        if (node.collapsedClusterId !== undefined)
          return { ...attributes, color: "rgba(0, 0, 0, 0)" };
        const focus = hoverFocusRef.current;
        if (!focus) {
          return attributes;
        }
        return focus.near.has(key)
          ? { ...attributes, zIndex: 1 }
          : {
              ...attributes,
              color: withAlpha(node.baseColor, HOVER_FADE_ALPHA),
              zIndex: 0,
            };
      },
    });
    graphRef.current = graph;
    relationsStoreRef.current = new GraphAdjacency();
    sigmaRef.current = sigma;
    const hoverIntent = createGraphHoverIntent();
    hoverIntentRef.current = hoverIntent;
    let latestPointer: { x: number; y: number } | null = null;

    sigma.on("afterRender", drawOverlay);
    sigma.on("resize", () => applyZoomLimits(sigma, sigma.getGraph()));
    const unboundCamera = boundCameraToGraph(sigma, graph);
    // Ctrl-click (Cmd-click on a Mac) also opens the title's panel; a plain
    // click only focuses the graph on it.
    sigma.on("clickNode", ({ node, event }) => {
      const original = event.original;
      activateNodeRef.current(
        node,
        "ctrlKey" in original && (original.ctrlKey || original.metaKey),
      );
    });
    sigma.on("enterNode", ({ node, event }) => {
      if (!hoverableNodesRef.current.has(node)) return;
      hoverIntent.enter(node, () => {
        if (
          sigmaRef.current !== sigma ||
          sigma.getGraph() !== graph ||
          !graph.hasNode(node) ||
          !hoverableNodesRef.current.has(node)
        )
          return;
        hoveredEdgeRef.current = null;
        const attributes = graph.getNodeAttributes(node);
        if (latestPointer) {
          const containerBox = container.getBoundingClientRect();
          const box = graphPosterBounds(
            sigma.graphToViewport(attributes),
            sigma.scaleSize(
              sigma.getNodeDisplayData(node)?.size ?? attributes.size,
            ),
          );
          const x = latestPointer.x - containerBox.left;
          const y = latestPointer.y - containerBox.top;
          if (x < box.left || x > box.right || y < box.top || y > box.bottom)
            return;
        }
        hoverFocusRef.current = {
          key: node,
          near: new Set([
            node,
            ...(relationsStoreRef.current?.neighbors(node) ?? []),
          ]),
          accent: readCssColor(container, "--scry-accent", "#5b64ff"),
        };
        sigma.refresh({
          partialGraph: { nodes: graph.nodes() },
          skipIndexation: true,
        });
        setHover({
          kind: "node",
          key: node,
          x: event.x,
          y: event.y,
          lines: [attributes.label],
        });
        container.style.cursor = "pointer";
      });
    });
    sigma.on("leaveNode", ({ node }) => {
      hoverIntent.leave(node);
      if (hoverFocusRef.current?.key !== node) return;
      hoverFocusRef.current = null;
      sigma.refresh({
        partialGraph: { nodes: graph.nodes() },
        skipIndexation: true,
      });
      setHover(null);
      container.style.cursor = "";
    });
    const clearEdgeHover = () => {
      if (!hoveredEdgeRef.current) return;
      hoveredEdgeRef.current = null;
      setHover(null);
    };
    let edgeHoverFrame: number | null = null;
    const moveEdgeHover = (event: MouseEvent) => {
      // Track the live pointer even if GPU picking reports its leave late.
      latestPointer = { x: event.clientX, y: event.clientY };
      if (edgeHoverFrame !== null) cancelAnimationFrame(edgeHoverFrame);
      edgeHoverFrame = requestAnimationFrame(() => {
        edgeHoverFrame = null;
        if (hoverFocusRef.current || sigma.getCamera().isAnimating()) {
          clearEdgeHover();
          return;
        }
        const bounds = container.getBoundingClientRect();
        const point = {
          x: event.clientX - bounds.left,
          y: event.clientY - bounds.top,
        };
        const hit = hitGraphRelation(
          relationHitsRef.current,
          point,
          hoverableNodesRef.current,
        );
        if (!hit) {
          clearEdgeHover();
          return;
        }
        const current = hoveredEdgeRef.current;
        if (current?.from === hit.from && current.to === hit.to) return;
        hoveredEdgeRef.current = { from: hit.from, to: hit.to };
        setHover({
          kind: "edge",
          from: hit.from,
          to: hit.to,
          ...point,
          lines: hit.kinds,
        });
      });
    };
    container.addEventListener("mousemove", moveEdgeHover);
    container.addEventListener("mouseleave", clearEdgeHover);
    container.addEventListener("mouseleave", hoverIntent.cancel);
    // A touch never leaves a node, so the tooltip goes when the finger lifts.
    const clearHover = () => {
      hoverIntent.cancel();
      setHover(null);
    };
    container.addEventListener("touchend", clearHover);
    container.addEventListener("touchcancel", clearHover);

    const resizeObserver = new ResizeObserver(() => {
      if (container.offsetWidth === 0 || container.offsetHeight === 0) {
        return;
      }
      sigma.resize();
      sigma.refresh();
      resumeLayoutRef.current();
      if (focusCenterRef.current !== null) {
        setFocusFrameVersion((version) => version + 1);
      }
    });
    resizeObserver.observe(container);

    return () => {
      resizeObserver.disconnect();
      container.removeEventListener("mousemove", moveEdgeHover);
      container.removeEventListener("mouseleave", clearEdgeHover);
      container.removeEventListener("mouseleave", hoverIntent.cancel);
      hoverIntent.cancel();
      hoverIntentRef.current = null;
      if (edgeHoverFrame !== null) cancelAnimationFrame(edgeHoverFrame);
      container.removeEventListener("touchend", clearHover);
      container.removeEventListener("touchcancel", clearHover);
      if (frameRef.current !== null) {
        window.cancelAnimationFrame(frameRef.current);
        frameRef.current = null;
      }
      if (tweenFrameRef.current !== null) {
        window.cancelAnimationFrame(tweenFrameRef.current);
        tweenFrameRef.current = null;
      }
      window.clearTimeout(posterZoom.timer);
      unboundCamera();
      sigma.kill();
      sigmaRef.current = null;
      graphRef.current = null;
      relationsStoreRef.current = null;
    };
  }, [drawOverlay]);

  const nodeSizes = React.useMemo(
    () => nodeSizesByTitleId(Array.from(titlesById.values()), metric),
    [metric, titlesById],
  );

  // Sync the drawn graph. The overview lets its force layout settle from the
  // last positions; a focused view sets the focused node's neighbours on
  // rings round it. Either way nodes already drawn glide to their new place.
  React.useEffect(() => {
    const sigma = sigmaRef.current;
    const graph = graphRef.current;
    if (!sigma || !graph || !displayed || !join) {
      return;
    }
    titlesByIdRef.current = titlesById;
    focusCenterRef.current = focusCenterKey;
    selectedKeyRef.current = focusCenterKey;
    const focusing = focusCenterKey !== null;
    const sizes = nodeSizes;
    const mutedEdge = readCssColor(
      sigma.getContainer(),
      "--scry-muted2",
      "#7a8499",
    );

    // Where each drawn node is now, so it can glide from there.
    const current = new Map<string, LayoutPoint>();
    const currentViewport = new Map<string, LayoutPoint>();
    const currentRadii = new Map<string, number>();
    const incoming = new Set(displayed.nodes.map((node) => node.subjectKey));
    graph.forEachNode((key, attributes) => {
      current.set(key, { x: attributes.x, y: attributes.y });
      if (focusing && incoming.has(key)) {
        currentViewport.set(key, sigma.graphToViewport(attributes));
        currentRadii.set(
          key,
          sigma.scaleSize(
            sigma.getNodeDisplayData(key)?.size ?? attributes.size,
          ),
        );
      }
    });
    // Detach during replacement: Sigma re-indexes the whole remaining graph
    // synchronously for every nodeDropped event, even with scheduled rendering.
    // Keep this store's identity for the renderer's event handlers.
    hoverFocusRef.current = null;
    hoveredEdgeRef.current = null;
    hoverIntentRef.current?.cancel();
    viewportNodesRef.current = new Set();
    viewportRelationsRef.current.clear();
    relationHitsRef.current = [];
    sigma.setGraph(
      new Graph<NodeAttributes, EdgeAttributes>({
        type: "undirected",
        multi: false,
      }),
    );
    graph.clear();
    if (frameRef.current !== null) {
      window.cancelAnimationFrame(frameRef.current);
      frameRef.current = null;
    }
    if (tweenFrameRef.current !== null) {
      window.cancelAnimationFrame(tweenFrameRef.current);
      tweenFrameRef.current = null;
    }

    let layout: LayoutState | null = null;
    let targets: Map<string, LayoutPoint>;
    // Node sizes are in graph units; a focused view sizes its nodes in
    // screen pixels through this.
    let focusPixelsPerUnit = 1;
    if (focusCenterKey !== null) {
      // The rings share the overview's frame, so nodes glide in one space.
      // They are sized in screen pixels for the camera at rest on the
      // origin, leaving room beside the outer ring for labels. The frame is
      // pinned first: left to itself it would fit whatever the nodes cover
      // now, and change scale under the rings once they move.
      const extent = extentRef.current;
      sigma.setCustomBBox({ x: [-extent, extent], y: [-extent, extent] });
      const rest = restCamera(sigma);
      // Rebase the existing screen positions into the resting frame before
      // moving nodes. A camera zoom running beside the glide would magnify
      // the destination's pixel-sized posters before shrinking them again.
      for (const [key, point] of currentViewport)
        current.set(key, sigma.viewportToGraph(point, { cameraState: rest }));
      sigma.getCamera().cancelAnimation();
      sigma.getCamera().setState(rest);
      const unit = 1000;
      const origin = sigma.graphToViewport(
        { x: 0, y: 0 },
        { cameraState: rest },
      );
      const along = sigma.graphToViewport(
        { x: unit, y: 0 },
        { cameraState: rest },
      );
      const pixelsPerUnit = Math.max(1e-6, Math.abs(along.x - origin.x) / unit);
      focusPixelsPerUnit = pixelsPerUnit;
      const { width, height } = sigma.getDimensions();
      const outerPixels = Math.max(
        FOCUS_MIN_OUTER_RADIUS_PX,
        Math.min(
          // A narrow canvas gives up label room before ring room; labels
          // that do not fit are left off.
          Math.max(width / 2 - FOCUS_LABEL_ROOM_PX, (width / 2) * 0.8),
          (height / 2) * FOCUS_OUTER_RADIUS_SHARE,
        ),
      );
      targets = egoLayout(
        focusCenterKey,
        focusRings.map((ring) => ring.map((section) => section.keys)),
        outerPixels / pixelsPerUnit,
        FOCUS_RING_SPACING_PX / pixelsPerUnit,
      );
    } else if (largeLayout) {
      targets = new Map(
        displayed.nodes.map((node) => [
          node.subjectKey,
          largeLayout.positions.get(node.subjectKey)!,
        ]),
      );
      extentRef.current = largeLayout.extent;
      sigma.setCustomBBox({
        x: [-largeLayout.extent, largeLayout.extent],
        y: [-largeLayout.extent, largeLayout.extent],
      });
    } else {
      layout = createLayout(
        displayed.nodes.map((node) => ({
          key: node.subjectKey,
          clusterId: node.clusterId,
          // Ghosts never stretch an outline, so they keep to their links too.
          free: node.ghost || !namedClusterIdsRef.current.has(node.clusterId),
        })),
        displayed.edges,
        // New ghosts start beside the node that reached them, so exploring
        // settles the layout rather than restarting it.
        seedGhostPositions(positionsRef.current, traversal.expansions),
        allClusterIds,
      );
      targets = layoutPositions(layout);
      // A fixed frame around the cluster ring keeps the camera still while
      // the layout moves nodes.
      let extent = 0;
      for (const center of layout.centers.values()) {
        extent = Math.max(extent, Math.abs(center.x), Math.abs(center.y));
      }
      extent += 220;
      extentRef.current = extent;
      sigma.setCustomBBox({ x: [-extent, extent], y: [-extent, extent] });
      for (const [key, point] of targets) positionsRef.current.set(key, point);
    }
    // A node new to the drawing starts where it goes, or in a focused view
    // at the focused node, so its neighbours fan out from it.
    const fanFrom =
      focusCenterKey !== null
        ? (current.get(focusCenterKey) ?? targets.get(focusCenterKey))
        : undefined;
    const from = new Map<string, LayoutPoint>();
    const fromSizes = new Map<string, number>();
    const targetSizes = new Map<string, number>();
    for (const [key, target] of targets) {
      from.set(key, current.get(key) ?? fanFrom ?? target);
    }

    const zoomOrigin = sigma.graphToViewport({ x: 0, y: 0 });
    const zoomAlong = sigma.graphToViewport({ x: 34, y: 0 });
    const zoomSpacing = Math.hypot(
      zoomAlong.x - zoomOrigin.x,
      zoomAlong.y - zoomOrigin.y,
    );
    const initialDetailOpacity =
      largeLayout && !focusing ? graphDetailOpacity(zoomSpacing) : 1;
    // Preserve the overview boundary on the very first materialized frame,
    // before the camera effect runs its next coalesced update.
    transitionBlobsRef.current = focusing
      ? []
      : graphTransitionBlobs(
          groupsRef.current,
          largeLayout,
          new Set(
            displayed.nodes
              .filter((node) => node.collapsedClusterId === undefined)
              .map((node) => node.clusterId),
          ),
          zoomSpacing,
          sigma.scaleSize(1),
        );
    // Selection decorates each node as it is built, so attaching the graph
    // is the one pass over it.
    const accent = readCssColor(
      sigma.getContainer(),
      "--scry-accent",
      "#5b64ff",
    );
    const selectedTitle = selectedTitleIdRef.current;
    const selectedKey = selectedKeyRef.current;
    const ghosts = new Set<string>();
    for (const node of displayed.nodes) {
      if (node.ghost) ghosts.add(node.subjectKey);
      const position = from.get(node.subjectKey) ?? { x: 0, y: 0 };
      const color = clusterColor(node.clusterId, clusterOrder);
      let attributes: NodeAttributes;
      if (node.collapsedClusterId !== undefined) {
        const size = largeLayout
          ? 24 * Math.sqrt(node.memberCount ?? 1)
          : Math.min(36, 10 + Math.sqrt(node.memberCount ?? 1));
        attributes = {
          ...position,
          size:
            largeLayout && !focusing
              ? (size * (zoomSpacing / 34)) / sigma.scaleSize(1)
              : size,
          color,
          baseSize: size,
          baseColor: color,
          label: nodeLabel(node),
          collapsedClusterId: node.collapsedClusterId,
          hull: largeLayout?.hulls.get(node.collapsedClusterId),
          dots: largeLayout?.dots.get(node.collapsedClusterId),
          clusterId: node.clusterId,
        };
      } else if (node.ghost) {
        const item = traversal.ghostItems.get(node.subjectKey);
        const ghostColor = withAlpha(color, 0.35);
        const size =
          node.subjectKey === focusCenterKey
            ? (currentRadii.get(node.subjectKey) ??
                FOCUS_CENTER_RADIUS_PX * 1.35) /
              1.35 /
              focusPixelsPerUnit
            : focusing
              ? FOCUS_GHOST_RADIUS_PX / focusPixelsPerUnit
              : GHOST_NODE_SIZE;
        attributes = {
          ...position,
          size,
          color: ghostColor,
          baseSize: size,
          baseColor: ghostColor,
          label: nodeLabel(node),
          posterUrl: selectPosterVariantUrl(item?.posterUrl, "w70") ?? null,
          clusterId: node.clusterId,
          ghost: true,
        };
      } else {
        const title = join.titleBySubjectKey.get(node.subjectKey);
        const metricSize = title ? (sizes.get(title.id) ?? 4) : 4;
        const size =
          node.subjectKey === focusCenterKey
            ? (currentRadii.get(node.subjectKey) ??
                FOCUS_CENTER_RADIUS_PX * 1.35) /
              1.35 /
              focusPixelsPerUnit
            : focusing
              ? focusNeighborRadiusPx(metricSize) / focusPixelsPerUnit
              : metricSize;
        attributes = {
          ...position,
          size,
          color,
          baseSize: size,
          baseColor: color,
          label: nodeLabel(node),
          titleId: title?.id,
          posterUrl: selectPosterVariantUrl(title?.posterUrl, "w70") ?? null,
          clusterId: node.clusterId,
        };
      }
      if (attributes.collapsedClusterId === undefined) {
        if (largeLayout && !focusing)
          attributes.size =
            graphDetailPosterSize(attributes.baseSize, zoomSpacing) /
            sigma.scaleSize(1);
      }
      // Selection decorates the prepared geometry, not the rating metric.
      // Restoring baseSize here would undo the calibrated poster scale.
      const previousRadius = currentRadii.get(node.subjectKey);
      if (focusing && previousRadius !== undefined) {
        targetSizes.set(node.subjectKey, attributes.size);
        const startSize =
          previousRadius /
          sigma.scaleSize(1) /
          (node.subjectKey === focusCenterKey ? 1.35 : 1);
        fromSizes.set(node.subjectKey, startSize);
        attributes.size = startSize;
      }
      attributes.unselectedSize = attributes.size;
      attributes.unselectedColor = attributes.color;
      const isSelected =
        selectedKey !== null
          ? node.subjectKey === selectedKey
          : attributes.titleId !== undefined &&
            attributes.titleId === selectedTitle;
      if (isSelected) {
        attributes.color = accent;
        attributes.size *= 1.35;
      }
      if (graph.hasNode(node.subjectKey)) {
        graph.replaceNodeAttributes(node.subjectKey, attributes);
      } else {
        graph.addNode(node.subjectKey, attributes);
      }
    }
    // Keep adjacency and edge metadata off the renderer: even hidden WebGL
    // edges run a frame pass on every camera update.
    const relations = new GraphAdjacency<EdgeAttributes>(graph.nodes());
    for (const edge of displayed.edges) {
      if (relations.hasEdgeBetween(edge.from, edge.to)) {
        continue;
      }
      const toGhost = ghosts.has(edge.from) || ghosts.has(edge.to);
      const weight = Math.min(1, Math.max(0, edge.weight));
      // In a focused view, links between the neighbours sit behind the
      // focused title's own links.
      const ring =
        focusing && edge.from !== focusCenterKey && edge.to !== focusCenterKey;
      // A focused view draws few links, so they can be bolder.
      const alpha = focusing
        ? (0.3 + weight * 0.4) * (toGhost ? 0.7 : 1) * (ring ? 0.55 : 1)
        : (0.18 + weight * 0.4) * (toGhost ? 0.6 : 1);
      const attributes: EdgeAttributes = {
        size: (0.4 + weight * 2.2) * (ring ? 0.6 : 1),
        color: mutedEdge.startsWith("#")
          ? withAlpha(mutedEdge, alpha)
          : mutedEdge,
        kinds: edge.kinds,
      };
      relations.add(edge.from, edge.to, attributes);
    }

    relationsStoreRef.current = relations;
    applyDetailOpacity(sigma, detailOpacityRef, initialDetailOpacity);
    sigma.setGraph(graph);
    applyZoomLimits(sigma, graph);
    layoutRef.current = layout;
    const container = sigma.getContainer();
    const tick = () => {
      frameRef.current = null;
      const running = layoutRef.current;
      if (!layout || running !== layout) {
        return;
      }
      const decision = layoutLoopDecision(
        { width: container.offsetWidth, height: container.offsetHeight },
        running,
      );
      if (decision === "done") {
        // The overview just opened has settled: fit what it drew.
        if (fitWhenSettledRef.current && focusCenterRef.current === null) {
          fitWhenSettledRef.current = false;
          const camera = framingCamera(
            sigma,
            graph,
            () => true,
            OVERVIEW_FIT_PADDING,
          );
          if (camera) {
            void sigma.getCamera().animate(camera, { duration: 400 });
          }
        }
        return;
      }
      if (decision !== "step") {
        // Paused while hidden: the next non-zero size resumes it.
        return;
      }
      stepLayout(running, LAYOUT_STEPS_PER_FRAME);
      const positions = layoutPositions(running);
      for (const [key, point] of positions)
        positionsRef.current.set(key, point);
      graph.updateEachNodeAttributes((key, attributes) => {
        const point = positions.get(key);
        if (point) {
          attributes.x = point.x;
          attributes.y = point.y;
        }
        return attributes;
      }, POSITION_HINTS);
      frameRef.current = window.requestAnimationFrame(tick);
    };
    resumeLayoutRef.current = () => {
      if (
        layout &&
        frameRef.current === null &&
        tweenFrameRef.current === null &&
        layoutRef.current === layout
      ) {
        frameRef.current = window.requestAnimationFrame(tick);
      }
    };

    const moving = Array.from(targets).some(([key, target]) => {
      const start = from.get(key);
      return (
        (start !== undefined &&
          (start.x !== target.x || start.y !== target.y)) ||
        fromSizes.get(key) !== targetSizes.get(key)
      );
    });
    if (moving) {
      let started: number | null = null;
      const glide = (now: number) => {
        tweenFrameRef.current = null;
        started ??= now;
        const progress = Math.min(1, (now - started) / FOCUS_TWEEN_MS);
        const positions = tweenPositions(from, targets, progress);
        graph.updateEachNodeAttributes(
          (key, attributes) => {
            const point = positions.get(key);
            if (point) {
              attributes.x = point.x;
              attributes.y = point.y;
            }
            const startSize = fromSizes.get(key);
            const targetSize = targetSizes.get(key);
            if (startSize !== undefined && targetSize !== undefined) {
              attributes.unselectedSize =
                startSize + (targetSize - startSize) * easeOutCubic(progress);
              attributes.size =
                attributes.unselectedSize * (key === focusCenterKey ? 1.35 : 1);
            }
            return attributes;
          },
          fromSizes.size
            ? { attributes: ["x", "y", "size", "unselectedSize"] }
            : POSITION_HINTS,
        );
        if (progress < 1) {
          tweenFrameRef.current = window.requestAnimationFrame(glide);
        } else {
          flushPendingSelect();
          resumeLayoutRef.current();
        }
      };
      tweenFrameRef.current = window.requestAnimationFrame(glide);
    } else {
      flushPendingSelect();
    }
    // Entering, moving or leaving focus brings the camera back to rest on
    // the frame, where the rings fit; so does any new ring layout, since
    // the rings are sized for the camera at rest.
    if (cameraCenterRef.current !== focusCenterKey || focusCenterKey !== null) {
      cameraCenterRef.current = focusCenterKey;
      fitWhenSettledRef.current = focusCenterKey === null;
      if (!focusing)
        void sigma
          .getCamera()
          .animate(restCamera(sigma), { duration: FOCUS_TWEEN_MS });
    }
    if (largeLayout && focusCenterKey === null && fitWhenSettledRef.current) {
      fitWhenSettledRef.current = false;
      sigma.getCamera().setState(restCamera(sigma));
    }
    resumeLayoutRef.current();
  }, [
    allClusterIds,
    clusterOrder,
    displayed,
    flushPendingSelect,
    focusCenterKey,
    focusFrameVersion,
    focusRings,
    join,
    largeLayout,
    nodeLabel,
    nodeSizes,
    titlesById,
    traversal,
  ]);

  // Only nearby neighbourhoods unfold. Camera events are coalesced so a
  // trackpad gesture makes at most one detail decision per animation frame.
  React.useEffect(() => {
    const sigma = sigmaRef.current;
    if (
      !sigma ||
      !join ||
      join.graph.nodes.length <= LIBRARY_GRAPH_COLLAPSE_THRESHOLD ||
      focusCenterKey !== null
    )
      return;
    let frame: number | null = null;
    let lastProjection = "";
    let lastDetailOpacity: number | null = null;
    const update = () => {
      frame = null;
      if (fitWhenSettledRef.current) return;
      if (!largeLayout) return;
      const regions = largeLayout.regions.map((region) => {
        const corners = [
          sigma.graphToViewport({ x: region.left, y: region.top }),
          sigma.graphToViewport({ x: region.right, y: region.top }),
          sigma.graphToViewport({ x: region.left, y: region.bottom }),
          sigma.graphToViewport({ x: region.right, y: region.bottom }),
        ];
        return {
          id: region.id,
          left: Math.min(...corners.map((p) => p.x)),
          right: Math.max(...corners.map((p) => p.x)),
          top: Math.min(...corners.map((p) => p.y)),
          bottom: Math.max(...corners.map((p) => p.y)),
        };
      });
      const origin = sigma.graphToViewport({ x: 0, y: 0 });
      const along = sigma.graphToViewport({ x: 34, y: 0 });
      const spacing = Math.hypot(along.x - origin.x, along.y - origin.y);
      const projection = `${origin.x},${origin.y},${along.x},${along.y}`;
      if (projection === lastProjection) return;
      lastProjection = projection;
      const detailOpacity = graphDetailOpacity(spacing);
      const opacityChanged = detailOpacity !== lastDetailOpacity;
      lastDetailOpacity = detailOpacity;
      // A glide the sync effect started (leaving a focused view, say) is
      // left to finish: it moves nodes in graph space, so a camera change
      // does not disturb it, and its last frame flushes the pending
      // selection. The first projection after subscribing always differs,
      // so cancelling here froze every such glide on its first frame.
      const graph = graphRef.current;
      if (graph && opacityChanged) {
        const expanded = new Set<number>();
        graph.forEachNode((_key, attributes) => {
          if (attributes.collapsedClusterId === undefined)
            expanded.add(attributes.clusterId);
        });
        transitionBlobsRef.current = graphTransitionBlobs(
          groupsRef.current,
          largeLayout,
          expanded,
          spacing,
          sigma.scaleSize(1),
        );
        // The titles fade as one layer; their colours stay as they are.
        applyDetailOpacity(sigma, detailOpacityRef, detailOpacity);
        sigma.scheduleRender();
      }
      const { width, height } = sigma.getDimensions();
      setExpandedClusterIds((previous) =>
        graphDetailClusters(previous, regions, spacing, width, height),
      );
    };
    const schedule = () => {
      if (frame === null) frame = window.requestAnimationFrame(update);
    };
    const camera = sigma.getCamera();
    camera.on("updated", schedule);
    sigma.on("afterRender", schedule);
    schedule();
    return () => {
      camera.off("updated", schedule);
      sigma.off("afterRender", schedule);
      if (frame !== null) window.cancelAnimationFrame(frame);
    };
  }, [focusCenterKey, join, largeLayout, displayed]);

  const keyboardNodesRef = React.useRef(keyboardNodes);
  React.useEffect(() => {
    keyboardNodesRef.current = keyboardNodes;
  }, [keyboardNodes]);
  const focusGroup = React.useCallback((clusterId: number) => {
    const sigma = sigmaRef.current;
    const graph = graphRef.current;
    if (!sigma || !graph) {
      return;
    }
    // The arrow keys then cycle through this cluster.
    setFocusedKey(firstNodeInCluster(keyboardNodesRef.current, clusterId));
    const camera = framingCamera(
      sigma,
      graph,
      (attributes) => attributes.clusterId === clusterId,
      1.5,
    );
    if (camera) {
      void sigma.getCamera().animate(camera, { duration: 400 });
    }
  }, []);

  const selectNeighborhood = React.useCallback(
    (clusterId: number) => {
      const next = toggleClusterExpanded(selectedNeighborhoods, clusterId);
      setSelectedNeighborhoods(next);
      const sigma = sigmaRef.current;
      const graph = graphRef.current;
      if (!sigma || !graph) return;
      const layout = largeLayoutRef.current;
      if (!layout) {
        const camera = framingCamera(
          sigma,
          graph,
          (attributes) => next.size === 0 || next.has(attributes.clusterId),
          1.5,
        );
        if (camera) void sigma.getCamera().animate(camera, { duration: 300 });
        return;
      }
      // Fit the complete selected clouds, even when the renderer currently
      // holds only their collapsed center nodes. Unplaced titles are excluded
      // from a named selection but included when clearing the selection.
      const bounds = graphNeighborhoodBounds(layout.regions, next);
      if (!bounds) return;
      const corners = [
        { x: bounds.left, y: bounds.top },
        { x: bounds.right, y: bounds.top },
        { x: bounds.right, y: bounds.bottom },
        { x: bounds.left, y: bounds.bottom },
      ].map((point) => sigma.graphToViewport(point));
      const { width, height } = sigma.getDimensions();
      if (width <= 0 || height <= 0) return;
      const center = sigma.graphToFramedGraph({
        x: (bounds.left + bounds.right) / 2,
        y: (bounds.top + bounds.bottom) / 2,
      });
      const ratio =
        sigma.getCamera().getState().ratio *
        OVERVIEW_FIT_PADDING *
        Math.max(
          (Math.max(...corners.map((p) => p.x)) -
            Math.min(...corners.map((p) => p.x))) /
            width,
          (Math.max(...corners.map((p) => p.y)) -
            Math.min(...corners.map((p) => p.y))) /
            height,
        );
      void sigma.getCamera().animate({ ...center, ratio }, { duration: 300 });
    },
    [selectedNeighborhoods],
  );

  // A focused view rests on its rings; the overview fits its titles, which
  // the layout gathers well inside the frame around the cluster ring.
  const resetCamera = React.useCallback(() => {
    const sigma = sigmaRef.current;
    const graph = graphRef.current;
    if (!sigma) {
      return;
    }
    const camera =
      focusCenterRef.current === null && graph
        ? framingCamera(sigma, graph, () => true, OVERVIEW_FIT_PADDING)
        : null;
    void sigma
      .getCamera()
      .animate(camera ?? restCamera(sigma), { duration: 300 });
  }, []);

  React.useImperativeHandle(controlsRef, () => ({ fit: resetCamera }), [
    resetCamera,
  ]);

  // The node picked in the graph, while it is still drawn.
  const displayedNodesByKey = React.useMemo(
    () =>
      new Map((displayed?.nodes ?? []).map((node) => [node.subjectKey, node])),
    [displayed],
  );
  const selectedNode =
    selectedKey !== null
      ? (displayedNodesByKey.get(selectedKey) ?? null)
      : null;
  const shownFocusedKey =
    focusedKey !== null && displayedNodesByKey.has(focusedKey)
      ? focusedKey
      : null;

  React.useEffect(() => {
    selectedKeyRef.current = selectedNode?.subjectKey ?? null;
    applySelection();
    sigmaRef.current?.refresh();
  }, [applySelection, selectedNode]);
  React.useEffect(() => {
    focusedKeyRef.current = shownFocusedKey;
    sigmaRef.current?.scheduleRender();
  }, [shownFocusedKey]);

  // What a node's neighbours are asked for by: an owned node's graph target
  // key in the graph's facet, or an explored title's first key in the
  // discovery vocabulary in its own facet.
  const neighborAnchor = React.useCallback(
    (node: GraphNode): { targetKey: string; facet: Facet } | null => {
      if (node.collapsedClusterId !== undefined) {
        return null;
      }
      const item = node.ghost
        ? traversal.ghostItems.get(node.subjectKey)
        : undefined;
      if (item) {
        const targetKey = neighborTargetKey(item);
        return targetKey
          ? { targetKey, facet: discoveryItemFacet(item) ?? facet }
          : null;
      }
      const targetKey = node.targetKey.trim();
      return targetKey ? { targetKey, facet } : null;
    },
    [facet, traversal.ghostItems],
  );

  // Labels of expanded nodes as they read when expanded, so the trail can
  // still name an anchor that a filter has since hidden.
  const [anchorLabels, setAnchorLabels] = React.useState<
    ReadonlyMap<string, string>
  >(() => new Map());

  const expandNode = React.useCallback(
    async (node: GraphNode) => {
      const key = node.subjectKey;
      const anchor = neighborAnchor(node);
      if (!anchor) {
        return;
      }
      const label = nodeLabel(node);
      const anchorTags = node.ghost
        ? traversal.ghostItems.get(key)?.canonicalTags
        : join?.titleBySubjectKey.get(key)?.canonicalTags;
      const token = beginExpansion(expandTokenRef);
      setExpandingKey(key);
      setExpandNotice(null);
      try {
        const cacheKey = `${anchor.facet}|${anchor.targetKey}`;
        let neighbors = neighborCacheRef.current.get(cacheKey);
        if (!neighbors) {
          const { data, error } = await client
            .query<NeighborsData>(
              discoveryNeighborsQuery,
              { input: { ...anchor, limit: NEIGHBOR_LIMIT } },
              // A title added or requested since an earlier read must not
              // come back from the cache as still addable.
              { requestPolicy: "network-only" },
            )
            .toPromise();
          if (error) {
            throw error;
          }
          neighbors = {
            items: data?.discoveryNeighbors?.items ?? [],
            groups: data?.discoveryNeighbors?.groups ?? [],
          };
          neighborCacheRef.current.set(cacheKey, neighbors);
        }
        const { items, groups } = neighbors;
        if (!isCurrentExpansion(expandTokenRef, token)) {
          return;
        }
        if (items.length === 0) {
          setExpandNotice({ key, kind: "empty" });
          return;
        }
        const found = items;
        setReasonsByPair((current) => {
          const next = new Map(current);
          for (const item of found) {
            const target =
              ownedSubjectForItem(item, ownedIndexRef.current) ??
              ghostSubjectKey(item);
            if (target && target !== key) {
              next.set(
                edgePairKey(key, target),
                neighborReason(anchorTags, item, moreLikeThisEdgeKinds(item)),
              );
            }
          }
          return next;
        });
        setAnchorLabels((current) =>
          current.get(key) === label
            ? current
            : new Map(current).set(key, label),
        );
        setTraversal((current) =>
          expandTraversal(
            current,
            key,
            items,
            ownedIndexRef.current,
            LIBRARY_GRAPH_GHOST_CAP,
            groups,
          ),
        );
      } catch (error) {
        if (isCurrentExpansion(expandTokenRef, token)) {
          setGlobalStatus(
            discoveryErrorMessage(error, t, t("status.apiError")),
            { level: "ERROR" },
          );
        }
      } finally {
        if (isCurrentExpansion(expandTokenRef, token)) {
          setExpandingKey(null);
        }
      }
    },
    [
      client,
      join,
      neighborAnchor,
      nodeLabel,
      setGlobalStatus,
      t,
      traversal.ghostItems,
    ],
  );

  const [historyExpansion, setHistoryExpansion] = React.useState<string | null>(
    null,
  );
  const historySnapshot = React.useMemo(
    () => ({
      traversal,
      reasonsByPair,
      selectedKey,
      focusedKey,
      selectionCardOpen,
      expandingKey,
    }),
    [
      traversal,
      reasonsByPair,
      selectedKey,
      focusedKey,
      selectionCardOpen,
      expandingKey,
    ],
  );
  const restoreHistory = React.useCallback((saved: typeof historySnapshot) => {
    invalidateExpansions(expandTokenRef);
    const pending = pendingSelectRef.current;
    if (pending) window.clearTimeout(pending.timer);
    pendingSelectRef.current = null;
    setTraversal(saved.traversal);
    setReasonsByPair(saved.reasonsByPair);
    setSelectedKey(saved.selectedKey);
    setFocusedKey(saved.focusedKey);
    setSelectionCardOpen(saved.selectionCardOpen);
    setExpandingKey(null);
    setExpandNotice(null);
    setHistoryExpansion(saved.expandingKey);
  }, []);
  const pushGraphHistory = useLibraryGraphHistory(
    sourceKey,
    historySnapshot,
    restoreHistory,
  );
  React.useEffect(() => {
    if (!historyExpansion) return;
    const node = exploredNodesByKey.get(historyExpansion);
    if (!node) return;
    setHistoryExpansion(null);
    void expandNode(node);
  }, [historyExpansion, exploredNodesByKey, expandNode]);

  // Leaves the focused view for the library overview, dropping what was
  // explored from it.
  const exitFocus = React.useCallback(() => {
    if (selectedKey !== null) pushGraphHistory();
    setHistoryExpansion(null);
    invalidateExpansions(expandTokenRef);
    setTraversal(emptyTraversal<DiscoveryItem>());
    setReasonsByPair(new Map());
    setSelectedKey(null);
    setFocusedKey(null);
    setExpandingKey(null);
    setExpandNotice(null);
  }, [pushGraphHistory, selectedKey]);

  // Picking a node focuses the view on it and brings in more like it. The
  // trail follows the focus: a node already on it cuts the trail back to it,
  // and any other node is explored from.
  const activateNode = React.useCallback(
    (key: string, openTitle: boolean) => {
      const graph = graphRef.current;
      if (!graph || !graph.hasNode(key)) {
        return;
      }
      const attributes = graph.getNodeAttributes(key);
      if (attributes.collapsedClusterId !== undefined) {
        const clusterId = attributes.collapsedClusterId;
        const sigma = sigmaRef.current;
        if (largeLayout && sigma) {
          const point = { x: attributes.x, y: attributes.y };
          const origin = sigma.graphToViewport(point);
          const step = sigma.graphToViewport({ x: point.x + 34, y: point.y });
          const spacing = Math.hypot(step.x - origin.x, step.y - origin.y);
          const center = sigma.graphToFramedGraph(point);
          sigma.getCamera().setState({
            x: center.x,
            y: center.y,
            ratio: (sigma.getCamera().getState().ratio * spacing) / 50,
          });
          return;
        }
        setExpandedClusterIds((current) =>
          toggleClusterExpanded(current, clusterId),
        );
        return;
      }
      invalidateExpansions(expandTokenRef);
      setExpandingKey(null);
      setExpandNotice(null);
      if (selectedKey !== key) pushGraphHistory();
      setHistoryExpansion(null);
      setSelectedKey(key);
      setSelectionCardOpen(true);
      setFocusedKey(key);
      const title = attributes.titleId
        ? titlesByIdRef.current.get(attributes.titleId)
        : undefined;
      const previous = pendingSelectRef.current;
      if (previous) {
        window.clearTimeout(previous.timer);
        pendingSelectRef.current = null;
      }
      if (title && openTitle) {
        // Opened when the glide lands; the timer covers a glide that never
        // starts or is cut short.
        pendingSelectRef.current = {
          title,
          timer: window.setTimeout(
            flushPendingSelect,
            FOCUS_TWEEN_MS + SELECT_AFTER_GLIDE_SLACK_MS,
          ),
        };
      }
      const position = traversal.expansions.findIndex(
        (expansion) => expansion.anchor === key,
      );
      if (position >= 0) {
        setTraversal((current) => collapseTraversalToCrumb(current, position));
        return;
      }
      const node = exploredNodesByKey.get(key);
      if (node) {
        void expandNode(node);
      }
    },
    [
      expandNode,
      exploredNodesByKey,
      flushPendingSelect,
      traversal.expansions,
      largeLayout,
      selectedKey,
      pushGraphHistory,
    ],
  );
  React.useEffect(() => {
    activateNodeRef.current = activateNode;
  }, [activateNode]);

  const actedGhostKeyRef = React.useRef<string | null>(null);
  const onCatalogChangedRef = React.useRef(onCatalogChanged);
  React.useEffect(() => {
    onCatalogChangedRef.current = onCatalogChanged;
  }, [onCatalogChanged]);
  React.useEffect(() => {
    catalogChangedRef.current = ({ addedTitleId }) => {
      const acted = actedGhostKeyRef.current;
      if (acted && addedTitleId) {
        setTraversal((current) =>
          stampGhostTitle(current, acted, addedTitleId),
        );
      }
      // Replies read before the change may offer the title as addable.
      neighborCacheRef.current.clear();
      // The listing catches up in the background; the dialog does not wait.
      void onCatalogChangedRef.current?.();
    };
  }, []);

  // A crumb focuses its node again, cutting the trail back to it.
  const collapseToCrumb = React.useCallback(
    (index: number) => {
      const anchor = traversal.expansions[index]?.anchor;
      if (!anchor) {
        exitFocus();
        return;
      }
      invalidateExpansions(expandTokenRef);
      setExpandingKey(null);
      setExpandNotice(null);
      if (selectedKey !== anchor) pushGraphHistory();
      setHistoryExpansion(null);
      setTraversal((current) => collapseTraversalToCrumb(current, index));
      setSelectedKey(anchor);
      setSelectionCardOpen(true);
      setFocusedKey(anchor);
    },
    [exitFocus, traversal.expansions, selectedKey, pushGraphHistory],
  );

  const handleCanvasKeyDown = React.useCallback(
    (event: React.KeyboardEvent<HTMLDivElement>) => {
      switch (event.key) {
        case "ArrowRight":
        case "ArrowDown":
        case "ArrowLeft":
        case "ArrowUp": {
          event.preventDefault();
          const step =
            event.key === "ArrowRight" || event.key === "ArrowDown" ? 1 : -1;
          setFocusedKey(
            nextNodeInCluster(
              keyboardNodes,
              shownFocusedKey ?? selectedNode?.subjectKey ?? null,
              shownFocusedKey ? step : 0,
            ),
          );
          return;
        }
        case "Enter":
        case " ": {
          if (shownFocusedKey) {
            event.preventDefault();
            activateNode(shownFocusedKey, event.ctrlKey || event.metaKey);
          }
          return;
        }
        case "Escape": {
          // An open title panel takes Escape first; the page closes it.
          if (selectedTitleId !== null) {
            return;
          }
          if (selectedNode) {
            event.preventDefault();
            if (selectionCardOpen) {
              setSelectionCardOpen(false);
            } else {
              exitFocus();
            }
          } else if (shownFocusedKey) {
            event.preventDefault();
            setFocusedKey(null);
          }
          return;
        }
        default:
          return;
      }
    },
    [
      activateNode,
      exitFocus,
      keyboardNodes,
      selectedNode,
      selectionCardOpen,
      selectedTitleId,
      shownFocusedKey,
    ],
  );

  // A hovered node or edge that a filter removed has no tooltip.
  const displayedKeys = React.useMemo(
    () => new Set(displayed?.nodes.map((node) => node.subjectKey) ?? []),
    [displayed],
  );
  const shownHover =
    hover &&
    (hover.kind === "node"
      ? displayedKeys.has(hover.key)
      : displayedKeys.has(hover.from) && displayedKeys.has(hover.to))
      ? hover
      : null;
  // Why a hovered result is a neighbour: from its link, or for a node in a
  // focused view, from its link to the focused node.
  const hoverReasonPair =
    shownHover?.kind === "edge"
      ? edgePairKey(shownHover.from, shownHover.to)
      : shownHover?.kind === "node" &&
          focusCenterKey !== null &&
          shownHover.key !== focusCenterKey
        ? edgePairKey(focusCenterKey, shownHover.key)
        : null;
  const hoverReason =
    hoverReasonPair !== null ? reasonsByPair.get(hoverReasonPair) : undefined;
  const hoverReasonLines = hoverReason
    ? neighborReasonLines(hoverReason, t)
    : [];
  // An edge with no reason falls back to naming its kinds.
  const hoverKindsOnly =
    shownHover?.kind === "edge" && hoverReasonLines.length === 0;
  const hoverTitle = shownHover?.kind === "node" ? shownHover.lines : [];
  const hoverLines = hoverKindsOnly
    ? libraryGraphEdgeKindLabels(shownHover.lines, t)
    : hoverReasonLines;
  const visibleNodeCount = join?.graph.nodes.length ?? 0;
  const selectedVisible =
    selectedTitleId !== null && titlesById.has(selectedTitleId);
  // Titles still paging, or the graph still loading for them, show as
  // loading; a finished listing with nothing to draw shows as empty.
  const graphStatus: "loading" | "empty" | "notBuilt" | "failed" | "ready" =
    loadStatus === "failed"
      ? "failed"
      : loadStatus === "notBuilt"
        ? "notBuilt"
        : loadStatus === "pending" || !join || paintedJoin !== join
          ? "loading"
          : visibleNodeCount === 0
            ? "empty"
            : "ready";
  // Once a graph is drawn, reloads (a library scan refetching titles, say)
  // keep it on screen; only one that lasts gets the small corner indicator.
  const loadingShown = graphStatus === "loading" || previewing;
  const refreshing = loadingShown && paintedJoin !== null;
  const [refreshIndicatorDue, setRefreshIndicatorDue] = React.useState(false);
  React.useEffect(() => {
    if (!refreshing) return;
    const timer = window.setTimeout(
      () => setRefreshIndicatorDue(true),
      REFRESH_INDICATOR_DELAY_MS,
    );
    return () => {
      window.clearTimeout(timer);
      setRefreshIndicatorDue(false);
    };
  }, [refreshing]);

  const crumbs = React.useMemo(
    () =>
      traversal.expansions.map((expansion) => ({
        anchor: expansion.anchor,
        label: anchorLabels.get(expansion.anchor) ?? expansion.anchor,
      })),
    [anchorLabels, traversal.expansions],
  );
  const selectedItem =
    selectedNode?.ghost === true
      ? (traversal.ghostItems.get(selectedNode.subjectKey) ?? null)
      : null;
  const { canAddItem, canRequestItem, onAction, onOpenResolved } =
    moreLikeThisActions.stripProps;
  // Owned nodes and discovered catalog matches open their overview instead
  // of offering Add or Request.
  const selectedCatalogTitle = graphSelectionCatalogTitle(
    selectedNode
      ? join?.titleBySubjectKey.get(selectedNode.subjectKey)
      : undefined,
    selectedItem,
    catalogIndex,
    titles ?? [],
  );
  const selectedResolved =
    selectedCatalogTitle !== null ||
    (selectedItem !== null && isLibraryItem(selectedItem, catalogIndex));
  const openSelected = selectedCatalogTitle
    ? () => onOpenOverview(selectedCatalogTitle)
    : selectedItem?.resolvedTitleId?.trim() && onOpenResolved
      ? () => void onOpenResolved(selectedItem)
      : null;
  const selectedAddable =
    selectedItem !== null &&
    !selectedResolved &&
    !selectedItem.ownedInInput &&
    (canAddItem?.(selectedItem) ?? false);
  const selectedRequestable =
    selectedItem !== null &&
    !selectedResolved &&
    !selectedItem.ownedInInput &&
    !selectedAddable &&
    (canRequestItem?.(selectedItem) ?? false);
  const focusedNode = shownFocusedKey
    ? (displayedNodesByKey.get(shownFocusedKey) ?? null)
    : null;
  const focusedItem = focusedNode?.ghost
    ? (traversal.ghostItems.get(focusedNode.subjectKey) ?? null)
    : null;
  const focusedLabel = focusedNode
    ? focusedItem && !isLibraryItem(focusedItem, catalogIndex)
      ? t("libraryGraph.focusedOutsideLibrary", {
          title: nodeLabel(focusedNode),
        })
      : nodeLabel(focusedNode)
    : "";

  return (
    <section
      aria-label={t("libraryGraph.regionLabel")}
      aria-controls={selectedVisible ? contextPanelId : undefined}
      className="flex h-full min-h-[24rem] flex-col gap-3"
    >
      {focusCenterKey !== null ? (
        <nav
          aria-label={t("libraryGraph.trailLabel")}
          className="flex flex-wrap items-center gap-1.5"
          data-testid="library-graph-trail"
        >
          {crumbs.map((crumb, index) =>
            crumbs.length > 4 &&
            index > 0 &&
            index < crumbs.length - 2 ? null : (
              <React.Fragment key={crumb.anchor}>
                {index > 0 ? (
                  <span
                    aria-hidden="true"
                    className="text-[var(--scry-muted2)]"
                  >
                    ›
                  </span>
                ) : null}
                {crumbs.length > 4 && index === crumbs.length - 2 ? (
                  <>
                    <span
                      aria-hidden="true"
                      className="text-[var(--scry-muted2)]"
                    >
                      …
                    </span>
                    <span
                      aria-hidden="true"
                      className="text-[var(--scry-muted2)]"
                    >
                      ›
                    </span>
                  </>
                ) : null}
                <button
                  type="button"
                  onClick={() => collapseToCrumb(index)}
                  aria-current={
                    index === crumbs.length - 1 ? "step" : undefined
                  }
                  className="max-w-[12rem] truncate rounded-full border border-[var(--scry-border2)] bg-[var(--scry-inset)] px-2.5 py-0.5 text-[12px] text-[var(--scry-body)] transition hover:bg-[var(--scry-hover)] aria-[current=step]:border-[var(--scry-accent)]"
                >
                  {crumb.label}
                </button>
              </React.Fragment>
            ),
          )}
          <Button
            type="button"
            variant="outline"
            size="sm"
            onClick={exitFocus}
            className="ml-auto h-7 gap-1.5 rounded-[8px] border-[var(--scry-border2)] bg-[var(--scry-inset)] px-2.5 text-[12px] shadow-none"
          >
            <Undo2 className="h-3.5 w-3.5" />
            {t("libraryGraph.backToOverview")}
          </Button>
        </nav>
      ) : null}
      <div className="relative min-h-[20rem] flex-1 overflow-hidden rounded-[14px] border border-[var(--scry-border2)] bg-[var(--scry-surfD)]">
        <div
          ref={containerRef}
          tabIndex={0}
          role="application"
          aria-label={t("libraryGraph.canvasLabel")}
          onKeyDown={handleCanvasKeyDown}
          className="absolute inset-0 touch-none outline-none focus-visible:ring-2 focus-visible:ring-[var(--scry-accent)] max-sm:bottom-auto max-sm:aspect-square max-sm:max-h-full"
          data-testid="library-graph-canvas"
        />
        <div aria-live="polite" className="sr-only">
          {focusedLabel}
        </div>
        <canvas
          ref={overlayRef}
          aria-hidden="true"
          className="pointer-events-none absolute left-0 top-0"
        />
        {neighborhoodLegend.length > 0 ? (
          <ul
            aria-label={t(
              focusCenterKey !== null
                ? "libraryGraph.recommendationGroupsLabel"
                : "libraryGraph.clustersLabel",
            )}
            className="absolute left-3 top-3 z-10 flex max-h-[calc(100%-1.5rem)] max-w-[calc(100%-1.5rem)] flex-col gap-1.5 overflow-y-auto overscroll-contain rounded-[10px] border border-[var(--scry-border2)] bg-[var(--scry-card2)]/90 p-2 text-[12px] shadow-lg backdrop-blur-sm"
          >
            {neighborhoodLegend.map((band) => (
              <li key={band.key}>
                <button
                  type="button"
                  disabled={focusCenterKey !== null}
                  aria-pressed={
                    focusCenterKey === null
                      ? selectedNeighborhoods.has(Number(band.key))
                      : undefined
                  }
                  onClick={() => selectNeighborhood(Number(band.key))}
                  className="flex w-full min-w-0 items-center gap-2 rounded-[6px] border border-transparent px-2 py-1 text-left text-[var(--scry-ink2)] aria-pressed:border-[var(--scry-accent)] hover:bg-[var(--scry-hover)]"
                  style={{ backgroundColor: withAlpha(band.color, 0.14) }}
                >
                  <span
                    aria-hidden="true"
                    className="h-2 w-2 shrink-0 rounded-full"
                    style={{ backgroundColor: band.color }}
                  />
                  <span className="min-w-0 break-words">{band.name}</span>
                  <span className="ml-auto shrink-0 pl-2 tabular-nums text-[var(--scry-muted2)]">
                    {band.count}
                  </span>
                </button>
              </li>
            ))}
          </ul>
        ) : null}
        {groups
          .filter((group) => group.named)
          .map((group) => (
            <button
              key={group.id}
              type="button"
              ref={(element) => {
                if (element) {
                  hullLabelRefs.current.set(group.id, element);
                } else {
                  hullLabelRefs.current.delete(group.id);
                }
              }}
              onClick={() => focusGroup(group.id)}
              style={{ visibility: "hidden" }}
              className="absolute left-0 top-0 max-w-[14rem] truncate rounded-full border-2 bg-[var(--scry-card2)] px-3 py-1 text-[13px] font-bold text-[var(--scry-ink)] shadow-[0_2px_10px_rgba(0,0,0,0.6)]"
            >
              <span
                aria-hidden="true"
                className="mr-1.5 inline-block h-2 w-2 rounded-full align-middle"
                style={{ backgroundColor: group.color }}
              />
              {group.name}
            </button>
          ))}
        {shownHover && hoverTitle.length + hoverLines.length > 0 ? (
          <div
            role="tooltip"
            className="pointer-events-none absolute z-10 max-w-[16rem] rounded-[8px] border border-[var(--scry-border2)] bg-[var(--scry-card2)] px-2.5 py-1.5 text-[12px] text-[var(--scry-ink2)] shadow-lg"
            style={{
              left: shownHover.x + 12,
              top: shownHover.y + 12,
            }}
          >
            {hoverKindsOnly ? (
              <div className="mb-0.5 text-[11px] text-[var(--scry-muted2)]">
                {t("libraryGraph.linkedBy")}
              </div>
            ) : null}
            {hoverTitle.map((line) => (
              <div key={`title:${line}`} className="font-semibold">
                {line}
              </div>
            ))}
            {hoverLines.map((line) => (
              <div
                key={`why:${line}`}
                className={
                  hoverTitle.length > 0 ? "text-[var(--scry-muted2)]" : ""
                }
              >
                {line}
              </div>
            ))}
          </div>
        ) : null}
        {loadingShown && (!refreshing || refreshIndicatorDue) ? (
          <div
            role="status"
            className={
              refreshing
                ? "pointer-events-none absolute right-4 top-4 w-48 rounded-lg bg-[var(--scry-surfD)] p-3 text-[13px] text-[var(--scry-muted2)]"
                : "pointer-events-none absolute inset-0 flex flex-col items-center justify-center gap-3 p-6 text-center text-[13px] text-[var(--scry-muted2)]"
            }
          >
            {t("label.loading")}
            <style>{`@keyframes library-graph-loading { from { transform: translateX(-100%); } to { transform: translateX(340%); } }`}</style>
            <div
              role="progressbar"
              aria-label={t("label.loading")}
              aria-valuemin={0}
              aria-valuemax={100}
              aria-valuenow={
                loadStatus === "pending"
                  ? undefined
                  : join && !previewing
                    ? 95
                    : 60 +
                      30 *
                        Math.min(
                          1,
                          Math.max(currentTitles.length, loadedTitles) /
                            Math.max(1, totalTitles),
                        )
              }
              className="h-2 w-full max-w-xs overflow-hidden rounded-full bg-[var(--scry-border2)]"
            >
              <div
                className="h-full rounded-full bg-[var(--scry-accent)] transition-[width] duration-300"
                style={
                  loadStatus === "pending"
                    ? {
                        width: "30%",
                        animation:
                          "library-graph-loading 1.2s ease-in-out infinite",
                      }
                    : {
                        width: `${join && !previewing ? 95 : 60 + 30 * Math.min(1, Math.max(currentTitles.length, loadedTitles) / Math.max(1, totalTitles))}%`,
                      }
                }
              />
            </div>
          </div>
        ) : graphStatus === "empty" ||
          graphStatus === "notBuilt" ||
          graphStatus === "failed" ? (
          <div
            className="absolute inset-0 flex items-center justify-center p-6 text-center text-[13px] text-[var(--scry-muted2)]"
            data-testid="library-graph-status"
          >
            {graphStatus === "notBuilt"
              ? t("libraryGraph.notBuilt")
              : graphStatus === "failed"
                ? t("status.apiError")
                : t("libraryGraph.empty")}
          </div>
        ) : null}
        {selectionCardOpen &&
        selectedNode &&
        selectedNode.collapsedClusterId === undefined ? (
          <LibraryGraphSelectionCard
            key={selectedNode.subjectKey}
            label={nodeLabel(selectedNode)}
            item={selectedItem}
            title={selectedCatalogTitle}
            facet={facet}
            ghost={selectedNode.ghost}
            inLibrary={!selectedNode.ghost || selectedResolved}
            onClose={() => {
              setSelectionCardOpen(false);
              containerRef.current?.focus({ preventScroll: true });
            }}
          >
            <div className="flex flex-wrap items-center justify-end gap-1.5">
              {selectedResolved && openSelected ? (
                <Button
                  type="button"
                  size="sm"
                  onClick={() => {
                    setSelectionCardOpen(false);
                    openSelected();
                  }}
                  className="h-11 min-w-24 gap-2 rounded-[10px] px-5 text-sm font-semibold"
                >
                  {t("nav.group.overview")}
                  <ArrowUpRight className="h-4 w-4" aria-hidden="true" />
                </Button>
              ) : null}
              {selectedItem && (selectedAddable || selectedRequestable) ? (
                <Button
                  type="button"
                  size="sm"
                  onClick={() => {
                    actedGhostKeyRef.current = selectedNode.subjectKey;
                    void onAction?.(selectedItem);
                  }}
                  className="h-11 min-w-24 rounded-[10px] px-5 text-sm font-semibold"
                >
                  {selectedAddable
                    ? t("discovery.add")
                    : t("discovery.request")}
                </Button>
              ) : null}
            </div>
            {expandingKey === selectedNode.subjectKey ? (
              <p role="status" className="text-[var(--scry-muted2)]">
                {t("libraryGraph.findingMoreLikeThis")}
              </p>
            ) : null}
            {expandNotice?.key === selectedNode.subjectKey &&
            expandNotice.kind === "empty" ? (
              <p role="status" className="text-[var(--scry-muted2)]">
                {t("libraryGraph.moreLikeThisEmpty")}
              </p>
            ) : null}
          </LibraryGraphSelectionCard>
        ) : null}
      </div>
      {moreLikeThisActions.dialogs}
    </section>
  );
}
