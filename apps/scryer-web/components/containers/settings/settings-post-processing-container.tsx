import * as React from "react";
import { useClient } from "urql";
import { SettingsPostProcessingSection } from "@/components/views/settings/settings-post-processing-section";
import { postProcessingSettingsQuery } from "@/lib/graphql/queries";
import { saveAdminSettingsMutation } from "@/lib/graphql/mutations";
import type { AdminSetting } from "@/lib/types/admin-settings";
import { getSettingDisplayValue } from "@/lib/utils/settings";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";

type PostProcessingSettings = {
  movieScript: string;
  seriesScript: string;
  animeScript: string;
  timeoutSecs: number;
};

const DEFAULTS: PostProcessingSettings = {
  movieScript: "",
  seriesScript: "",
  animeScript: "",
  timeoutSecs: 1800,
};

function parseSetting(items: AdminSetting[], key: string, fallback: string): string {
  const record = items.find((item) => item.keyName === key);
  const raw = getSettingDisplayValue(record).trim();
  return raw.length > 0 ? raw : fallback;
}

export function SettingsPostProcessingContainer() {
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const client = useClient();
  const [settings, setSettings] = React.useState<PostProcessingSettings>(DEFAULTS);
  const [saving, setSaving] = React.useState(false);
  const [loading, setLoading] = React.useState(true);

  React.useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const { data, error } = await client.query(postProcessingSettingsQuery, {}).toPromise();
        if (error) throw error;
        if (cancelled) return;
        const items: AdminSetting[] = data?.postProcessingSettings?.items ?? [];
        setSettings({
          movieScript: parseSetting(items, "post_processing.script.movie", ""),
          seriesScript: parseSetting(items, "post_processing.script.series", ""),
          animeScript: parseSetting(items, "post_processing.script.anime", ""),
          timeoutSecs: Number(parseSetting(items, "post_processing.timeout_secs", "1800")),
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
            { keyName: "post_processing.script.movie", value: settings.movieScript },
            { keyName: "post_processing.script.series", value: settings.seriesScript },
            { keyName: "post_processing.script.anime", value: settings.animeScript },
            { keyName: "post_processing.timeout_secs", value: String(settings.timeoutSecs) },
          ],
        },
      }).toPromise();
      if (error) throw error;
      setGlobalStatus(t("settings.postProcessingSaved"));
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToUpdate"));
    } finally {
      setSaving(false);
    }
  }, [client, setGlobalStatus, settings, t]);

  return (
    <SettingsPostProcessingSection
      settings={settings}
      setSettings={setSettings}
      saving={saving}
      loading={loading}
      onSave={handleSave}
    />
  );
}
