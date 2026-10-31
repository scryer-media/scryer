export type TitleExternalRating = {
  source: string;
  value: number | null;
  score: number | null;
  normalized: number;
  votes: number | null;
  url: string;
};

export type TitleExternalRatingInput = {
  source: string;
  value?: number | null;
  score?: number | null;
  normalized: number;
  votes?: number | null;
  url?: string | null;
};

export type RatingSourceInfo = {
  label: string;
  logoSrc: string | null;
  format: "default" | "percent" | "hundred";
};

export function normalizedRatingSource(source: string): string {
  const normalized = source
    .trim()
    .toLowerCase()
    .replace(/[\s_.-]+/g, "");
  return normalized === "myanimelist" || normalized === "myanimelistnet"
    ? "mal"
    : normalized;
}

const POPCORNMETER_LOGO_SRC = "/rating-sources/popcornmeter.svg";
const METACRITIC_LOGO_SRC = "/rating-sources/metacritic.svg";
const HIDDEN_RATING_SOURCE_IDS = new Set([
  "ebert",
  "rogerebert",
  "rogerebertcom",
]);
const RATING_SOURCE_ORDER = new Map<string, number>([
  ["imdb", 10],
  ["rottentomatoes", 20],
  ["tomatoes", 20],
  ["audience", 21],
  ["popcorn", 21],
  ["popcornmeter", 21],
  ["metacritic", 30],
  ["mcuser", 31],
  ["metacriticuser", 31],
  ["letterboxd", 40],
  ["tmdb", 50],
  ["tvdb", 60],
  ["thetvdb", 60],
  ["trakt", 70],
  ["mal", 80],
  ["myanimelist", 80],
  ["myanimelistnet", 80],
  ["anilist", 81],
  ["anidb", 82],
  ["mdblist", 90],
]);

/**
 * Pick the highest-priority attributed rating source from a list, respecting
 * RATING_SOURCE_ORDER (lower rank wins, earliest entry breaks ties). Returns null
 * when the list carries no usable attribution, so callers can render a neutral
 * score badge instead of inventing a provider such as MDBList.
 */
export function topOrderedRatingSource(
  sources: readonly string[],
): string | null {
  let bestSource: string | null = null;
  let bestOrder = Number.POSITIVE_INFINITY;
  for (const source of sources) {
    const trimmed = source.trim();
    if (trimmed.length === 0) {
      continue;
    }
    const order =
      RATING_SOURCE_ORDER.get(normalizedRatingSource(trimmed)) ?? 1_000;
    if (order < bestOrder) {
      bestSource = trimmed;
      bestOrder = order;
    }
  }
  return bestSource;
}

function fallbackSourceLabel(source: string): string {
  return source
    .trim()
    .split(/[\s_-]+/)
    .filter(Boolean)
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join(" ");
}

export function ratingSourceInfo(source: string): RatingSourceInfo {
  const normalized = normalizedRatingSource(source);
  switch (normalized) {
    case "imdb":
      return {
        label: "IMDb",
        logoSrc: "/rating-sources/imdb.svg",
        format: "default",
      };
    case "rottentomatoes":
    case "tomatoes":
      return {
        label: "Rotten Tomatoes",
        logoSrc: "/rating-sources/rotten-tomatoes.svg",
        format: "percent",
      };
    case "audience":
      return {
        label: "RT Audience",
        logoSrc: POPCORNMETER_LOGO_SRC,
        format: "percent",
      };
    case "popcorn":
    case "popcornmeter":
      return {
        label: "Popcornmeter",
        logoSrc: POPCORNMETER_LOGO_SRC,
        format: "percent",
      };
    case "metacritic":
      return {
        label: "Metacritic",
        logoSrc: METACRITIC_LOGO_SRC,
        format: "hundred",
      };
    case "mcuser":
    case "metacriticuser":
      return {
        label: "Metacritic User",
        logoSrc: METACRITIC_LOGO_SRC,
        format: "hundred",
      };
    case "tmdb":
      return {
        label: "TMDb",
        logoSrc: "/rating-sources/tmdb.svg",
        format: "default",
      };
    case "tvdb":
    case "thetvdb":
      return {
        label: "TVDB",
        logoSrc: "/media-sites/tvdb.svg",
        format: "default",
      };
    case "trakt":
      return {
        label: "Trakt",
        logoSrc: "/rating-sources/trakt.svg",
        format: "default",
      };
    case "letterboxd":
      return {
        label: "Letterboxd",
        logoSrc: "/rating-sources/letterboxd.svg",
        format: "default",
      };
    case "mdblist":
      return {
        label: "MDBList",
        logoSrc: "/rating-sources/mdblist.avif",
        format: "default",
      };
    case "mal":
    case "myanimelist":
    case "myanimelistnet":
      return {
        label: "MyAnimeList",
        logoSrc: "/media-sites/mal.svg",
        format: "default",
      };
    case "anilist":
      return {
        label: "AniList",
        logoSrc: "/media-sites/anilist.svg",
        format: "default",
      };
    case "anidb":
      return {
        label: "AniDB",
        logoSrc: "/media-sites/anidb.png",
        format: "default",
      };
    default:
      return {
        label: fallbackSourceLabel(source),
        logoSrc: null,
        format: "default",
      };
  }
}

export function compactRatingNumber(value: number): string {
  if (Number.isInteger(value)) {
    return value.toString();
  }
  return value.toFixed(1).replace(/\.0$/, "");
}

export function visibleTitleExternalRatings(
  ratings: TitleExternalRating[],
): TitleExternalRating[] {
  const visibleSources = new Set<string>();

  return ratings
    .map((rating, index) => {
      const normalized = normalizedRatingSource(rating.source);
      return {
        index,
        normalized,
        rating,
        source: ratingSourceInfo(rating.source),
      };
    })
    .filter(({ normalized }) => !HIDDEN_RATING_SOURCE_IDS.has(normalized))
    .sort((a, b) => {
      const orderDelta =
        (RATING_SOURCE_ORDER.get(a.normalized) ?? 1_000) -
        (RATING_SOURCE_ORDER.get(b.normalized) ?? 1_000);
      if (orderDelta !== 0) {
        return orderDelta;
      }
      const labelDelta = a.source.label.localeCompare(
        b.source.label,
        undefined,
        {
          sensitivity: "base",
        },
      );
      return labelDelta || a.index - b.index;
    })
    .filter(({ normalized }) => {
      if (visibleSources.has(normalized)) {
        return false;
      }
      visibleSources.add(normalized);
      return true;
    })
    .map(({ rating }) => rating);
}

export function normalizeTitleExternalRating(
  rating: TitleExternalRatingInput,
): TitleExternalRating {
  return {
    source: rating.source,
    value: rating.value ?? null,
    score: rating.score ?? null,
    normalized: rating.normalized,
    votes: rating.votes ?? null,
    url: rating.url ?? "",
  };
}

function scoreOutOfHundred(rating: TitleExternalRating): number {
  const score = rating.score ?? rating.value;
  if (score != null) {
    if (score <= 1) {
      return score * 100;
    }
    if (score <= 10) {
      return score * 10;
    }
    return score;
  }
  return rating.normalized * 10;
}

export function ratingValueLabel(
  rating: TitleExternalRating,
  source = ratingSourceInfo(rating.source),
): string {
  if (source.format === "percent") {
    return `${Math.round(scoreOutOfHundred(rating))}%`;
  }
  if (source.format === "hundred") {
    return compactRatingNumber(scoreOutOfHundred(rating));
  }
  const value = rating.value ?? rating.normalized;
  return compactRatingNumber(value);
}

export function externalRatingLabelForAliases(
  ratings: TitleExternalRatingInput[],
  aliases: readonly string[],
): string | null {
  const normalizedAliases = new Set(aliases.map(normalizedRatingSource));
  const match = visibleTitleExternalRatings(
    ratings.map(normalizeTitleExternalRating),
  ).find((rating) =>
    normalizedAliases.has(normalizedRatingSource(rating.source)),
  );

  return match ? ratingValueLabel(match, ratingSourceInfo(match.source)) : null;
}
