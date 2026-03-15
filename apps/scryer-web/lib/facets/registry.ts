import { Clapperboard, Film, Tv, type LucideIcon } from "lucide-react";
import type { Facet } from "@/lib/types/titles";
import type { ViewCategoryId } from "@/lib/types/quality-profiles";
import type { MetadataCatalogMonitorType } from "@/lib/hooks/use-global-search";

export type FacetDefinition = {
  /** Domain enum value: "movie" | "tv" | "anime" */
  id: Facet;
  /** URL path segment: "movies" | "series" | "anime" */
  viewId: string;
  /** Settings/quality-profile scope: "movie" | "series" | "anime" */
  scopeId: ViewCategoryId;
  /** Key in MetadataSearchResults from SMG: "movie" | "series" | "anime" */
  metadataKey: string;

  icon: LucideIcon;
  navLabelKey: string;
  searchLabelKey: string;
  settingsLabelKey: string;
  overviewLabelKey: string;

  folderSettingKey: string;
  rootFoldersKey: string;
  defaultLibraryPath: string;
  renameTemplateKey: string;

  hasEpisodes: boolean;
  defaultMonitorType: MetadataCatalogMonitorType;

  /** TVDB search type hint sent to SMG */
  tvdbSearchType: string;
};

export const FACET_REGISTRY: FacetDefinition[] = [
  {
    id: "movie",
    viewId: "movies",
    scopeId: "movie",
    metadataKey: "movie",
    icon: Film,
    navLabelKey: "nav.movies",
    searchLabelKey: "search.facetMovie",
    settingsLabelKey: "settings.moviesSettings",
    overviewLabelKey: "title.manageMovies",
    folderSettingKey: "movies.path",
    rootFoldersKey: "movies.root_folders",
    defaultLibraryPath: "/media/movies",
    renameTemplateKey: "rename.template.movie.global",
    hasEpisodes: false,
    defaultMonitorType: "monitored",
    tvdbSearchType: "movie",
  },
  {
    id: "tv",
    viewId: "series",
    scopeId: "series",
    metadataKey: "series",
    icon: Tv,
    navLabelKey: "nav.series",
    searchLabelKey: "search.facetTv",
    settingsLabelKey: "settings.seriesSettings",
    overviewLabelKey: "title.manageSeries",
    folderSettingKey: "series.path",
    rootFoldersKey: "series.root_folders",
    defaultLibraryPath: "/media/series",
    renameTemplateKey: "rename.template.series.global",
    hasEpisodes: true,
    defaultMonitorType: "futureEpisodes",
    tvdbSearchType: "series",
  },
  {
    id: "anime",
    viewId: "anime",
    scopeId: "anime",
    metadataKey: "anime",
    icon: Clapperboard,
    navLabelKey: "nav.anime",
    searchLabelKey: "search.facetAnime",
    settingsLabelKey: "settings.animeSettings",
    overviewLabelKey: "title.manageAnime",
    folderSettingKey: "anime.path",
    rootFoldersKey: "anime.root_folders",
    defaultLibraryPath: "/media/anime",
    renameTemplateKey: "rename.template.anime.global",
    hasEpisodes: true,
    defaultMonitorType: "futureEpisodes",
    tvdbSearchType: "anime",
  },
];

// --- Derived lookup maps ---

export const FACETS_BY_ID = new Map(FACET_REGISTRY.map((f) => [f.id, f]));
export const FACETS_BY_VIEW = new Map(FACET_REGISTRY.map((f) => [f.viewId, f]));
export const FACETS_BY_SCOPE = new Map(FACET_REGISTRY.map((f) => [f.scopeId, f]));
export const FACETS_BY_METADATA_KEY = new Map(FACET_REGISTRY.map((f) => [f.metadataKey, f]));

export const MEDIA_VIEW_IDS = FACET_REGISTRY.map((f) => f.viewId);
export const FACET_IDS = FACET_REGISTRY.map((f) => f.id);
export const SCOPE_IDS = FACET_REGISTRY.map((f) => f.scopeId);

export function isMediaView(viewId: string): boolean {
  return FACETS_BY_VIEW.has(viewId);
}

export function facetForView(viewId: string): FacetDefinition | undefined {
  return FACETS_BY_VIEW.get(viewId);
}

export function facetById(id: string): FacetDefinition | undefined {
  return FACETS_BY_ID.get(id as Facet);
}

export function facetByScope(scopeId: string): FacetDefinition | undefined {
  return FACETS_BY_SCOPE.get(scopeId as ViewCategoryId);
}
