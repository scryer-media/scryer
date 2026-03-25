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
  opensubtitlesUsername: string;
  opensubtitlesPassword: string;
  languages: string[];
  autoDownloadOnImport: boolean;
  minimumScoreSeries: number;
  minimumScoreMovie: number;
  searchIntervalHours: number;
  includeAiTranslated: boolean;
  includeMachineTranslated: boolean;
  syncEnabled: boolean;
  syncThresholdSeries: number;
  syncThresholdMovie: number;
  syncMaxOffsetSeconds: number;
};

const DEFAULTS: SubtitleSettings = {
  enabled: false,
  opensubtitlesUsername: "",
  opensubtitlesPassword: "",
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

function parseSetting(items: AdminSetting[], key: string, fallback: string): string {
  const record = items.find((item) => item.keyName === key);
  const raw = getSettingDisplayValue(record).trim();
  return raw.length > 0 ? raw : fallback;
}

function parseLanguagesFromJson(json: string): string[] {
  try {
    const arr = JSON.parse(json);
    if (!Array.isArray(arr)) return [];
    return arr.map((l: { code?: string }) => l.code ?? "").filter(Boolean);
  } catch {
    return [];
  }
}

function languagesToJson(codes: string[]): string {
  return JSON.stringify(codes.map((code) => ({ code, hearing_impaired: false, forced: false })));
}

function buildSaveItems(settings: SubtitleSettings) {
  return [
    { keyName: "subtitles.enabled", value: String(settings.enabled) },
    ...(settings.opensubtitlesUsername ? [{ keyName: "subtitles.opensubtitles_username", value: settings.opensubtitlesUsername }] : []),
    ...(settings.opensubtitlesPassword ? [{ keyName: "subtitles.opensubtitles_password", value: settings.opensubtitlesPassword }] : []),
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
    { keyName: "subtitles.sync_max_offset_seconds", value: String(settings.syncMaxOffsetSeconds) },
  ];
}

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
        const items: AdminSetting[] = data?.subtitleSettings?.items ?? [];
        setSettings({
          enabled: parseSetting(items, "subtitles.enabled", "false") === "true",
          opensubtitlesUsername: parseSetting(items, "subtitles.opensubtitles_username", ""),
          opensubtitlesPassword: "",
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
          syncMaxOffsetSeconds: Number(parseSetting(items, "subtitles.sync_max_offset_seconds", "60")),
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
      .mutation(saveAdminSettingsMutation, {
        input: { scope: "system", items: buildSaveItems(settings) },
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
