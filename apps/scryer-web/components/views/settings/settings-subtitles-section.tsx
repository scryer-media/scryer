import { Button } from "@/components/ui/button";
import { Input, integerInputProps, sanitizeDigits } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Loader2, Subtitles } from "lucide-react";
import { useTranslate } from "@/lib/context/translate-context";
import type { SubtitleSettings } from "@/components/containers/settings/settings-subtitles-container";

type Props = {
  settings: SubtitleSettings;
  setSettings: (s: SubtitleSettings) => void;
  saving: boolean;
  loading: boolean;
  onSave: () => void;
};

function Toggle({ checked, onChange, label }: { checked: boolean; onChange: (v: boolean) => void; label: string }) {
  return (
    <div className="flex items-center gap-3">
      <Label>{label}</Label>
      <button
        type="button"
        role="switch"
        aria-checked={checked}
        className={`relative inline-flex h-6 w-11 shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-colors ${checked ? "bg-primary" : "bg-muted"}`}
        onClick={() => onChange(!checked)}
      >
        <span
          className={`pointer-events-none inline-block h-5 w-5 rounded-full bg-background shadow-lg transition-transform ${checked ? "translate-x-5" : "translate-x-0"}`}
        />
      </button>
    </div>
  );
}

export function SettingsSubtitlesSection({
  settings,
  setSettings,
  saving,
  loading,
  onSave,
}: Props) {
  const t = useTranslate();
  const update = (patch: Partial<SubtitleSettings>) =>
    setSettings({ ...settings, ...patch });
  const parseIntegerInput = (raw: string) => {
    const nextValue = sanitizeDigits(raw);
    return nextValue === "" ? 0 : Number(nextValue);
  };

  if (loading) {
    return (
      <div className="flex items-center gap-2 text-sm text-muted-foreground">
        <Loader2 className="h-4 w-4 animate-spin" />
        {t("label.loading")}
      </div>
    );
  }

  return (
    <div className="space-y-6 text-sm">
      <div className="flex items-center gap-2 text-base font-semibold">
        <Subtitles className="h-4 w-4" />
        {t("settings.subtitles")}
      </div>

      <Toggle checked={settings.enabled} onChange={(v) => update({ enabled: v })} label={t("settings.sub.enabled")} />

      {/* OpenSubtitles Credentials */}
      <div className="space-y-1">
        <Label className="text-sm font-medium">{t("settings.sub.credentials")}</Label>
        <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
          <div className="space-y-1">
            <Label>{t("settings.sub.apiKey")}</Label>
            <Input
              type="password"
              value={settings.opensubtitlesApiKey}
              onChange={(e) => update({ opensubtitlesApiKey: e.target.value })}
              placeholder="s38zmz..."
            />
          </div>
          <div />
          <div className="space-y-1">
            <Label>{t("settings.sub.username")}</Label>
            <Input
              value={settings.opensubtitlesUsername}
              onChange={(e) => update({ opensubtitlesUsername: e.target.value })}
            />
          </div>
          <div className="space-y-1">
            <Label>{t("settings.sub.password")}</Label>
            <Input
              type="password"
              value={settings.opensubtitlesPassword}
              onChange={(e) => update({ opensubtitlesPassword: e.target.value })}
            />
          </div>
        </div>
      </div>

      {/* Languages */}
      <div className="space-y-1">
        <Label>{t("settings.sub.languages")}</Label>
        <Input
          value={settings.languages}
          onChange={(e) => update({ languages: e.target.value })}
          placeholder="eng, spa, fre"
        />
        <p className="text-xs text-muted-foreground">{t("settings.sub.languagesHelp")}</p>
      </div>

      {/* Score Thresholds & Search */}
      <div className="grid grid-cols-1 gap-4 sm:grid-cols-2">
        <div className="space-y-1">
          <Label>{t("settings.sub.minScoreSeries")}</Label>
          <Input
            {...integerInputProps}
            value={settings.minimumScoreSeries}
            onChange={(e) => update({ minimumScoreSeries: parseIntegerInput(e.target.value) })}
          />
        </div>
        <div className="space-y-1">
          <Label>{t("settings.sub.minScoreMovie")}</Label>
          <Input
            {...integerInputProps}
            value={settings.minimumScoreMovie}
            onChange={(e) => update({ minimumScoreMovie: parseIntegerInput(e.target.value) })}
          />
        </div>
        <div className="space-y-1">
          <Label>{t("settings.sub.searchInterval")}</Label>
          <Input
            {...integerInputProps}
            value={settings.searchIntervalHours}
            onChange={(e) => update({ searchIntervalHours: parseIntegerInput(e.target.value) })}
          />
        </div>
      </div>

      {/* Toggles */}
      <div className="space-y-3">
        <Toggle checked={settings.autoDownloadOnImport} onChange={(v) => update({ autoDownloadOnImport: v })} label={t("settings.sub.autoDownload")} />
        <Toggle checked={!settings.includeAiTranslated} onChange={(v) => update({ includeAiTranslated: !v })} label={t("settings.sub.excludeAi")} />
        <Toggle checked={!settings.includeMachineTranslated} onChange={(v) => update({ includeMachineTranslated: !v })} label={t("settings.sub.excludeMachine")} />
      </div>

      {/* Sync */}
      <div className="space-y-3">
        <Toggle checked={settings.syncEnabled} onChange={(v) => update({ syncEnabled: v })} label={t("settings.sub.syncEnabled")} />
        {settings.syncEnabled ? (
          <div className="grid grid-cols-1 gap-4 sm:grid-cols-2">
            <div className="space-y-1">
              <Label>{t("settings.sub.syncThresholdSeries")}</Label>
              <Input
                {...integerInputProps}
                value={settings.syncThresholdSeries}
                onChange={(e) => update({ syncThresholdSeries: parseIntegerInput(e.target.value) })}
              />
            </div>
            <div className="space-y-1">
              <Label>{t("settings.sub.syncThresholdMovie")}</Label>
              <Input
                {...integerInputProps}
                value={settings.syncThresholdMovie}
                onChange={(e) => update({ syncThresholdMovie: parseIntegerInput(e.target.value) })}
              />
            </div>
          </div>
        ) : null}
      </div>

      <Button onClick={onSave} disabled={saving}>
        {saving ? (
          <>
            <Loader2 className="mr-2 h-4 w-4 animate-spin" />
            {t("label.saving")}
          </>
        ) : (
          t("settings.save")
        )}
      </Button>
    </div>
  );
}
