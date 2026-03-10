import type { SettingsSection, ViewId, ContentSettingsSection } from "@/components/root/types";
import type { ViewCategoryId } from "@/lib/types/quality-profiles";
import type { Facet } from "@/lib/types/titles";
import { FACET_REGISTRY, MEDIA_VIEW_IDS, SCOPE_IDS } from "@/lib/facets/registry";

// --- Non-facet constants (unchanged) ---

export const TLS_CERT_PATH_KEY = "tls.cert_path";
export const TLS_KEY_PATH_KEY = "tls.key_path";
export const QUALITY_PROFILE_ID_KEY = "quality.profile_id";
export const QUALITY_PROFILE_CATALOG_KEY = "quality.profiles";
export const RENAME_TEMPLATE_KEY = "rename.template";
export const RENAME_COLLISION_POLICY_KEY = "rename.collision_policy";
export const RENAME_MISSING_METADATA_POLICY_KEY = "rename.missing_metadata_policy";
export const RENAME_COLLISION_POLICY_GLOBAL_KEY = "rename.collision_policy.global";
export const RENAME_MISSING_METADATA_POLICY_GLOBAL_KEY = "rename.missing_metadata_policy.global";
export const QUALITY_PROFILE_INHERIT_VALUE = "__inherit__";
export const ANIME_FILLER_POLICY_KEY = "anime.filler_policy";
export const ANIME_RECAP_POLICY_KEY = "anime.recap_policy";
export const ANIME_MONITOR_SPECIALS_KEY = "anime.monitor_specials";
export const ANIME_INTER_SEASON_MOVIES_KEY = "anime.inter_season_movies";
export const ANIME_PREFERRED_SUB_GROUP_KEY = "anime.preferred_sub_group";

// NFO sidecar writing on import (per facet)
export const NFO_WRITE_ON_IMPORT_MOVIE_KEY = "nfo.write_on_import.movie";
export const NFO_WRITE_ON_IMPORT_SERIES_KEY = "nfo.write_on_import.series";
export const NFO_WRITE_ON_IMPORT_ANIME_KEY = "nfo.write_on_import.anime";

// Plexmatch hint writing on import (series/anime only)
export const PLEXMATCH_WRITE_ON_IMPORT_SERIES_KEY = "plexmatch.write_on_import.series";
export const PLEXMATCH_WRITE_ON_IMPORT_ANIME_KEY = "plexmatch.write_on_import.anime";

// --- Derived from registry ---

export const MOVIE_FOLDER_KEY = FACET_REGISTRY.find((f) => f.id === "movie")!.folderSettingKey;
export const DEFAULT_MOVIE_LIBRARY_PATH = FACET_REGISTRY.find((f) => f.id === "movie")!.defaultLibraryPath;
export const SERIES_FOLDER_KEY = FACET_REGISTRY.find((f) => f.id === "tv")!.folderSettingKey;
export const DEFAULT_SERIES_LIBRARY_PATH = FACET_REGISTRY.find((f) => f.id === "tv")!.defaultLibraryPath;

export const RENAME_TEMPLATE_MOVIE_GLOBAL_KEY = FACET_REGISTRY.find((f) => f.id === "movie")!.renameTemplateKey;
export const RENAME_TEMPLATE_SERIES_GLOBAL_KEY = FACET_REGISTRY.find((f) => f.id === "tv")!.renameTemplateKey;
export const RENAME_TEMPLATE_ANIME_GLOBAL_KEY = FACET_REGISTRY.find((f) => f.id === "anime")!.renameTemplateKey;

export const QUALITY_PROFILE_SCOPE_ID_MOVIES = "movie" as const;
export const QUALITY_PROFILE_SCOPE_ID_SERIES = "series" as const;
export const QUALITY_PROFILE_SCOPE_ID_ANIME = "anime" as const;
export const QUALITY_PROFILE_SCOPE_IDS = SCOPE_IDS as readonly ViewCategoryId[];

export const RENAME_TEMPLATE_GLOBAL_KEYS: Record<ViewCategoryId, string> = Object.fromEntries(
  FACET_REGISTRY.map((f) => [f.scopeId, f.renameTemplateKey]),
) as Record<ViewCategoryId, string>;

// --- URL constants ---

export const URL_SECTION_SETTINGS = "settings";
export const URL_SECTION_MOVIES = "movies";
export const URL_SECTION_SERIES = "series";
export const URL_SECTION_ANIME = "anime";
export const URL_SECTION_ACTIVITY = "activity";
export const URL_SECTION_WANTED = "wanted";
export const URL_SECTION_HISTORY = "history";
export const URL_SECTION_SYSTEM = "system";
export const URL_PARAM_LANGUAGE = "lang";
export const URL_PARAM_VIEW_DEPRECATED = "view";
export const URL_PARAM_SETTINGS_SECTION_DEPRECATED = "settingsSection";
export const URL_PARAM_CONTENT_SECTION_DEPRECATED = "contentSection";

export const URL_PATH_SEGMENTS: ViewId[] = [
  ...MEDIA_VIEW_IDS as string[] as ViewId[],
  URL_SECTION_ACTIVITY,
  URL_SECTION_WANTED,
  URL_SECTION_HISTORY,
  URL_SECTION_SETTINGS,
  URL_SECTION_SYSTEM,
];

export const SETTINGS_SECTION_PATH_TO_ID: Record<string, SettingsSection> = {
  profile: "profile",
  general: "general",
  users: "users",
  indexers: "indexers",
  "download-clients": "downloadClients",
  downloadClients: "downloadClients",
  "quality-profiles": "qualityProfiles",
  qualityProfiles: "qualityProfiles",
  "delay-profiles": "delayProfiles",
  delayProfiles: "delayProfiles",
  acquisition: "acquisition",
  rules: "rules",
  plugins: "plugins",
  notifications: "notifications",
  "post-processing": "post-processing",
};

export const CONTENT_SECTION_PATH_TO_ID: Record<string, ContentSettingsSection> = {
  overview: "overview",
  settings: "settings",
  media: "settings",
};

export const CONTENT_SETTINGS_SUB_PAGE_PATH_TO_ID: Record<string, ContentSettingsSection> = {
  general: "general",
  quality: "quality",
  renaming: "renaming",
  routing: "routing",
};

export const viewToFacet: Record<string, Facet> = Object.fromEntries(
  FACET_REGISTRY.map((f) => [f.viewId, f.id]),
);

export const CATEGORY_SCOPE_MAP: Record<string, ViewCategoryId> = Object.fromEntries(
  FACET_REGISTRY.map((f) => [f.viewId, f.scopeId]),
);
