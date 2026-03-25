import * as React from "react";
import { saveAdminSettingsMutation } from "@/lib/graphql/mutations";
import { adminSettingsQuery } from "@/lib/graphql/queries";
import { useClient } from "urql";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import {
  buildDelayProfileTemplate,
  createDelayProfileId,
  DELAY_PROFILE_CATALOG_KEY,
  parseDelayProfileCatalog,
  serializeDelayProfileCatalog,
  validateDelayProfileDraft,
} from "@/lib/utils/delay-profiles";
import { getSettingStringFromItems } from "@/lib/utils/settings";
import type { AdminSetting } from "@/lib/types";
import type { DelayProfileDraft, ParsedDelayProfile } from "@/lib/types/delay-profiles";
import { useSettingsSubscription } from "@/lib/hooks/use-settings-subscription";

export function useDelayProfilesManager() {
  const client = useClient();
  const t = useTranslate();
  const showStatus = useGlobalStatus();

  const [loading, setLoading] = React.useState(true);
  const [saving, setSaving] = React.useState(false);
  const [profiles, setProfiles] = React.useState<ParsedDelayProfile[]>([]);
  const [draft, setDraft] = React.useState<DelayProfileDraft>(() =>
    buildDelayProfileTemplate([]),
  );
  const [parseError, setParseError] = React.useState("");

  const loadProfiles = React.useCallback(async () => {
    setLoading(true);
    try {
      const result = await client.query(adminSettingsQuery, {
        scope: "system",
        category: "acquisition",
      });
      const items: AdminSetting[] =
        result.data?.adminSettings?.items ?? [];
      const catalogJson = getSettingStringFromItems(
        items,
        DELAY_PROFILE_CATALOG_KEY,
        "[]",
      );
      const parsed = parseDelayProfileCatalog(catalogJson);
      setProfiles(parsed);
      setParseError("");
    } catch (err) {
      setParseError(String(err));
    } finally {
      setLoading(false);
    }
  }, [client]);

  React.useEffect(() => {
    loadProfiles();
  }, [loadProfiles]);

  useSettingsSubscription(
    React.useCallback(
      (keys: string[]) => {
        if (keys.includes(DELAY_PROFILE_CATALOG_KEY)) {
          loadProfiles();
        }
      },
      [loadProfiles],
    ),
  );

  const saveProfiles = React.useCallback(
    async (nextProfiles: ParsedDelayProfile[]) => {
      setSaving(true);
      try {
        const catalogText = serializeDelayProfileCatalog(nextProfiles);
        const result = await client.mutation(saveAdminSettingsMutation, {
          input: {
            scope: "system",
            items: [{ keyName: DELAY_PROFILE_CATALOG_KEY, value: catalogText }],
          },
        });
        if (result.error) {
          showStatus(t("settings.delayProfileSaveError"));
          return;
        }
        setProfiles(nextProfiles);
        showStatus(t("settings.delayProfilesSaved"));
      } catch {
        showStatus(t("settings.delayProfileSaveError"));
      } finally {
        setSaving(false);
      }
    },
    [client, t, showStatus],
  );

  const saveProfile = React.useCallback(
    async (event?: React.FormEvent<HTMLFormElement>) => {
      event?.preventDefault();
      if (!draft.name.trim()) {
        showStatus(t("settings.delayProfileNameRequired"));
        return;
      }
      const isNew = !draft.id;
      const id = isNew
        ? createDelayProfileId(draft.name, profiles)
        : draft.id;
      const entry: ParsedDelayProfile = { ...draft, id };
      const validationError = validateDelayProfileDraft(entry);
      if (validationError) {
        showStatus(validationError);
        return;
      }
      const next = isNew
        ? [...profiles, entry]
        : profiles.map((p) => (p.id === id ? entry : p));
      await saveProfiles(next);
      setDraft(buildDelayProfileTemplate(next));
    },
    [draft, profiles, saveProfiles, showStatus, t],
  );

  const deleteProfile = React.useCallback(
    async (profileId: string) => {
      const next = profiles.filter((p) => p.id !== profileId);
      await saveProfiles(next);
    },
    [profiles, saveProfiles],
  );

  const loadProfileById = React.useCallback(
    (profileId: string) => {
      const found = profiles.find((p) => p.id === profileId);
      if (found) setDraft({ ...found });
    },
    [profiles],
  );

  const resetDraft = React.useCallback(() => {
    setDraft(buildDelayProfileTemplate(profiles));
  }, [profiles]);

  return {
    loading,
    saving,
    profiles,
    parseError,
    draft,
    setDraft,
    saveProfile,
    deleteProfile,
    loadProfileById,
    resetDraft,
    refreshProfiles: loadProfiles,
  };
}
