import type { InteractiveSearchIndexerProgress } from "@/lib/graphql/release-search";
import type { Release } from "@/lib/types";

export type EpisodePanelTab = "details" | "blocklist";

export interface EpisodePanelState {
  searchResultsByEpisode: Record<string, Release[]>;
  searchIndexerProgressByEpisode: Record<
    string,
    InteractiveSearchIndexerProgress[]
  >;
  searchLoadingByEpisode: Record<string, boolean>;
  autoSearchLoadingByEpisode: Record<string, boolean>;
}

export type EpisodePanelAction =
  | {
      type: "SET_SEARCH_SNAPSHOT";
      episodeId: string;
      results: Release[];
      indexers: InteractiveSearchIndexerProgress[];
    }
  | { type: "RESET_SEARCH"; episodeId: string }
  | { type: "SET_SEARCH_LOADING"; episodeId: string; loading: boolean }
  | { type: "SET_AUTO_SEARCH_LOADING"; episodeId: string; loading: boolean };

export const initialEpisodePanelState: EpisodePanelState = {
  searchResultsByEpisode: {},
  searchIndexerProgressByEpisode: {},
  searchLoadingByEpisode: {},
  autoSearchLoadingByEpisode: {},
};

export function episodePanelReducer(
  state: EpisodePanelState,
  action: EpisodePanelAction,
): EpisodePanelState {
  switch (action.type) {
    case "SET_SEARCH_SNAPSHOT":
      return {
        ...state,
        searchResultsByEpisode: {
          ...state.searchResultsByEpisode,
          [action.episodeId]: action.results,
        },
        searchIndexerProgressByEpisode: {
          ...state.searchIndexerProgressByEpisode,
          [action.episodeId]: action.indexers,
        },
      };

    case "RESET_SEARCH": {
      const { [action.episodeId]: _results, ...searchResultsByEpisode } =
        state.searchResultsByEpisode;
      const {
        [action.episodeId]: _progress,
        ...searchIndexerProgressByEpisode
      } = state.searchIndexerProgressByEpisode;
      return {
        ...state,
        searchResultsByEpisode,
        searchIndexerProgressByEpisode,
      };
    }

    case "SET_SEARCH_LOADING": {
      if (action.loading) {
        return {
          ...state,
          searchLoadingByEpisode: {
            ...state.searchLoadingByEpisode,
            [action.episodeId]: true,
          },
        };
      }
      const { [action.episodeId]: _removed, ...rest } =
        state.searchLoadingByEpisode;
      return { ...state, searchLoadingByEpisode: rest };
    }

    case "SET_AUTO_SEARCH_LOADING": {
      if (action.loading) {
        return {
          ...state,
          autoSearchLoadingByEpisode: {
            ...state.autoSearchLoadingByEpisode,
            [action.episodeId]: true,
          },
        };
      }
      const { [action.episodeId]: _removed, ...rest } =
        state.autoSearchLoadingByEpisode;
      return { ...state, autoSearchLoadingByEpisode: rest };
    }

    default:
      return state;
  }
}
