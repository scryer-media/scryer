import type { TitleRecord } from "@/lib/types";

/**
 * Fold a refreshed title record into the one the UI is already showing.
 *
 * Catalog list projections are much narrower than the side-panel selection,
 * and they narrow further still with the visible table columns and view mode.
 * GraphQL answers an unselected field with `undefined` and a cleared one with
 * `null`, so an absent field always means "not requested" and never "deleted":
 * carrying it over from `current` is what keeps a reactive list refresh from
 * blanking the selected title's panel -- most visibly the Watch-in control,
 * which unmounts the moment `playbackLinks` goes undefined.
 *
 * Artwork needs the stronger rule (a null poster is just as unusable as a
 * missing one), and `metadataFetchedAt` keeps the newest stamp either side has.
 */
export function mergePreferLoadedImageFields(
  current: TitleRecord,
  incoming: TitleRecord,
): TitleRecord {
  const incomingHasPoster = Boolean(
    incoming.posterUrl || incoming.posterSourceUrl,
  );
  const incomingHasBackground = Boolean(
    incoming.backgroundUrl || incoming.backgroundSourceUrl,
  );

  const merged = { ...current } as Record<string, unknown>;
  for (const [field, value] of Object.entries(incoming)) {
    if (value !== undefined) {
      merged[field] = value;
    }
  }

  return {
    ...(merged as TitleRecord),
    posterUrl: incomingHasPoster
      ? incoming.posterUrl
      : (current.posterUrl ?? null),
    posterSourceUrl: incomingHasPoster
      ? incoming.posterSourceUrl
      : (current.posterSourceUrl ?? null),
    backgroundUrl: incomingHasBackground
      ? incoming.backgroundUrl
      : (current.backgroundUrl ?? null),
    backgroundSourceUrl: incomingHasBackground
      ? incoming.backgroundSourceUrl
      : (current.backgroundSourceUrl ?? null),
    metadataFetchedAt: incoming.metadataFetchedAt ?? current.metadataFetchedAt,
  };
}
