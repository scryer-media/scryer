/** Search kinds a title-less query subject may take. */
export type InteractiveSearchKind = "MOVIE" | "SERIES" | "ANIME" | "RAW";

/**
 * The job accepts exactly one subject: a catalog title (`titleId`, optionally
 * narrowed to a season, or to a season and episode) or a raw operator query
 * (`query` + `kind`). `indexerIds` and `categories` restrict either subject.
 */
export type InteractiveReleaseSearchInput = {
  titleId?: string;
  seriesMovieLinkId?: string;
  season?: string;
  episode?: string;
  query?: string;
  kind?: InteractiveSearchKind;
  indexerIds?: string[];
  categories?: string[];
  limit?: number;
};

/** What part of a catalog title an interactive search covers. */
export type TitleReleaseSearchScope =
  | { kind: "title" }
  /** The whole season: searched as a season pack. */
  | { kind: "season"; season: string }
  | { kind: "episode"; season: string; episode: string };

/**
 * The start input for a title-subject search. A season scope names the season
 * and no episode, which the server reads as "search the whole season".
 */
export function titleReleaseSearchInput(
  titleId: string,
  scope: TitleReleaseSearchScope,
): InteractiveReleaseSearchInput {
  switch (scope.kind) {
    case "title":
      return { titleId };
    case "season":
      return { titleId, season: scope.season };
    case "episode":
      return { titleId, season: scope.season, episode: scope.episode };
  }
}
