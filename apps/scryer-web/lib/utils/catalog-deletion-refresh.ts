import {
  allOf,
  forEventTypes,
  forFacet,
  type ReactiveRefreshRegistration,
} from "../reactive/domain-event-feed.ts";

export type CatalogDeletionHandlers = {
  /** Committed deletions of these titles, so the list drops just those rows. */
  removed: (titleIds: string[]) => void;
  /**
   * The feed could not say what was deleted (degraded interval or a reconnect
   * without a cursor). The list keeps its rows rather than reloading in full.
   */
  unknown: () => void;
};

export function catalogDeletionRefresh(
  facet: string,
  handlers: CatalogDeletionHandlers,
): ReactiveRefreshRegistration {
  return {
    aliasKey: `catalog-deletion:${facet}`,
    predicate: allOf(forEventTypes("title_deleted"), forFacet(facet)),
    run: (events) => {
      if (events === null) {
        handlers.unknown();
        return;
      }
      const titleIds = [
        ...new Set(
          events
            .map((event) => event.titleId)
            .filter((titleId): titleId is string => Boolean(titleId)),
        ),
      ];
      if (titleIds.length > 0) {
        handlers.removed(titleIds);
      }
    },
  };
}
