import * as React from "react";
import Graph from "graphology";
import Sigma from "sigma";
import { useClient } from "urql";
import { Maximize2, Minimize2, Undo2, X } from "lucide-react";

import { Button } from "@/components/ui/button";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useTranslate } from "@/lib/context/translate-context";
import { discoveryNeighborsQuery } from "@/lib/graphql/queries";
import { useAuth } from "@/lib/hooks/use-auth";
import { useTitleMoreLikeThisActions } from "@/lib/hooks/use-title-more-like-this-actions";
import type { DiscoveryItem } from "@/lib/types/discovery";
import { discoveryItemFacet } from "@/lib/utils/discovery-actions";
import { discoveryErrorMessage } from "@/lib/utils/discovery-error-message";
import { discoveryItemDisplayTitle } from "@/lib/utils/discovery-display";
import {
  LIBRARY_PERMISSIONS,
  hasAnyLibraryPermission,
} from "@/lib/utils/permissions";
import { selectPosterVariantUrl } from "@/lib/utils/poster-images";
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
import {
  LIBRARY_GRAPH_COLLAPSE_THRESHOLD,
  LIBRARY_GRAPH_MIN_NODE_SIZE,
  LIBRARY_GRAPH_SIZE_METRICS,
  displayedLibraryGraph,
  joinLibraryGraphToTitles,
  libraryGraphEdgeKindLabels,
  libraryGraphGroupName,
  neighborReason,
  neighborReasonLines,
  nodeSizesByTitleId,
  recommendationSections,
  titleIndexByMatchKey,
  titleMatchKeys,
  titleTargetKey,
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
  isLibraryItem,
  itemMatchKeys,
  moreLikeThisEdgeKinds,
  neighborTargetKey,
  nextNodeInCluster,
  ownedNodeIndex,
  ownedSubjectForItem,
  stampGhostTitle,
  uncoveredKeyboardGroups,
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
const LAYOUT_STEPS_PER_FRAME = 6;
const MAX_POSTERS_PER_FRAME = 240;
const MIN_POSTER_RADIUS_PX = 9;
const HULL_PADDING_PX = 18;
/** How many neighbours one expansion asks for. */
const NEIGHBOR_LIMIT = 24;
const GHOST_NODE_SIZE = LIBRARY_GRAPH_MIN_NODE_SIZE + 1;
/** How long nodes take to move between the overview and a focused node. */
const FOCUS_TWEEN_MS = 450;
/** The outer ring of a focused node's neighbours, as a share of the frame. */
const FOCUS_OUTER_RADIUS_SHARE = 0.78;
/** The least on-screen gap between neighbours on one ring. */
const FOCUS_RING_SPACING_PX = 40;
/** Room left beside the outer ring for its labels. */
const FOCUS_LABEL_ROOM_PX = 130;
const FOCUS_MIN_OUTER_RADIUS_PX = 90;
const FOCUS_CENTER_SIZE = 16;
const FOCUS_NEIGHBOR_MIN_SIZE = 10;
const FOCUS_NEIGHBOR_MAX_SIZE = 14;
/** Focused-view labels longer than this are cut short. */
const FOCUS_LABEL_MAX_CHARS = 28;

const METRIC_LABEL_KEYS: Record<LibraryGraphSizeMetric, string> = {
  ratingEvidence: "libraryGraph.sizeBy.ratingEvidence",
  rating: "libraryGraph.sizeBy.rating",
  popularity: "libraryGraph.sizeBy.popularity",
};

type NodeAttributes = {
  x: number;
  y: number;
  size: number;
  color: string;
  baseSize: number;
  baseColor: string;
  label: string;
  titleId?: string;
  posterUrl?: string | null;
  collapsedClusterId?: number;
  clusterId: number;
  /** A title reached by exploring, not in the library. */
  ghost?: boolean;
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

function clusterColor(clusterId: number, order: readonly number[]): string {
  if (clusterId === UNPLACED_CLUSTER_ID) {
    return UNPLACED_COLOR;
  }
  const position = order.indexOf(clusterId);
  const index = position < 0 ? clusterId : position;
  // An id the order does not know may be negative; wrap it into range.
  const count = CLUSTER_COLORS.length;
  return CLUSTER_COLORS[((index % count) + count) % count];
}

// The camera at rest: unzoomed and centred on the layout's origin, where
// cluster rings and focused views are centred.
function restCamera(sigma: Sigma<NodeAttributes, EdgeAttributes>) {
  const origin = sigma.graphToFramedGraph({ x: 0, y: 0 });
  return { x: origin.x, y: origin.y, ratio: 1, angle: 0 };
}

function withAlpha(hex: string, alpha: number): string {
  const value = Number.parseInt(hex.slice(1), 16);
  return `rgba(${(value >> 16) & 255}, ${(value >> 8) & 255}, ${value & 255}, ${alpha})`;
}

function readCssColor(element: Element, name: string, fallback: string) {
  const value = getComputedStyle(element).getPropertyValue(name).trim();
  return value || fallback;
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
  libraryIds: readonly string[];
  selectedTitleId: string | null;
  contextPanelId?: string;
  onSelectTitle: (title: TitleRecord) => void;
  /** Refreshes the catalog listing after a title is added or requested. */
  onCatalogChanged?: () => Promise<void> | void;
};

// The catalog listing's title an explored item names: the title Scryer
// resolved it to, else the first title one of its keys names.
function catalogTitleForItem(
  item: DiscoveryItem,
  catalogIndex: ReadonlyMap<string, TitleRecord>,
  titles: readonly TitleRecord[],
): TitleRecord | null {
  const resolved = item.resolvedTitleId?.trim();
  const byId = resolved
    ? titles.find((title) => title.id === resolved)
    : undefined;
  if (byId) {
    return byId;
  }
  for (const key of itemMatchKeys(item)) {
    const title = catalogIndex.get(key);
    if (title) {
      return title;
    }
  }
  return null;
}

type NeighborsData = {
  discoveryNeighbors?: DiscoveryItem[] | null;
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
  libraryIds,
  selectedTitleId,
  contextPanelId,
  onSelectTitle,
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
  const layoutRef = React.useRef<LayoutState | null>(null);
  const frameRef = React.useRef<number | null>(null);
  const tweenFrameRef = React.useRef<number | null>(null);
  // The overview frame's half-width in layout units; focused rings share it.
  const extentRef = React.useRef(220);
  // The focused node the camera last came to rest for; undefined before the
  // first drawing.
  const cameraCenterRef = React.useRef<string | null | undefined>(undefined);
  // Bumped when a focused view's frame changes size, so its rings refit.
  const [focusFrameVersion, setFocusFrameVersion] = React.useState(0);
  // Restarts the layout loop after it paused for a hidden container.
  const resumeLayoutRef = React.useRef<() => void>(() => {});
  const positionsRef = React.useRef(new Map<string, LayoutPoint>());
  const posterCacheRef = React.useRef(new Map<string, HTMLImageElement>());
  const selectedTitleIdRef = React.useRef(selectedTitleId);
  const titlesByIdRef = React.useRef(new Map<string, TitleRecord>());
  const onSelectTitleRef = React.useRef(onSelectTitle);
  // The node picked in the graph (owned or ghost) and the node keyboard focus
  // rests on; the overlay reads both while it draws.
  const selectedKeyRef = React.useRef<string | null>(null);
  const focusedKeyRef = React.useRef<string | null>(null);
  // The node a focused view is centred on, while the view is focused.
  const focusCenterRef = React.useRef<string | null>(null);
  const activateNodeRef = React.useRef<(key: string) => void>(() => {});
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
  if (titlesComplete && titles !== currentTitles) {
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
  const [focusedKey, setFocusedKey] = React.useState<string | null>(null);
  // Why each explored result sits next to the title it was found from, by
  // the pair of nodes their link joins.
  const [reasonsByPair, setReasonsByPair] = React.useState<
    ReadonlyMap<string, NeighborReason>
  >(() => new Map());
  // Replies already read this session, by facet and target key, so going
  // back to a title does not ask again.
  const neighborCacheRef = React.useRef(new Map<string, DiscoveryItem[]>());
  const [expandingKey, setExpandingKey] = React.useState<string | null>(null);
  const [expandNotice, setExpandNotice] = React.useState<ExpandNotice | null>(
    null,
  );
  const [metric, setMetric] =
    React.useState<LibraryGraphSizeMetric>("ratingEvidence");
  const [expandedClusterIds, setExpandedClusterIds] = React.useState<
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
        color: isSelected ? accent : attributes.baseColor,
        size: isSelected ? attributes.baseSize * 1.35 : attributes.baseSize,
      };
    });
  }, []);

  React.useEffect(() => {
    selectedTitleIdRef.current = selectedTitleId;
    applySelection();
  }, [applySelection, selectedTitleId]);
  React.useEffect(() => {
    onSelectTitleRef.current = onSelectTitle;
  }, [onSelectTitle]);

  const catalogTargetKeys = React.useMemo(
    () =>
      (titles ?? [])
        .map((title) => titleTargetKey(title))
        .filter((key): key is string => key !== null)
        .sort(),
    [titles],
  );
  const hasTitles = catalogTargetKeys.length > 0;
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
  if (exploredSourceKey !== sourceKey) {
    setExploredSourceKey(sourceKey);
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
    if (!hasTitles || loadedSourceKeyRef.current === sourceKey) {
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
  }, [
    client,
    facet,
    hasTitles,
    libraryIdSignature,
    resetExploration,
    sourceKey,
  ]);

  const join = React.useMemo(
    () => (result && titles ? joinLibraryGraphToTitles(result, titles) : null),
    [result, titles],
  );
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
  const overview = React.useMemo(
    () =>
      explored && join
        ? displayedLibraryGraph(
            explored,
            expandedClusterIds,
            LIBRARY_GRAPH_COLLAPSE_THRESHOLD,
            join.graph,
          )
        : null,
    [expandedClusterIds, explored, join],
  );
  const displayed = React.useMemo(
    () =>
      explored && focusCenterKey !== null
        ? (egoGraphState(explored, focusCenterKey) ?? overview)
        : overview,
    [explored, focusCenterKey, overview],
  );
  // A focused view's rings, inner first, each as sections around the circle:
  // library titles as one, recommendations grouped by a shared genre or
  // theme.
  const focusRings = React.useMemo(() => {
    if (focusCenterKey === null || !displayed) {
      return [];
    }
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
            )
          : [{ tagKey: null, name: null, keys: ring }],
    );
  }, [displayed, focusCenterKey, traversal.ghostItems]);
  const recommendationBands = React.useMemo(
    () =>
      focusRings
        .flat()
        .filter((section) => section.name !== null)
        .map((section, index) => ({
          key: section.tagKey ?? String(index),
          name: section.name ?? "",
          keys: section.keys,
          color: CLUSTER_COLORS[index % CLUSTER_COLORS.length],
        })),
    [focusRings],
  );
  const recommendationBandsRef = React.useRef(recommendationBands);
  React.useEffect(() => {
    recommendationBandsRef.current = recommendationBands;
  }, [recommendationBands]);
  // Every cluster of the whole graph, before filtering, so layout centres hold
  // still when a filter empties a cluster. The unplaced group takes a slot on
  // the ring only when the graph has unplaced titles, so a single
  // neighbourhood sits on the origin.
  const allClusterIds = React.useMemo(() => {
    const ids = (result?.clusters ?? []).map((cluster) => cluster.id);
    return result?.nodes.some((node) => node.clusterId === UNPLACED_CLUSTER_ID)
      ? [UNPLACED_CLUSTER_ID, ...ids]
      : ids;
  }, [result]);
  const clusterOrder = React.useMemo(
    () =>
      (join?.graph.clusters ?? [])
        .map((cluster) => cluster.id)
        .sort((a, b) => a - b),
    [join],
  );
  const groups = React.useMemo(() => {
    if (!join) {
      return [];
    }
    const counts = new Map<number, number>();
    for (const node of join.graph.nodes) {
      counts.set(node.clusterId, (counts.get(node.clusterId) ?? 0) + 1);
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
    })).sort((a, b) =>
      a.id === UNPLACED_CLUSTER_ID
        ? 1
        : b.id === UNPLACED_CLUSTER_ID
          ? -1
          : a.id - b.id,
    );
  }, [clusterOrder, join, t]);
  // Neighbourhoods are named and outlined only when there are several; one
  // neighbourhood is the whole library.
  const showsNeighbourhoods =
    groups.filter((group) => group.id !== UNPLACED_CLUSTER_ID).length > 1;
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
        const name = discoveryItemDisplayTitle(item);
        return item.year ? `${name} (${item.year})` : name;
      }
      const title = join?.titleBySubjectKey.get(node.subjectKey);
      if (title) {
        return title.year ? `${title.name} (${title.year})` : title.name;
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
    const canvas = overlayRef.current;
    if (!sigma || !graph || !canvas) {
      return;
    }
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
    context.setTransform(ratio, 0, 0, ratio, 0, 0);
    context.clearRect(0, 0, width, height);

    const pointsByCluster = new Map<number, LayoutPoint[]>();
    const viewportByNode = new Map<string, LayoutPoint>();
    graph.forEachNode((key, attributes) => {
      const point = sigma.graphToViewport({
        x: attributes.x as number,
        y: attributes.y as number,
      });
      viewportByNode.set(key, point);
      // Ghosts are not in the library, so they never stretch a cluster hull.
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
    });

    // Cluster hulls, padded by stroking the hull with a wide round pen.
    context.lineJoin = "round";
    context.lineCap = "round";
    const focusCenter = focusCenterRef.current;
    for (const group of groupsRef.current) {
      const label = hullLabelRefs.current.get(group.id);
      const points = pointsByCluster.get(group.id);
      // A focused view draws no cluster hulls, and neither does a graph
      // with one neighbourhood, which would only outline everything.
      if (
        focusCenter !== null ||
        !showsNeighbourhoodsRef.current ||
        !points ||
        points.length === 0
      ) {
        if (label) {
          label.style.visibility = "hidden";
        }
        continue;
      }
      const hull = convexHull(points);
      context.beginPath();
      if (hull.length === 1) {
        // One node, or several on one spot: a zero-length path strokes
        // nothing, so the hull is the padding circle around that point.
        context.arc(hull[0].x, hull[0].y, HULL_PADDING_PX, 0, Math.PI * 2);
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
        // The wide round pen pads the hull by HULL_PADDING_PX all round.
        context.strokeStyle = withAlpha(group.color, 0.1);
        context.lineWidth = HULL_PADDING_PX * 2;
        context.stroke();
      }
      if (label) {
        const top = hull.reduce(
          (best, point) => (point.y < best.y ? point : best),
          hull[0],
        );
        const centerX =
          hull.reduce((total, point) => total + point.x, 0) / hull.length;
        const x = Math.min(Math.max(centerX, 60), width - 60);
        const y = Math.min(
          Math.max(top.y - HULL_PADDING_PX - 14, 14),
          height - 14,
        );
        label.style.visibility = "visible";
        label.style.transform = `translate(${x}px, ${y}px) translate(-50%, -50%)`;
      }
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
        let start = Infinity;
        let end = -Infinity;
        let pad = 0;
        for (const key of band.keys) {
          const point = viewportByNode.get(key);
          if (!point || !graph.hasNode(key)) {
            continue;
          }
          const radius = Math.hypot(
            point.x - bandCenter.x,
            point.y - bandCenter.y,
          );
          // Angles run clockwise from straight up, as the rings lay out.
          let angle = Math.atan2(
            point.y - bandCenter.y,
            point.x - bandCenter.x,
          );
          if (angle < -Math.PI / 2) {
            angle += Math.PI * 2;
          }
          const size = sigma.scaleSize(
            graph.getNodeAttribute(key, "size") as number,
          );
          inner = Math.min(inner, radius);
          outer = Math.max(outer, radius);
          start = Math.min(start, angle);
          end = Math.max(end, angle);
          pad = Math.max(pad, size + 6);
        }
        if (outer === 0) {
          continue;
        }
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

    // Posters over the nodes large enough on screen to show one.
    let drawn = 0;
    graph.forEachNode((key, attributes) => {
      if (drawn >= MAX_POSTERS_PER_FRAME) {
        return;
      }
      const url = attributes.posterUrl as string | null | undefined;
      if (!url) {
        return;
      }
      const radius = sigma.scaleSize(attributes.size as number);
      const point = viewportByNode.get(key);
      if (
        !point ||
        radius < MIN_POSTER_RADIUS_PX ||
        point.x < -radius ||
        point.y < -radius ||
        point.x > width + radius ||
        point.y > height + radius
      ) {
        return;
      }
      let image = posterCacheRef.current.get(url);
      if (!image) {
        image = new Image();
        image.decoding = "async";
        image.onload = () => sigmaRef.current?.scheduleRender();
        image.src = url;
        posterCacheRef.current.set(url, image);
      }
      if (!image.complete || image.naturalWidth === 0) {
        return;
      }
      context.save();
      if (attributes.ghost) {
        context.globalAlpha = 0.55;
      }
      context.beginPath();
      context.arc(point.x, point.y, radius, 0, Math.PI * 2);
      context.clip();
      const scale = Math.max(
        (radius * 2) / image.naturalWidth,
        (radius * 2) / image.naturalHeight,
      );
      const drawWidth = image.naturalWidth * scale;
      const drawHeight = image.naturalHeight * scale;
      context.drawImage(
        image,
        point.x - drawWidth / 2,
        point.y - drawHeight / 2,
        drawWidth,
        drawHeight,
      );
      context.restore();
      if (
        selectedKeyRef.current !== null
          ? key === selectedKeyRef.current
          : attributes.titleId &&
            attributes.titleId === selectedTitleIdRef.current
      ) {
        context.beginPath();
        context.arc(point.x, point.y, radius + 2, 0, Math.PI * 2);
        context.lineWidth = 3;
        context.strokeStyle = readCssColor(canvas, "--scry-accent", "#5b64ff");
        context.stroke();
      }
      drawn += 1;
    });

    // Ghosts wear a dashed ring so they read apart from library titles.
    const muted = readCssColor(canvas, "--scry-muted2", UNPLACED_COLOR);
    context.save();
    context.setLineDash([3, 3]);
    context.lineWidth = 1.5;
    context.strokeStyle = muted;
    graph.forEachNode((key, attributes) => {
      if (!attributes.ghost) {
        return;
      }
      const point = viewportByNode.get(key);
      const radius = sigma.scaleSize(attributes.size as number);
      if (
        !point ||
        point.x < -radius ||
        point.y < -radius ||
        point.x > width + radius ||
        point.y > height + radius
      ) {
        return;
      }
      context.beginPath();
      context.arc(point.x, point.y, radius + 2, 0, Math.PI * 2);
      context.stroke();
    });
    context.restore();

    // A focused view names the nodes it shows, each label pointing away
    // from the centre so the ring stays clear. Labels are placed centre
    // first, then the keyboard-focused node, then outward; one that would
    // overlap a placed label is left off, and the hover card names it.
    const centerPoint =
      focusCenter !== null ? viewportByNode.get(focusCenter) : undefined;
    if (centerPoint) {
      context.save();
      context.textBaseline = "middle";
      context.lineJoin = "round";
      context.lineWidth = 3;
      context.strokeStyle = readCssColor(canvas, "--scry-surfD", "#0b0e15");
      context.fillStyle = readCssColor(canvas, "--scry-ink2", "#e6e8ef");
      const keyboardKey = focusedKeyRef.current;
      const order: string[] = [];
      graph.forEachNode((key) => {
        if (viewportByNode.has(key)) {
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
          distance(a) - distance(b) ||
          (a < b ? -1 : a > b ? 1 : 0),
      );
      const placed: {
        left: number;
        top: number;
        right: number;
        bottom: number;
      }[] = [];
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
        context.fillStyle = readCssColor(canvas, "--scry-surfD", "#0b0e15");
        context.fill();
        context.fillStyle = withAlpha(label.color, 0.25);
        context.fill();
        context.lineWidth = 1;
        context.strokeStyle = label.color;
        context.stroke();
        context.fillStyle = readCssColor(canvas, "--scry-ink2", "#e6e8ef");
        context.fillText(label.text, x, y);
        placed.push(box);
      }
      context.lineWidth = 3;
      context.strokeStyle = readCssColor(canvas, "--scry-surfD", "#0b0e15");
      context.fillStyle = readCssColor(canvas, "--scry-ink2", "#e6e8ef");
      for (const key of order) {
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
        const radius = sigma.scaleSize(
          graph.getNodeAttribute(key, "size") as number,
        );
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
      const radius = sigma.scaleSize(
        graph.getNodeAttribute(focused, "size") as number,
      );
      context.beginPath();
      context.arc(focusPoint.x, focusPoint.y, radius + 5, 0, Math.PI * 2);
      context.lineWidth = 2;
      context.strokeStyle = readCssColor(canvas, "--scry-ink2", "#e6e8ef");
      context.stroke();
    }
  }, []);

  // One renderer for the life of the view.
  React.useEffect(() => {
    const container = containerRef.current;
    if (!container) {
      return;
    }
    const graph: LibraryGraphStore = new Graph<NodeAttributes, EdgeAttributes>({
      type: "undirected",
      multi: false,
    });
    const sigma = new Sigma<NodeAttributes, EdgeAttributes>(graph, container, {
      settings: {
        // The view can sit in a hidden drawer (narrow screens with a title
        // open), where the container has no size; sigma must not throw there.
        allowInvalidContainer: true,
        renderLabels: false,
        renderEdgeLabels: false,
        enableEdgeEvents: true,
        autoRescale: false,
        gestureTarget: "graph",
      },
    });
    graphRef.current = graph;
    sigmaRef.current = sigma;

    sigma.on("afterRender", drawOverlay);
    sigma.on("clickNode", ({ node }) => {
      activateNodeRef.current(node);
    });
    sigma.on("enterNode", ({ node, event }) => {
      const attributes = graph.getNodeAttributes(node);
      setHover({
        kind: "node",
        key: node,
        x: event.x,
        y: event.y,
        lines: [attributes.label],
      });
      container.style.cursor = "pointer";
    });
    sigma.on("leaveNode", () => {
      setHover(null);
      container.style.cursor = "";
    });
    sigma.on("enterEdge", ({ edge, event }) => {
      setHover({
        kind: "edge",
        from: graph.source(edge),
        to: graph.target(edge),
        x: event.x,
        y: event.y,
        lines: graph.getEdgeAttribute(edge, "kinds"),
      });
    });
    sigma.on("leaveEdge", () => setHover(null));
    // A touch never leaves a node, so the tooltip goes when the finger lifts.
    const clearHover = () => setHover(null);
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
      sigma.kill();
      sigmaRef.current = null;
      graphRef.current = null;
    };
  }, [drawOverlay]);

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
    const focusing = focusCenterKey !== null;
    const sizes = nodeSizesByTitleId(Array.from(titlesById.values()), metric);
    const mutedEdge = readCssColor(
      sigma.getContainer(),
      "--scry-muted2",
      "#7a8499",
    );

    // Where each drawn node is now, so it can glide from there.
    const current = new Map<string, LayoutPoint>();
    graph.forEachNode((key, attributes) => {
      current.set(key, { x: attributes.x, y: attributes.y });
    });
    const keep = new Set(displayed.nodes.map((node) => node.subjectKey));
    graph.forEachNode((key) => {
      if (!keep.has(key)) {
        graph.dropNode(key);
      }
    });
    graph.clearEdges();
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
    if (focusCenterKey !== null) {
      // The rings share the overview's frame, so nodes glide in one space.
      // They are sized in screen pixels for the camera at rest on the
      // origin, leaving room beside the outer ring for labels. The frame is
      // pinned first: left to itself it would fit whatever the nodes cover
      // now, and change scale under the rings once they move.
      const extent = extentRef.current;
      sigma.setCustomBBox({ x: [-extent, extent], y: [-extent, extent] });
      const rest = restCamera(sigma);
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
    } else {
      layout = createLayout(
        displayed.nodes.map((node) => ({
          key: node.subjectKey,
          clusterId: node.clusterId,
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
      positionsRef.current = targets;
    }
    // A node new to the drawing starts where it goes, or in a focused view
    // at the focused node, so its neighbours fan out from it.
    const fanFrom =
      focusCenterKey !== null
        ? (current.get(focusCenterKey) ?? targets.get(focusCenterKey))
        : undefined;
    const from = new Map<string, LayoutPoint>();
    for (const [key, target] of targets) {
      from.set(key, current.get(key) ?? fanFrom ?? target);
    }

    for (const node of displayed.nodes) {
      const position = from.get(node.subjectKey) ?? { x: 0, y: 0 };
      const color = clusterColor(node.clusterId, clusterOrder);
      let attributes: NodeAttributes;
      if (node.collapsedClusterId !== undefined) {
        const size = Math.min(36, 10 + Math.sqrt(node.memberCount ?? 1));
        attributes = {
          ...position,
          size,
          color,
          baseSize: size,
          baseColor: color,
          label: nodeLabel(node),
          collapsedClusterId: node.collapsedClusterId,
          clusterId: node.clusterId,
        };
      } else if (node.ghost) {
        const item = traversal.ghostItems.get(node.subjectKey);
        const ghostColor = withAlpha(color, 0.35);
        const size =
          node.subjectKey === focusCenterKey
            ? FOCUS_CENTER_SIZE
            : focusing
              ? FOCUS_NEIGHBOR_MIN_SIZE
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
            ? FOCUS_CENTER_SIZE
            : focusing
              ? Math.min(
                  FOCUS_NEIGHBOR_MAX_SIZE,
                  Math.max(FOCUS_NEIGHBOR_MIN_SIZE, metricSize),
                )
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
      if (graph.hasNode(node.subjectKey)) {
        graph.replaceNodeAttributes(node.subjectKey, attributes);
      } else {
        graph.addNode(node.subjectKey, attributes);
      }
    }
    for (const edge of displayed.edges) {
      if (graph.hasEdge(edge.from, edge.to)) {
        continue;
      }
      const toGhost =
        graph.getNodeAttribute(edge.from, "ghost") === true ||
        graph.getNodeAttribute(edge.to, "ghost") === true;
      const weight = Math.min(1, Math.max(0, edge.weight));
      // A focused view draws few links, so they can be bolder.
      const alpha = focusing
        ? (0.3 + weight * 0.4) * (toGhost ? 0.7 : 1)
        : (0.18 + weight * 0.4) * (toGhost ? 0.6 : 1);
      const attributes: EdgeAttributes = {
        size: 0.4 + weight * 2.2,
        color: mutedEdge.startsWith("#")
          ? withAlpha(mutedEdge, alpha)
          : mutedEdge,
        kinds: edge.kinds,
      };
      graph.addUndirectedEdge(edge.from, edge.to, attributes);
    }

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
      if (decision !== "step") {
        // Paused while hidden: the next non-zero size resumes it.
        return;
      }
      stepLayout(running, LAYOUT_STEPS_PER_FRAME);
      const positions = layoutPositions(running);
      positionsRef.current = positions;
      graph.updateEachNodeAttributes((key, attributes) => {
        const point = positions.get(key);
        return point ? { ...attributes, x: point.x, y: point.y } : attributes;
      });
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
        start !== undefined && (start.x !== target.x || start.y !== target.y)
      );
    });
    if (moving) {
      let started: number | null = null;
      const glide = (now: number) => {
        tweenFrameRef.current = null;
        started ??= now;
        const progress = Math.min(1, (now - started) / FOCUS_TWEEN_MS);
        const positions = tweenPositions(from, targets, progress);
        graph.updateEachNodeAttributes((key, attributes) => {
          const point = positions.get(key);
          return point ? { ...attributes, x: point.x, y: point.y } : attributes;
        });
        if (progress < 1) {
          tweenFrameRef.current = window.requestAnimationFrame(glide);
        } else {
          resumeLayoutRef.current();
        }
      };
      tweenFrameRef.current = window.requestAnimationFrame(glide);
    }
    // Entering, moving or leaving focus brings the camera back to rest on
    // the frame, where the rings fit; so does any new ring layout, since
    // the rings are sized for the camera at rest.
    if (cameraCenterRef.current !== focusCenterKey || focusCenterKey !== null) {
      cameraCenterRef.current = focusCenterKey;
      void sigma
        .getCamera()
        .animate(restCamera(sigma), { duration: FOCUS_TWEEN_MS });
    }
    applySelection();
    sigma.refresh();
    resumeLayoutRef.current();
  }, [
    allClusterIds,
    applySelection,
    clusterOrder,
    displayed,
    focusCenterKey,
    focusFrameVersion,
    focusRings,
    join,
    metric,
    nodeLabel,
    titlesById,
    traversal,
  ]);

  // Explored titles that settled in no library cluster get a chip of their
  // own, so the keyboard can reach them.
  const exploredGroups = React.useMemo(
    () =>
      uncoveredKeyboardGroups(
        keyboardNodes,
        new Set(groups.map((group) => group.id)),
      ),
    [groups, keyboardNodes],
  );
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
    let minX = Number.POSITIVE_INFINITY;
    let minY = Number.POSITIVE_INFINITY;
    let maxX = Number.NEGATIVE_INFINITY;
    let maxY = Number.NEGATIVE_INFINITY;
    graph.forEachNode((_key, attributes) => {
      if (attributes.clusterId !== clusterId) {
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
      return;
    }
    const span = Math.max(maxX - minX, maxY - minY);
    void sigma.getCamera().animate(
      {
        x: (minX + maxX) / 2,
        y: (minY + maxY) / 2,
        ratio: Math.min(1, Math.max(0.08, span * 1.5)),
      },
      { duration: 400 },
    );
  }, []);

  const resetCamera = React.useCallback(() => {
    const sigma = sigmaRef.current;
    if (sigma) {
      void sigma.getCamera().animate(restCamera(sigma), { duration: 300 });
    }
  }, []);

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
        let items = neighborCacheRef.current.get(cacheKey);
        if (!items) {
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
          items = data?.discoveryNeighbors ?? [];
          neighborCacheRef.current.set(cacheKey, items);
        }
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
          expandTraversal(current, key, items, ownedIndexRef.current),
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

  // Leaves the focused view for the library overview, dropping what was
  // explored from it.
  const exitFocus = React.useCallback(() => {
    invalidateExpansions(expandTokenRef);
    setTraversal(emptyTraversal<DiscoveryItem>());
    setReasonsByPair(new Map());
    setSelectedKey(null);
    setFocusedKey(null);
    setExpandingKey(null);
    setExpandNotice(null);
  }, []);

  // Picking a node focuses the view on it and brings in more like it. The
  // trail follows the focus: a node already on it cuts the trail back to it,
  // and any other node is explored from.
  const activateNode = React.useCallback(
    (key: string) => {
      const graph = graphRef.current;
      if (!graph || !graph.hasNode(key)) {
        return;
      }
      const attributes = graph.getNodeAttributes(key);
      if (attributes.collapsedClusterId !== undefined) {
        const clusterId = attributes.collapsedClusterId;
        setExpandedClusterIds((current) =>
          toggleClusterExpanded(current, clusterId),
        );
        return;
      }
      invalidateExpansions(expandTokenRef);
      setExpandingKey(null);
      setExpandNotice(null);
      setSelectedKey(key);
      setFocusedKey(key);
      const title = attributes.titleId
        ? titlesByIdRef.current.get(attributes.titleId)
        : undefined;
      if (title) {
        onSelectTitleRef.current(title);
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
    [expandNode, exploredNodesByKey, traversal.expansions],
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
      setTraversal((current) => collapseTraversalToCrumb(current, index));
      setSelectedKey(anchor);
      setFocusedKey(anchor);
    },
    [exitFocus, traversal.expansions],
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
            activateNode(shownFocusedKey);
          }
          return;
        }
        case "Escape": {
          if (selectedNode) {
            event.preventDefault();
            exitFocus();
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
    [activateNode, exitFocus, keyboardNodes, selectedNode, shownFocusedKey],
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
  // Above the threshold clusters start collapsed; an expanded one can fold
  // back from its chip.
  const collapsing = visibleNodeCount > LIBRARY_GRAPH_COLLAPSE_THRESHOLD;
  const selectedVisible =
    selectedTitleId !== null && titlesById.has(selectedTitleId);
  // Titles still paging, or the graph still loading for them, show as
  // loading; a finished listing with nothing to draw shows as empty.
  const graphStatus: "loading" | "empty" | "notBuilt" | "failed" | "ready" =
    !titlesComplete
      ? "loading"
      : !hasTitles
        ? "empty"
        : loadStatus === "pending"
          ? "loading"
          : loadStatus === "notBuilt"
            ? "notBuilt"
            : loadStatus === "failed"
              ? "failed"
              : visibleNodeCount === 0
                ? "empty"
                : "ready";

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
  // A ghost that is one of the viewer's titles, though not drawn as a
  // library node, opens instead of offering Add or Request: through the
  // listing when it is there, else through the title Scryer resolved.
  const selectedResolved =
    selectedItem !== null && isLibraryItem(selectedItem, catalogIndex);
  const selectedCatalogTitle = selectedItem
    ? catalogTitleForItem(selectedItem, catalogIndex, titles ?? [])
    : null;
  const openSelected = selectedCatalogTitle
    ? () => onSelectTitle(selectedCatalogTitle)
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
      <div className="flex flex-wrap items-center gap-2">
        <span className="text-[12px] text-[var(--scry-muted2)]">
          {t("libraryGraph.sizeByLabel")}
        </span>
        <ToggleGroup
          type="single"
          variant="outline"
          size="sm"
          value={metric}
          onValueChange={(value) => {
            const next = LIBRARY_GRAPH_SIZE_METRICS.find(
              (candidate) => candidate === value,
            );
            if (next) {
              setMetric(next);
            }
          }}
          aria-label={t("libraryGraph.sizeByLabel")}
          className="rounded-[9px] border !border-[var(--scry-border2)] bg-[var(--scry-inset)] p-[0.1875rem]"
        >
          {LIBRARY_GRAPH_SIZE_METRICS.map((candidate) => (
            <ToggleGroupItem
              key={candidate}
              value={candidate}
              size="sm"
              className="h-7 rounded-[7px] px-2.5 text-[12px] text-[var(--scry-muted2)] data-[state=on]:!bg-[var(--scry-accent)] data-[state=on]:!text-primary-foreground"
            >
              {t(METRIC_LABEL_KEYS[candidate])}
            </ToggleGroupItem>
          ))}
        </ToggleGroup>
        <Button
          type="button"
          variant="outline"
          size="sm"
          onClick={resetCamera}
          className="ml-auto h-8 gap-1.5 rounded-[8px] border-[var(--scry-border2)] bg-[var(--scry-inset)] px-2.5 text-[12px] shadow-none"
          aria-label={t("libraryGraph.resetView")}
          title={t("libraryGraph.resetView")}
        >
          <Maximize2 className="h-3.5 w-3.5" />
          <span className="max-sm:sr-only">{t("libraryGraph.resetView")}</span>
        </Button>
      </div>
      {(showsNeighbourhoods || exploredGroups.length > 0) &&
      focusCenterKey === null ? (
        <nav
          aria-label={t("libraryGraph.clustersLabel")}
          className="flex gap-1.5 overflow-x-auto pb-1"
        >
          {(showsNeighbourhoods ? groups : []).map((group) => (
            <span key={group.id} className="flex shrink-0 items-center gap-0.5">
              <button
                type="button"
                onClick={() => focusGroup(group.id)}
                className="flex items-center gap-1.5 rounded-full border border-[var(--scry-border2)] bg-[var(--scry-inset)] px-3 py-1 text-[12px] text-[var(--scry-body)] transition hover:bg-[var(--scry-hover)]"
              >
                <span
                  aria-hidden="true"
                  className="h-2.5 w-2.5 rounded-full"
                  style={{ backgroundColor: group.color }}
                />
                <span>{group.name}</span>
                <span className="text-[var(--scry-muted2)]">{group.count}</span>
              </button>
              {collapsing && expandedClusterIds.has(group.id) ? (
                <button
                  type="button"
                  onClick={() =>
                    setExpandedClusterIds((current) =>
                      toggleClusterExpanded(current, group.id),
                    )
                  }
                  aria-label={t("libraryGraph.collapseCluster", {
                    name: group.name,
                  })}
                  title={t("libraryGraph.collapseCluster", {
                    name: group.name,
                  })}
                  className="flex h-6 w-6 items-center justify-center rounded-full text-[var(--scry-muted2)] transition hover:bg-[var(--scry-hover)]"
                >
                  <Minimize2 className="h-3.5 w-3.5" />
                </button>
              ) : null}
            </span>
          ))}
          {exploredGroups.map((group) => (
            <button
              key={`explored-${group.id}`}
              type="button"
              onClick={() => focusGroup(group.id)}
              className="flex shrink-0 items-center gap-1.5 rounded-full border border-dashed border-[var(--scry-border2)] bg-[var(--scry-inset)] px-3 py-1 text-[12px] text-[var(--scry-body)] transition hover:bg-[var(--scry-hover)]"
            >
              <span>{t("libraryGraph.exploredGroup")}</span>
              <span className="text-[var(--scry-muted2)]">{group.count}</span>
            </button>
          ))}
        </nav>
      ) : null}
      {result?.truncated ? (
        <p
          className="text-[12px] text-[var(--scry-muted2)]"
          data-testid="library-graph-truncated"
        >
          {t("libraryGraph.truncated")}
        </p>
      ) : null}
      {focusCenterKey !== null ? (
        <nav
          aria-label={t("libraryGraph.trailLabel")}
          className="flex flex-wrap items-center gap-1.5"
          data-testid="library-graph-trail"
        >
          {crumbs.map((crumb, index) => (
            <React.Fragment key={crumb.anchor}>
              {index > 0 ? (
                <span aria-hidden="true" className="text-[var(--scry-muted2)]">
                  ›
                </span>
              ) : null}
              <button
                type="button"
                onClick={() => collapseToCrumb(index)}
                aria-current={index === crumbs.length - 1 ? "step" : undefined}
                className="max-w-[12rem] truncate rounded-full border border-[var(--scry-border2)] bg-[var(--scry-inset)] px-2.5 py-0.5 text-[12px] text-[var(--scry-body)] transition hover:bg-[var(--scry-hover)] aria-[current=step]:border-[var(--scry-accent)]"
              >
                {crumb.label}
              </button>
            </React.Fragment>
          ))}
          {recommendationBands.length > 0 ? (
            <ul
              aria-label={t("libraryGraph.recommendationGroupsLabel")}
              className="flex flex-wrap items-center gap-1.5"
            >
              {recommendationBands.map((band) => (
                <li
                  key={band.key}
                  className="flex items-center gap-1.5 rounded-full border border-[var(--scry-border2)] px-2.5 py-0.5 text-[12px] text-[var(--scry-body)]"
                  style={{ backgroundColor: withAlpha(band.color, 0.14) }}
                >
                  <span
                    aria-hidden="true"
                    className="h-2 w-2 rounded-full"
                    style={{ backgroundColor: band.color }}
                  />
                  <span>{band.name}</span>
                  <span className="text-[var(--scry-muted2)]">
                    {band.keys.length}
                  </span>
                </li>
              ))}
            </ul>
          ) : null}
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
          className="absolute inset-0 touch-none outline-none focus-visible:ring-2 focus-visible:ring-[var(--scry-accent)]"
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
        {groups
          .filter((group) => group.id !== UNPLACED_CLUSTER_ID)
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
              className="absolute left-0 top-0 max-w-[14rem] truncate rounded-full border bg-[var(--scry-card2)]/90 px-2.5 py-0.5 text-[11px] font-semibold text-[var(--scry-ink2)] shadow-sm backdrop-blur-sm"
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
        {graphStatus === "loading" ? (
          <div
            role="status"
            className="absolute inset-0 flex items-center justify-center p-6 text-center text-[13px] text-[var(--scry-muted2)]"
          >
            {t("label.loading")}
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
        {selectedNode && selectedNode.collapsedClusterId === undefined ? (
          <div
            role="region"
            aria-label={nodeLabel(selectedNode)}
            data-testid="library-graph-selection"
            data-ghost={selectedNode.ghost ? "true" : "false"}
            className="absolute bottom-3 left-3 z-10 flex w-[min(20rem,calc(100%-1.5rem))] flex-col gap-2 rounded-[12px] border border-[var(--scry-border2)] bg-[var(--scry-card2)] p-3 text-[12px] text-[var(--scry-ink2)] shadow-lg"
          >
            <div className="flex items-start gap-2">
              {selectedItem?.posterUrl ? (
                <img
                  src={
                    selectPosterVariantUrl(selectedItem.posterUrl, "w70") ?? ""
                  }
                  alt=""
                  className="h-14 w-10 shrink-0 rounded-[4px] border border-dashed border-[var(--scry-muted2)] object-cover opacity-80"
                />
              ) : null}
              <div className="min-w-0 flex-1">
                <div className="truncate text-[13px] font-semibold">
                  {nodeLabel(selectedNode)}
                </div>
                {selectedItem ? (
                  <div className="text-[var(--scry-muted2)]">
                    {selectedResolved
                      ? t("libraryGraph.inLibrary")
                      : t("libraryGraph.notInLibrary")}
                  </div>
                ) : null}
              </div>
              <button
                type="button"
                onClick={exitFocus}
                aria-label={t("libraryGraph.backToOverview")}
                title={t("libraryGraph.backToOverview")}
                className="flex h-6 w-6 shrink-0 items-center justify-center rounded-full text-[var(--scry-muted2)] transition hover:bg-[var(--scry-hover)]"
              >
                <X className="h-3.5 w-3.5" />
              </button>
            </div>
            <div className="flex flex-wrap items-center gap-1.5">
              {selectedResolved && openSelected ? (
                <Button
                  type="button"
                  size="sm"
                  onClick={openSelected}
                  className="h-7 rounded-[8px] px-2.5 text-[12px]"
                >
                  {t("libraryGraph.openTitle")}
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
                  className="h-7 rounded-[8px] px-2.5 text-[12px]"
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
          </div>
        ) : null}
      </div>
      {moreLikeThisActions.dialogs}
    </section>
  );
}
