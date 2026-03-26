import * as React from "react";
import {
  deleteDelayProfileMutation,
  upsertDelayProfileMutation,
} from "@/lib/graphql/mutations";
import { delayProfilesQuery } from "@/lib/graphql/queries";
import { useClient } from "urql";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import {
  buildDelayProfileTemplate,
  createDelayProfileId,
  DELAY_PROFILE_CATALOG_KEY,
  validateDelayProfileDraft,
} from "@/lib/utils/delay-profiles";
import type { DelayProfileDraft, ParsedDelayProfile } from "@/lib/types/delay-profiles";
import { useSettingsSubscription } from "@/lib/hooks/use-settings-subscription";

type DelayProfilePayload = {
  id: string;
  name: string;
  usenetDelayMinutes: number;
  torrentDelayMinutes: number;
  preferredProtocol: "usenet" | "torrent";
  minAgeMinutes: number;
  bypassScoreThreshold?: number | null;
  appliesToFacets: Array<"movie" | "tv" | "anime">;
  tags: string[];
  priority: number;
  enabled: boolean;
};

function fromDelayProfilePayload(profile: DelayProfilePayload): ParsedDelayProfile {
  return {
    id: profile.id,
    name: profile.name,
    usenet_delay_minutes: profile.usenetDelayMinutes,
    torrent_delay_minutes: profile.torrentDelayMinutes,
    preferred_protocol: profile.preferredProtocol,
    min_age_minutes: profile.minAgeMinutes,
    bypass_score_threshold: profile.bypassScoreThreshold ?? null,
    applies_to_facets: profile.appliesToFacets.map((facet) => facet === "tv" ? "series" : facet),
    tags: profile.tags,
    priority: profile.priority,
    enabled: profile.enabled,
  };
}

function toDelayProfileInput(profile: ParsedDelayProfile) {
  return {
    id: profile.id,
    name: profile.name.trim(),
    usenetDelayMinutes: profile.usenet_delay_minutes,
    torrentDelayMinutes: profile.torrent_delay_minutes,
    preferredProtocol: profile.preferred_protocol,
    minAgeMinutes: profile.min_age_minutes,
    bypassScoreThreshold: profile.bypass_score_threshold,
    appliesToFacets: profile.applies_to_facets.map((facet) => facet === "series" ? "tv" : facet),
    tags: profile.tags,
    priority: profile.priority,
    enabled: profile.enabled,
  };
}

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
      const result = await client.query(delayProfilesQuery, {}).toPromise();
      if (result.error) throw result.error;
      const parsed = (result.data?.delayProfiles ?? []).map(fromDelayProfilePayload);
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

      setSaving(true);
      try {
        const result = await client
          .mutation(upsertDelayProfileMutation, {
            input: toDelayProfileInput(entry),
          })
          .toPromise();
        if (result.error) {
          showStatus(t("settings.delayProfileSaveError"));
          return;
        }
        const saved = result.data?.upsertDelayProfile
          ? fromDelayProfilePayload(result.data.upsertDelayProfile)
          : entry;
        const next = isNew
          ? [...profiles, saved]
          : profiles.map((profile) => (profile.id === saved.id ? saved : profile));
        setProfiles(next);
        setDraft(buildDelayProfileTemplate(next));
        showStatus(t("settings.delayProfilesSaved"));
      } catch {
        showStatus(t("settings.delayProfileSaveError"));
      } finally {
        setSaving(false);
      }
    },
    [client, draft, profiles, showStatus, t],
  );

  const deleteProfile = React.useCallback(
    async (profileId: string) => {
      setSaving(true);
      try {
        const result = await client
          .mutation(deleteDelayProfileMutation, { input: { id: profileId } })
          .toPromise();
        if (result.error) {
          showStatus(t("settings.delayProfileSaveError"));
          return;
        }
        const next = profiles.filter((profile) => profile.id !== profileId);
        setProfiles(next);
      } catch {
        showStatus(t("settings.delayProfileSaveError"));
      } finally {
        setSaving(false);
      }
    },
    [client, profiles, showStatus, t],
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
