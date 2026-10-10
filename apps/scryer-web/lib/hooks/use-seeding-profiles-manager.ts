import * as React from "react";
import { useClient } from "urql";

import {
  createSeedingProfileMutation,
  deleteSeedingProfileMutation,
  setDefaultSeedingProfileMutation,
  setMinimumSeedersFloorMutation,
  updateSeedingProfileMutation,
} from "@/lib/graphql/mutations";
import {
  defaultSeedingProfileQuery,
  seedingProfilesQuery,
} from "@/lib/graphql/queries";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useTranslate } from "@/lib/context/translate-context";
import { useSettingsSubscription } from "@/lib/hooks/use-settings-subscription";
import type {
  SeedingProfileDraft,
  SeedingProfileRecord,
} from "@/lib/types/seeding-profiles";
import {
  buildSeedingProfileTemplate,
  DEFAULT_SEEDING_PROFILE_SETTING_KEY,
  extractSeedingProfileErrorMessage,
  seedingProfileToDraft,
  toCreateSeedingProfileInput,
  toUpdateSeedingProfileInput,
  validateSeedingProfileDraft,
} from "@/lib/utils/seeding-profiles";

export type SeedingProfilesManager = {
  loading: boolean;
  saving: boolean;
  profiles: SeedingProfileRecord[];
  defaultProfileId: string | null;
  /** Verbatim server message from the last failed load/save/delete. */
  errorMessage: string;
  clearErrorMessage: () => void;
  draft: SeedingProfileDraft;
  setDraft: React.Dispatch<React.SetStateAction<SeedingProfileDraft>>;
  saveProfile: (event?: React.FormEvent<HTMLFormElement>) => Promise<boolean>;
  deleteProfile: (profileId: string) => Promise<boolean>;
  loadProfileById: (profileId: string) => void;
  resetDraft: () => void;
  setDefaultProfile: (profileId: string | null) => Promise<boolean>;
  /** System floor applied when no profile covers the indexer. */
  minimumSeedersFloor: number;
  setMinimumSeedersFloor: (floor: number) => Promise<boolean>;
  refreshProfiles: () => Promise<void>;
};

export function useSeedingProfilesManager(): SeedingProfilesManager {
  const client = useClient();
  const t = useTranslate();
  const showStatus = useGlobalStatus();

  const [loading, setLoading] = React.useState(true);
  const [saving, setSaving] = React.useState(false);
  const [profiles, setProfiles] = React.useState<SeedingProfileRecord[]>([]);
  const [defaultProfileId, setDefaultProfileId] = React.useState<string | null>(
    null,
  );
  const [minimumSeedersFloor, setMinimumSeedersFloorState] =
    React.useState<number>(1);
  const [draft, setDraft] = React.useState<SeedingProfileDraft>(() =>
    buildSeedingProfileTemplate(),
  );
  const [errorMessage, setErrorMessage] = React.useState("");

  const clearErrorMessage = React.useCallback(() => setErrorMessage(""), []);

  const loadProfiles = React.useCallback(async () => {
    setLoading(true);
    try {
      const [listResult, defaultResult] = await Promise.all([
        client
          .query(seedingProfilesQuery, {}, { requestPolicy: "network-only" })
          .toPromise(),
        client
          .query(
            defaultSeedingProfileQuery,
            {},
            { requestPolicy: "network-only" },
          )
          .toPromise(),
      ]);
      if (listResult.error) throw listResult.error;
      if (defaultResult.error) throw defaultResult.error;
      setProfiles((listResult.data?.seedingProfiles ?? []) as SeedingProfileRecord[]);
      setDefaultProfileId(
        defaultResult.data?.defaultSeedingProfile?.seedingProfileId ?? null,
      );
      setMinimumSeedersFloorState(
        defaultResult.data?.defaultSeedingProfile?.minimumSeedersFloor ?? 1,
      );
      setErrorMessage("");
    } catch (err) {
      setErrorMessage(
        extractSeedingProfileErrorMessage(err) ?? t("status.failedToLoad"),
      );
    } finally {
      setLoading(false);
    }
  }, [client, t]);

  React.useEffect(() => {
    void loadProfiles();
  }, [loadProfiles]);

  useSettingsSubscription(
    React.useCallback(
      (keys: string[]) => {
        if (keys.includes(DEFAULT_SEEDING_PROFILE_SETTING_KEY)) {
          void loadProfiles();
        }
      },
      [loadProfiles],
    ),
  );

  const saveProfile = React.useCallback(
    async (event?: React.FormEvent<HTMLFormElement>) => {
      event?.preventDefault();
      const validationError = validateSeedingProfileDraft(draft);
      if (validationError) {
        setErrorMessage(validationError);
        showStatus(validationError);
        return false;
      }

      const isNew = !draft.id;
      setSaving(true);
      try {
        const result = isNew
          ? await client
              .mutation(createSeedingProfileMutation, {
                input: toCreateSeedingProfileInput(draft),
              })
              .toPromise()
          : await client
              .mutation(updateSeedingProfileMutation, {
                input: toUpdateSeedingProfileInput(draft),
              })
              .toPromise();
        if (result.error) throw result.error;

        const saved = (isNew
          ? result.data?.createSeedingProfile
          : result.data?.updateSeedingProfile) as
          | SeedingProfileRecord
          | undefined;
        if (saved) {
          setProfiles((previous) =>
            isNew
              ? [...previous, saved]
              : previous.map((profile) =>
                  profile.id === saved.id ? saved : profile,
                ),
          );
        }
        setDraft(buildSeedingProfileTemplate());
        setErrorMessage("");
        showStatus(t("settings.seedingProfileSaved"), { level: "SUCCESS" });
        return true;
      } catch (err) {
        const message =
          extractSeedingProfileErrorMessage(err) ??
          t("settings.seedingProfileSaveError");
        setErrorMessage(message);
        showStatus(message, { level: "ERROR" });
        return false;
      } finally {
        setSaving(false);
      }
    },
    [client, draft, showStatus, t],
  );

  const deleteProfile = React.useCallback(
    async (profileId: string) => {
      setSaving(true);
      try {
        const result = await client
          .mutation(deleteSeedingProfileMutation, { id: profileId })
          .toPromise();
        if (result.error) throw result.error;
        setProfiles((previous) =>
          previous.filter((profile) => profile.id !== profileId),
        );
        setErrorMessage("");
        showStatus(t("settings.seedingProfileDeleted"), { level: "SUCCESS" });
        return true;
      } catch (err) {
        // The backend names every indexer, routing entry, and the global
        // default still pointing at this profile. Surface it as-is.
        const message =
          extractSeedingProfileErrorMessage(err) ??
          t("settings.seedingProfileDeleteError");
        setErrorMessage(message);
        showStatus(message, { level: "ERROR" });
        return false;
      } finally {
        setSaving(false);
      }
    },
    [client, showStatus, t],
  );

  const setMinimumSeedersFloor = React.useCallback(
    async (floor: number) => {
      const previousFloor = minimumSeedersFloor;
      setMinimumSeedersFloorState(floor);
      setSaving(true);
      try {
        const result = await client
          .mutation(setMinimumSeedersFloorMutation, {
            input: { minimumSeedersFloor: floor },
          })
          .toPromise();
        if (result.error) throw result.error;
        setMinimumSeedersFloorState(
          result.data?.setMinimumSeedersFloor?.minimumSeedersFloor ?? floor,
        );
        setErrorMessage("");
        showStatus(t("settings.seedingMinimumSeedersFloorSaved"), { level: "SUCCESS" });
        return true;
      } catch (err) {
        setMinimumSeedersFloorState(previousFloor);
        const message =
          extractSeedingProfileErrorMessage(err) ??
          t("settings.seedingProfileSaveError");
        setErrorMessage(message);
        showStatus(message, { level: "ERROR" });
        return false;
      } finally {
        setSaving(false);
      }
    },
    [client, minimumSeedersFloor, showStatus, t],
  );

  const setDefaultProfile = React.useCallback(
    async (profileId: string | null) => {
      const previousDefault = defaultProfileId;
      setDefaultProfileId(profileId);
      setSaving(true);
      try {
        const result = await client
          .mutation(setDefaultSeedingProfileMutation, {
            input: { seedingProfileId: profileId },
          })
          .toPromise();
        if (result.error) throw result.error;
        setDefaultProfileId(
          result.data?.setDefaultSeedingProfile?.seedingProfileId ?? null,
        );
        setErrorMessage("");
        showStatus(t("settings.seedingProfileDefaultSaved"), { level: "SUCCESS" });
        return true;
      } catch (err) {
        setDefaultProfileId(previousDefault);
        const message =
          extractSeedingProfileErrorMessage(err) ??
          t("settings.seedingProfileSaveError");
        setErrorMessage(message);
        showStatus(message, { level: "ERROR" });
        return false;
      } finally {
        setSaving(false);
      }
    },
    [client, defaultProfileId, showStatus, t],
  );

  const loadProfileById = React.useCallback(
    (profileId: string) => {
      const found = profiles.find((profile) => profile.id === profileId);
      if (found) {
        setDraft(seedingProfileToDraft(found));
      }
    },
    [profiles],
  );

  const resetDraft = React.useCallback(() => {
    setDraft(buildSeedingProfileTemplate());
  }, []);

  return {
    loading,
    saving,
    profiles,
    defaultProfileId,
    errorMessage,
    clearErrorMessage,
    draft,
    setDraft,
    saveProfile,
    deleteProfile,
    loadProfileById,
    resetDraft,
    setDefaultProfile,
    minimumSeedersFloor,
    setMinimumSeedersFloor,
    refreshProfiles: loadProfiles,
  };
}
