import * as React from "react";
import { Input, integerInputProps, sanitizeDigits } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Loader2, Subtitles } from "lucide-react";
import { useTranslate } from "@/lib/context/translate-context";
import { SubtitleLanguagePicker } from "@/components/common/subtitle-language-picker";
import type { SubtitleSettings } from "@/components/containers/settings/settings-subtitles-container";

type Props = {
  settings: SubtitleSettings;
  setSettings: (s: SubtitleSettings) => void;
  saving: boolean;
  loading: boolean;
};

function Toggle({ checked, onChange, label, disabled }: { checked: boolean; onChange: (v: boolean) => void; label: string; disabled?: boolean }) {
  return (
    <div className="flex items-center gap-3">
      <Label className={disabled ? "text-muted-foreground" : ""}>{label}</Label>
      <button
        type="button"
        role="switch"
        aria-checked={checked}
        disabled={disabled}
        className={`relative inline-flex h-6 w-11 shrink-0 rounded-full border-2 border-transparent transition-colors ${disabled ? "cursor-not-allowed opacity-50" : "cursor-pointer"} ${checked ? "bg-primary" : "bg-muted"}`}
        onClick={() => !disabled && onChange(!checked)}
      >
        <span
          className={`pointer-events-none inline-block h-5 w-5 rounded-full bg-background shadow-lg transition-transform ${checked ? "translate-x-5" : "translate-x-0"}`}
        />
      </button>
    </div>
  );
}

/** Text input that holds local state and only commits on blur. */
function BlurInput({ value, onCommit, ...rest }: { value: string; onCommit: (v: string) => void } & Omit<React.ComponentProps<typeof Input>, "onChange" | "onBlur" | "value">) {
  const [local, setLocal] = React.useState(value);
  React.useEffect(() => { setLocal(value); }, [value]);
  return (
    <Input
      {...rest}
      value={local}
      onChange={(e) => setLocal(e.target.value)}
      onBlur={() => { if (local !== value) onCommit(local); }}
    />
  );
}

/** Integer input that holds local state and only commits on blur. */
function BlurIntegerInput({ value, onCommit, disabled }: { value: number; onCommit: (v: number) => void; disabled?: boolean }) {
  const [local, setLocal] = React.useState(String(value));
  React.useEffect(() => { setLocal(String(value)); }, [value]);
  return (
    <Input
      {...integerInputProps}
      value={local}
      onChange={(e) => setLocal(sanitizeDigits(e.target.value))}
      onBlur={() => {
        const parsed = local === "" ? 0 : Number(local);
        if (parsed !== value) onCommit(parsed);
      }}
      disabled={disabled}
    />
  );
}

export function SettingsSubtitlesSection({
  settings,
  setSettings,
  saving,
  loading,
}: Props) {
  const t = useTranslate();
  const update = (patch: Partial<SubtitleSettings>) =>
    setSettings({ ...settings, ...patch });

  const disabled = !settings.enabled;
  const syncDisabled = disabled || !settings.syncEnabled;

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
        {saving ? <Loader2 className="h-3.5 w-3.5 animate-spin text-muted-foreground" /> : null}
      </div>

      <Toggle checked={settings.enabled} onChange={(v) => update({ enabled: v })} label={t("settings.sub.enabled")} />

      <div className={`space-y-6 ${disabled ? "pointer-events-none select-none opacity-40" : ""}`}>
        {/* OpenSubtitles Credentials */}
        <div className="space-y-1">
          <Label className="text-sm font-medium">{t("settings.sub.credentials")}</Label>
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
            <div className="space-y-1">
              <Label>{t("settings.sub.username")}</Label>
              <BlurInput
                value={settings.opensubtitlesUsername}
                onCommit={(v) => update({ opensubtitlesUsername: v })}
                disabled={disabled}
              />
            </div>
            <div className="space-y-1">
              <Label>{t("settings.sub.password")}</Label>
              <BlurInput
                type="password"
                value={settings.opensubtitlesPassword}
                onCommit={(v) => update({ opensubtitlesPassword: v })}
                disabled={disabled}
                placeholder="••••••••"
              />
            </div>
          </div>
        </div>

        {/* Languages */}
        <div className="space-y-1">
          <Label>{t("settings.sub.languages")}</Label>
          <SubtitleLanguagePicker
            value={settings.languages}
            onChange={(codes) => update({ languages: codes })}
          />
        </div>

        {/* Score Thresholds & Search */}
        <div className="space-y-3">
          <div className="grid grid-cols-1 gap-4 sm:grid-cols-2">
            <div className="space-y-1">
              <Label>{t("settings.sub.minScoreSeries")}</Label>
              <BlurIntegerInput
                value={settings.minimumScoreSeries}
                onCommit={(v) => update({ minimumScoreSeries: v })}
                disabled={disabled}
              />
            </div>
            <div className="space-y-1">
              <Label>{t("settings.sub.minScoreMovie")}</Label>
              <BlurIntegerInput
                value={settings.minimumScoreMovie}
                onCommit={(v) => update({ minimumScoreMovie: v })}
                disabled={disabled}
              />
            </div>
            <div className="space-y-1">
              <Label>{t("settings.sub.searchInterval")}</Label>
              <BlurIntegerInput
                value={settings.searchIntervalHours}
                onCommit={(v) => update({ searchIntervalHours: v })}
                disabled={disabled}
              />
            </div>
          </div>
          <p className="text-xs text-muted-foreground">{t("settings.sub.minScoreHelp")}</p>
        </div>

        {/* Toggles */}
        <div className="space-y-3">
          <Toggle checked={settings.autoDownloadOnImport} onChange={(v) => update({ autoDownloadOnImport: v })} label={t("settings.sub.autoDownload")} disabled={disabled} />
          <Toggle checked={!settings.includeAiTranslated} onChange={(v) => update({ includeAiTranslated: !v })} label={t("settings.sub.excludeAi")} disabled={disabled} />
          <Toggle checked={!settings.includeMachineTranslated} onChange={(v) => update({ includeMachineTranslated: !v })} label={t("settings.sub.excludeMachine")} disabled={disabled} />
        </div>

        {/* Sync */}
        <div className="space-y-3">
          <Toggle checked={settings.syncEnabled} onChange={(v) => update({ syncEnabled: v })} label={t("settings.sub.syncEnabled")} disabled={disabled} />
          <p className="text-xs text-muted-foreground">{t("settings.sub.syncEnabledHelp")}</p>
          <div className="grid grid-cols-1 gap-4 sm:grid-cols-3">
            <div className="space-y-1">
              <Label>{t("settings.sub.syncThresholdSeries")}</Label>
              <BlurIntegerInput
                value={settings.syncThresholdSeries}
                onCommit={(v) => update({ syncThresholdSeries: v })}
                disabled={syncDisabled}
              />
            </div>
            <div className="space-y-1">
              <Label>{t("settings.sub.syncThresholdMovie")}</Label>
              <BlurIntegerInput
                value={settings.syncThresholdMovie}
                onCommit={(v) => update({ syncThresholdMovie: v })}
                disabled={syncDisabled}
              />
            </div>
            <div className="space-y-1">
              <Label>{t("settings.sub.syncMaxOffset")}</Label>
              <BlurIntegerInput
                value={settings.syncMaxOffsetSeconds}
                onCommit={(v) => update({ syncMaxOffsetSeconds: v })}
                disabled={syncDisabled}
              />
            </div>
          </div>
          <p className="text-xs text-muted-foreground">{t("settings.sub.syncThresholdHelp")}</p>
          <p className="text-xs text-muted-foreground">{t("settings.sub.syncMaxOffsetHelp")}</p>
        </div>
      </div>
    </div>
  );
}
