// Metadata types returned by the backend metadata proxy resolvers.
// Field names are camelCase to match async_graphql output.

export type MetadataTvdbSearchItem = {
  tvdbId: string;
  name: string;
  imdbId: string | null;
  slug: string | null;
  type: string | null;
  year: number | null;
  status: string | null;
  overview: string | null;
  popularity: number | null;
  posterUrl: string | null;
  language: string | null;
  runtimeMinutes: number | null;
  sortTitle: string | null;
};

export type MetadataMoviePayload = {
  tvdbId: string;
  name: string;
  slug: string;
  year: number | null;
  status: string;
  overview: string;
  posterUrl: string;
  language: string;
  runtimeMinutes: number;
  sortTitle: string;
  imdbId: string;
  genres: string[];
  studio: string;
  tmdbReleaseDate: string | null;
};

export type MetadataSeriesPayload = {
  tvdbId: string;
  name: string;
  sortName: string;
  slug: string;
  year: number | null;
  status: string;
  firstAired: string;
  overview: string;
  network: string;
  runtimeMinutes: number;
  posterUrl: string;
  country: string;
  genres: string[];
  aliases: string[];
  seasons: MetadataSeason[];
  episodes: MetadataEpisode[];
};

export type MetadataSeason = {
  tvdbId: string;
  number: number;
  label: string;
  episodeType: string;
};

export type MetadataEpisode = {
  tvdbId: string;
  episodeNumber: number;
  seasonNumber: number;
  name: string;
  aired: string;
  runtimeMinutes: number;
  isFiller: boolean;
};
