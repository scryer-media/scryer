import type { ViewCategoryId } from "@/lib/types/quality-profiles";
import type { Facet } from "@/lib/types/titles";
import { FACET_REGISTRY, SCOPE_IDS } from "@/lib/facets/registry";

// --- Non-facet constants (unchanged) ---

export const TLS_CERT_PATH_KEY = "tls.cert_path";
export const TLS_KEY_PATH_KEY = "tls.key_path";
export const QUALITY_PROFILE_ID_KEY = "quality.profile_id";
export const REQUEST_QUALITY_PROFILE_IDS_KEY = "quality.request_profile_ids";
export const QUALITY_PROFILE_CATALOG_KEY = "quality.profiles";
export const SCORING_PERSONA_KEY = "quality.scoring_persona";
export const RENAME_ENABLED_KEY = "rename.enabled";
export const RENAME_TEMPLATE_KEY = "rename.template";
export const RENAME_COLLISION_POLICY_KEY = "rename.collision_policy";
export const RENAME_MISSING_METADATA_POLICY_KEY =
  "rename.missing_metadata_policy";
export const RENAME_COLLISION_POLICY_GLOBAL_KEY =
  "rename.collision_policy.global";
export const RENAME_MISSING_METADATA_POLICY_GLOBAL_KEY =
  "rename.missing_metadata_policy.global";
export const QUALITY_PROFILE_INHERIT_VALUE = "__inherit__";
export const ANIME_FILLER_POLICY_KEY = "anime.filler_policy";
export const ANIME_RECAP_POLICY_KEY = "anime.recap_policy";
export const ANIME_MONITOR_SPECIALS_KEY = "anime.monitor_specials";
export const ANIME_INTER_SEASON_MOVIES_KEY = "anime.inter_season_movies";
export const ANIME_MONITOR_FILLER_MOVIES_KEY = "anime.monitor_filler_movies";

// NFO sidecar writing on import (per facet)
export const NFO_WRITE_ON_IMPORT_MOVIE_KEY = "nfo.write_on_import.movie";
export const NFO_WRITE_ON_IMPORT_SERIES_KEY = "nfo.write_on_import.series";
export const NFO_WRITE_ON_IMPORT_ANIME_KEY = "nfo.write_on_import.anime";

// Plexmatch hint writing on import (series/anime only)
export const PLEXMATCH_WRITE_ON_IMPORT_SERIES_KEY =
  "plexmatch.write_on_import.series";
export const PLEXMATCH_WRITE_ON_IMPORT_ANIME_KEY =
  "plexmatch.write_on_import.anime";
export const IMPORT_MODE_KEY = "import.mode";
export const SET_PERMISSIONS_LINUX_KEY = "permissions.set_linux";
export const FILE_CHMOD_KEY = "permissions.file_chmod";
export const FOLDER_CHMOD_KEY = "permissions.folder_chmod";
export const CHOWN_GROUP_KEY = "permissions.chown_group";

// --- Derived from registry ---

export const MOVIE_FOLDER_KEY = FACET_REGISTRY.find(
  (f) => f.id === "MOVIE",
)!.folderSettingKey;
export const DEFAULT_MOVIE_LIBRARY_PATH = FACET_REGISTRY.find(
  (f) => f.id === "MOVIE",
)!.defaultLibraryPath;
export const SERIES_FOLDER_KEY = FACET_REGISTRY.find(
  (f) => f.id === "SERIES",
)!.folderSettingKey;
export const DEFAULT_SERIES_LIBRARY_PATH = FACET_REGISTRY.find(
  (f) => f.id === "SERIES",
)!.defaultLibraryPath;

export const RENAME_TEMPLATE_MOVIE_GLOBAL_KEY = FACET_REGISTRY.find(
  (f) => f.id === "MOVIE",
)!.renameTemplateKey;
export const RENAME_TEMPLATE_SERIES_GLOBAL_KEY = FACET_REGISTRY.find(
  (f) => f.id === "SERIES",
)!.renameTemplateKey;
export const RENAME_TEMPLATE_ANIME_GLOBAL_KEY = FACET_REGISTRY.find(
  (f) => f.id === "ANIME",
)!.renameTemplateKey;

export const QUALITY_PROFILE_SCOPE_ID_MOVIES = "movie" as const;
export const QUALITY_PROFILE_SCOPE_ID_SERIES = "series" as const;
export const QUALITY_PROFILE_SCOPE_ID_ANIME = "anime" as const;
export const QUALITY_PROFILE_SCOPE_IDS = SCOPE_IDS as readonly ViewCategoryId[];

export const RENAME_TEMPLATE_GLOBAL_KEYS: Record<ViewCategoryId, string> =
  Object.fromEntries(
    FACET_REGISTRY.map((f) => [f.scopeId, f.renameTemplateKey]),
  ) as Record<ViewCategoryId, string>;

export const URL_PARAM_LANGUAGE = "lang";
export const URL_PARAM_VIEW_DEPRECATED = "view";
export const URL_PARAM_SETTINGS_SECTION_DEPRECATED = "settingsSection";
export const URL_PARAM_CONTENT_SECTION_DEPRECATED = "contentSection";

export const viewToFacet: Record<string, Facet> = Object.fromEntries(
  FACET_REGISTRY.map((f) => [f.viewId, f.id]),
);

export const CATEGORY_SCOPE_MAP: Record<string, ViewCategoryId> =
  Object.fromEntries(FACET_REGISTRY.map((f) => [f.viewId, f.scopeId]));
