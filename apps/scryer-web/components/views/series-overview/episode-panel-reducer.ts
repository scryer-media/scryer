import type { Release } from "@/lib/types";

export type EpisodePanelTab = "details" | "search" | "blocklist";

export interface EpisodePanelState {
  expandedEpisodeRows: Set<string>;
  episodeActiveTab: Record<string, EpisodePanelTab>;
  searchResultsByEpisode: Record<string, Release[]>;
  searchLoadingByEpisode: Record<string, boolean>;
  autoSearchLoadingByEpisode: Record<string, boolean>;
}

export type EpisodePanelAction =
  | { type: "TOGGLE_EPISODE_ROW"; episodeId: string }
  | { type: "SET_EPISODE_TAB"; episodeId: string; tab: EpisodePanelTab }
  | { type: "SET_SEARCH_RESULTS"; episodeId: string; results: Release[] }
  | { type: "SET_SEARCH_LOADING"; episodeId: string; loading: boolean }
  | { type: "SET_AUTO_SEARCH_LOADING"; episodeId: string; loading: boolean };

export const initialEpisodePanelState: EpisodePanelState = {
  expandedEpisodeRows: new Set(),
  episodeActiveTab: {},
  searchResultsByEpisode: {},
  searchLoadingByEpisode: {},
  autoSearchLoadingByEpisode: {},
};

export function episodePanelReducer(
  state: EpisodePanelState,
  action: EpisodePanelAction,
): EpisodePanelState {
  switch (action.type) {
    case "TOGGLE_EPISODE_ROW": {
      const next = new Set(state.expandedEpisodeRows);
      if (next.has(action.episodeId)) {
        next.delete(action.episodeId);
      } else {
        next.add(action.episodeId);
      }
      return { ...state, expandedEpisodeRows: next };
    }

    case "SET_EPISODE_TAB":
      return {
        ...state,
        episodeActiveTab: {
          ...state.episodeActiveTab,
          [action.episodeId]: action.tab,
        },
      };

    case "SET_SEARCH_RESULTS":
      return {
        ...state,
        searchResultsByEpisode: {
          ...state.searchResultsByEpisode,
          [action.episodeId]: action.results,
        },
      };

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
      const { [action.episodeId]: _removed, ...rest } = state.searchLoadingByEpisode;
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
      const { [action.episodeId]: _removed, ...rest } = state.autoSearchLoadingByEpisode;
      return { ...state, autoSearchLoadingByEpisode: rest };
    }

    default:
      return state;
  }
}
