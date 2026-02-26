// SMG (Scryer Metadata Gateway) query strings and types
// Extracted from lib/metadataGateway.ts for use with urql SMG client

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export type MetadataTvdbSearchItem = {
  tvdb_id: string;
  name: string;
  imdb_id: string | null;
  slug: string | null;
  type: string | null;
  year: number | null;
  status: string | null;
  overview: string | null;
  popularity: number | null;
  poster_url: string | null;
  language: string | null;
  runtime_minutes: number | null;
  sort_title: string | null;
  normalization_notes: string[] | null;
};

export type MetadataSearchResult = {
  source: string;
  cached: boolean;
  query: string;
  generated_at: string;
  cache_until: string;
  total_results: number;
  results: MetadataTvdbSearchItem[];
};

export type MetadataSeason = {
  tvdb_id: string;
  season_id: string;
  label: string | null;
  number: number | null;
  episode_type: string | null;
};

export type MetadataEpisode = {
  tvdb_id: string;
  episode_id: string;
  episode_number: number;
  season_number: number;
  name: string | null;
  aired: string | null;
  runtime_minutes: number | null;
  is_filler: boolean | null;
};

export type MetadataSeriesPayload = {
  tvdb_id: string;
  name: string;
  sort_name: string | null;
  slug: string | null;
  type: string | null;
  status: string | null;
  year: number | null;
  first_aired: string | null;
  overview: string | null;
  network: string | null;
  runtime_minutes: number | null;
  poster_url: string | null;
  country: string | null;
  genres: string[];
  aliases: string[];
  seasons: MetadataSeason[];
  episodes: MetadataEpisode[];
  normalization_notes: string[] | null;
};

export type MetadataSeriesResult = {
  source: string;
  cached: boolean;
  generated_at: string;
  cache_until: string;
  series: MetadataSeriesPayload;
};

export type MetadataMoviePayload = {
  tvdb_id: number;
  name: string;
  slug: string;
  type: string;
  year: number;
  status: string;
  overview: string;
  poster_url: string;
  language: string;
  runtime_minutes: number;
  sort_title: string;
  imdb_id: string;
  genres: string[];
  studio: string;
  tmdb_id: number | null;
  tmdb_popularity: number | null;
  tmdb_vote_average: number | null;
  tmdb_vote_count: number | null;
};

export type MetadataMovieResult = {
  source: string;
  cached: boolean;
  generated_at: string;
  movie: MetadataMoviePayload;
};

export type MetadataSearchAllResult = {
  movie: MetadataSearchResult;
  series: MetadataSearchResult;
  anime: MetadataSearchResult;
};

// ---------------------------------------------------------------------------
// Query strings
// ---------------------------------------------------------------------------

const SEARCH_RESULT_FIELDS = `
  source
  cached
  query
  generated_at
  cache_until
  total_results
  results {
    tvdb_id
    imdb_id
    name
    slug
    type
    year
    status
    overview
    poster_url
    language
    runtime_minutes
    popularity
    sort_title
    normalization_notes
  }
`;

export const SMG_SEARCH_QUERY = `
  query TvdbSearch($query: String!, $type: String = "series", $page: Int = 1, $limit: Int = 25, $language: String = "eng") {
    searchTvdb(query: $query, type: $type, page: $page, limit: $limit, language: $language) {
      ${SEARCH_RESULT_FIELDS}
    }
  }
`;

export const SMG_SEARCH_ALL_QUERY = `
  query TvdbSearchAll($query: String!, $limit: Int = 25, $language: String = "eng") {
    movieResults: searchTvdb(query: $query, type: "movie", limit: $limit, language: $language) {
      ${SEARCH_RESULT_FIELDS}
    }
    seriesResults: searchTvdb(query: $query, type: "series", limit: $limit, language: $language) {
      ${SEARCH_RESULT_FIELDS}
    }
    animeResults: searchTvdb(query: $query, type: "anime", limit: $limit, language: $language) {
      ${SEARCH_RESULT_FIELDS}
    }
  }
`;

export const SMG_SERIES_QUERY = `
  query TvdbSeries($id: String!, $includeEpisodes: Boolean = true, $language: String = "eng") {
    series(id: $id, includeEpisodes: $includeEpisodes, language: $language) {
      source
      cached
      generated_at
      cache_until
      series {
        tvdb_id
        name
        sort_name
        slug
        type
        status
        year
        first_aired
        overview
        network
        runtime_minutes
        poster_url
        country
        genres
        aliases
        seasons {
          tvdb_id
          season_id
          label
          number
          episode_type
        }
        episodes {
          tvdb_id
          episode_id
          episode_number
          season_number
          name
          aired
          runtime_minutes
          is_filler
        }
        normalization_notes
      }
    }
  }
`;

export const SMG_MOVIE_QUERY = `
  query TvdbMovie($tvdbId: Int!, $language: String = "eng") {
    movie(tvdbId: $tvdbId, language: $language) {
      source
      cached
      generated_at
      movie {
        tvdb_id
        name
        slug
        year
        status
        overview
        poster_url
        language
        runtime_minutes
        sort_title
        imdb_id
        genres
        studio
        tmdb_id
        tmdb_popularity
        tmdb_vote_average
        tmdb_vote_count
      }
    }
  }
`;
