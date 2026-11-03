// A stored snapshot of how the titles in one facet's libraries relate: the
// nodes, the weighted edges between them and the clusters they fall into.
// Field names follow the gateway's result one for one, in the web client's
// camelCase. Layout coordinates never arrive; the client lays the graph out.

/** The cluster id a node carries when it was not placed in any cluster. */
export const UNPLACED_CLUSTER_ID = -1;

export type LibraryGraphNode = {
  subjectKey: string;
  /** The title this node stands for, keyed the way discovery targets are. */
  targetKey: string;
  /** The node's cluster, or {@link UNPLACED_CLUSTER_ID}. */
  clusterId: number;
};

export type LibraryGraphEdge = {
  from: string;
  to: string;
  weight: number;
  /**
   * Why the two titles are linked: `semantic`, `relation:<kind>` (such as
   * `relation:sequel` or `relation:co_watch`) or `affinity:<family>:<label>`
   * (such as `affinity:theme:isekai`).
   */
  kinds: string[];
};

export type LibraryGraphCluster = {
  id: number;
  /** An allowlisted label key, or null when the cluster is named by its medoid. */
  labelKey: string | null;
  label: string;
  /** The subject key of the node that best represents the cluster. */
  medoid: string;
  size: number;
};

export type LibraryGraphResult = {
  generatedAt: string;
  nodes: LibraryGraphNode[];
  edges: LibraryGraphEdge[];
  clusters: LibraryGraphCluster[];
  /** How many titles a built graph was built from; absent on traversal results. */
  subjectCount?: number;
  /** Whether a built graph was cut short of every title; absent on traversal results. */
  truncated?: boolean;
};
