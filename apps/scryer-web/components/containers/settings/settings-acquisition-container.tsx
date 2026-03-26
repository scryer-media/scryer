import * as React from "react";
import { useClient } from "urql";
import { SettingsAcquisitionSection } from "@/components/views/settings/settings-acquisition-section";
import { acquisitionSettingsQuery } from "@/lib/graphql/queries";
import { updateAcquisitionSettingsMutation } from "@/lib/graphql/mutations";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import type { AcquisitionSettings } from "@/lib/types/settings";

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

export function SettingsAcquisitionContainer() {
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
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
        setSettings({
          ...DEFAULTS,
          ...data?.acquisitionSettings,
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
      const { error } = await client.mutation(updateAcquisitionSettingsMutation, {
        input: settings,
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
      settings={settings}
      setSettings={setSettings}
      saving={saving}
      loading={loading}
      onSave={handleSave}
    />
  );
}
