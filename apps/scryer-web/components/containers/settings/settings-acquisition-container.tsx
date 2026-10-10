import { useCallback, useEffect, useState } from "react";
import { useClient } from "urql";
import {
  SettingsAcquisitionSection,
  type AcquisitionSettings,
} from "@/components/views/settings/settings-acquisition-section";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { acquisitionSettingsQuery } from "@/lib/graphql/queries";
import { updateAcquisitionSettingsMutation } from "@/lib/graphql/mutations";
import { useAuth } from "@/lib/hooks/use-auth";
import { APP_PERMISSIONS, hasAnyAppPermission } from "@/lib/utils/permissions";

type AcquisitionSettingsQueryResult = {
  acquisitionSettings?: AcquisitionSettings | null;
};

type UpdateAcquisitionSettingsResult = {
  updateAcquisitionSettings?: AcquisitionSettings | null;
};

// The acquisition settings expose the convergence knobs — RSS is
// the steady-state path; active search converges each scope once per indexer.
export function SettingsAcquisitionContainer() {
  const t = useTranslate();
  const client = useClient();
  const setGlobalStatus = useGlobalStatus();
  const { user } = useAuth();
  const canManage = hasAnyAppPermission(user, [APP_PERMISSIONS.manageSystemSettings]);

  const [settings, setSettings] = useState<AcquisitionSettings | null>(null);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);

  const fetchSettings = useCallback(async () => {
    setLoading(true);
    try {
      const { data, error } = await client
        .query<AcquisitionSettingsQueryResult>(
          acquisitionSettingsQuery,
          {},
          { requestPolicy: "network-only" },
        )
        .toPromise();
      if (error) throw error;
      setSettings(data?.acquisitionSettings ?? null);
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToLoad"), { level: "ERROR" });
    } finally {
      setLoading(false);
    }
  }, [client, setGlobalStatus, t]);

  useEffect(() => {
    void fetchSettings();
  }, [fetchSettings]);

  const saveSettings = useCallback(
    async (next: AcquisitionSettings) => {
      if (!canManage) return;
      setSaving(true);
      try {
        const { data, error } = await client
          .mutation<UpdateAcquisitionSettingsResult>(updateAcquisitionSettingsMutation, {
            input: next,
          })
          .toPromise();
        if (error) throw error;
        setSettings(data?.updateAcquisitionSettings ?? next);
        setGlobalStatus(t("settings.acquisitionSaved"), { level: "SUCCESS" });
      } catch (error) {
        setGlobalStatus(error instanceof Error ? error.message : t("status.failedToUpdate"), { level: "ERROR" });
      } finally {
        setSaving(false);
      }
    },
    [canManage, client, setGlobalStatus, t],
  );

  return (
    <SettingsAcquisitionSection
      settings={settings}
      loading={loading}
      saving={saving}
      canManage={canManage}
      onSave={saveSettings}
    />
  );
}
