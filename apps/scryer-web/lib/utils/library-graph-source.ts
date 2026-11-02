// Where the graph view gets its library graph. The view depends only on
// LibraryGraphSource; today the fixture source is the only implementation.

import type { Facet } from "../types/titles.ts";
import type { LibraryGraphResult } from "../types/library-graph.ts";
import { libraryGraphFixture } from "./library-graph-fixture.ts";

export type LibraryGraphRequest = {
  facet: Facet;
  /** The libraries in view; empty means every library the viewer can read. */
  libraryIds: readonly string[];
};

export interface LibraryGraphSource {
  load(request: LibraryGraphRequest): Promise<LibraryGraphResult>;
}

/**
 * A source that answers every request with the synthetic fixture. Target keys
 * passed in name the first fixture nodes, so the fixture can be drawn over a
 * real catalog.
 */
export function createFixtureLibraryGraphSource(
  targetKeys?: readonly string[],
): LibraryGraphSource {
  return {
    load: (request) =>
      Promise.resolve(
        libraryGraphFixture({ facet: request.facet, targetKeys }),
      ),
  };
}

// The server query that will feed the graph view is not available yet. When
// it is, it gets its own LibraryGraphSource implementation here, mapping its
// payload onto LibraryGraphResult, and this factory returns it in place of the
// fixture. Nothing else in the view changes.
export function libraryGraphSourceForCatalog(
  catalogTargetKeys: readonly string[],
): LibraryGraphSource {
  return createFixtureLibraryGraphSource(catalogTargetKeys);
}
