import { useCallback, useEffect, useRef, useState } from "react";
import type { Client } from "urql";

import {
  rulePackRegistryQuery,
  rulePackTemplatesQuery,
  trackedRulePacksQuery,
} from "@/lib/graphql/queries";
import {
  installTrackedRulePackMutation,
  setTrackedRulePackSettingsMutation,
} from "@/lib/graphql/mutations";
import type { TrackedRulePackRecord } from "@/components/views/settings/tracked-rule-packs-section";
import {
  SETUP_RULE_PACK_RECOMMENDATIONS,
  defaultRulePackTemplateIds,
  enabledRulePackTemplateIds,
  rulePackEnabled,
  rulePackSettingsInput,
} from "@/lib/utils/setup-recommendations";

export type SetupRulePackState = {
  packId: string;
  titleKey: string;
  reasonKey: string;
  /** Installed, or offered by the catalog so it can be installed. */
  available: boolean;
  enabled: boolean;
  busy: boolean;
  error: string | null;
};

function errorMessage(error: unknown, fallback: string) {
  return error instanceof Error && error.message.trim() ? error.message.trim() : fallback;
}

/**
 * The community rule packs setup offers as on/off switches. TRaSH Guides is
 * installed and on from first start, so its switch starts on; SeaDex starts
 * off until turned on here, which installs it.
 *
 * Turning a pack off disables its rules rather than uninstalling it, so
 * turning it back on in the same visit restores the rules that were on.
 * Loads once `active` is first true.
 */
export function useSetupRulePacks({
  client,
  active,
  t,
}: {
  client: Client;
  active: boolean;
  t: (key: string) => string;
}) {
  const [tracked, setTracked] = useState<TrackedRulePackRecord[]>([]);
  const [catalogPackIds, setCatalogPackIds] = useState<Set<string>>(new Set());
  const [loaded, setLoaded] = useState(false);
  const [busyPackIds, setBusyPackIds] = useState<string[]>([]);
  const [errors, setErrors] = useState<Partial<Record<string, string>>>({});
  const loadStarted = useRef(false);
  const enabledBeforeOff = useRef(new Map<string, string[]>());

  const refreshTracked = useCallback(async () => {
    const { data, error } = await client.query(trackedRulePacksQuery, {}).toPromise();
    if (error) throw error;
    const packs: TrackedRulePackRecord[] = data?.trackedRulePacks ?? [];
    setTracked(packs);
    return packs;
  }, [client]);

  useEffect(() => {
    if (!active || loadStarted.current) return;
    loadStarted.current = true;
    void Promise.allSettled([
      refreshTracked(),
      client
        .query(rulePackRegistryQuery, {})
        .toPromise()
        .then(({ data }) => {
          const entries: Array<{ id: string }> = data?.rulePackRegistry ?? [];
          setCatalogPackIds(new Set(entries.map((entry) => entry.id)));
        }),
    ]).then(() => setLoaded(true));
  }, [active, client, refreshTracked]);

  const packDefaults = useCallback(
    async (packId: string) => {
      const { data, error } = await client
        .query(rulePackTemplatesQuery, { packId })
        .toPromise();
      if (error) throw error;
      return defaultRulePackTemplateIds(data?.rulePackTemplates ?? []);
    },
    [client],
  );

  const setPackEnabled = useCallback(
    async (packId: string, enabled: boolean) => {
      setBusyPackIds((current) => [...current, packId]);
      setErrors((current) => ({ ...current, [packId]: undefined }));
      try {
        const pack = tracked.find((entry) => entry.packId === packId);
        if (!pack) {
          if (!enabled) return;
          const { error } = await client
            .mutation(installTrackedRulePackMutation, {
              packId,
              templateIds: await packDefaults(packId),
            })
            .toPromise();
          if (error) throw error;
        } else {
          let templateIds: string[] = [];
          if (enabled) {
            templateIds =
              enabledBeforeOff.current.get(packId) ?? (await packDefaults(packId));
          } else {
            const on = enabledRulePackTemplateIds(pack);
            if (on.length > 0) enabledBeforeOff.current.set(packId, on);
          }
          const { error } = await client
            .mutation(
              setTrackedRulePackSettingsMutation,
              rulePackSettingsInput(pack, templateIds),
            )
            .toPromise();
          if (error) throw error;
        }
        await refreshTracked();
      } catch (error) {
        setErrors((current) => ({
          ...current,
          [packId]: errorMessage(error, t("status.failedToUpdate")),
        }));
      } finally {
        setBusyPackIds((current) => current.filter((id) => id !== packId));
      }
    },
    [client, packDefaults, refreshTracked, t, tracked],
  );

  const packs: SetupRulePackState[] = SETUP_RULE_PACK_RECOMMENDATIONS.map(
    (recommendation) => {
      const pack = tracked.find((entry) => entry.packId === recommendation.packId);
      return {
        ...recommendation,
        available: pack !== undefined || catalogPackIds.has(recommendation.packId),
        enabled: rulePackEnabled(pack),
        busy: busyPackIds.includes(recommendation.packId),
        error: errors[recommendation.packId] ?? null,
      };
    },
  );

  return { rulePacks: packs, rulePacksLoading: !loaded, setRulePackEnabled: setPackEnabled };
}
