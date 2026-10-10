import * as React from "react";
import { SettingsSaveGuard } from "@/lib/utils/settings-save-guard";
import { useClient } from "urql";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";

import {
  saveQualityProfileSettingsMutation,
  updateMediaSettingsMutation,
} from "@/lib/graphql/mutations";
import { mediaSettingsInitQuery } from "@/lib/graphql/queries";
import {
  DEFAULT_MOVIE_LIBRARY_PATH,
  DEFAULT_SERIES_LIBRARY_PATH,
  CHOWN_GROUP_KEY,
  FILE_CHMOD_KEY,
  FOLDER_CHMOD_KEY,
  IMPORT_MODE_KEY,
  NFO_WRITE_ON_IMPORT_ANIME_KEY,
  NFO_WRITE_ON_IMPORT_MOVIE_KEY,
  NFO_WRITE_ON_IMPORT_SERIES_KEY,
  PLEXMATCH_WRITE_ON_IMPORT_ANIME_KEY,
  PLEXMATCH_WRITE_ON_IMPORT_SERIES_KEY,
  QUALITY_PROFILE_CATALOG_KEY,
  QUALITY_PROFILE_ID_KEY,
  QUALITY_PROFILE_INHERIT_VALUE,
  RENAME_ENABLED_KEY,
  SCORING_PERSONA_KEY,
  SET_PERMISSIONS_LINUX_KEY,
  QUALITY_PROFILE_SCOPE_IDS,
} from "@/lib/constants/settings";
import type { ViewId } from "@/components/root/types";
import type { SearchableQualityProfileBody } from "@/lib/utils/media-content";
import type {
  FacetScoringPersonaSelectionRecord,
  ParsedQualityProfileEntry,
  ScoringPersonaId,
} from "@/lib/types/quality-profiles";
import {
  coerceProfileSetting,
  buildDefaultCategoryPersonaSelections,
  isValidProfileSelection,
  qualityProfileSettingsToCatalogText,
  qualityProfileSettingsToCategoryOverrides,
  qualityProfileSettingsToCategoryPersonaSelections,
  resolveQualityProfileCatalogState,
} from "@/lib/utils/quality-profiles";
import {
  facetScopedMediaSettingsScopeId,
  normalizeAnimeMediaSettings,
  normalizeFillerPolicy,
  normalizeRecapPolicy,
  normalizeRenameCollisionPolicy,
  normalizeRenameMissingMetadataPolicy,
  updateFacetScopedStringArrayRecord,
  updateFacetScopedStringRecord,
} from "@/lib/utils/media-settings-scope";
import type { RootFolderOption } from "@/lib/types/titles";
import type {
  QualityProfileSettingsPayload,
  ViewCategoryId,
} from "@/lib/types/quality-profiles";
import type { ImportMode, MediaSettings } from "@/lib/types/settings";
import { FACET_REGISTRY } from "@/lib/facets/registry";
import { useSettingsSubscription } from "@/lib/hooks/use-settings-subscription";
import {
  localPathStyleFromRuntimeValue,
  type LocalPathStyle,
} from "@/lib/utils/local-path-style";

type UseMediaSettingsArgs = {
  activeQualityScopeId: ViewCategoryId;
  view: ViewId;
};

export type UseMediaSettingsResult = {
  moviesPath: string;
  setMoviesPath: (value: string) => void;
  seriesPath: string;
  setSeriesPath: (value: string) => void;
  rootFolders: RootFolderOption[];
  saveRootFolders: (folders: RootFolderOption[]) => void;
  localPathStyle: LocalPathStyle | undefined;
  mediaSettingsLoading: boolean;
  mediaSettingsSaving: boolean;
  qualityProfiles: SearchableQualityProfileBody[];
  qualityProfileEntries: ParsedQualityProfileEntry[];
  qualityProfileParseError: string;
  globalQualityProfileId: string;
  globalScoringPersona: ScoringPersonaId;
  categoryQualityProfileOverrides: Record<ViewCategoryId, string>;
  categoryRequiredAudioLanguages: Record<ViewCategoryId, string[]>;
  saveCategoryRequiredAudioLanguages: (languages: string[]) => Promise<void> | void;
  categoryPersonaSelections: Record<ViewCategoryId, FacetScoringPersonaSelectionRecord>;
  categoryFolderTemplates: Record<ViewCategoryId, string>;
  setCategoryFolderTemplates: React.Dispatch<
    React.SetStateAction<Record<ViewCategoryId, string>>
  >;
  categorySeasonFolderTemplates: Record<ViewCategoryId, string>;
  setCategorySeasonFolderTemplates: React.Dispatch<
    React.SetStateAction<Record<ViewCategoryId, string>>
  >;
  categoryUseSeasonFolders: Record<ViewCategoryId, boolean>;
  setCategoryUseSeasonFolders: React.Dispatch<
    React.SetStateAction<Record<ViewCategoryId, boolean>>
  >;
  categorySpecialsFolderTemplates: Record<ViewCategoryId, string>;
  setCategorySpecialsFolderTemplates: React.Dispatch<
    React.SetStateAction<Record<ViewCategoryId, string>>
  >;
  categoryRenameTemplates: Record<ViewCategoryId, string>;
  setCategoryRenameTemplates: React.Dispatch<
    React.SetStateAction<Record<ViewCategoryId, string>>
  >;
  categoryRenameEnabled: Record<ViewCategoryId, string>;
  setCategoryRenameEnabled: React.Dispatch<
    React.SetStateAction<Record<ViewCategoryId, string>>
  >;
  categoryRenameCollisionPolicies: Record<ViewCategoryId, string>;
  setCategoryRenameCollisionPolicies: React.Dispatch<
    React.SetStateAction<Record<ViewCategoryId, string>>
  >;
  categoryRenameMissingMetadataPolicies: Record<ViewCategoryId, string>;
  setCategoryRenameMissingMetadataPolicies: React.Dispatch<
    React.SetStateAction<Record<ViewCategoryId, string>>
  >;
  categoryFillerPolicies: Record<ViewCategoryId, string>;
  setCategoryFillerPolicies: React.Dispatch<
    React.SetStateAction<Record<ViewCategoryId, string>>
  >;
  categoryRecapPolicies: Record<ViewCategoryId, string>;
  setCategoryRecapPolicies: React.Dispatch<
    React.SetStateAction<Record<ViewCategoryId, string>>
  >;
  categoryMonitorSpecials: Record<ViewCategoryId, string>;
  setCategoryMonitorSpecials: React.Dispatch<
    React.SetStateAction<Record<ViewCategoryId, string>>
  >;
  categoryInterSeasonMovies: Record<ViewCategoryId, string>;
  setCategoryInterSeasonMovies: React.Dispatch<
    React.SetStateAction<Record<ViewCategoryId, string>>
  >;
  categoryMonitorFillerMovies: Record<ViewCategoryId, string>;
  setCategoryMonitorFillerMovies: React.Dispatch<
    React.SetStateAction<Record<ViewCategoryId, string>>
  >;
  nfoWriteOnImport: Record<ViewCategoryId, string>;
  setNfoWriteOnImport: React.Dispatch<
    React.SetStateAction<Record<ViewCategoryId, string>>
  >;
  plexmatchWriteOnImport: Record<ViewCategoryId, string>;
  setPlexmatchWriteOnImport: React.Dispatch<
    React.SetStateAction<Record<ViewCategoryId, string>>
  >;
  importMode: Record<ViewCategoryId, ImportMode>;
  setImportMode: React.Dispatch<
    React.SetStateAction<Record<ViewCategoryId, ImportMode>>
  >;
  setPermissionsLinux: Record<ViewCategoryId, string>;
  setSetPermissionsLinux: React.Dispatch<
    React.SetStateAction<Record<ViewCategoryId, string>>
  >;
  fileChmod: Record<ViewCategoryId, string>;
  setFileChmod: React.Dispatch<React.SetStateAction<Record<ViewCategoryId, string>>>;
  folderChmod: Record<ViewCategoryId, string>;
  setFolderChmod: React.Dispatch<React.SetStateAction<Record<ViewCategoryId, string>>>;
  chownGroup: Record<ViewCategoryId, string>;
  setChownGroup: React.Dispatch<React.SetStateAction<Record<ViewCategoryId, string>>>;
  saveSetting: (scope: string, scopeId: string | undefined, keyName: string, value: string) => Promise<void> | void;
  saveCategoryQualityProfileOverride: (value: string) => Promise<void> | void;
  saveCategoryScoringPersonaOverride: (
    persona: ScoringPersonaId | null,
  ) => Promise<void> | void;
  updateCategoryMediaProfileSettings: (
    event: React.FormEvent<HTMLFormElement>,
  ) => Promise<void> | void;
  refreshMediaSettings: () => Promise<void>;
  refreshCategoryValidation: () => void;
};

const DEFAULT_RENAME_COLLISION_POLICY = "SKIP";
const DEFAULT_RENAME_MISSING_METADATA_POLICY = "FALLBACK_TITLE";
const DEFAULT_FILLER_POLICY = "DOWNLOAD_ALL";
const DEFAULT_RECAP_POLICY = "DOWNLOAD_ALL";
const DEFAULT_FOLDER_TEMPLATE = "{title} ({year})";
const DEFAULT_SEASON_FOLDER_TEMPLATE = "Season {season}";
const DEFAULT_SPECIALS_FOLDER_TEMPLATE = "Specials";
const DEFAULT_RENAME_TEMPLATE =
  "{title} - S{season_order:2}E{episode:2} ({absolute_episode}) - {quality}.{ext}";
const DEFAULT_CATEGORY_REQUIRED_AUDIO_LANGUAGES: Record<ViewCategoryId, string[]> = {
  MOVIE: [],
  SERIES: [],
  ANIME: [],
};

function buildMediaSettingsInitVariables(activeQualityScopeId: ViewCategoryId) {
  return {
    scope: activeQualityScopeId,
  };
}

export function useMediaSettings({
  activeQualityScopeId,
  view,
}: UseMediaSettingsArgs): UseMediaSettingsResult {
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const client = useClient();
  const [moviesPath, setMoviesPath] = React.useState(
    DEFAULT_MOVIE_LIBRARY_PATH,
  );
  const [seriesPath, setSeriesPath] = React.useState(
    DEFAULT_SERIES_LIBRARY_PATH,
  );
  const [rootFolders, setRootFolders] = React.useState<RootFolderOption[]>([]);
  const [localPathStyle, setLocalPathStyle] =
    React.useState<LocalPathStyle | undefined>(undefined);
  const [mediaSettingsLoading, setMediaSettingsLoading] = React.useState(false);
  const [mediaSettingsSaving, setMediaSettingsSaving] = React.useState(false);
  const [qualityProfiles, setQualityProfiles] = React.useState<
    SearchableQualityProfileBody[]
  >([]);
  const [qualityProfileEntries, setQualityProfileEntries] = React.useState<
    ParsedQualityProfileEntry[]
  >([]);
  const [qualityProfileParseError, setQualityProfileParseError] =
    React.useState("");
  const [globalQualityProfileId, setGlobalQualityProfileId] =
    React.useState("");
  const [globalScoringPersona, setGlobalScoringPersona] =
    React.useState<ScoringPersonaId>("BALANCED");
  const [categoryQualityProfileOverrides, setCategoryQualityProfileOverrides] =
    React.useState<Record<ViewCategoryId, string>>({
      MOVIE: QUALITY_PROFILE_INHERIT_VALUE,
      SERIES: QUALITY_PROFILE_INHERIT_VALUE,
      ANIME: QUALITY_PROFILE_INHERIT_VALUE,
    });
  const [categoryRequiredAudioLanguages, setCategoryRequiredAudioLanguages] =
    React.useState<Record<ViewCategoryId, string[]>>({
      ...DEFAULT_CATEGORY_REQUIRED_AUDIO_LANGUAGES,
    });
  const [categoryPersonaSelections, setCategoryPersonaSelections] =
    React.useState<Record<ViewCategoryId, FacetScoringPersonaSelectionRecord>>({
      ...buildDefaultCategoryPersonaSelections(),
    });
  const [categoryFolderTemplates, setCategoryFolderTemplates] = React.useState<
    Record<ViewCategoryId, string>
  >({
    MOVIE: DEFAULT_FOLDER_TEMPLATE,
    SERIES: DEFAULT_FOLDER_TEMPLATE,
    ANIME: DEFAULT_FOLDER_TEMPLATE,
  });
  const [categorySeasonFolderTemplates, setCategorySeasonFolderTemplates] =
    React.useState<Record<ViewCategoryId, string>>({
      MOVIE: DEFAULT_SEASON_FOLDER_TEMPLATE,
      SERIES: DEFAULT_SEASON_FOLDER_TEMPLATE,
      ANIME: DEFAULT_SEASON_FOLDER_TEMPLATE,
    });
  const [categoryUseSeasonFolders, setCategoryUseSeasonFolders] = React.useState<
    Record<ViewCategoryId, boolean>
  >({
    MOVIE: true,
    SERIES: true,
    ANIME: true,
  });
  const [categorySpecialsFolderTemplates, setCategorySpecialsFolderTemplates] =
    React.useState<Record<ViewCategoryId, string>>({
      MOVIE: DEFAULT_SPECIALS_FOLDER_TEMPLATE,
      SERIES: DEFAULT_SPECIALS_FOLDER_TEMPLATE,
      ANIME: DEFAULT_SPECIALS_FOLDER_TEMPLATE,
    });
  const [categoryRenameTemplates, setCategoryRenameTemplates] = React.useState<
    Record<ViewCategoryId, string>
  >({
    MOVIE: DEFAULT_RENAME_TEMPLATE,
    SERIES: DEFAULT_RENAME_TEMPLATE,
    ANIME: DEFAULT_RENAME_TEMPLATE,
  });
  const [categoryRenameEnabled, setCategoryRenameEnabled] = React.useState<
    Record<ViewCategoryId, string>
  >({
    MOVIE: "true",
    SERIES: "true",
    ANIME: "true",
  });
  const [categoryRenameCollisionPolicies, setCategoryRenameCollisionPolicies] =
    React.useState<Record<ViewCategoryId, string>>({
      MOVIE: DEFAULT_RENAME_COLLISION_POLICY,
      SERIES: DEFAULT_RENAME_COLLISION_POLICY,
      ANIME: DEFAULT_RENAME_COLLISION_POLICY,
    });
  const [
    categoryRenameMissingMetadataPolicies,
    setCategoryRenameMissingMetadataPolicies,
  ] = React.useState<Record<ViewCategoryId, string>>({
    MOVIE: DEFAULT_RENAME_MISSING_METADATA_POLICY,
    SERIES: DEFAULT_RENAME_MISSING_METADATA_POLICY,
    ANIME: DEFAULT_RENAME_MISSING_METADATA_POLICY,
  });
  const [categoryFillerPolicies, setCategoryFillerPolicies] = React.useState<
    Record<ViewCategoryId, string>
  >({
    MOVIE: DEFAULT_FILLER_POLICY,
    SERIES: DEFAULT_FILLER_POLICY,
    ANIME: DEFAULT_FILLER_POLICY,
  });
  const [categoryRecapPolicies, setCategoryRecapPolicies] = React.useState<
    Record<ViewCategoryId, string>
  >({
    MOVIE: DEFAULT_RECAP_POLICY,
    SERIES: DEFAULT_RECAP_POLICY,
    ANIME: DEFAULT_RECAP_POLICY,
  });
  const [categoryMonitorSpecials, setCategoryMonitorSpecials] = React.useState<
    Record<ViewCategoryId, string>
  >({
    MOVIE: "true",
    SERIES: "true",
    ANIME: "false",
  });
  const [categoryInterSeasonMovies, setCategoryInterSeasonMovies] =
    React.useState<Record<ViewCategoryId, string>>({
      MOVIE: "true",
      SERIES: "true",
      ANIME: "true",
    });
  const [categoryMonitorFillerMovies, setCategoryMonitorFillerMovies] =
    React.useState<Record<ViewCategoryId, string>>({
      MOVIE: "false",
      SERIES: "false",
      ANIME: "false",
    });
  const [nfoWriteOnImport, setNfoWriteOnImport] = React.useState<
    Record<ViewCategoryId, string>
  >({
    MOVIE: "false",
    SERIES: "false",
    ANIME: "false",
  });
  const [importMode, setImportMode] = React.useState<Record<ViewCategoryId, ImportMode>>({
    MOVIE: "HARDLINK_OR_COPY",
    SERIES: "HARDLINK_OR_COPY",
    ANIME: "HARDLINK_OR_COPY",
  });
  const [plexmatchWriteOnImport, setPlexmatchWriteOnImport] = React.useState<
    Record<ViewCategoryId, string>
  >({
    MOVIE: "false",
    SERIES: "false",
    ANIME: "false",
  });
  const [setPermissionsLinux, setSetPermissionsLinux] = React.useState<
    Record<ViewCategoryId, string>
  >({
    MOVIE: "false",
    SERIES: "false",
    ANIME: "false",
  });
  const [fileChmod, setFileChmod] = React.useState<Record<ViewCategoryId, string>>({
    MOVIE: "",
    SERIES: "",
    ANIME: "",
  });
  const [folderChmod, setFolderChmod] = React.useState<Record<ViewCategoryId, string>>({
    MOVIE: "",
    SERIES: "",
    ANIME: "",
  });
  const [chownGroup, setChownGroup] = React.useState<Record<ViewCategoryId, string>>({
    MOVIE: "",
    SERIES: "",
    ANIME: "",
  });

  const saveRootFolders = React.useCallback(
    (folders: RootFolderOption[]) => {
      setRootFolders(folders);
      client
        .mutation(updateMediaSettingsMutation, {
          input: {
            scope: activeQualityScopeId,
            rootFolders: folders.map((folder) => ({
              path: folder.path,
              isDefault: folder.isDefault,
            })),
          },
        })
        .toPromise()
        .then(({ error }) => {
          if (error) {
            setGlobalStatus(error.message, { level: "ERROR" });
          }
        });
    },
    [activeQualityScopeId, client, setGlobalStatus],
  );

  const [generalSaveGuard] = React.useState(() => new SettingsSaveGuard());
  const saveSetting = React.useCallback(
    (_scope: string, _scopeId: string | undefined, keyName: string, value: string) => {
      const boolValue = value.trim().toLowerCase() === "true";
      let input:
        | Record<string, boolean | string>
        | null = null;

      switch (keyName) {
        case "anime.filler_policy":
          input = {
            scope: "ANIME",
            fillerPolicy: value,
          };
          break;
        case "anime.recap_policy":
          input = {
            scope: "ANIME",
            recapPolicy: value,
          };
          break;
        case "anime.monitor_specials":
          input = {
            scope: "ANIME",
            monitorSpecials: boolValue,
          };
          break;
        case "anime.inter_season_movies":
          input = {
            scope: "ANIME",
            interSeasonMovies: boolValue,
          };
          break;
        case "anime.monitor_filler_movies":
          input = {
            scope: "ANIME",
            monitorFillerMovies: boolValue,
          };
          break;
        case NFO_WRITE_ON_IMPORT_MOVIE_KEY:
          input = { scope: "MOVIE", nfoWriteOnImport: boolValue };
          break;
        case NFO_WRITE_ON_IMPORT_SERIES_KEY:
          input = { scope: "SERIES", nfoWriteOnImport: boolValue };
          break;
        case NFO_WRITE_ON_IMPORT_ANIME_KEY:
          input = { scope: "ANIME", nfoWriteOnImport: boolValue };
          break;
        case PLEXMATCH_WRITE_ON_IMPORT_SERIES_KEY:
          input = { scope: "SERIES", plexmatchWriteOnImport: boolValue };
          break;
        case PLEXMATCH_WRITE_ON_IMPORT_ANIME_KEY:
          input = { scope: "ANIME", plexmatchWriteOnImport: boolValue };
          break;
        case RENAME_ENABLED_KEY:
          input = {
            scope: (_scopeId ?? activeQualityScopeId) as ViewCategoryId,
            renameEnabled: boolValue,
          };
          break;
        case IMPORT_MODE_KEY:
          input = {
            scope: (_scopeId ?? activeQualityScopeId) as ViewCategoryId,
            importMode: value,
          };
          break;
        case SET_PERMISSIONS_LINUX_KEY:
          input = {
            scope: (_scopeId ?? activeQualityScopeId) as ViewCategoryId,
            setPermissionsLinux: boolValue,
          };
          break;
        case FILE_CHMOD_KEY:
          input = {
            scope: (_scopeId ?? activeQualityScopeId) as ViewCategoryId,
            fileChmod: value,
          };
          break;
        case FOLDER_CHMOD_KEY:
          input = {
            scope: (_scopeId ?? activeQualityScopeId) as ViewCategoryId,
            folderChmod: value,
          };
          break;
        case CHOWN_GROUP_KEY:
          input = {
            scope: (_scopeId ?? activeQualityScopeId) as ViewCategoryId,
            chownGroup: value,
          };
          break;
        default:
          break;
      }

      if (!input) {
        return;
      }

      const scope = input.scope as ViewCategoryId;
      const field = Object.keys(input).find((key) => key !== "scope")!;
      const fields: Record<string, [string, (value: string) => void]> = {
        fillerPolicy: [categoryFillerPolicies[scope], (next) => setCategoryFillerPolicies((prev) => updateFacetScopedStringRecord(prev, scope, next))],
        recapPolicy: [categoryRecapPolicies[scope], (next) => setCategoryRecapPolicies((prev) => updateFacetScopedStringRecord(prev, scope, next))],
        monitorSpecials: [categoryMonitorSpecials[scope], (next) => setCategoryMonitorSpecials((prev) => updateFacetScopedStringRecord(prev, scope, next))],
        interSeasonMovies: [categoryInterSeasonMovies[scope], (next) => setCategoryInterSeasonMovies((prev) => updateFacetScopedStringRecord(prev, scope, next))],
        monitorFillerMovies: [categoryMonitorFillerMovies[scope], (next) => setCategoryMonitorFillerMovies((prev) => updateFacetScopedStringRecord(prev, scope, next))],
        nfoWriteOnImport: [nfoWriteOnImport[scope], (next) => setNfoWriteOnImport((prev) => updateFacetScopedStringRecord(prev, scope, next))],
        plexmatchWriteOnImport: [plexmatchWriteOnImport[scope], (next) => setPlexmatchWriteOnImport((prev) => updateFacetScopedStringRecord(prev, scope, next))],
        renameEnabled: [categoryRenameEnabled[scope], (next) => setCategoryRenameEnabled((prev) => updateFacetScopedStringRecord(prev, scope, next))],
        importMode: [importMode[scope], (next) => setImportMode((prev) => ({ ...prev, [scope]: next as ImportMode }))],
        setPermissionsLinux: [setPermissionsLinux[scope], (next) => setSetPermissionsLinux((prev) => updateFacetScopedStringRecord(prev, scope, next))],
        fileChmod: [fileChmod[scope], (next) => setFileChmod((prev) => updateFacetScopedStringRecord(prev, scope, next))],
        folderChmod: [folderChmod[scope], (next) => setFolderChmod((prev) => updateFacetScopedStringRecord(prev, scope, next))],
        chownGroup: [chownGroup[scope], (next) => setChownGroup((prev) => updateFacetScopedStringRecord(prev, scope, next))],
      };
      const [previousValue, restore] = fields[field];
      if (!generalSaveGuard.begin()) {
        restore(previousValue);
        return;
      }
      setMediaSettingsSaving(true);
      return client
        .mutation(updateMediaSettingsMutation, { input })
        .toPromise()
        .then(({ data, error }) => {
          if (error) throw error;
          const savedValue = data?.updateMediaSettings?.[field];
          restore(savedValue === undefined ? String(input[field]) : String(savedValue ?? ""));
        })
        .catch((error: unknown) => {
          restore(previousValue);
          setGlobalStatus(error instanceof Error ? error.message : t("status.failedToUpdate"), { level: "ERROR" });
        })
        .finally(() => {
          generalSaveGuard.end();
          setMediaSettingsSaving(false);
        });
    },
    [activeQualityScopeId, client, setGlobalStatus, generalSaveGuard, t,
      categoryFillerPolicies, categoryRecapPolicies, categoryMonitorSpecials,
      categoryInterSeasonMovies, categoryMonitorFillerMovies, nfoWriteOnImport,
      plexmatchWriteOnImport, categoryRenameEnabled, importMode,
      setPermissionsLinux, fileChmod, folderChmod, chownGroup],
  );

  const normalizeQualityProfiles = React.useCallback(
    (rawValue: string) => {
      const resolved = resolveQualityProfileCatalogState(rawValue);
      const nextParseError =
        !resolved.isRawValid && rawValue.trim().length
          ? t("settings.qualityProfileCatalogInvalid")
          : "";

      setQualityProfileParseError((currentParseError) =>
        currentParseError === nextParseError
          ? currentParseError
          : nextParseError,
      );

      setQualityProfileEntries(resolved.entries);
      return resolved.profiles;
    },
    [t],
  );

  const applyMediaSettingsFromPayload = React.useCallback(
    (
      qualityProfileSettings: QualityProfileSettingsPayload | null | undefined,
      mediaSettings: MediaSettings | null | undefined,
    ) => {
      if (mediaSettings) {
        if (view === "movies") {
          const nextPath = mediaSettings.libraryPath.trim() || DEFAULT_MOVIE_LIBRARY_PATH;
          setMoviesPath((currentPath) =>
            currentPath === nextPath ? currentPath : nextPath,
          );
        }

        if (view === "series") {
          const nextPath = mediaSettings.libraryPath.trim() || DEFAULT_SERIES_LIBRARY_PATH;
          setSeriesPath((currentPath) =>
            currentPath === nextPath ? currentPath : nextPath,
          );
        }

        setRootFolders((currentFolders) => {
          const nextFolders = mediaSettings.rootFolders ?? [];
          const same =
            currentFolders.length === nextFolders.length &&
            currentFolders.every(
              (folder, index) =>
                folder.path === nextFolders[index]?.path &&
                folder.isDefault === nextFolders[index]?.isDefault,
            );
          return same ? currentFolders : nextFolders;
        });
      }

      if (qualityProfileSettings) {
        const nextProfileText = qualityProfileSettingsToCatalogText(qualityProfileSettings);
        const nextProfiles = normalizeQualityProfiles(nextProfileText);

        const rawGlobalProfileId =
          coerceProfileSetting(
            qualityProfileSettings?.globalProfileId ?? "",
          ) || "";
        const resolvedGlobalId =
          rawGlobalProfileId &&
          nextProfiles.some((p) => p.id === rawGlobalProfileId)
            ? rawGlobalProfileId
            : (nextProfiles[0]?.id ?? "");
        setGlobalQualityProfileId((current) =>
          current === resolvedGlobalId ? current : resolvedGlobalId,
        );
        setGlobalScoringPersona((current) =>
          current === (qualityProfileSettings?.globalScoringPersona ?? "BALANCED")
            ? current
            : (qualityProfileSettings?.globalScoringPersona ?? "BALANCED"),
        );

        setQualityProfiles((currentProfiles) =>
          currentProfiles.length === nextProfiles.length &&
          currentProfiles.every(
            (profile, index) =>
              profile.id === nextProfiles[index]?.id &&
              profile.name === nextProfiles[index]?.name,
          )
            ? currentProfiles
            : nextProfiles,
        );

        const nextOverrides = qualityProfileSettingsToCategoryOverrides(qualityProfileSettings);
        setCategoryQualityProfileOverrides((previous) =>
          QUALITY_PROFILE_SCOPE_IDS.every((scopeId) => previous[scopeId] === nextOverrides[scopeId])
            ? previous
            : nextOverrides,
        );
        const nextPersonaSelections =
          qualityProfileSettingsToCategoryPersonaSelections(qualityProfileSettings);
        setCategoryPersonaSelections((previous) =>
          QUALITY_PROFILE_SCOPE_IDS.every((scopeId) => {
            const current = previous[scopeId];
            const next = nextPersonaSelections[scopeId];
            return (
              current.overridePersona === next.overridePersona &&
              current.effectivePersona === next.effectivePersona &&
              current.inheritsGlobal === next.inheritsGlobal
            );
          })
            ? previous
            : nextPersonaSelections,
        );

      }

      if (mediaSettings) {
        const mediaSettingsScopeId = facetScopedMediaSettingsScopeId(mediaSettings);
        setCategoryRequiredAudioLanguages((previous) => {
          const nextLanguages = mediaSettings.requiredAudioLanguages ?? [];
          return updateFacetScopedStringArrayRecord(
            previous,
            mediaSettingsScopeId,
            nextLanguages,
          );
        });
        setCategoryFolderTemplates((previous) => {
          const nextTemplate = mediaSettings.folderTemplate || DEFAULT_FOLDER_TEMPLATE;
          return updateFacetScopedStringRecord(
            previous,
            mediaSettingsScopeId,
            nextTemplate,
          );
        });
        setCategorySeasonFolderTemplates((previous) =>
          updateFacetScopedStringRecord(
            previous,
            mediaSettingsScopeId,
            mediaSettings.seasonFolderTemplate || DEFAULT_SEASON_FOLDER_TEMPLATE,
          ),
        );
        setCategoryUseSeasonFolders((previous) => {
          const nextValue = mediaSettings.useSeasonFolders !== false;
          return previous[mediaSettingsScopeId] === nextValue
            ? previous
            : { ...previous, [mediaSettingsScopeId]: nextValue };
        });
        setCategorySpecialsFolderTemplates((previous) =>
          updateFacetScopedStringRecord(
            previous,
            mediaSettingsScopeId,
            mediaSettings.specialsFolderTemplate || DEFAULT_SPECIALS_FOLDER_TEMPLATE,
          ),
        );
        setCategoryRenameTemplates((previous) => {
          const nextTemplate = mediaSettings.renameTemplate || DEFAULT_RENAME_TEMPLATE;
          return updateFacetScopedStringRecord(
            previous,
            mediaSettingsScopeId,
            nextTemplate,
          );
        });
        setCategoryRenameEnabled((previous) => {
          const nextValue = mediaSettings.renameEnabled === false ? "false" : "true";
          return updateFacetScopedStringRecord(
            previous,
            mediaSettingsScopeId,
            nextValue,
          );
        });

        setCategoryRenameCollisionPolicies((previous) => {
          const nextPolicy = normalizeRenameCollisionPolicy(
            mediaSettings.renameCollisionPolicy,
          );
          return updateFacetScopedStringRecord(
            previous,
            mediaSettingsScopeId,
            nextPolicy,
          );
        });

        setCategoryRenameMissingMetadataPolicies((previous) => {
          const nextPolicy = normalizeRenameMissingMetadataPolicy(
            mediaSettings.renameMissingMetadataPolicy,
          );
          return updateFacetScopedStringRecord(
            previous,
            mediaSettingsScopeId,
            nextPolicy,
          );
        });

        if (mediaSettings.scope === "ANIME") {
          const animeSettings = normalizeAnimeMediaSettings(mediaSettings);
          setCategoryFillerPolicies((previous) =>
            updateFacetScopedStringRecord(previous, mediaSettingsScopeId, animeSettings.fillerPolicy),
          );
          setCategoryRecapPolicies((previous) =>
            updateFacetScopedStringRecord(previous, mediaSettingsScopeId, animeSettings.recapPolicy),
          );
          setCategoryMonitorSpecials((previous) =>
            updateFacetScopedStringRecord(previous, mediaSettingsScopeId, animeSettings.monitorSpecials),
          );
          setCategoryInterSeasonMovies((previous) =>
            updateFacetScopedStringRecord(previous, mediaSettingsScopeId, animeSettings.interSeasonMovies),
          );
          setCategoryMonitorFillerMovies((previous) =>
            updateFacetScopedStringRecord(previous, mediaSettingsScopeId, animeSettings.monitorFillerMovies),
          );
        }

        setNfoWriteOnImport((previous) => {
          const nextValue = mediaSettings.nfoWriteOnImport ? "true" : "false";
          return updateFacetScopedStringRecord(
            previous,
            mediaSettingsScopeId,
            nextValue,
          );
        });

        if (mediaSettings.plexmatchWriteOnImport !== null) {
          setPlexmatchWriteOnImport((previous) => {
            const nextValue = mediaSettings.plexmatchWriteOnImport ? "true" : "false";
            return updateFacetScopedStringRecord(
              previous,
              mediaSettingsScopeId,
              nextValue,
            );
          });
        }

        setImportMode((previous) => {
          const nextMode: ImportMode =
            mediaSettings.importMode === "MOVE" ? "MOVE" : "HARDLINK_OR_COPY";
          return previous[mediaSettingsScopeId] === nextMode
            ? previous
            : { ...previous, [mediaSettingsScopeId]: nextMode };
        });

        setSetPermissionsLinux((previous) =>
          updateFacetScopedStringRecord(
            previous,
            mediaSettingsScopeId,
            mediaSettings.setPermissionsLinux ? "true" : "false",
          ),
        );
        setFileChmod((previous) =>
          updateFacetScopedStringRecord(
            previous,
            mediaSettingsScopeId,
            mediaSettings.fileChmod ?? "",
          ),
        );
        setFolderChmod((previous) =>
          updateFacetScopedStringRecord(
            previous,
            mediaSettingsScopeId,
            mediaSettings.folderChmod ?? "",
          ),
        );
        setChownGroup((previous) =>
          updateFacetScopedStringRecord(
            previous,
            mediaSettingsScopeId,
            mediaSettings.chownGroup ?? "",
          ),
        );
      }
    },
    [
      normalizeQualityProfiles,
      setGlobalScoringPersona,
      view,
    ],
  );

  const refreshMediaSettings = React.useCallback(async () => {
    const version = generalSaveGuard.readVersion();
    if (version === null) return;
    setMediaSettingsLoading(true);
    try {
      const variables = buildMediaSettingsInitVariables(activeQualityScopeId);
      const { data, error } = await client
        .query(mediaSettingsInitQuery, variables)
        .toPromise();
      if (error) throw error;
      if (!generalSaveGuard.accepts(version)) return;

      setLocalPathStyle(
        localPathStyleFromRuntimeValue(data?.runtimeInfo?.runtimePathStyle),
      );
      applyMediaSettingsFromPayload(
        data.qualityProfileSettings,
        data.mediaSettings,
      );
    } catch (error) {
      setGlobalStatus(
        error instanceof Error ? error.message : t("status.failedToLoad"),
        { level: "ERROR" },
      );
    } finally {
      setMediaSettingsLoading(false);
    }
  }, [
    activeQualityScopeId,
    applyMediaSettingsFromPayload,
    generalSaveGuard,
    client,
    setGlobalStatus,
    t,
  ]);

  const saveCategoryQualityProfileOverride = React.useCallback(
    async (rawValue: string) => {
      const previousProfile = categoryQualityProfileOverrides[activeQualityScopeId];
      const selectedProfile = coerceProfileSetting(rawValue);

      if (selectedProfile === previousProfile) {
        return;
      }

      setCategoryQualityProfileOverrides((previous) =>
        previous[activeQualityScopeId] === selectedProfile
          ? previous
          : { ...previous, [activeQualityScopeId]: selectedProfile },
      );

      if (
        selectedProfile !== QUALITY_PROFILE_INHERIT_VALUE &&
        !isValidProfileSelection(qualityProfiles, selectedProfile)
      ) {
        const invalidId = selectedProfile || t("label.default");
        const message = t("settings.qualityProfileUnknown", { id: invalidId });
        setCategoryQualityProfileOverrides((previous) => ({
          ...previous,
          [activeQualityScopeId]: previousProfile,
        }));
        setQualityProfileParseError(message);
        setGlobalStatus(message, { level: "WARNING" });
        return;
      }

      if (!qualityProfiles.length) {
        const message = t("settings.qualityProfileCatalogInvalid");
        setCategoryQualityProfileOverrides((previous) => ({
          ...previous,
          [activeQualityScopeId]: previousProfile,
        }));
        setQualityProfileParseError(message);
        setGlobalStatus(message);
        return;
      }

      setMediaSettingsSaving(true);
      setQualityProfileParseError("");

      try {
        const { data: qualityProfileData, error: qualityProfileError } = await client
          .mutation(saveQualityProfileSettingsMutation, {
            input: {
              profiles: [],
              globalProfileId: null,
              categorySelections: [
                {
                  scope: activeQualityScopeId,
                  profileId:
                    selectedProfile === QUALITY_PROFILE_INHERIT_VALUE
                      ? null
                      : selectedProfile,
                  inheritGlobal: selectedProfile === QUALITY_PROFILE_INHERIT_VALUE,
                },
              ],
              replaceExisting: false,
            },
          })
          .toPromise();
        if (qualityProfileError) throw qualityProfileError;

        applyMediaSettingsFromPayload(
          qualityProfileData?.saveQualityProfileSettings,
          undefined,
        );
        setGlobalStatus(t("settings.qualitySettingsSaved"), { level: "SUCCESS" });
      } catch (error) {
        setCategoryQualityProfileOverrides((previous) => ({
          ...previous,
          [activeQualityScopeId]: previousProfile,
        }));
        setGlobalStatus(
          error instanceof Error ? error.message : t("status.failedToUpdate"),
          { level: "ERROR" },
        );
      } finally {
        setMediaSettingsSaving(false);
      }
    },
    [
      activeQualityScopeId,
      applyMediaSettingsFromPayload,
      categoryQualityProfileOverrides,
      client,
      qualityProfiles,
      setGlobalStatus,
      t,
    ],
  );

  const saveCategoryScoringPersonaOverride = React.useCallback(
    async (persona: ScoringPersonaId | null) => {
      const previousSelection = categoryPersonaSelections[activeQualityScopeId];
      const nextSelection: FacetScoringPersonaSelectionRecord = {
        scope: activeQualityScopeId,
        overridePersona: persona,
        effectivePersona: persona ?? globalScoringPersona,
        inheritsGlobal: persona === null,
      };

      setCategoryPersonaSelections((previous) => ({
        ...previous,
        [activeQualityScopeId]: nextSelection,
      }));
      setMediaSettingsSaving(true);
      try {
        const { data, error } = await client
          .mutation(saveQualityProfileSettingsMutation, {
            input: {
              profiles: [],
              globalProfileId: null,
              globalScoringPersona: null,
              categorySelections: [],
              categoryPersonaSelections: [
                {
                  scope: activeQualityScopeId,
                  persona,
                  inheritGlobal: persona === null,
                },
              ],
              replaceExisting: false,
            },
          })
          .toPromise();
        if (error) {
          throw error;
        }

        applyMediaSettingsFromPayload(
          data?.saveQualityProfileSettings,
          undefined,
        );
        setGlobalStatus(t("settings.qualitySettingsSaved"), { level: "SUCCESS" });
      } catch (error) {
        setCategoryPersonaSelections((previous) => ({
          ...previous,
          [activeQualityScopeId]: previousSelection,
        }));
        setGlobalStatus(
          error instanceof Error ? error.message : t("status.failedToUpdate"),
          { level: "ERROR" },
        );
        throw error;
      } finally {
        setMediaSettingsSaving(false);
      }
    },
    [
      activeQualityScopeId,
      applyMediaSettingsFromPayload,
      categoryPersonaSelections,
      client,
      globalScoringPersona,
      setGlobalStatus,
      t,
    ],
  );

  const saveCategoryRequiredAudioLanguages = React.useCallback(
    async (languages: string[]) => {
      const previousLanguages = categoryRequiredAudioLanguages[activeQualityScopeId] ?? [];
      const nextLanguages = [...languages];
      const same =
        previousLanguages.length === nextLanguages.length &&
        previousLanguages.every((value, index) => value === nextLanguages[index]);
      if (same) {
        return;
      }

      setCategoryRequiredAudioLanguages((previous) => ({
        ...previous,
        [activeQualityScopeId]: nextLanguages,
      }));
      setMediaSettingsSaving(true);

      try {
        const { data, error } = await client
          .mutation(updateMediaSettingsMutation, {
            input: {
              scope: activeQualityScopeId,
              requiredAudioLanguages: nextLanguages,
            },
          })
          .toPromise();
        if (error) {
          throw error;
        }

        applyMediaSettingsFromPayload(undefined, data?.updateMediaSettings);
        setGlobalStatus(t("settings.qualitySettingsSaved"), { level: "SUCCESS" });
      } catch (error) {
        setCategoryRequiredAudioLanguages((previous) => ({
          ...previous,
          [activeQualityScopeId]: previousLanguages,
        }));
        setGlobalStatus(
          error instanceof Error ? error.message : t("status.failedToUpdate"),
          { level: "ERROR" },
        );
        throw error;
      } finally {
        setMediaSettingsSaving(false);
      }
    },
    [
      activeQualityScopeId,
      applyMediaSettingsFromPayload,
      categoryRequiredAudioLanguages,
      client,
      setGlobalStatus,
      t,
    ],
  );

  const updateCategoryMediaProfileSettings = React.useCallback(
    async (event: React.FormEvent<HTMLFormElement>) => {
      event.preventDefault();
      const folderTemplate =
        categoryFolderTemplates[activeQualityScopeId].trim();
      const seasonFolderTemplate =
        categorySeasonFolderTemplates[activeQualityScopeId].trim();
      const specialsFolderTemplate =
        categorySpecialsFolderTemplates[activeQualityScopeId].trim();
      const episodicScope = activeQualityScopeId !== "MOVIE";
      const renameEnabled =
        categoryRenameEnabled[activeQualityScopeId] !== "false";
      const renameTemplate =
        categoryRenameTemplates[activeQualityScopeId].trim();
      const renameCollisionPolicy = normalizeRenameCollisionPolicy(
        categoryRenameCollisionPolicies[activeQualityScopeId],
      );
      const renameMissingMetadataPolicy = normalizeRenameMissingMetadataPolicy(
        categoryRenameMissingMetadataPolicies[activeQualityScopeId],
      );
      const renameConfigInput = renameEnabled
        ? {
            renameTemplate,
            renameCollisionPolicy,
            renameMissingMetadataPolicy,
          }
        : {};

      if (!folderTemplate) {
        setGlobalStatus(t("settings.folderTemplateRequired"));
        return;
      }
      if (episodicScope && !seasonFolderTemplate) {
        setGlobalStatus(t("settings.seasonFolderTemplateRequired"));
        return;
      }
      if (episodicScope && !specialsFolderTemplate) {
        setGlobalStatus(t("settings.specialsFolderTemplateRequired"));
        return;
      }
      if (renameEnabled && !renameTemplate) {
        setGlobalStatus(t("settings.renameTemplateRequired"));
        return;
      }
      setMediaSettingsSaving(true);
      setQualityProfileParseError("");

      try {
        const { data: mediaData, error: mediaError } = await client
          .mutation(updateMediaSettingsMutation, {
            input: {
              scope: activeQualityScopeId,
              requiredAudioLanguages:
                categoryRequiredAudioLanguages[activeQualityScopeId] ?? [],
              folderTemplate,
              ...(episodicScope
                ? {
                    seasonFolderTemplate,
                    specialsFolderTemplate,
                    useSeasonFolders:
                      categoryUseSeasonFolders[activeQualityScopeId] !== false,
                  }
                : {}),
              renameEnabled,
              ...renameConfigInput,
              nfoWriteOnImport: nfoWriteOnImport[activeQualityScopeId] === "true",
              ...(activeQualityScopeId === "ANIME"
                ? {
                    fillerPolicy: normalizeFillerPolicy(categoryFillerPolicies.ANIME),
                    recapPolicy: normalizeRecapPolicy(categoryRecapPolicies.ANIME),
                    monitorSpecials: categoryMonitorSpecials.ANIME === "true",
                    interSeasonMovies: categoryInterSeasonMovies.ANIME !== "false",
                    monitorFillerMovies: categoryMonitorFillerMovies.ANIME === "true",
                    plexmatchWriteOnImport:
                      plexmatchWriteOnImport[activeQualityScopeId] === "true",
                  }
                : activeQualityScopeId === "SERIES"
                  ? {
                      plexmatchWriteOnImport:
                        plexmatchWriteOnImport[activeQualityScopeId] === "true",
                    }
                  : {}),
            },
          })
          .toPromise();
        if (mediaError) throw mediaError;

        applyMediaSettingsFromPayload(
          undefined,
          mediaData?.updateMediaSettings,
        );

        const successMessage =
          view === "movies"
            ? t("settings.movieSettingsSaved")
            : view === "series"
              ? t("settings.seriesSettingsSaved")
              : t("settings.mediaSettingsSaved");
        setGlobalStatus(successMessage, { level: "SUCCESS" });
      } catch (error) {
        setGlobalStatus(
          error instanceof Error ? error.message : t("status.failedToUpdate"),
          { level: "ERROR" },
        );
      } finally {
        setMediaSettingsSaving(false);
      }
    },
    [
      activeQualityScopeId,
      categoryFillerPolicies,
      categoryFolderTemplates,
      categoryUseSeasonFolders,
      categorySeasonFolderTemplates,
      categorySpecialsFolderTemplates,
      categoryRequiredAudioLanguages,
      categoryRecapPolicies,
      categoryInterSeasonMovies,
      categoryMonitorFillerMovies,
      categoryMonitorSpecials,
      categoryRenameCollisionPolicies,
      categoryRenameEnabled,
      categoryRenameMissingMetadataPolicies,
      categoryRenameTemplates,
      nfoWriteOnImport,
      plexmatchWriteOnImport,
      applyMediaSettingsFromPayload,
      client,
      setGlobalStatus,
      t,
      view,
    ],
  );

  const refreshCategoryValidation = React.useCallback(() => {
    if (qualityProfiles.length === 0) {
      return;
    }

    const hasInvalidProfile = QUALITY_PROFILE_SCOPE_IDS.some((scopeId) => {
      const normalizedCategoryProfile = coerceProfileSetting(
        categoryQualityProfileOverrides[scopeId],
      );
      return (
        normalizedCategoryProfile !== QUALITY_PROFILE_INHERIT_VALUE &&
        !isValidProfileSelection(qualityProfiles, normalizedCategoryProfile)
      );
    });

    if (!hasInvalidProfile) {
      setQualityProfileParseError("");
      return;
    }

    const invalidValue = Object.entries(categoryQualityProfileOverrides).find(
      ([scopeId, profileId]) => {
        const normalizedProfileId = coerceProfileSetting(profileId);
        const isScopeAllowed = QUALITY_PROFILE_SCOPE_IDS.includes(
          scopeId as (typeof QUALITY_PROFILE_SCOPE_IDS)[number],
        );
        const isInvalid =
          normalizedProfileId !== QUALITY_PROFILE_INHERIT_VALUE &&
          !isValidProfileSelection(qualityProfiles, normalizedProfileId);
        return isScopeAllowed && isInvalid;
      },
    )?.[1];

    const invalidProfileId = invalidValue
      ? coerceProfileSetting(invalidValue)
      : t("label.default");
    setQualityProfileParseError(
      t("settings.qualityProfileUnknown", { id: invalidProfileId }),
    );
  }, [categoryQualityProfileOverrides, qualityProfiles, t]);

  React.useEffect(() => {
    refreshCategoryValidation();
  }, [refreshCategoryValidation]);

  const mediaSettingsKeys = React.useMemo(
    () =>
      new Set([
        QUALITY_PROFILE_CATALOG_KEY,
        QUALITY_PROFILE_ID_KEY,
        SCORING_PERSONA_KEY,
        "audio.required_languages",
        RENAME_ENABLED_KEY,
        "rename.template",
        "rename.template.movie.global",
        "rename.template.series.global",
        "rename.template.anime.global",
        "rename.collision_policy",
        "rename.collision_policy.global",
        "rename.collision_policy.movie.global",
        "rename.collision_policy.series.global",
        "rename.collision_policy.anime.global",
        "rename.missing_metadata_policy",
        "rename.missing_metadata_policy.global",
        "rename.missing_metadata_policy.movie.global",
        "rename.missing_metadata_policy.series.global",
        "rename.missing_metadata_policy.anime.global",
        "anime.filler_policy",
        "anime.recap_policy",
        "anime.monitor_specials",
        "anime.inter_season_movies",
        "anime.monitor_filler_movies",
        NFO_WRITE_ON_IMPORT_MOVIE_KEY,
        NFO_WRITE_ON_IMPORT_SERIES_KEY,
        NFO_WRITE_ON_IMPORT_ANIME_KEY,
        PLEXMATCH_WRITE_ON_IMPORT_SERIES_KEY,
        PLEXMATCH_WRITE_ON_IMPORT_ANIME_KEY,
        IMPORT_MODE_KEY,
        SET_PERMISSIONS_LINUX_KEY,
        FILE_CHMOD_KEY,
        FOLDER_CHMOD_KEY,
        CHOWN_GROUP_KEY,
        ...FACET_REGISTRY.map((f) => f.rootFoldersKey),
        ...FACET_REGISTRY.map((f) => f.folderSettingKey),
      ]),
    [],
  );

  useSettingsSubscription(
    React.useCallback(
      (keys: string[]) => {
        if (keys.some((k) => mediaSettingsKeys.has(k))) {
          void refreshMediaSettings();
        }
      },
      [mediaSettingsKeys, refreshMediaSettings],
    ),
  );

  return {
    moviesPath,
    setMoviesPath,
    seriesPath,
    setSeriesPath,
    rootFolders,
    saveRootFolders,
    localPathStyle,
    mediaSettingsLoading,
    mediaSettingsSaving,
    qualityProfiles,
    qualityProfileEntries,
    qualityProfileParseError,
    globalQualityProfileId,
    globalScoringPersona,
    categoryQualityProfileOverrides,
    categoryRequiredAudioLanguages,
    saveCategoryRequiredAudioLanguages,
    categoryPersonaSelections,
    categoryFolderTemplates,
    setCategoryFolderTemplates,
    categorySeasonFolderTemplates,
    setCategorySeasonFolderTemplates,
    categoryUseSeasonFolders,
    setCategoryUseSeasonFolders,
    categorySpecialsFolderTemplates,
    setCategorySpecialsFolderTemplates,
    categoryRenameTemplates,
    setCategoryRenameTemplates,
    categoryRenameEnabled,
    setCategoryRenameEnabled,
    categoryRenameCollisionPolicies,
    setCategoryRenameCollisionPolicies,
    categoryRenameMissingMetadataPolicies,
    setCategoryRenameMissingMetadataPolicies,
    categoryFillerPolicies,
    setCategoryFillerPolicies,
    categoryRecapPolicies,
    setCategoryRecapPolicies,
    categoryMonitorSpecials,
    setCategoryMonitorSpecials,
    categoryInterSeasonMovies,
    setCategoryInterSeasonMovies,
    categoryMonitorFillerMovies,
    setCategoryMonitorFillerMovies,
    nfoWriteOnImport,
    setNfoWriteOnImport,
    plexmatchWriteOnImport,
    setPlexmatchWriteOnImport,
    importMode,
    setImportMode,
    setPermissionsLinux,
    setSetPermissionsLinux,
    fileChmod,
    setFileChmod,
    folderChmod,
    setFolderChmod,
    chownGroup,
    setChownGroup,
    saveSetting,
    saveCategoryQualityProfileOverride,
    saveCategoryScoringPersonaOverride,
    updateCategoryMediaProfileSettings,
    refreshMediaSettings,
    refreshCategoryValidation,
  };
}
