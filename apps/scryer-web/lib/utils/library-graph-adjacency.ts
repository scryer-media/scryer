// The links between drawn nodes, kept off the renderer. The view asks it only
// for adjacency and each link's attributes, so plain maps serve, and rebuilding
// it on every neighbourhood fold costs far less than a full graph store.

type Relation<A> = { from: string; to: string; attributes: A };

const NO_EDGES: readonly string[] = [];

export class GraphAdjacency<A> {
  private readonly relations: Relation<A>[] = [];
  // Each node's neighbours, to the index of the link joining them. Looking a
  // pair up through these never builds a pair key.
  private readonly neighborLinks = new Map<string, Map<string, number>>();
  private readonly edgeKeys = new Map<string, string[]>();

  constructor(nodes: Iterable<string> = []) {
    for (const node of nodes) {
      this.neighborLinks.set(node, new Map());
      this.edgeKeys.set(node, []);
    }
  }

  /** Links two nodes once; a repeat pair, a self-link or an unknown node is
   * ignored. Returns whether the link was added. */
  add(from: string, to: string, attributes: A): boolean {
    if (from === to) return false;
    const fromLinks = this.neighborLinks.get(from);
    const toLinks = this.neighborLinks.get(to);
    if (!fromLinks || !toLinks || fromLinks.has(to)) return false;
    const index = this.relations.length;
    this.relations.push({ from, to, attributes });
    fromLinks.set(to, index);
    toLinks.set(from, index);
    const key = String(index);
    this.edgeKeys.get(from)!.push(key);
    this.edgeKeys.get(to)!.push(key);
    return true;
  }

  hasNode(node: string): boolean {
    return this.neighborLinks.has(node);
  }

  /** Whether a link joins the two nodes, in either direction. */
  hasEdgeBetween(a: string, b: string): boolean {
    return this.neighborLinks.get(a)?.has(b) ?? false;
  }

  hasEdge(key: string): boolean {
    return this.relation(key) !== undefined;
  }

  /** The keys of the node's links. */
  edges(node: string): readonly string[] {
    return this.edgeKeys.get(node) ?? NO_EDGES;
  }

  /** The nodes linked to this one. */
  neighbors(node: string): string[] {
    return Array.from(this.neighborLinks.get(node)?.keys() ?? []);
  }

  extremities(key: string): [string, string] {
    const relation = this.relation(key);
    if (!relation) throw new Error(`unknown graph link ${key}`);
    return [relation.from, relation.to];
  }

  getEdgeAttributes(key: string): A {
    const relation = this.relation(key);
    if (!relation) throw new Error(`unknown graph link ${key}`);
    return relation.attributes;
  }

  private relation(key: string): Relation<A> | undefined {
    const index = Number(key);
    return Number.isInteger(index) && String(index) === key
      ? this.relations[index]
      : undefined;
  }
}
