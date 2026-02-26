import * as React from "react";
import { useClient } from "urql";
import { SettingsAcquisitionSection } from "@/components/views/settings/settings-acquisition-section";
import { acquisitionSettingsQuery } from "@/lib/graphql/queries";
import { saveAdminSettingsMutation } from "@/lib/graphql/mutations";
import type { AdminSetting } from "@/lib/types/admin-settings";
import { getSettingDisplayValue } from "@/lib/utils/settings";
import type { Translate } from "@/components/root/types";

type Props = {
  t: Translate;
  setGlobalStatus: (status: string) => void;
};

type AcquisitionSettings = {
  enabled: boolean;
  upgradeCooldownHours: number;
  sameTierMinDelta: number;
  crossTierMinDelta: number;
  forcedUpgradeDeltaBypass: number;
  pollIntervalSeconds: number;
  syncIntervalSeconds: number;
  batchSize: number;
};

const DEFAULTS: AcquisitionSettings = {
  enabled: true,
  upgradeCooldownHours: 24,
  sameTierMinDelta: 120,
  crossTierMinDelta: 30,
  forcedUpgradeDeltaBypass: 400,
  pollIntervalSeconds: 60,
  syncIntervalSeconds: 3600,
  batchSize: 50,
};

function parseSetting(items: AdminSetting[], key: string, fallback: string): string {
  const record = items.find((item) => item.keyName === key);
  const raw = getSettingDisplayValue(record).trim();
  return raw.length > 0 ? raw : fallback;
}

export function SettingsAcquisitionContainer({ t, setGlobalStatus }: Props) {
  const client = useClient();
  const [settings, setSettings] = React.useState<AcquisitionSettings>(DEFAULTS);
  const [saving, setSaving] = React.useState(false);
  const [loading, setLoading] = React.useState(true);

  React.useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const { data, error } = await client.query(acquisitionSettingsQuery, {}).toPromise();
        if (error) throw error;
        if (cancelled) return;
        const items: AdminSetting[] = data?.acquisitionSettings?.items ?? [];
        setSettings({
          enabled: parseSetting(items, "acquisition.enabled", "true") === "true",
          upgradeCooldownHours: Number(parseSetting(items, "acquisition.upgrade_cooldown_hours", "24")),
          sameTierMinDelta: Number(parseSetting(items, "acquisition.same_tier_min_delta", "120")),
          crossTierMinDelta: Number(parseSetting(items, "acquisition.cross_tier_min_delta", "30")),
          forcedUpgradeDeltaBypass: Number(parseSetting(items, "acquisition.forced_upgrade_delta_bypass", "400")),
          pollIntervalSeconds: Number(parseSetting(items, "acquisition.poll_interval_seconds", "60")),
          syncIntervalSeconds: Number(parseSetting(items, "acquisition.sync_interval_seconds", "3600")),
          batchSize: Number(parseSetting(items, "acquisition.batch_size", "50")),
        });
      } catch {
        // Use defaults on failure
      } finally {
        if (!cancelled) setLoading(false);
      }
    })();
    return () => { cancelled = true; };
  }, [client]);

  const handleSave = React.useCallback(async () => {
    setSaving(true);
    try {
      const { error } = await client.mutation(saveAdminSettingsMutation, {
        input: {
          scope: "system",
          items: [
            { keyName: "acquisition.enabled", value: String(settings.enabled) },
            { keyName: "acquisition.upgrade_cooldown_hours", value: String(settings.upgradeCooldownHours) },
            { keyName: "acquisition.same_tier_min_delta", value: String(settings.sameTierMinDelta) },
            { keyName: "acquisition.cross_tier_min_delta", value: String(settings.crossTierMinDelta) },
            { keyName: "acquisition.forced_upgrade_delta_bypass", value: String(settings.forcedUpgradeDeltaBypass) },
            { keyName: "acquisition.poll_interval_seconds", value: String(settings.pollIntervalSeconds) },
            { keyName: "acquisition.sync_interval_seconds", value: String(settings.syncIntervalSeconds) },
            { keyName: "acquisition.batch_size", value: String(settings.batchSize) },
          ],
        },
      }).toPromise();
      if (error) throw error;
      setGlobalStatus(t("settings.acquisitionSaved"));
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToUpdate"));
    } finally {
      setSaving(false);
    }
  }, [client, setGlobalStatus, settings, t]);

  return (
    <SettingsAcquisitionSection
      t={t}
      settings={settings}
      setSettings={setSettings}
      saving={saving}
      loading={loading}
      onSave={handleSave}
    />
  );
}
