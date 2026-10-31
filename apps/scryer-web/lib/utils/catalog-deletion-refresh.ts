import {
  allOf,
  forEventTypes,
  forFacet,
  type ReactiveRefreshRegistration,
} from "../reactive/domain-event-feed.ts";

export function catalogDeletionRefresh(
  facet: string,
  run: () => void,
): ReactiveRefreshRegistration {
  return {
    aliasKey: `catalog-deletion:${facet}`,
    predicate: allOf(forEventTypes("title_deleted"), forFacet(facet)),
    run,
  };
}
