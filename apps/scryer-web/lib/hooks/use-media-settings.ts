import * as React from "react";
import { useClient } from "urql";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";

import { saveAdminSettingsMutation } from "@/lib/graphql/mutations";
import { mediaSettingsInitQuery } from "@/lib/graphql/queries";
import {
  ANIME_FILLER_POLICY_KEY,
  ANIME_RECAP_POLICY_KEY,
  ANIME_INTER_SEASON_MOVIES_KEY,
  ANIME_MONITOR_SPECIALS_KEY,
  ANIME_PREFERRED_SUB_GROUP_KEY,
  DEFAULT_MOVIE_LIBRARY_PATH,
  DEFAULT_SERIES_LIBRARY_PATH,
  MOVIE_FOLDER_KEY,
  NFO_WRITE_ON_IMPORT_ANIME_KEY,
  NFO_WRITE_ON_IMPORT_MOVIE_KEY,
  NFO_WRITE_ON_IMPORT_SERIES_KEY,
  PLEXMATCH_WRITE_ON_IMPORT_ANIME_KEY,
  PLEXMATCH_WRITE_ON_IMPORT_SERIES_KEY,
  SERIES_FOLDER_KEY,
  QUALITY_PROFILE_CATALOG_KEY,
  QUALITY_PROFILE_ID_KEY,
  QUALITY_PROFILE_INHERIT_VALUE,
  QUALITY_PROFILE_SCOPE_IDS,
  RENAME_COLLISION_POLICY_GLOBAL_KEY,
  RENAME_COLLISION_POLICY_KEY,
  RENAME_MISSING_METADATA_POLICY_GLOBAL_KEY,
  RENAME_MISSING_METADATA_POLICY_KEY,
  RENAME_TEMPLATE_GLOBAL_KEYS,
  RENAME_TEMPLATE_KEY,
} from "@/lib/constants/settings";
import type { ViewId } from "@/components/root/types";
import type { SearchableQualityProfileBody } from "@/lib/utils/media-content";
import {
  coerceProfileSetting,
  isValidProfileSelection,
  resolveQualityProfileCatalogState,
} from "@/lib/utils/quality-profiles";
import { getSettingDisplayValue } from "@/lib/utils/settings";
import type { AdminSetting, AdminSettingsResponse } from "@/lib/types";
import type { ViewCategoryId } from "@/lib/types/quality-profiles";

type UseMediaSettingsArgs = {
  activeQualityScopeId: ViewCategoryId;
  view: ViewId;
};

export type UseMediaSettingsResult = {
  moviesPath: string;
  setMoviesPath: (value: string) => void;
  seriesPath: string;
  setSeriesPath: (value: string) => void;
  mediaSettingsLoading: boolean;
  mediaSettingsSaving: boolean;
  qualityProfiles: SearchableQualityProfileBody[];
  qualityProfileParseError: string;
  globalQualityProfileId: string;
  categoryQualityProfileOverrides: Record<ViewCategoryId, string>;
  setCategoryQualityProfileOverrides: React.Dispatch<
    React.SetStateAction<Record<ViewCategoryId, string>>
  >;
  categoryRenameTemplates: Record<ViewCategoryId, string>;
  setCategoryRenameTemplates: React.Dispatch<
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
  categoryPreferredSubGroup: Record<ViewCategoryId, string>;
  setCategoryPreferredSubGroup: React.Dispatch<
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
  updateCategoryMediaProfileSettings: (event: React.FormEvent<HTMLFormElement>) => Promise<void> | void;
  refreshMediaSettings: () => Promise<void>;
  refreshCategoryValidation: () => void;
};

const DEFAULT_RENAME_COLLISION_POLICY = "skip";
const DEFAULT_RENAME_MISSING_METADATA_POLICY = "fallback_title";
const DEFAULT_FILLER_POLICY = "download_all";
const ALLOWED_RENAME_COLLISION_POLICIES = new Set(["skip", "error", "replace_if_better"]);
const ALLOWED_RENAME_MISSING_METADATA_POLICIES = new Set(["skip", "fallback_title"]);
const ALLOWED_FILLER_POLICIES = new Set(["download_all", "skip_filler"]);
const DEFAULT_RECAP_POLICY = "download_all";
const ALLOWED_RECAP_POLICIES = new Set(["download_all", "skip_recap"]);
const DEFAULT_RENAME_TEMPLATE =
  "{title} - S{season_order:2}E{episode:2} ({absolute_episode}) - {quality}.{ext}";

const NFO_WRITE_KEYS: Record<ViewCategoryId, string> = {
  movie: NFO_WRITE_ON_IMPORT_MOVIE_KEY,
  series: NFO_WRITE_ON_IMPORT_SERIES_KEY,
  anime: NFO_WRITE_ON_IMPORT_ANIME_KEY,
};

const PLEXMATCH_WRITE_KEYS: Partial<Record<ViewCategoryId, string>> = {
  series: PLEXMATCH_WRITE_ON_IMPORT_SERIES_KEY,
  anime: PLEXMATCH_WRITE_ON_IMPORT_ANIME_KEY,
};

export function useMediaSettings({
  activeQualityScopeId,
  view,
}: UseMediaSettingsArgs): UseMediaSettingsResult {
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const client = useClient();
  const [moviesPath, setMoviesPath] = React.useState(DEFAULT_MOVIE_LIBRARY_PATH);
  const [seriesPath, setSeriesPath] = React.useState(DEFAULT_SERIES_LIBRARY_PATH);
  const [mediaSettingsLoading, setMediaSettingsLoading] = React.useState(false);
  const [mediaSettingsSaving, setMediaSettingsSaving] = React.useState(false);
  const [qualityProfiles, setQualityProfiles] = React.useState<SearchableQualityProfileBody[]>([]);
  const [qualityProfileParseError, setQualityProfileParseError] = React.useState("");
  const [globalQualityProfileId, setGlobalQualityProfileId] = React.useState("");
  const [categoryQualityProfileOverrides, setCategoryQualityProfileOverrides] = React.useState<
    Record<ViewCategoryId, string>
  >({
    movie: QUALITY_PROFILE_INHERIT_VALUE,
    series: QUALITY_PROFILE_INHERIT_VALUE,
    anime: QUALITY_PROFILE_INHERIT_VALUE,
  });
  const [categoryRenameTemplates, setCategoryRenameTemplates] = React.useState<
    Record<ViewCategoryId, string>
  >({
    movie: DEFAULT_RENAME_TEMPLATE,
    series: DEFAULT_RENAME_TEMPLATE,
    anime: DEFAULT_RENAME_TEMPLATE,
  });
  const [categoryRenameCollisionPolicies, setCategoryRenameCollisionPolicies] = React.useState<
    Record<ViewCategoryId, string>
  >({
    movie: DEFAULT_RENAME_COLLISION_POLICY,
    series: DEFAULT_RENAME_COLLISION_POLICY,
    anime: DEFAULT_RENAME_COLLISION_POLICY,
  });
  const [categoryRenameMissingMetadataPolicies, setCategoryRenameMissingMetadataPolicies] =
    React.useState<Record<ViewCategoryId, string>>({
      movie: DEFAULT_RENAME_MISSING_METADATA_POLICY,
      series: DEFAULT_RENAME_MISSING_METADATA_POLICY,
      anime: DEFAULT_RENAME_MISSING_METADATA_POLICY,
    });
  const [categoryFillerPolicies, setCategoryFillerPolicies] = React.useState<
    Record<ViewCategoryId, string>
  >({
    movie: DEFAULT_FILLER_POLICY,
    series: DEFAULT_FILLER_POLICY,
    anime: DEFAULT_FILLER_POLICY,
  });
  const [categoryRecapPolicies, setCategoryRecapPolicies] = React.useState<
    Record<ViewCategoryId, string>
  >({
    movie: DEFAULT_RECAP_POLICY,
    series: DEFAULT_RECAP_POLICY,
    anime: DEFAULT_RECAP_POLICY,
  });
  const [categoryMonitorSpecials, setCategoryMonitorSpecials] = React.useState<
    Record<ViewCategoryId, string>
  >({
    movie: "true",
    series: "true",
    anime: "false",
  });
  const [categoryInterSeasonMovies, setCategoryInterSeasonMovies] = React.useState<
    Record<ViewCategoryId, string>
  >({
    movie: "true",
    series: "true",
    anime: "true",
  });
  const [categoryPreferredSubGroup, setCategoryPreferredSubGroup] = React.useState<
    Record<ViewCategoryId, string>
  >({
    movie: "",
    series: "",
    anime: "",
  });
  const [nfoWriteOnImport, setNfoWriteOnImport] = React.useState<
    Record<ViewCategoryId, string>
  >({
    movie: "false",
    series: "false",
    anime: "false",
  });
  const [plexmatchWriteOnImport, setPlexmatchWriteOnImport] = React.useState<
    Record<ViewCategoryId, string>
  >({
    movie: "false",
    series: "false",
    anime: "false",
  });

  const normalizeQualityProfiles = React.useCallback(
    (rawValue: string) => {
      const resolved = resolveQualityProfileCatalogState(rawValue);
      const nextParseError =
        !resolved.isRawValid && rawValue.trim().length
          ? t("settings.qualityProfileCatalogInvalid")
          : "";

      setQualityProfileParseError((currentParseError) =>
        currentParseError === nextParseError ? currentParseError : nextParseError,
      );

      return resolved.profiles;
    },
    [t],
  );

  const normalizeRenameCollisionPolicy = React.useCallback((rawValue: string | null | undefined) => {
    const normalized = (rawValue || "").trim().toLowerCase();
    return ALLOWED_RENAME_COLLISION_POLICIES.has(normalized)
      ? normalized
      : DEFAULT_RENAME_COLLISION_POLICY;
  }, []);

  const normalizeRenameMissingMetadataPolicy = React.useCallback(
    (rawValue: string | null | undefined) => {
      const normalized = (rawValue || "").trim().toLowerCase();
      return ALLOWED_RENAME_MISSING_METADATA_POLICIES.has(normalized)
        ? normalized
        : DEFAULT_RENAME_MISSING_METADATA_POLICY;
    },
    [],
  );

  const normalizeFillerPolicy = React.useCallback((rawValue: string | null | undefined) => {
    const normalized = (rawValue || "").trim().toLowerCase();
    return ALLOWED_FILLER_POLICIES.has(normalized) ? normalized : DEFAULT_FILLER_POLICY;
  }, []);

  const normalizeRecapPolicy = React.useCallback((rawValue: string | null | undefined) => {
    const normalized = (rawValue || "").trim().toLowerCase();
    return ALLOWED_RECAP_POLICIES.has(normalized) ? normalized : DEFAULT_RECAP_POLICY;
  }, []);

  const applyMediaSettingsFromPayload = React.useCallback(
    (
      payload: AdminSettingsResponse,
      mediaPayload: AdminSettingsResponse,
      categoryPayloads: AdminSettingsResponse[] = [],
    ) => {
      const systemItemsByKey = Object.fromEntries(
        payload.items.map((item) => [item.keyName, item]),
      ) as Record<string, (typeof payload.items)[number]>;
      const moviePathRecord =
        mediaPayload.items.find((item) => item.keyName === MOVIE_FOLDER_KEY) ??
        payload.items.find((item) => item.keyName === MOVIE_FOLDER_KEY);
      const seriesPathRecord =
        mediaPayload.items.find((item) => item.keyName === SERIES_FOLDER_KEY) ??
        payload.items.find((item) => item.keyName === SERIES_FOLDER_KEY);
      const profileCatalogRecord = payload.items.find(
        (item) => item.keyName === QUALITY_PROFILE_CATALOG_KEY,
      );

      if (moviePathRecord) {
        const nextPath = getSettingDisplayValue(moviePathRecord);
        setMoviesPath((currentPath) => {
          const resolvedPath = nextPath || DEFAULT_MOVIE_LIBRARY_PATH;
          return currentPath === resolvedPath ? currentPath : resolvedPath;
        });
      }

      if (seriesPathRecord) {
        const nextPath = getSettingDisplayValue(seriesPathRecord);
        setSeriesPath((currentPath) => {
          const resolvedPath = nextPath || DEFAULT_SERIES_LIBRARY_PATH;
          return currentPath === resolvedPath ? currentPath : resolvedPath;
        });
      }

      const nextProfileText = payload.qualityProfiles ?? getSettingDisplayValue(profileCatalogRecord);
      const nextProfiles = normalizeQualityProfiles(nextProfileText);

      const globalProfileRecord = payload.items.find(
        (item) => item.keyName === QUALITY_PROFILE_ID_KEY,
      );
      const rawGlobalProfileId = getSettingDisplayValue(globalProfileRecord).trim();
      const resolvedGlobalId =
        rawGlobalProfileId && nextProfiles.some((p) => p.id === rawGlobalProfileId)
          ? rawGlobalProfileId
          : (nextProfiles[0]?.id ?? "");
      setGlobalQualityProfileId((current) =>
        current === resolvedGlobalId ? current : resolvedGlobalId,
      );

      setQualityProfiles((currentProfiles) =>
        currentProfiles.length === nextProfiles.length &&
        currentProfiles.every((profile, index) =>
          profile.id === nextProfiles[index]?.id &&
          profile.name === nextProfiles[index]?.name,
        )
          ? currentProfiles
          : nextProfiles,
      );

      setCategoryQualityProfileOverrides((previous) => {
        let hasUpdate = false;
        const next = { ...previous };
        for (const categoryBody of categoryPayloads) {
          const scopeId = categoryBody.scopeId as ViewCategoryId | undefined;
          if (!scopeId) {
            continue;
          }
          if (!QUALITY_PROFILE_SCOPE_IDS.includes(scopeId)) {
            continue;
          }

          const categoryProfileRecord = categoryBody.items.find(
            (item) => item.keyName === QUALITY_PROFILE_ID_KEY,
          );
          const categoryProfileValue = coerceProfileSetting(
            getSettingDisplayValue(categoryProfileRecord),
          );
          const nextValue = categoryProfileValue || QUALITY_PROFILE_INHERIT_VALUE;
          if (next[scopeId] !== nextValue) {
            next[scopeId] = nextValue;
            hasUpdate = true;
          }
        }
        return hasUpdate ? next : previous;
      });

      setCategoryRenameTemplates((previous) => {
        let hasUpdate = false;
        const next = { ...previous };

        for (const categoryBody of categoryPayloads) {
          const scopeId = categoryBody.scopeId as ViewCategoryId | undefined;
          if (!scopeId || !QUALITY_PROFILE_SCOPE_IDS.includes(scopeId)) {
            continue;
          }

          const categoryTemplateRecord = categoryBody.items.find(
            (item) => item.keyName === RENAME_TEMPLATE_KEY,
          );
          const scopedTemplate = getSettingDisplayValue(categoryTemplateRecord).trim();
          const globalTemplateRecord = systemItemsByKey[RENAME_TEMPLATE_GLOBAL_KEYS[scopeId]];
          const globalTemplate = getSettingDisplayValue(globalTemplateRecord).trim();
          const nextTemplate = scopedTemplate || globalTemplate || DEFAULT_RENAME_TEMPLATE;

          if (next[scopeId] !== nextTemplate) {
            next[scopeId] = nextTemplate;
            hasUpdate = true;
          }
        }

        return hasUpdate ? next : previous;
      });

      setCategoryRenameCollisionPolicies((previous) => {
        let hasUpdate = false;
        const next = { ...previous };
        const globalPolicy = normalizeRenameCollisionPolicy(
          getSettingDisplayValue(systemItemsByKey[RENAME_COLLISION_POLICY_GLOBAL_KEY]),
        );

        for (const categoryBody of categoryPayloads) {
          const scopeId = categoryBody.scopeId as ViewCategoryId | undefined;
          if (!scopeId || !QUALITY_PROFILE_SCOPE_IDS.includes(scopeId)) {
            continue;
          }

          const categoryPolicyRecord = categoryBody.items.find(
            (item) => item.keyName === RENAME_COLLISION_POLICY_KEY,
          );
          const scopedPolicy = normalizeRenameCollisionPolicy(
            getSettingDisplayValue(categoryPolicyRecord),
          );
          const isScopedValueSet = getSettingDisplayValue(categoryPolicyRecord).trim().length > 0;
          const nextPolicy = isScopedValueSet ? scopedPolicy : globalPolicy;
          if (next[scopeId] !== nextPolicy) {
            next[scopeId] = nextPolicy;
            hasUpdate = true;
          }
        }

        return hasUpdate ? next : previous;
      });

      setCategoryRenameMissingMetadataPolicies((previous) => {
        let hasUpdate = false;
        const next = { ...previous };
        const globalPolicy = normalizeRenameMissingMetadataPolicy(
          getSettingDisplayValue(systemItemsByKey[RENAME_MISSING_METADATA_POLICY_GLOBAL_KEY]),
        );

        for (const categoryBody of categoryPayloads) {
          const scopeId = categoryBody.scopeId as ViewCategoryId | undefined;
          if (!scopeId || !QUALITY_PROFILE_SCOPE_IDS.includes(scopeId)) {
            continue;
          }

          const categoryPolicyRecord = categoryBody.items.find(
            (item) => item.keyName === RENAME_MISSING_METADATA_POLICY_KEY,
          );
          const scopedPolicy = normalizeRenameMissingMetadataPolicy(
            getSettingDisplayValue(categoryPolicyRecord),
          );
          const isScopedValueSet = getSettingDisplayValue(categoryPolicyRecord).trim().length > 0;
          const nextPolicy = isScopedValueSet ? scopedPolicy : globalPolicy;
          if (next[scopeId] !== nextPolicy) {
            next[scopeId] = nextPolicy;
            hasUpdate = true;
          }
        }

        return hasUpdate ? next : previous;
      });

      setCategoryFillerPolicies((previous) => {
        const animeBody = categoryPayloads.find(
          (body) => body.scopeId === "anime",
        );
        if (!animeBody) return previous;

        const fillerRecord = animeBody.items.find(
          (item) => item.keyName === ANIME_FILLER_POLICY_KEY,
        );
        const nextPolicy = normalizeFillerPolicy(getSettingDisplayValue(fillerRecord));
        if (previous.anime === nextPolicy) return previous;
        return { ...previous, anime: nextPolicy };
      });

      setCategoryRecapPolicies((previous) => {
        const animeBody = categoryPayloads.find(
          (body) => body.scopeId === "anime",
        );
        if (!animeBody) return previous;

        const recapRecord = animeBody.items.find(
          (item) => item.keyName === ANIME_RECAP_POLICY_KEY,
        );
        const nextPolicy = normalizeRecapPolicy(getSettingDisplayValue(recapRecord));
        if (previous.anime === nextPolicy) return previous;
        return { ...previous, anime: nextPolicy };
      });

      setCategoryMonitorSpecials((previous) => {
        const animeBody = categoryPayloads.find(
          (body) => body.scopeId === "anime",
        );
        if (!animeBody) return previous;

        const monitorRecord = animeBody.items.find(
          (item) => item.keyName === ANIME_MONITOR_SPECIALS_KEY,
        );
        const rawValue = getSettingDisplayValue(monitorRecord).trim().toLowerCase();
        const nextValue = rawValue === "true" ? "true" : "false";
        if (previous.anime === nextValue) return previous;
        return { ...previous, anime: nextValue };
      });

      setCategoryInterSeasonMovies((previous) => {
        const animeBody = categoryPayloads.find(
          (body) => body.scopeId === "anime",
        );
        if (!animeBody) return previous;

        const record = animeBody.items.find(
          (item) => item.keyName === ANIME_INTER_SEASON_MOVIES_KEY,
        );
        const raw = getSettingDisplayValue(record).trim().toLowerCase();
        const next = raw === "false" ? "false" : "true";
        if (previous.anime === next) return previous;
        return { ...previous, anime: next };
      });

      setCategoryPreferredSubGroup((previous) => {
        const animeBody = categoryPayloads.find(
          (body) => body.scopeId === "anime",
        );
        if (!animeBody) return previous;

        const record = animeBody.items.find(
          (item) => item.keyName === ANIME_PREFERRED_SUB_GROUP_KEY,
        );
        const next = getSettingDisplayValue(record).trim();
        if (previous.anime === next) return previous;
        return { ...previous, anime: next };
      });

      // NFO write-on-import (system-scoped, keyed per facet)
      setNfoWriteOnImport((previous) => {
        let hasUpdate = false;
        const next = { ...previous };
        for (const [scopeId, key] of Object.entries(NFO_WRITE_KEYS)) {
          const raw = getSettingDisplayValue(systemItemsByKey[key]).trim().toLowerCase();
          const val = raw === "true" ? "true" : "false";
          if (next[scopeId as ViewCategoryId] !== val) {
            next[scopeId as ViewCategoryId] = val;
            hasUpdate = true;
          }
        }
        return hasUpdate ? next : previous;
      });

      // Plexmatch write-on-import (system-scoped, series + anime only)
      setPlexmatchWriteOnImport((previous) => {
        let hasUpdate = false;
        const next = { ...previous };
        for (const [scopeId, key] of Object.entries(PLEXMATCH_WRITE_KEYS)) {
          const raw = getSettingDisplayValue(systemItemsByKey[key]).trim().toLowerCase();
          const val = raw === "true" ? "true" : "false";
          if (next[scopeId as ViewCategoryId] !== val) {
            next[scopeId as ViewCategoryId] = val;
            hasUpdate = true;
          }
        }
        return hasUpdate ? next : previous;
      });
    },
    [
      normalizeQualityProfiles,
      normalizeRenameCollisionPolicy,
      normalizeRenameMissingMetadataPolicy,
      normalizeFillerPolicy,
      normalizeRecapPolicy,
    ],
  );

  const refreshMediaSettings = React.useCallback(async () => {
    setMediaSettingsLoading(true);
    try {
      const { data, error } = await client.query(mediaSettingsInitQuery, {}).toPromise();
      if (error) throw error;

      applyMediaSettingsFromPayload(
        data.systemSettings,
        data.mediaSettings,
        [data.movieSettings, data.seriesSettings, data.animeSettings],
      );
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToLoad"));
    } finally {
      setMediaSettingsLoading(false);
    }
  }, [applyMediaSettingsFromPayload, client, setGlobalStatus, t]);

  const updateCategoryMediaProfileSettings = React.useCallback(
    async (event: React.FormEvent<HTMLFormElement>) => {
      event.preventDefault();
      const path =
        view === "movies" ? moviesPath.trim() : view === "series" ? seriesPath.trim() : "";
      if ((view === "movies" || view === "series") && !path) {
        const requiredMessage =
          view === "movies"
            ? t("settings.moviesPathRequired")
            : t("settings.seriesPathRequired");
        setGlobalStatus(requiredMessage);
        return;
      }

      const selectedProfile = coerceProfileSetting(categoryQualityProfileOverrides[activeQualityScopeId]);
      const renameTemplate = categoryRenameTemplates[activeQualityScopeId].trim();
      const renameCollisionPolicy = normalizeRenameCollisionPolicy(
        categoryRenameCollisionPolicies[activeQualityScopeId],
      );
      const renameMissingMetadataPolicy = normalizeRenameMissingMetadataPolicy(
        categoryRenameMissingMetadataPolicies[activeQualityScopeId],
      );

      if (!renameTemplate) {
        setGlobalStatus(t("settings.renameTemplateRequired"));
        return;
      }
      if (
        selectedProfile !== QUALITY_PROFILE_INHERIT_VALUE &&
        !isValidProfileSelection(qualityProfiles, selectedProfile)
      ) {
        const invalidId = selectedProfile || t("label.default");
        setQualityProfileParseError(t("settings.qualityProfileUnknown", { id: invalidId }));
        setGlobalStatus(t("settings.qualityProfileUnknown", { id: invalidId }));
        return;
      }

      if (!qualityProfiles.length) {
        setQualityProfileParseError(t("settings.qualityProfileCatalogInvalid"));
        setGlobalStatus(t("settings.qualityProfileCatalogInvalid"));
        return;
      }

      setMediaSettingsSaving(true);
      setQualityProfileParseError("");

      try {
        const globalSaveItems: Array<{ keyName: string; value: string }> = [];
        let globalSaveResponse: { saveAdminSettings: AdminSettingsResponse } | null = null;

        if (view === "movies" || view === "series") {
          const pathKey = view === "movies" ? MOVIE_FOLDER_KEY : SERIES_FOLDER_KEY;
          globalSaveItems.push({ keyName: pathKey, value: path });
          const { data: globalData, error: globalError } = await client.mutation(
            saveAdminSettingsMutation,
            {
              input: {
                scope: "media",
                items: globalSaveItems,
              },
            },
          ).toPromise();
          if (globalError) throw globalError;
          globalSaveResponse = globalData;
        }

        const { data: categoryData, error: categoryError } = await client.mutation(
          saveAdminSettingsMutation,
          {
            input: {
              scope: "system",
              scopeId: activeQualityScopeId,
              items: [
                { keyName: QUALITY_PROFILE_ID_KEY, value: selectedProfile },
                { keyName: RENAME_TEMPLATE_KEY, value: renameTemplate },
                { keyName: RENAME_COLLISION_POLICY_KEY, value: renameCollisionPolicy },
                {
                  keyName: RENAME_MISSING_METADATA_POLICY_KEY,
                  value: renameMissingMetadataPolicy,
                },
                ...(activeQualityScopeId === "anime"
                  ? [
                      { keyName: ANIME_FILLER_POLICY_KEY, value: normalizeFillerPolicy(categoryFillerPolicies.anime) },
                      { keyName: ANIME_RECAP_POLICY_KEY, value: normalizeRecapPolicy(categoryRecapPolicies.anime) },
                      { keyName: ANIME_MONITOR_SPECIALS_KEY, value: categoryMonitorSpecials.anime },
                      { keyName: ANIME_INTER_SEASON_MOVIES_KEY, value: categoryInterSeasonMovies.anime },
                      { keyName: ANIME_PREFERRED_SUB_GROUP_KEY, value: categoryPreferredSubGroup.anime },
                    ]
                  : []),
              ],
            },
          },
        ).toPromise();
        if (categoryError) throw categoryError;
        const categoryResponse = categoryData;

        if ((view === "movies" || view === "series") && globalSaveResponse) {
          const pathKey = view === "movies" ? MOVIE_FOLDER_KEY : SERIES_FOLDER_KEY;
          const pathRecord = globalSaveResponse.saveAdminSettings.items.find(
            (item) => item.keyName === pathKey,
          );
          const nextPath = getSettingDisplayValue(pathRecord);
          if (view === "movies") {
            setMoviesPath(nextPath || DEFAULT_MOVIE_LIBRARY_PATH);
          } else {
            setSeriesPath(nextPath || DEFAULT_SERIES_LIBRARY_PATH);
          }
        } else if (view === "movies") {
          setMoviesPath(path || DEFAULT_MOVIE_LIBRARY_PATH);
        } else if (view === "series") {
          setSeriesPath(path || DEFAULT_SERIES_LIBRARY_PATH);
        }

        const categoryProfileRecord = categoryResponse.saveAdminSettings.items.find(
          (item: AdminSetting) => item.keyName === QUALITY_PROFILE_ID_KEY,
        );
        const persistedTemplateRecord = categoryResponse.saveAdminSettings.items.find(
          (item: AdminSetting) => item.keyName === RENAME_TEMPLATE_KEY,
        );
        const persistedCollisionPolicyRecord = categoryResponse.saveAdminSettings.items.find(
          (item: AdminSetting) => item.keyName === RENAME_COLLISION_POLICY_KEY,
        );
        const persistedMissingMetadataPolicyRecord = categoryResponse.saveAdminSettings.items.find(
          (item: AdminSetting) => item.keyName === RENAME_MISSING_METADATA_POLICY_KEY,
        );
        const persistedProfile = coerceProfileSetting(getSettingDisplayValue(categoryProfileRecord));
        setCategoryQualityProfileOverrides((previous) => ({
          ...previous,
          [activeQualityScopeId]: persistedProfile || QUALITY_PROFILE_INHERIT_VALUE,
        }));
        setCategoryRenameTemplates((previous) => ({
          ...previous,
          [activeQualityScopeId]: getSettingDisplayValue(persistedTemplateRecord).trim() || renameTemplate,
        }));
        setCategoryRenameCollisionPolicies((previous) => ({
          ...previous,
          [activeQualityScopeId]: normalizeRenameCollisionPolicy(
            getSettingDisplayValue(persistedCollisionPolicyRecord) || renameCollisionPolicy,
          ),
        }));
        setCategoryRenameMissingMetadataPolicies((previous) => ({
          ...previous,
          [activeQualityScopeId]: normalizeRenameMissingMetadataPolicy(
            getSettingDisplayValue(persistedMissingMetadataPolicyRecord) ||
              renameMissingMetadataPolicy,
          ),
        }));

        if (activeQualityScopeId === "anime") {
          const persistedFillerPolicyRecord = categoryResponse.saveAdminSettings.items.find(
            (item: AdminSetting) => item.keyName === ANIME_FILLER_POLICY_KEY,
          );
          setCategoryFillerPolicies((previous) => ({
            ...previous,
            anime: normalizeFillerPolicy(
              getSettingDisplayValue(persistedFillerPolicyRecord) || categoryFillerPolicies.anime,
            ),
          }));

          const persistedRecapPolicyRecord = categoryResponse.saveAdminSettings.items.find(
            (item: AdminSetting) => item.keyName === ANIME_RECAP_POLICY_KEY,
          );
          setCategoryRecapPolicies((previous) => ({
            ...previous,
            anime: normalizeRecapPolicy(
              getSettingDisplayValue(persistedRecapPolicyRecord) || categoryRecapPolicies.anime,
            ),
          }));

          const persistedMonitorSpecialsRecord = categoryResponse.saveAdminSettings.items.find(
            (item: AdminSetting) => item.keyName === ANIME_MONITOR_SPECIALS_KEY,
          );
          const rawMonitor = getSettingDisplayValue(persistedMonitorSpecialsRecord).trim().toLowerCase();
          setCategoryMonitorSpecials((previous) => ({
            ...previous,
            anime: rawMonitor === "true" ? "true" : "false",
          }));

          const persistedInterSeasonMoviesRecord = categoryResponse.saveAdminSettings.items.find(
            (item: AdminSetting) => item.keyName === ANIME_INTER_SEASON_MOVIES_KEY,
          );
          const rawInterSeason = getSettingDisplayValue(persistedInterSeasonMoviesRecord).trim().toLowerCase();
          setCategoryInterSeasonMovies((previous) => ({
            ...previous,
            anime: rawInterSeason === "false" ? "false" : "true",
          }));

          const persistedSubGroupRecord = categoryResponse.saveAdminSettings.items.find(
            (item: AdminSetting) => item.keyName === ANIME_PREFERRED_SUB_GROUP_KEY,
          );
          setCategoryPreferredSubGroup((previous) => ({
            ...previous,
            anime: getSettingDisplayValue(persistedSubGroupRecord).trim(),
          }));
        }

        // Save NFO/plexmatch sidecar settings (system scope, no scopeId)
        {
          const sidecarItems: Array<{ keyName: string; value: string }> = [];
          const nfoKey = NFO_WRITE_KEYS[activeQualityScopeId];
          if (nfoKey) {
            sidecarItems.push({ keyName: nfoKey, value: nfoWriteOnImport[activeQualityScopeId] });
          }
          const plexKey = PLEXMATCH_WRITE_KEYS[activeQualityScopeId];
          if (plexKey) {
            sidecarItems.push({ keyName: plexKey, value: plexmatchWriteOnImport[activeQualityScopeId] });
          }
          if (sidecarItems.length > 0) {
            const { error: sidecarError } = await client.mutation(
              saveAdminSettingsMutation,
              { input: { scope: "system", items: sidecarItems } },
            ).toPromise();
            if (sidecarError) throw sidecarError;
          }
        }

        const successMessage =
          view === "movies"
            ? t("settings.movieSettingsSaved")
            : view === "series"
              ? t("settings.seriesSettingsSaved")
              : t("settings.mediaSettingsSaved");
        setGlobalStatus(successMessage);
      } catch (error) {
        setGlobalStatus(error instanceof Error ? error.message : t("status.failedToUpdate"));
      } finally {
        setMediaSettingsSaving(false);
      }
    },
    [
      activeQualityScopeId,
      categoryFillerPolicies,
      categoryRecapPolicies,
      categoryInterSeasonMovies,
      categoryMonitorSpecials,
      categoryPreferredSubGroup,
      categoryQualityProfileOverrides,
      categoryRenameCollisionPolicies,
      categoryRenameMissingMetadataPolicies,
      categoryRenameTemplates,
      nfoWriteOnImport,
      plexmatchWriteOnImport,
      normalizeFillerPolicy,
      normalizeRecapPolicy,
      normalizeRenameCollisionPolicy,
      normalizeRenameMissingMetadataPolicy,
      qualityProfiles,
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
      const normalizedCategoryProfile = coerceProfileSetting(categoryQualityProfileOverrides[scopeId]);
      return (
        normalizedCategoryProfile !== QUALITY_PROFILE_INHERIT_VALUE &&
        !isValidProfileSelection(qualityProfiles, normalizedCategoryProfile)
      );
    });

    if (!hasInvalidProfile) {
      setQualityProfileParseError("");
      return;
    }

    const invalidValue = Object.entries(categoryQualityProfileOverrides).find(([scopeId, profileId]) => {
      const normalizedProfileId = coerceProfileSetting(profileId);
      const isScopeAllowed = QUALITY_PROFILE_SCOPE_IDS.includes(scopeId as (typeof QUALITY_PROFILE_SCOPE_IDS)[number]);
      const isInvalid =
        normalizedProfileId !== QUALITY_PROFILE_INHERIT_VALUE &&
        !isValidProfileSelection(qualityProfiles, normalizedProfileId);
      return isScopeAllowed && isInvalid;
    })?.[1];

    const invalidProfileId = invalidValue
      ? coerceProfileSetting(invalidValue)
      : t("label.default");
    setQualityProfileParseError(t("settings.qualityProfileUnknown", { id: invalidProfileId }));
  }, [categoryQualityProfileOverrides, qualityProfiles, t]);

  React.useEffect(() => {
    refreshCategoryValidation();
  }, [refreshCategoryValidation]);

  return {
    moviesPath,
    setMoviesPath,
    seriesPath,
    setSeriesPath,
    mediaSettingsLoading,
    mediaSettingsSaving,
    qualityProfiles,
    qualityProfileParseError,
    globalQualityProfileId,
    categoryQualityProfileOverrides,
    setCategoryQualityProfileOverrides,
    categoryRenameTemplates,
    setCategoryRenameTemplates,
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
    categoryPreferredSubGroup,
    setCategoryPreferredSubGroup,
    nfoWriteOnImport,
    setNfoWriteOnImport,
    plexmatchWriteOnImport,
    setPlexmatchWriteOnImport,
    updateCategoryMediaProfileSettings,
    refreshMediaSettings,
    refreshCategoryValidation,
  };
}
