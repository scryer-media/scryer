import * as React from "react";
import { deleteQualityProfileMutation, saveAdminSettingsMutation } from "@/lib/graphql/mutations";
import { qualityProfilesInitQuery } from "@/lib/graphql/queries";
import { useClient } from "urql";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import {
  buildQualityProfileTemplate,
  coerceProfileSetting,
  createUniqueProfileId,
  dedupeOrdered,
  isValidProfileSelection,
  normalizeProfileId,
  normalizeProfileIdFromName,
  normalizeQualityProfilesForSave,
  normalizeQualityProfilesForUi,
  parseQualityProfileCatalog,
  parseQualityProfileCatalogEntries,
  qualityProfileCatalogEntryFromDraft,
  resolveQualityProfileCatalogState,
  sortStringByNumericDesc,
  toProfileOptions,
  toQualityProfileDraft,
} from "@/lib/utils/quality-profiles";
import {
  AUDIO_CODEC_CHOICES,
  QUALITY_SOURCE_CHOICES,
  QUALITY_TIER_CHOICES,
  VIDEO_CODEC_CHOICES,
} from "@/lib/constants/quality-profiles";
import {
  QUALITY_PROFILE_CATALOG_KEY,
  QUALITY_PROFILE_ID_KEY,
  QUALITY_PROFILE_INHERIT_VALUE,
  QUALITY_PROFILE_SCOPE_IDS,
} from "@/lib/constants/settings";
import { useSettingsSubscription } from "@/lib/hooks/use-settings-subscription";
import { getSettingDisplayValue } from "@/lib/utils/settings";
import type {
  AdminSetting,
  AdminSettingsResponse,
  CommittedQualityProfileDraft,
  DownloadClientRecord,
  ParsedQualityProfile,
  ParsedQualityProfileEntry,
  QualityProfileCriteriaPayload,
  QualityProfileDraft,
  QualityProfileListField,
  ViewCategoryId,
} from "@/lib/types";

const DEFAULT_CATEGORY_QUALITY_PROFILES: Record<ViewCategoryId, string> = {
  movie: QUALITY_PROFILE_INHERIT_VALUE,
  series: QUALITY_PROFILE_INHERIT_VALUE,
  anime: QUALITY_PROFILE_INHERIT_VALUE,
};

const DEFAULT_CATEGORY_QUALITY_SAVING: Record<ViewCategoryId, boolean> = {
  movie: false,
  series: false,
  anime: false,
};

function resolveGlobalQualityProfileId(
  profiles: ParsedQualityProfile[],
  candidate: string | null | undefined,
): string {
  const normalized = normalizeProfileId(candidate ?? "");
  if (
    normalized &&
    normalized !== QUALITY_PROFILE_INHERIT_VALUE &&
    profiles.some((profile) => profile.id === normalized)
  ) {
    return normalized;
  }

  return profiles[0]?.id ?? "default";
}

type UseQualityProfilesManagerArgs = Record<string, never>;

export type UseQualityProfilesManagerResult = {
  mediaSettingsLoading: boolean;
  initialLoadComplete: boolean;
  qualityProfilesSaving: boolean;
  qualityProfiles: ParsedQualityProfile[];
  qualityProfileParseError: string;
  qualityProfileDraft: QualityProfileDraft;
  updateQualityProfileDraft: (
    patch: Partial<QualityProfileDraft> | ((current: QualityProfileDraft) => QualityProfileDraft),
  ) => void;
  commitQualityProfileDraftToCatalog: () => CommittedQualityProfileDraft | null;
  availableSourceAllowlist: typeof QUALITY_SOURCE_CHOICES;
  availableVideoCodecAllowlist: typeof VIDEO_CODEC_CHOICES;
  availableAudioCodecAllowlist: typeof AUDIO_CODEC_CHOICES;
  activeQualityProfileTierOptions: string[];
  availableQualityTiers: Array<{ value: string; label: string }>;
  archivalQualityOptions: Array<{ value: string; label: string }>;
  activeSourceAllowlist: string[];
  activeSourceBlocklist: string[];
  activeVideoCodecAllowlist: string[];
  activeVideoCodecBlocklist: string[];
  activeAudioCodecAllowlist: string[];
  activeAudioCodecBlocklist: string[];
  qualityCategoryLabels: Record<ViewCategoryId, string>;
  getQualityProfileCriteria: (profileId: string) => QualityProfileCriteriaPayload | undefined;
  getQualityProfileBoolean: (
    profileId: string,
    field: keyof QualityProfileCriteriaPayload,
    fallback: boolean,
  ) => boolean;
  loadQualityProfileById: (profileId: string) => void;
  moveProfileListToAllowed: (
    allowedField: QualityProfileListField,
    deniedField: QualityProfileListField,
    value: string,
  ) => void;
  moveProfileListToDenied: (
    allowedField: QualityProfileListField,
    deniedField: QualityProfileListField,
    value: string,
  ) => void;
  addQualityTier: (qualityTier: string) => void;
  removeQualityTier: (qualityTier: string) => void;
  updateQualityProfilesGlobal: (event?: React.FormEvent<HTMLFormElement>) => Promise<void> | void;
  saveGlobalQualityProfile: (value: string) => Promise<void> | void;
  globalQualityProfileId: string;
  setGlobalQualityProfileId: (value: string) => void;
  categoryQualityProfileOverrides: Record<ViewCategoryId, string>;
  setCategoryQualityProfileOverrides: React.Dispatch<
    React.SetStateAction<Record<ViewCategoryId, string>>
  >;
  categoryQualityProfileSaving: Record<ViewCategoryId, boolean>;
  saveCategoryQualityProfile: (scopeId: ViewCategoryId, value: string) => Promise<void> | void;
  deleteQualityProfile: (profileId: string) => Promise<void>;
  refreshQualityProfiles: () => Promise<void>;
  downloadClients: DownloadClientRecord[];
  toProfileOptions: typeof toProfileOptions;
};

export function useQualityProfilesManager(
  _args: UseQualityProfilesManagerArgs = {},
): UseQualityProfilesManagerResult {
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const client = useClient();
  const [mediaSettingsLoading, setMediaSettingsLoading] = React.useState(false);
  const [qualityProfilesSaving, setQualityProfilesSaving] = React.useState(false);
  const [qualityProfilesText, setQualityProfilesText] = React.useState("");
  const [qualityProfiles, setQualityProfiles] = React.useState<ParsedQualityProfile[]>([]);
  const [qualityProfileParseError, setQualityProfileParseError] = React.useState("");
  const [qualityProfileDraft, setQualityProfileDraft] = React.useState<QualityProfileDraft>(() =>
    toQualityProfileDraft(null, "default", "4K"),
  );
  const [qualityProfileDraftOriginalName, setQualityProfileDraftOriginalName] = React.useState("");
  const [globalQualityProfileId, setGlobalQualityProfileId] = React.useState("default");
  const [categoryQualityProfileOverrides, setCategoryQualityProfileOverrides] = React.useState<
    Record<ViewCategoryId, string>
  >({ ...DEFAULT_CATEGORY_QUALITY_PROFILES });
  const [categoryQualityProfileSaving, setCategoryQualityProfileSaving] = React.useState<
    Record<ViewCategoryId, boolean>
  >({ ...DEFAULT_CATEGORY_QUALITY_SAVING });
  const [downloadClients, setDownloadClients] = React.useState<DownloadClientRecord[]>([]);
  const [initialLoadComplete, setInitialLoadComplete] = React.useState(false);

  const [, setSelectedQualityProfileId] = React.useState("default");

  const qualityProfileCatalogEntries = React.useMemo(
    () => parseQualityProfileCatalogEntries(qualityProfilesText),
    [qualityProfilesText],
  );

  const qualityProfileEntryById = React.useMemo(() => {
    const map = new Map<string, ParsedQualityProfileEntry>();
    qualityProfileCatalogEntries.forEach((entry) => {
      if (typeof entry.id === "string" && entry.id.trim()) {
        map.set(entry.id.trim(), entry);
      }
    });
    return map;
  }, [qualityProfileCatalogEntries]);

  const activeQualityProfileTierOptions = React.useMemo(
    () =>
      dedupeOrdered(qualityProfileDraft.quality_tiers)
        .filter((value) => value.length > 0)
        .sort(sortStringByNumericDesc),
    [qualityProfileDraft.quality_tiers],
  );
  const availableQualityTiers = React.useMemo(
    () =>
      QUALITY_TIER_CHOICES.filter(
        (option) =>
          !activeQualityProfileTierOptions.some(
            (value) => value.toUpperCase() === option.value.toUpperCase(),
          ),
      ).sort(
        (left, right) =>
          sortStringByNumericDesc(left.label, right.label) ||
          left.value.localeCompare(right.value),
      ),
    [activeQualityProfileTierOptions],
  );
  const archivalQualityOptions = React.useMemo(
    () => [
      { value: "__default__", label: t("qualityProfile.useDefaultQualityFallback") },
      ...activeQualityProfileTierOptions.map((value) => ({ value, label: value })),
    ],
    [activeQualityProfileTierOptions, t],
  );
  const activeSourceAllowlist = React.useMemo(
    () => dedupeOrdered(qualityProfileDraft.source_allowlist).filter((v) => v.length > 0),
    [qualityProfileDraft.source_allowlist],
  );
  const activeSourceBlocklist = React.useMemo(
    () => dedupeOrdered(qualityProfileDraft.source_blocklist).filter((v) => v.length > 0),
    [qualityProfileDraft.source_blocklist],
  );
  const activeVideoCodecAllowlist = React.useMemo(
    () => dedupeOrdered(qualityProfileDraft.video_codec_allowlist).filter((v) => v.length > 0),
    [qualityProfileDraft.video_codec_allowlist],
  );
  const activeVideoCodecBlocklist = React.useMemo(
    () => dedupeOrdered(qualityProfileDraft.video_codec_blocklist).filter((v) => v.length > 0),
    [qualityProfileDraft.video_codec_blocklist],
  );
  const activeAudioCodecAllowlist = React.useMemo(
    () => dedupeOrdered(qualityProfileDraft.audio_codec_allowlist).filter((v) => v.length > 0),
    [qualityProfileDraft.audio_codec_allowlist],
  );
  const activeAudioCodecBlocklist = React.useMemo(
    () => dedupeOrdered(qualityProfileDraft.audio_codec_blocklist).filter((v) => v.length > 0),
    [qualityProfileDraft.audio_codec_blocklist],
  );
  const qualityCategoryLabels = React.useMemo(
    () =>
      ({
        movie: t("search.facetMovie"),
        series: t("search.facetTv"),
        anime: t("search.facetAnime"),
      }) as Record<ViewCategoryId, string>,
    [t],
  );

  const getQualityProfileCriteria = React.useCallback(
    (profileId: string) => qualityProfileEntryById.get(profileId)?.criteria,
    [qualityProfileEntryById],
  );

  const getQualityProfileBoolean = React.useCallback(
    (
      profileId: string,
      field: keyof QualityProfileCriteriaPayload,
      fallback: boolean,
    ): boolean => {
      const criteria = qualityProfileEntryById.get(profileId)?.criteria;
      const value = criteria?.[field];
      return typeof value === "boolean" ? value : fallback;
    },
    [qualityProfileEntryById],
  );

  const applyQualityProfileSettingsFromAdminPayload = React.useCallback(
    (payload: AdminSettingsResponse, categoryPayloads: AdminSettingsResponse[] = [], preserveProfileId?: string) => {
      const mediaItems = payload?.items || [];
      const profileCatalogRecord = mediaItems.find((item) => item.keyName === QUALITY_PROFILE_CATALOG_KEY);
      const globalProfileRecord = mediaItems.find((item) => item.keyName === QUALITY_PROFILE_ID_KEY);

      const nextProfileText =
        payload.qualityProfiles ?? getSettingDisplayValue(profileCatalogRecord);
      const resolved = resolveQualityProfileCatalogState(nextProfileText);
      const resolvedProfiles = resolved.profiles;

      if (!resolved.isRawValid && nextProfileText.trim().length) {
        setQualityProfileParseError(t("settings.qualityProfileCatalogInvalid"));
      } else {
        setQualityProfileParseError("");
      }

      const validGlobalProfile = resolveGlobalQualityProfileId(
        resolvedProfiles,
        getSettingDisplayValue(globalProfileRecord),
      );

      const catalogEntries = resolved.entries;
      const defaultDraftSource =
        catalogEntries.find((entry) => entry.id === "default") ?? catalogEntries[0] ?? null;
      const candidateProfileId = preserveProfileId?.trim() || "";
      const preservedProfileSource = candidateProfileId
        ? catalogEntries.find((entry) => entry.id === candidateProfileId) ?? null
        : null;
      const nextDraftSource = preservedProfileSource ?? defaultDraftSource;
      const nextDefaultDraft = nextDraftSource
        ? toQualityProfileDraft(nextDraftSource, nextDraftSource.id, nextDraftSource.name || "4K")
        : buildQualityProfileTemplate(
            resolvedProfiles[0]?.id ?? "default",
            resolvedProfiles[0]?.name || "default",
          );
      const nextDraftId =
        nextDraftSource?.id ?? defaultDraftSource?.id ?? resolvedProfiles[0]?.id ?? "default";

      setQualityProfilesText(resolved.text);
      setQualityProfiles(resolvedProfiles);
      setSelectedQualityProfileId(nextDraftId);
      setQualityProfileDraft(nextDefaultDraft);
      setQualityProfileDraftOriginalName(nextDefaultDraft.name);
      setGlobalQualityProfileId(validGlobalProfile);

      setCategoryQualityProfileOverrides((previous) => {
        const next = { ...previous };
        let hasUpdate = false;
        for (const categoryBody of categoryPayloads) {
          const scopeId = categoryBody.scopeId as ViewCategoryId | undefined;
          if (!scopeId) continue;
          if (!QUALITY_PROFILE_SCOPE_IDS.includes(scopeId as (typeof QUALITY_PROFILE_SCOPE_IDS)[number])) {
            continue;
          }
          const categoryProfileRecord = categoryBody.items.find(
            (item) => item.keyName === QUALITY_PROFILE_ID_KEY,
          );
          const hasExplicitOverride = categoryProfileRecord?.hasOverride ?? false;
          const nextValue = hasExplicitOverride
            ? (coerceProfileSetting(getSettingDisplayValue(categoryProfileRecord)) || QUALITY_PROFILE_INHERIT_VALUE)
            : QUALITY_PROFILE_INHERIT_VALUE;
          if (next[scopeId] !== nextValue) {
            next[scopeId] = nextValue;
            hasUpdate = true;
          }
        }
        return hasUpdate ? next : previous;
      });
    },
    [t],
  );

  const deleteQualityProfile = React.useCallback(
    async (profileId: string) => {
      const trimmed = profileId.trim();
      if (!trimmed) return;

      setQualityProfilesSaving(true);
      try {
        const { data, error } = await client.mutation(
          deleteQualityProfileMutation,
          { input: { profileId: trimmed } },
        ).toPromise();
        if (error) throw error;

        applyQualityProfileSettingsFromAdminPayload(data.deleteQualityProfile, []);
        setGlobalStatus(t("settings.qualitySettingsSaved"));
      } catch (error) {
        setGlobalStatus(error instanceof Error ? error.message : t("status.failedToDelete"));
      } finally {
        setQualityProfilesSaving(false);
      }
    },
    [applyQualityProfileSettingsFromAdminPayload, client, setGlobalStatus, t],
  );

  const refreshQualityProfiles = React.useCallback(async () => {
    setMediaSettingsLoading(true);
    try {
      const { data, error } = await client.query(qualityProfilesInitQuery, {}).toPromise();
      if (error) throw error;

      setDownloadClients(data.downloadClientConfigs || []);
      applyQualityProfileSettingsFromAdminPayload(
        data.systemSettings,
        [data.movieSettings, data.seriesSettings, data.animeSettings],
      );
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToLoad"));
    } finally {
      setMediaSettingsLoading(false);
      setInitialLoadComplete(true);
    }
  }, [applyQualityProfileSettingsFromAdminPayload, client, setGlobalStatus, t]);

  React.useEffect(() => {
    void refreshQualityProfiles();
  }, [refreshQualityProfiles]);

  useSettingsSubscription(
    React.useCallback(
      (keys: string[]) => {
        if (keys.includes(QUALITY_PROFILE_CATALOG_KEY) || keys.includes(QUALITY_PROFILE_ID_KEY)) {
          void refreshQualityProfiles();
        }
      },
      [refreshQualityProfiles],
    ),
  );

  const loadQualityProfileById = React.useCallback(
    (profileId: string) => {
      const selectedEntry = qualityProfileCatalogEntries.find((entry) => entry.id.trim() === profileId);
      if (!selectedEntry) return;
      setSelectedQualityProfileId(profileId);
      const nextDraft = toQualityProfileDraft(selectedEntry, profileId, profileId);
      setQualityProfileDraft(nextDraft);
      setQualityProfileDraftOriginalName(nextDraft.name);
      setQualityProfileParseError("");
    },
    [qualityProfileCatalogEntries],
  );

  const setQualityProfileDraftAndCatalog = React.useCallback(
    (
      patch: Partial<QualityProfileDraft> | ((current: QualityProfileDraft) => QualityProfileDraft),
    ) => {
      setQualityProfileDraft((current) =>
        typeof patch === "function" ? patch(current) : { ...current, ...patch },
      );
    },
    [],
  );

  const updateQualityProfileDraft = React.useCallback(
    (
      patch: Partial<QualityProfileDraft> | ((current: QualityProfileDraft) => QualityProfileDraft),
    ) => {
      setQualityProfileDraftAndCatalog(patch);
    },
    [setQualityProfileDraftAndCatalog],
  );

  const commitQualityProfileDraftToCatalog = React.useCallback((): CommittedQualityProfileDraft | null => {
    const catalogRaw = qualityProfilesText.trim();
    if (!catalogRaw) {
      setQualityProfileParseError(t("settings.qualityProfileCatalogInvalid"));
      setGlobalStatus(t("settings.qualityProfileCatalogInvalid"));
      return null;
    }

    let parsedCatalog: unknown;
    try {
      parsedCatalog = JSON.parse(catalogRaw);
    } catch {
      setQualityProfileParseError(t("settings.qualityProfileCatalogInvalid"));
      setGlobalStatus(t("settings.qualityProfileCatalogInvalid"));
      return null;
    }
    if (!Array.isArray(parsedCatalog)) {
      setQualityProfileParseError(t("settings.qualityProfileCatalogInvalid"));
      setGlobalStatus(t("settings.qualityProfileCatalogInvalid"));
      return null;
    }

    const sourceEntries = parseQualityProfileCatalogEntries(catalogRaw);
    const nextIdFromDraft = qualityProfileDraft.id;
    const nextName = qualityProfileDraft.name.trim();
    if (!nextName) {
      setQualityProfileParseError(t("settings.qualityProfileNameRequired"));
      setGlobalStatus(t("settings.qualityProfileNameRequired"));
      return null;
    }

    const originalName = qualityProfileDraftOriginalName.trim();
    const hasNameChange = Boolean(originalName) && originalName !== nextName;
    const existingIds = sourceEntries.map((entry) => entry.id.trim()).filter((entryId) => entryId.length > 0);
    const shouldCreateNewProfile =
      hasNameChange || !sourceEntries.some((entry) => entry.id === nextIdFromDraft);

    const nextDraft: QualityProfileDraft = {
      ...qualityProfileDraft,
      name: nextName,
      id: hasNameChange
        ? createUniqueProfileId(normalizeProfileIdFromName(nextName), existingIds)
        : nextIdFromDraft,
    };
    const nextDraftEntry = qualityProfileCatalogEntryFromDraft(nextDraft);
    const nextEntries = shouldCreateNewProfile
      ? [...sourceEntries, nextDraftEntry]
      : sourceEntries.map((entry) => (entry.id === nextIdFromDraft ? nextDraftEntry : entry));
    const normalized = normalizeQualityProfilesForUi(JSON.stringify(nextEntries));

    setQualityProfilesText(normalized);
    setQualityProfiles(parseQualityProfileCatalog(normalized));
    setSelectedQualityProfileId(nextDraft.id);
    setQualityProfileDraft(nextDraft);
    setQualityProfileDraftOriginalName(nextDraft.name);
    setQualityProfileParseError("");

    return { catalogText: normalized, draftEntry: nextDraftEntry };
  }, [qualityProfilesText, qualityProfileDraft, qualityProfileDraftOriginalName, t, setGlobalStatus]);

  const addQualityTier = React.useCallback(
    (qualityTier: string) => {
      const normalized = qualityTier.trim().toUpperCase();
      if (!normalized) return;
      updateQualityProfileDraft((current) => ({
        ...current,
        quality_tiers: dedupeOrdered([...current.quality_tiers, normalized]),
      }));
    },
    [updateQualityProfileDraft],
  );

  const removeQualityTier = React.useCallback(
    (qualityTier: string) => {
      updateQualityProfileDraft((current) => ({
        ...current,
        quality_tiers: current.quality_tiers.filter((value) => value !== qualityTier),
      }));
    },
    [updateQualityProfileDraft],
  );

  const moveProfileListItem = React.useCallback(
    (
      allowedField: QualityProfileListField,
      deniedField: QualityProfileListField,
      direction: "allowed" | "denied",
      value: string,
    ) => {
      const normalized = value.trim();
      if (!normalized) return;
      updateQualityProfileDraft((current) => {
        const nextAllowed = new Set(current[allowedField]);
        const nextDenied = new Set(current[deniedField]);
        if (direction === "allowed") {
          if (nextAllowed.size > 0) nextAllowed.add(normalized);
          nextDenied.delete(normalized);
        } else {
          nextDenied.add(normalized);
          nextAllowed.delete(normalized);
        }
        return {
          ...current,
          [allowedField]: dedupeOrdered(Array.from(nextAllowed)),
          [deniedField]: dedupeOrdered(Array.from(nextDenied)),
        };
      });
    },
    [updateQualityProfileDraft],
  );

  const moveProfileListToAllowed = React.useCallback(
    (allowedField: QualityProfileListField, deniedField: QualityProfileListField, value: string) =>
      moveProfileListItem(allowedField, deniedField, "allowed", value),
    [moveProfileListItem],
  );

  const moveProfileListToDenied = React.useCallback(
    (allowedField: QualityProfileListField, deniedField: QualityProfileListField, value: string) =>
      moveProfileListItem(allowedField, deniedField, "denied", value),
    [moveProfileListItem],
  );

  const updateQualityProfilesGlobal = React.useCallback(
    async (event?: React.FormEvent<HTMLFormElement>) => {
      if (qualityProfilesSaving) {
        return;
      }

      event?.preventDefault();

      const committed = commitQualityProfileDraftToCatalog();
      if (committed === null) return;

      const normalizedCatalogText = normalizeQualityProfilesForSave(committed.catalogText.trim());
      const parsedProfiles = parseQualityProfileCatalog(normalizedCatalogText);
      if (!parsedProfiles.length) {
        setQualityProfileParseError(t("settings.qualityProfileCatalogInvalid"));
        setGlobalStatus(t("settings.qualityProfileCatalogInvalid"));
        return;
      }

      const normalizedGlobalProfile = resolveGlobalQualityProfileId(
        parsedProfiles,
        globalQualityProfileId,
      );

      setQualityProfilesSaving(true);
      setQualityProfileParseError("");
      try {
        const catalogPatchText = normalizeQualityProfilesForSave(
          JSON.stringify([committed.draftEntry]),
        );
        const { data: globalData, error: globalError } = await client.mutation(
          saveAdminSettingsMutation,
          {
            input: {
              scope: "system",
              items: [
                { keyName: QUALITY_PROFILE_CATALOG_KEY, value: catalogPatchText },
                { keyName: QUALITY_PROFILE_ID_KEY, value: normalizedGlobalProfile },
              ],
            },
          },
        ).toPromise();
        if (globalError) throw globalError;

        applyQualityProfileSettingsFromAdminPayload(globalData.saveAdminSettings, [], committed.draftEntry.id);
        setGlobalStatus(t("settings.qualitySettingsSaved"));
      } catch (error) {
        setGlobalStatus(error instanceof Error ? error.message : t("status.failedToUpdate"));
      } finally {
        setQualityProfilesSaving(false);
      }
    },
    [
      applyQualityProfileSettingsFromAdminPayload,
      client,
      commitQualityProfileDraftToCatalog,
      globalQualityProfileId,
      qualityProfilesSaving,
      setGlobalStatus,
      t,
    ],
  );

  const saveGlobalQualityProfile = React.useCallback(
    async (rawValue: string) => {
      if (!qualityProfiles.length) {
        setGlobalStatus(t("qualityProfile.noProfilesFound"));
        return;
      }

      const normalizedValue = resolveGlobalQualityProfileId(
        qualityProfiles,
        normalizeProfileId(rawValue),
      );

      setQualityProfileParseError("");
      setQualityProfilesSaving(true);

      try {
        const { data: profileData, error: profileError } = await client.mutation(
          saveAdminSettingsMutation,
          {
            input: {
              scope: "system",
              items: [{ keyName: QUALITY_PROFILE_ID_KEY, value: normalizedValue }],
            },
          },
        ).toPromise();
        if (profileError) throw profileError;

        const record = profileData.saveAdminSettings.items.find(
          (item: AdminSetting) => item.keyName === QUALITY_PROFILE_ID_KEY,
        );
        const persisted = resolveGlobalQualityProfileId(
          qualityProfiles,
          getSettingDisplayValue(record),
        );
        setGlobalQualityProfileId(persisted);
        const message = t("settings.qualitySettingsSaved");
        setGlobalStatus(message);
      } catch (error) {
        const message = error instanceof Error ? error.message : t("status.failedToUpdate");
        setGlobalStatus(message);
      } finally {
        setQualityProfilesSaving(false);
      }
    },
    [qualityProfiles, client, setGlobalStatus, t],
  );

  const saveCategoryQualityProfile = React.useCallback(
    async (scopeId: ViewCategoryId, value: string) => {
      const normalizedScope = QUALITY_PROFILE_SCOPE_IDS.includes(
        scopeId as (typeof QUALITY_PROFILE_SCOPE_IDS)[number],
      )
        ? scopeId
        : ("movie" as ViewCategoryId);
      const normalizedValue = coerceProfileSetting(value);

      if (
        normalizedValue !== QUALITY_PROFILE_INHERIT_VALUE &&
        !isValidProfileSelection(qualityProfiles, normalizedValue)
      ) {
        const message = t("settings.qualityProfileUnknown", {
          id: normalizedValue || t("label.default"),
        });
        setQualityProfileParseError(message);
        setGlobalStatus(message);
        return;
      }

      setQualityProfileParseError("");
      setCategoryQualityProfileSaving((previous) => ({
        ...previous,
        [normalizedScope]: true,
      }));

      try {
        const { data: categoryData, error: categoryError } = await client.mutation(
          saveAdminSettingsMutation,
          {
            input: {
              scope: "system",
              scopeId: normalizedScope,
              items: [
                {
                  keyName: QUALITY_PROFILE_ID_KEY,
                  value: normalizedValue,
                },
              ],
            },
          },
        ).toPromise();
        if (categoryError) throw categoryError;

        const record = categoryData.saveAdminSettings.items.find(
          (item: AdminSetting) => item.keyName === QUALITY_PROFILE_ID_KEY,
        );
        const hasOverride = record?.hasOverride ?? false;
        const persisted = hasOverride
          ? coerceProfileSetting(getSettingDisplayValue(record))
          : QUALITY_PROFILE_INHERIT_VALUE;
        setCategoryQualityProfileOverrides((previous) => ({
          ...previous,
          [normalizedScope]: persisted || QUALITY_PROFILE_INHERIT_VALUE,
        }));
        const message = t("settings.qualitySettingsSaved");
        setGlobalStatus(message);
      } catch (error) {
        const message = error instanceof Error ? error.message : t("status.failedToUpdate");
        setGlobalStatus(message);
      } finally {
        setCategoryQualityProfileSaving((previous) => ({
          ...previous,
          [normalizedScope]: false,
        }));
      }
    },
    [qualityProfiles, client, setGlobalStatus, t],
  );

  return {
    mediaSettingsLoading,
    initialLoadComplete,
    qualityProfilesSaving,
    qualityProfiles,
    qualityProfileParseError,
    qualityProfileDraft,
    updateQualityProfileDraft,
    commitQualityProfileDraftToCatalog,
    availableSourceAllowlist: QUALITY_SOURCE_CHOICES,
    availableVideoCodecAllowlist: VIDEO_CODEC_CHOICES,
    availableAudioCodecAllowlist: AUDIO_CODEC_CHOICES,
    activeQualityProfileTierOptions,
    availableQualityTiers,
    archivalQualityOptions,
    activeSourceAllowlist,
    activeSourceBlocklist,
    activeVideoCodecAllowlist,
    activeVideoCodecBlocklist,
    activeAudioCodecAllowlist,
    activeAudioCodecBlocklist,
    qualityCategoryLabels,
    getQualityProfileCriteria,
    getQualityProfileBoolean,
    loadQualityProfileById,
    moveProfileListToAllowed,
    moveProfileListToDenied,
    addQualityTier,
    removeQualityTier,
    updateQualityProfilesGlobal,
    saveGlobalQualityProfile,
    globalQualityProfileId,
    setGlobalQualityProfileId,
    categoryQualityProfileOverrides,
    setCategoryQualityProfileOverrides,
    categoryQualityProfileSaving,
    saveCategoryQualityProfile,
    deleteQualityProfile,
    refreshQualityProfiles,
    downloadClients,
    toProfileOptions,
  };
}
