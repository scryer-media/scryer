import * as React from "react";
import { useClient } from "urql";
import { SettingsSubtitlesSection } from "@/components/views/settings/settings-subtitles-section";
import { subtitleSettingsQuery } from "@/lib/graphql/queries";
import { updateSubtitleSettingsMutation } from "@/lib/graphql/mutations";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import type { SubtitleSettings } from "@/lib/types/settings";

const DEFAULTS: SubtitleSettings = {
  enabled: false,
  hasOpenSubtitlesApiKey: false,
  openSubtitlesUsername: "",
  openSubtitlesPassword: "",
  hasOpenSubtitlesPassword: false,
  languages: [],
  autoDownloadOnImport: false,
  minimumScoreSeries: 240,
  minimumScoreMovie: 70,
  searchIntervalHours: 6,
  includeAiTranslated: false,
  includeMachineTranslated: false,
  syncEnabled: true,
  syncThresholdSeries: 90,
  syncThresholdMovie: 70,
  syncMaxOffsetSeconds: 60,
};

export function SettingsSubtitlesContainer() {
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const client = useClient();
  const [settings, setSettings] = React.useState<SubtitleSettings>(DEFAULTS);
  const [saving, setSaving] = React.useState(false);
  const [loading, setLoading] = React.useState(true);
  const loadedRef = React.useRef(false);

  React.useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const { data, error } = await client.query(subtitleSettingsQuery, {}).toPromise();
        if (error) throw error;
        if (cancelled) return;
        const payload = data?.subtitleSettings;
        if (!payload) return;
        setSettings({
          ...DEFAULTS,
          ...payload,
          openSubtitlesPassword: "",
        });
        loadedRef.current = true;
      } catch {
        // Use defaults on failure
      } finally {
        if (!cancelled) setLoading(false);
      }
    })();
    return () => { cancelled = true; };
  }, [client]);

  // Auto-save on change (skip initial load)
  React.useEffect(() => {
    if (!loadedRef.current) return;
    setSaving(true);
    client
      .mutation(updateSubtitleSettingsMutation, {
        input: {
          enabled: settings.enabled,
          openSubtitlesUsername: settings.openSubtitlesUsername,
          ...(settings.openSubtitlesPassword ? { openSubtitlesPassword: settings.openSubtitlesPassword } : {}),
          languages: settings.languages.map((language) => ({
            code: language.code,
            hearingImpaired: language.hearingImpaired,
            forced: language.forced,
          })),
          autoDownloadOnImport: settings.autoDownloadOnImport,
          minimumScoreSeries: settings.minimumScoreSeries,
          minimumScoreMovie: settings.minimumScoreMovie,
          searchIntervalHours: settings.searchIntervalHours,
          includeAiTranslated: settings.includeAiTranslated,
          includeMachineTranslated: settings.includeMachineTranslated,
          syncEnabled: settings.syncEnabled,
          syncThresholdSeries: settings.syncThresholdSeries,
          syncThresholdMovie: settings.syncThresholdMovie,
          syncMaxOffsetSeconds: settings.syncMaxOffsetSeconds,
        },
      })
      .toPromise()
      .then(({ error }) => {
        if (error) {
          setGlobalStatus(error.message || t("status.failedToUpdate"));
        }
      })
      .catch((error: unknown) => {
        setGlobalStatus(error instanceof Error ? error.message : t("status.failedToUpdate"));
      })
      .finally(() => setSaving(false));
  }, [client, setGlobalStatus, settings, t]);

  return (
    <SettingsSubtitlesSection
      settings={settings}
      setSettings={setSettings}
      saving={saving}
      loading={loading}
    />
  );
}
