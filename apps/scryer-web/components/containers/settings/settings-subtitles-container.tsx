import * as React from "react";
import { useClient } from "urql";
import { SettingsSubtitlesSection } from "@/components/views/settings/settings-subtitles-section";
import { subtitleSettingsQuery } from "@/lib/graphql/queries";
import { saveAdminSettingsMutation } from "@/lib/graphql/mutations";
import type { AdminSetting } from "@/lib/types/admin-settings";
import { getSettingDisplayValue } from "@/lib/utils/settings";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";

export type SubtitleSettings = {
  enabled: boolean;
  opensubtitlesApiKey: string;
  opensubtitlesUsername: string;
  opensubtitlesPassword: string;
  languages: string;
  autoDownloadOnImport: boolean;
  minimumScoreSeries: number;
  minimumScoreMovie: number;
  searchIntervalHours: number;
  includeAiTranslated: boolean;
  includeMachineTranslated: boolean;
  syncEnabled: boolean;
  syncThresholdSeries: number;
  syncThresholdMovie: number;
};

const DEFAULTS: SubtitleSettings = {
  enabled: false,
  opensubtitlesApiKey: "",
  opensubtitlesUsername: "",
  opensubtitlesPassword: "",
  languages: "",
  autoDownloadOnImport: false,
  minimumScoreSeries: 240,
  minimumScoreMovie: 70,
  searchIntervalHours: 6,
  includeAiTranslated: false,
  includeMachineTranslated: false,
  syncEnabled: true,
  syncThresholdSeries: 90,
  syncThresholdMovie: 70,
};

function parseSetting(items: AdminSetting[], key: string, fallback: string): string {
  const record = items.find((item) => item.keyName === key);
  const raw = getSettingDisplayValue(record).trim();
  return raw.length > 0 ? raw : fallback;
}

function parseLanguagesFromJson(json: string): string {
  try {
    const arr = JSON.parse(json);
    if (!Array.isArray(arr)) return "";
    return arr.map((l: { code?: string }) => l.code ?? "").filter(Boolean).join(", ");
  } catch {
    return "";
  }
}

function languagesToJson(input: string): string {
  const codes = input
    .split(/[,\s]+/)
    .map((s) => s.trim().toLowerCase())
    .filter((s) => s.length >= 2 && s.length <= 3);
  return JSON.stringify(codes.map((code) => ({ code, hearing_impaired: false, forced: false })));
}

export function SettingsSubtitlesContainer() {
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const client = useClient();
  const [settings, setSettings] = React.useState<SubtitleSettings>(DEFAULTS);
  const [saving, setSaving] = React.useState(false);
  const [loading, setLoading] = React.useState(true);

  React.useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const { data, error } = await client.query(subtitleSettingsQuery, {}).toPromise();
        if (error) throw error;
        if (cancelled) return;
        const items: AdminSetting[] = data?.subtitleSettings?.items ?? [];
        setSettings({
          enabled: parseSetting(items, "subtitles.enabled", "false") === "true",
          opensubtitlesApiKey: parseSetting(items, "subtitles.opensubtitles_api_key", ""),
          opensubtitlesUsername: parseSetting(items, "subtitles.opensubtitles_username", ""),
          opensubtitlesPassword: parseSetting(items, "subtitles.opensubtitles_password", ""),
          languages: parseLanguagesFromJson(parseSetting(items, "subtitles.languages", "[]")),
          autoDownloadOnImport: parseSetting(items, "subtitles.auto_download_on_import", "false") === "true",
          minimumScoreSeries: Number(parseSetting(items, "subtitles.minimum_score_series", "240")),
          minimumScoreMovie: Number(parseSetting(items, "subtitles.minimum_score_movie", "70")),
          searchIntervalHours: Number(parseSetting(items, "subtitles.search_interval_hours", "6")),
          includeAiTranslated: parseSetting(items, "subtitles.include_ai_translated", "false") === "true",
          includeMachineTranslated: parseSetting(items, "subtitles.include_machine_translated", "false") === "true",
          syncEnabled: parseSetting(items, "subtitles.sync_enabled", "true") === "true",
          syncThresholdSeries: Number(parseSetting(items, "subtitles.sync_threshold_series", "90")),
          syncThresholdMovie: Number(parseSetting(items, "subtitles.sync_threshold_movie", "70")),
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
            { keyName: "subtitles.enabled", value: String(settings.enabled) },
            { keyName: "subtitles.opensubtitles_api_key", value: settings.opensubtitlesApiKey },
            { keyName: "subtitles.opensubtitles_username", value: settings.opensubtitlesUsername },
            { keyName: "subtitles.opensubtitles_password", value: settings.opensubtitlesPassword },
            { keyName: "subtitles.languages", value: languagesToJson(settings.languages) },
            { keyName: "subtitles.auto_download_on_import", value: String(settings.autoDownloadOnImport) },
            { keyName: "subtitles.minimum_score_series", value: String(settings.minimumScoreSeries) },
            { keyName: "subtitles.minimum_score_movie", value: String(settings.minimumScoreMovie) },
            { keyName: "subtitles.search_interval_hours", value: String(settings.searchIntervalHours) },
            { keyName: "subtitles.include_ai_translated", value: String(settings.includeAiTranslated) },
            { keyName: "subtitles.include_machine_translated", value: String(settings.includeMachineTranslated) },
            { keyName: "subtitles.sync_enabled", value: String(settings.syncEnabled) },
            { keyName: "subtitles.sync_threshold_series", value: String(settings.syncThresholdSeries) },
            { keyName: "subtitles.sync_threshold_movie", value: String(settings.syncThresholdMovie) },
          ],
        },
      }).toPromise();
      if (error) throw error;
      setGlobalStatus(t("settings.subtitlesSaved"));
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToUpdate"));
    } finally {
      setSaving(false);
    }
  }, [client, setGlobalStatus, settings, t]);

  return (
    <SettingsSubtitlesSection
      settings={settings}
      setSettings={setSettings}
      saving={saving}
      loading={loading}
      onSave={handleSave}
    />
  );
}
