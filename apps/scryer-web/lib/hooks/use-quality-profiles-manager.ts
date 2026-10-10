import * as React from "react";
import {
  deleteQualityProfileMutation,
  saveQualityProfileSettingsMutation,
} from "@/lib/graphql/mutations";
import { qualityProfilesInitQuery } from "@/lib/graphql/queries";
import { useClient } from "urql";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import {
  commitQualityProfileDraftToEntries,
  hasDuplicateQualityProfileName,
} from "@/lib/utils/quality-profile-draft-commit";
import {
  buildQualityProfileTemplate,
  buildDefaultCategoryPersonaSelections,
  coerceProfileSetting,
  createUniqueProfileId,
  dedupeOrdered,
  isValidProfileSelection,
  moveQualityTier,
  normalizeProfileId,
  normalizeQualityProfilesForUi,
  parseQualityProfileCatalog,
  qualityProfileSettingsToCatalogText,
  qualityProfileSettingsToCategoryOverrides,
  qualityProfileSettingsToCategoryPersonaSelections,
  qualityProfileEntryToMutationInput,
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
  SCORING_PERSONA_KEY,
  QUALITY_PROFILE_SCOPE_IDS,
} from "@/lib/constants/settings";
import { useSettingsSubscription } from "@/lib/hooks/use-settings-subscription";
import type {
  CommittedQualityProfileDraft,
  DownloadClientRecord,
  FacetScoringPersonaSelectionRecord,
  ParsedQualityProfile,
  ParsedQualityProfileEntry,
  QualityProfileCriteriaPayload,
  QualityProfileDraft,
  QualityProfileSettingsPayload,
  QualityProfileListField,
  ScoringPersonaId,
  ViewCategoryId,
} from "@/lib/types";

const DEFAULT_CATEGORY_QUALITY_PROFILES: Record<ViewCategoryId, string> = {
  MOVIE: QUALITY_PROFILE_INHERIT_VALUE,
  SERIES: QUALITY_PROFILE_INHERIT_VALUE,
  ANIME: QUALITY_PROFILE_INHERIT_VALUE,
};

const DEFAULT_CATEGORY_QUALITY_SAVING: Record<ViewCategoryId, boolean> = {
  MOVIE: false,
  SERIES: false,
  ANIME: false,
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

const QUALITY_PROFILES_SAVED_TOAST_ID = "settings-quality-profiles-saved-toast";

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
  startNewQualityProfileDraft: () => void;
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
  reorderQualityTier: (qualityTier: string, targetIndex: number) => void;
  updateQualityProfilesGlobal: (event?: React.FormEvent<HTMLFormElement>) => Promise<boolean> | boolean;
  saveGlobalQualityProfile: (value: string) => Promise<void> | void;
  saveGlobalScoringPersona: (persona: ScoringPersonaId) => Promise<void> | void;
  globalQualityProfileId: string;
  setGlobalQualityProfileId: (value: string) => void;
  globalScoringPersona: ScoringPersonaId;
  categoryQualityProfileOverrides: Record<ViewCategoryId, string>;
  setCategoryQualityProfileOverrides: React.Dispatch<
    React.SetStateAction<Record<ViewCategoryId, string>>
  >;
  categoryPersonaSelections: Record<ViewCategoryId, FacetScoringPersonaSelectionRecord>;
  saveCategoryScoringPersona: (
    scopeId: ViewCategoryId,
    persona: ScoringPersonaId | null,
  ) => Promise<void> | void;
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
  const [qualityProfileCatalogEntriesState, setQualityProfileCatalogEntriesState] = React.useState<
    ParsedQualityProfileEntry[]
  >([]);
  const [qualityProfiles, setQualityProfiles] = React.useState<ParsedQualityProfile[]>([]);
  // Monotonic counter of applied settings payloads; lets in-flight refetches
  // detect that a newer (mutation-authoritative) payload landed and discard
  // their stale snapshot instead of clobbering it.
  const qualityProfileApplyEpochRef = React.useRef(0);
  const [qualityProfileParseError, setQualityProfileParseError] = React.useState("");
  const [qualityProfileDraft, setQualityProfileDraft] = React.useState<QualityProfileDraft>(() =>
    toQualityProfileDraft(null, "default", "4K"),
  );
  const [globalQualityProfileId, setGlobalQualityProfileId] = React.useState("default");
  const [globalScoringPersona, setGlobalScoringPersona] =
    React.useState<ScoringPersonaId>("BALANCED");
  const [categoryQualityProfileOverrides, setCategoryQualityProfileOverrides] = React.useState<
    Record<ViewCategoryId, string>
  >({ ...DEFAULT_CATEGORY_QUALITY_PROFILES });
  const [categoryPersonaSelections, setCategoryPersonaSelections] = React.useState<
    Record<ViewCategoryId, FacetScoringPersonaSelectionRecord>
  >({ ...buildDefaultCategoryPersonaSelections() });
  const [categoryQualityProfileSaving, setCategoryQualityProfileSaving] = React.useState<
    Record<ViewCategoryId, boolean>
  >({ ...DEFAULT_CATEGORY_QUALITY_SAVING });
  const [downloadClients, setDownloadClients] = React.useState<DownloadClientRecord[]>([]);
  const [initialLoadComplete, setInitialLoadComplete] = React.useState(false);

  const [, setSelectedQualityProfileId] = React.useState("default");

  const qualityProfileCatalogEntries = qualityProfileCatalogEntriesState;

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
        .filter((value) => value.length > 0),
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
        MOVIE: t("search.facetMovie"),
        SERIES: t("search.facetSeries"),
        ANIME: t("search.facetAnime"),
      }) as Record<ViewCategoryId, string>,
    [t],
  );
  const categoryPersonaSelectionInputs = React.useMemo(
    () =>
      Object.values(categoryPersonaSelections).map((selection) => ({
        scope: selection.scope,
        persona: selection.overridePersona,
        inheritGlobal: selection.inheritsGlobal,
      })),
    [categoryPersonaSelections],
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

  const applyQualityProfileSettingsPayload = React.useCallback(
    (
      payload: QualityProfileSettingsPayload | null | undefined,
      preserveProfileId?: string,
      options?: { preserveDraft?: boolean },
    ) => {
      // Every apply advances the epoch so in-flight event-triggered refetches
      // that started before this (authoritative) payload can be discarded.
      qualityProfileApplyEpochRef.current += 1;
      const resolved = resolveQualityProfileCatalogState(
        qualityProfileSettingsToCatalogText(payload),
      );
      const resolvedProfiles = resolved.profiles;

      setQualityProfileParseError("");

      const validGlobalProfile = resolveGlobalQualityProfileId(
        resolvedProfiles,
        payload?.globalProfileId,
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

      setQualityProfileCatalogEntriesState(catalogEntries);
      setQualityProfiles(resolvedProfiles);
      if (!options?.preserveDraft) {
        setSelectedQualityProfileId(nextDraftId);
        setQualityProfileDraft(nextDefaultDraft);
      }
      setGlobalQualityProfileId(validGlobalProfile);
      setGlobalScoringPersona(payload?.globalScoringPersona ?? "BALANCED");

      const nextOverrides = qualityProfileSettingsToCategoryOverrides(payload);
      setCategoryQualityProfileOverrides((previous) =>
        QUALITY_PROFILE_SCOPE_IDS.every((scopeId) => previous[scopeId] === nextOverrides[scopeId])
          ? previous
          : nextOverrides,
      );
      const nextPersonaSelections =
        qualityProfileSettingsToCategoryPersonaSelections(payload);
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
    },
    [],
  );

  const deleteQualityProfile = React.useCallback(
    async (profileId: string) => {
      const trimmed = profileId.trim();
      if (!trimmed) return;

      setQualityProfilesSaving(true);
      try {
        const { data, error } = await client.mutation(
          deleteQualityProfileMutation,
          { id: trimmed },
        ).toPromise();
        if (error) throw error;

        applyQualityProfileSettingsPayload(data.deleteQualityProfile);
        setGlobalStatus(t("settings.qualitySettingsSaved"), {
          level: "SUCCESS",
          toastId: QUALITY_PROFILES_SAVED_TOAST_ID,
        });
      } catch (error) {
        setGlobalStatus(error instanceof Error ? error.message : t("status.failedToDelete"), { level: "ERROR" });
      } finally {
        setQualityProfilesSaving(false);
      }
    },
    [applyQualityProfileSettingsPayload, client, setGlobalStatus, t],
  );

  const refreshQualityProfiles = React.useCallback(
    async (options?: { preserveDraft?: boolean }) => {
      setMediaSettingsLoading(true);
      const epochAtStart = qualityProfileApplyEpochRef.current;
      try {
        const { data, error } = await client
          .query(qualityProfilesInitQuery, {}, { requestPolicy: "network-only" })
          .toPromise();
        if (error) throw error;

        // A mutation applied its authoritative payload while this refetch was
        // in flight; this snapshot is stale and must not clobber that state.
        if (qualityProfileApplyEpochRef.current !== epochAtStart) {
          return;
        }
        setDownloadClients(data.downloadClientConfigs || []);
        applyQualityProfileSettingsPayload(
          data.qualityProfileSettings,
          undefined,
          options,
        );
      } catch (error) {
        setGlobalStatus(error instanceof Error ? error.message : t("status.failedToLoad"), { level: "ERROR" });
      } finally {
        setMediaSettingsLoading(false);
        setInitialLoadComplete(true);
      }
    },
    [applyQualityProfileSettingsPayload, client, setGlobalStatus, t],
  );

  React.useEffect(() => {
    void refreshQualityProfiles();
  }, [refreshQualityProfiles]);

  useSettingsSubscription(
    React.useCallback(
      (keys: string[]) => {
        if (
          keys.includes(QUALITY_PROFILE_CATALOG_KEY) ||
          keys.includes(QUALITY_PROFILE_ID_KEY) ||
          keys.includes(SCORING_PERSONA_KEY)
        ) {
          // Event-triggered refreshes update the list but never the operator's
          // in-progress draft/selection (an unsaved editor must survive them).
          void refreshQualityProfiles({ preserveDraft: true });
        }
      },
      [refreshQualityProfiles],
    ),
  );

  const loadQualityProfileById = React.useCallback(
    (profileId: string) => {
      const selectedEntry = qualityProfileCatalogEntries.find(
        (entry) => entry.id.trim() === profileId,
      );
      if (!selectedEntry) return;
      setSelectedQualityProfileId(profileId);
      const nextDraft = toQualityProfileDraft(selectedEntry, profileId, profileId);
      setQualityProfileDraft(nextDraft);
      setQualityProfileParseError("");
    },
    [qualityProfileCatalogEntries],
  );

  const startNewQualityProfileDraft = React.useCallback(() => {
    const existingIds = qualityProfileCatalogEntries
      .map((entry) => entry.id.trim())
      .filter((entryId) => entryId.length > 0);
    const nextProfileId = createUniqueProfileId("quality-profile", existingIds);
    const nextDraft = buildQualityProfileTemplate(nextProfileId, "");
    setSelectedQualityProfileId(nextProfileId);
    setQualityProfileDraft(nextDraft);
    setQualityProfileParseError("");
  }, [qualityProfileCatalogEntries]);

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
    const sourceEntries = qualityProfileCatalogEntries;
    const nextName = qualityProfileDraft.name.trim();
    if (!nextName) {
      setQualityProfileParseError(t("settings.qualityProfileNameRequired"));
      setGlobalStatus(t("settings.qualityProfileNameRequired"));
      return null;
    }

    if (hasDuplicateQualityProfileName(sourceEntries, nextName, qualityProfileDraft.id)) {
      setQualityProfileParseError(t("settings.qualityProfileNameDuplicate"));
      setGlobalStatus(t("settings.qualityProfileNameDuplicate"));
      return null;
    }

    const committed = commitQualityProfileDraftToEntries(sourceEntries, {
      ...qualityProfileDraft,
      name: nextName,
    });
    const normalized = normalizeQualityProfilesForUi(JSON.stringify(committed.catalogEntries));

    setQualityProfileCatalogEntriesState(committed.catalogEntries);
    setQualityProfiles(parseQualityProfileCatalog(normalized));
    setSelectedQualityProfileId(committed.draftEntry.id);
    setQualityProfileDraft(
      toQualityProfileDraft(
        committed.draftEntry,
        committed.draftEntry.id,
        committed.draftEntry.name,
      ),
    );
    setQualityProfileParseError("");

    return committed;
  }, [
    qualityProfileCatalogEntries,
    qualityProfileDraft,
    setGlobalStatus,
    t,
  ]);

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

  const reorderQualityTier = React.useCallback(
    (qualityTier: string, targetIndex: number) => {
      updateQualityProfileDraft((current) => ({
        ...current,
        quality_tiers: moveQualityTier(current.quality_tiers, qualityTier, targetIndex),
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
        return false;
      }

      event?.preventDefault();

      const committed = commitQualityProfileDraftToCatalog();
      if (committed === null) return false;
      const parsedEntries = committed.catalogEntries;
      const parsedProfiles = parsedEntries.map(({ id, name }) => ({ id, name }));

      const normalizedGlobalProfile = resolveGlobalQualityProfileId(
        parsedProfiles,
        globalQualityProfileId,
      );

      setQualityProfilesSaving(true);
      setQualityProfileParseError("");
      try {
        const { data: globalData, error: globalError } = await client.mutation(
          saveQualityProfileSettingsMutation,
          {
            input: {
              profiles: parsedEntries.map(qualityProfileEntryToMutationInput),
              globalProfileId: normalizedGlobalProfile,
              globalScoringPersona,
              categorySelections: [],
              categoryPersonaSelections: categoryPersonaSelectionInputs,
              replaceExisting: true,
            },
          },
        ).toPromise();
        if (globalError) throw globalError;

        applyQualityProfileSettingsPayload(
          globalData.saveQualityProfileSettings,
          committed.draftEntry.id,
        );
        setGlobalStatus(t("settings.qualitySettingsSaved"), {
          level: "SUCCESS",
          toastId: QUALITY_PROFILES_SAVED_TOAST_ID,
        });
        return true;
      } catch (error) {
        setGlobalStatus(error instanceof Error ? error.message : t("status.failedToUpdate"), { level: "ERROR" });
        return false;
      } finally {
        setQualityProfilesSaving(false);
      }
    },
    [
      applyQualityProfileSettingsPayload,
      categoryPersonaSelectionInputs,
      client,
      commitQualityProfileDraftToCatalog,
      globalQualityProfileId,
      globalScoringPersona,
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
          saveQualityProfileSettingsMutation,
          {
            input: {
              profiles: [],
              globalProfileId: normalizedValue,
              globalScoringPersona,
              categorySelections: [],
              categoryPersonaSelections: categoryPersonaSelectionInputs,
              replaceExisting: false,
            },
          },
        ).toPromise();
        if (profileError) throw profileError;

        const persisted = resolveGlobalQualityProfileId(
          qualityProfiles,
          profileData.saveQualityProfileSettings.globalProfileId,
        );
        setGlobalQualityProfileId(persisted);
        const message = t("settings.qualitySettingsSaved");
        setGlobalStatus(message, {
          level: "SUCCESS",
          toastId: QUALITY_PROFILES_SAVED_TOAST_ID,
        });
      } catch (error) {
        const message = error instanceof Error ? error.message : t("status.failedToUpdate");
        setGlobalStatus(message, { level: "ERROR" });
      } finally {
        setQualityProfilesSaving(false);
      }
    },
    [
      categoryPersonaSelectionInputs,
      client,
      globalScoringPersona,
      qualityProfiles,
      setGlobalStatus,
      t,
    ],
  );

  const saveGlobalScoringPersona = React.useCallback(
    async (persona: ScoringPersonaId) => {
      setQualityProfilesSaving(true);
      try {
        const { data, error } = await client
          .mutation(saveQualityProfileSettingsMutation, {
            input: {
              profiles: [],
              globalProfileId: null,
              globalScoringPersona: persona,
              categorySelections: [],
              categoryPersonaSelections: [],
              replaceExisting: false,
            },
          })
          .toPromise();
        if (error) throw error;

        applyQualityProfileSettingsPayload(data?.saveQualityProfileSettings);
        setGlobalStatus(t("settings.qualitySettingsSaved"), {
          level: "SUCCESS",
          toastId: QUALITY_PROFILES_SAVED_TOAST_ID,
        });
      } catch (error) {
        setGlobalStatus(
          error instanceof Error ? error.message : t("status.failedToUpdate"),
          { level: "ERROR" },
        );
      } finally {
        setQualityProfilesSaving(false);
      }
    },
    [applyQualityProfileSettingsPayload, client, setGlobalStatus, t],
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
        setGlobalStatus(message, { level: "WARNING" });
        return;
      }

      setQualityProfileParseError("");
      setCategoryQualityProfileSaving((previous) => ({
        ...previous,
        [normalizedScope]: true,
      }));

      try {
        const { data: categoryData, error: categoryError } = await client.mutation(
          saveQualityProfileSettingsMutation,
          {
            input: {
              profiles: [],
              globalProfileId: null,
              globalScoringPersona,
              categorySelections: [
                {
                  scope: normalizedScope,
                  profileId:
                    normalizedValue === QUALITY_PROFILE_INHERIT_VALUE ? null : normalizedValue,
                  inheritGlobal: normalizedValue === QUALITY_PROFILE_INHERIT_VALUE,
                },
              ],
              categoryPersonaSelections: categoryPersonaSelectionInputs,
              replaceExisting: false,
            },
          },
        ).toPromise();
        if (categoryError) throw categoryError;

        const persisted =
          qualityProfileSettingsToCategoryOverrides(categoryData.saveQualityProfileSettings)[
            normalizedScope
          ];
        setCategoryQualityProfileOverrides((previous) => ({
          ...previous,
          [normalizedScope]: persisted || QUALITY_PROFILE_INHERIT_VALUE,
        }));
        const message = t("settings.qualitySettingsSaved");
        setGlobalStatus(message, {
          level: "SUCCESS",
          toastId: QUALITY_PROFILES_SAVED_TOAST_ID,
        });
      } catch (error) {
        const message = error instanceof Error ? error.message : t("status.failedToUpdate");
        setGlobalStatus(message, { level: "ERROR" });
      } finally {
        setCategoryQualityProfileSaving((previous) => ({
          ...previous,
          [normalizedScope]: false,
        }));
      }
    },
    [
      categoryPersonaSelectionInputs,
      client,
      globalScoringPersona,
      qualityProfiles,
      setGlobalStatus,
      t,
    ],
  );

  const saveCategoryScoringPersona = React.useCallback(
    async (scopeId: ViewCategoryId, persona: ScoringPersonaId | null) => {
      const normalizedScope = QUALITY_PROFILE_SCOPE_IDS.includes(
        scopeId as (typeof QUALITY_PROFILE_SCOPE_IDS)[number],
      )
        ? scopeId
        : ("movie" as ViewCategoryId);

      setCategoryQualityProfileSaving((previous) => ({
        ...previous,
        [normalizedScope]: true,
      }));
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
                  scope: normalizedScope,
                  persona,
                  inheritGlobal: persona === null,
                },
              ],
              replaceExisting: false,
            },
          })
          .toPromise();
        if (error) throw error;

        applyQualityProfileSettingsPayload(data?.saveQualityProfileSettings);
        setGlobalStatus(t("settings.qualitySettingsSaved"), {
          level: "SUCCESS",
          toastId: QUALITY_PROFILES_SAVED_TOAST_ID,
        });
      } catch (error) {
        setGlobalStatus(
          error instanceof Error ? error.message : t("status.failedToUpdate"),
          { level: "ERROR" },
        );
      } finally {
        setCategoryQualityProfileSaving((previous) => ({
          ...previous,
          [normalizedScope]: false,
        }));
      }
    },
    [applyQualityProfileSettingsPayload, client, setGlobalStatus, t],
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
    startNewQualityProfileDraft,
    moveProfileListToAllowed,
    moveProfileListToDenied,
    addQualityTier,
    removeQualityTier,
    reorderQualityTier,
    updateQualityProfilesGlobal,
    saveGlobalQualityProfile,
    saveGlobalScoringPersona,
    globalQualityProfileId,
    setGlobalQualityProfileId,
    globalScoringPersona,
    categoryQualityProfileOverrides,
    setCategoryQualityProfileOverrides,
    categoryPersonaSelections,
    saveCategoryScoringPersona,
    categoryQualityProfileSaving,
    saveCategoryQualityProfile,
    deleteQualityProfile,
    refreshQualityProfiles,
    downloadClients,
    toProfileOptions,
  };
}
