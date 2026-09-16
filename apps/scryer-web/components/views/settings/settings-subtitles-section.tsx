import * as React from "react";
import { Button } from "@/components/ui/button";
import { Input, integerInputProps, sanitizeDigits } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Download } from "lucide-react";
import { SettingsToggleSwitch } from "@/components/common/settings-toggle-switch";
import {
  PluginInstallProgressBar,
  type PluginInstallProgressRecord,
} from "@/components/views/settings/settings-plugins-section";
import { useTranslate } from "@/lib/context/translate-context";
import { SubtitleLanguagePicker } from "@/components/common/subtitle-language-picker";
import type { SubtitleSettings } from "@/lib/types/settings";
import { getSubtitleLanguage } from "@/lib/constants/subtitle-languages";
import { selectorId } from "@/lib/utils/dom-ids";
import { LoadingMark } from "@/components/common/loading-mark";

type Props = {
  settings: SubtitleSettings;
  setSettings: (s: SubtitleSettings) => void;
  loading: boolean;
  syncPluginActive: boolean;
  syncPluginAvailable: boolean;
  syncPluginBlockedReason?: string | null;
  syncPluginError?: string | null;
  syncPluginInstalling: boolean;
  syncPluginLoading: boolean;
  syncPluginName: string;
  syncPluginProgress?: PluginInstallProgressRecord | null;
  onInstallSyncPlugin: () => void;
};

const SUBTITLES_INSET_CLASS =
  "rounded-[12px] border border-[var(--scry-line2)] bg-[var(--scry-card2)] p-4";
const SUBTITLES_ROW_CLASS =
  "flex flex-col gap-2 rounded-[10px] border border-[var(--scry-border3)] bg-[var(--scry-inset)] px-3 py-2 sm:flex-row sm:items-center sm:justify-between";
const SUBTITLES_LABEL_CLASS = "text-[var(--scry-ink2)]";
const SUBTITLES_MUTED_CLASS = "text-[var(--scry-muted3)]";

function Toggle({
  id,
  checked,
  onChange,
  label,
  disabled,
}: {
  id: string;
  checked: boolean;
  onChange: (v: boolean) => void;
  label: string;
  disabled?: boolean;
}) {
  return (
    <div className="flex items-center gap-3">
      <Label
        htmlFor={id}
        className={disabled ? SUBTITLES_MUTED_CLASS : SUBTITLES_LABEL_CLASS}
      >
        {label}
      </Label>
      <SettingsToggleSwitch
        id={id}
        checked={checked}
        disabled={disabled}
        ariaLabel={label}
        onChange={onChange}
      />
    </div>
  );
}

/** Integer input that holds local state and only commits on blur. */
function BlurIntegerInput({
  id,
  value,
  onCommit,
  disabled,
  min = 0,
  max,
}: {
  id: string;
  value: number;
  onCommit: (v: number) => void;
  disabled?: boolean;
  min?: number;
  max?: number;
}) {
  const [local, setLocal] = React.useState(String(value));
  React.useEffect(() => { setLocal(String(value)); }, [value]);
  return (
    <Input
      id={id}
      {...integerInputProps}
      value={local}
      onChange={(e) => setLocal(sanitizeDigits(e.target.value))}
      onBlur={() => {
        let parsed = local === "" ? min : Number(local);
        parsed = Math.max(min, max == null ? parsed : Math.min(max, parsed));
        setLocal(String(parsed));
        if (parsed !== value) onCommit(parsed);
      }}
      disabled={disabled}
    />
  );
}

export function SettingsSubtitlesSection({
  settings,
  setSettings,
  loading,
  syncPluginActive,
  syncPluginAvailable,
  syncPluginBlockedReason,
  syncPluginError,
  syncPluginInstalling,
  syncPluginLoading,
  syncPluginName,
  syncPluginProgress,
  onInstallSyncPlugin,
}: Props) {
  const t = useTranslate();
  const update = (patch: Partial<SubtitleSettings>) =>
    setSettings({ ...settings, ...patch });
  const updateLanguageCodes = (codes: string[]) => {
    const existingByCode = new Map(
      settings.languages.map((language) => [language.code, language]),
    );
    update({
      languages: codes.map((code) => existingByCode.get(code) ?? {
        code,
        hearingImpaired: false,
        forced: false,
      }),
    });
  };

  const disabled = !settings.enabled;
  const syncDisabled = disabled || !settings.syncEnabled;
  const syncPluginDescription =
    syncPluginBlockedReason ??
    (syncPluginLoading
      ? t("settings.sub.syncPluginLoadingCatalog")
      : syncPluginAvailable
        ? t("settings.sub.syncPluginRequiredDescription", { plugin: syncPluginName })
        : t("settings.sub.syncPluginUnavailable"));

  if (loading) {
    return (
      <div className={`flex items-center gap-2 text-sm ${SUBTITLES_MUTED_CLASS}`}>
        <LoadingMark className="h-4 w-4" />
        {t("label.loading")}
      </div>
    );
  }

  return (
    <div className="space-y-4 text-sm">
      <div className={`space-y-4 ${disabled ? "pointer-events-none select-none opacity-40" : ""}`}>
        {/* Languages */}
        <div className={`${SUBTITLES_INSET_CLASS} space-y-3`}>
          <Label className={SUBTITLES_LABEL_CLASS}>{t("settings.sub.languages")}</Label>
          <div id="settings-subtitles-languages">
            <SubtitleLanguagePicker
              value={settings.languages.map((language) => language.code)}
              onChange={updateLanguageCodes}
            />
          </div>
          {settings.languages.length > 0 ? (
            <div className="space-y-2 pt-2">
              {settings.languages.map((language) => {
                const subtitleLanguage = getSubtitleLanguage(language.code);
                return (
                  <div
                    key={language.code}
                    className={SUBTITLES_ROW_CLASS}
                  >
                    <div className="min-w-0">
                      <p className="text-sm font-medium text-[var(--scry-ink2)]">
                        {subtitleLanguage?.name ?? language.code}
                      </p>
                      <p className={`text-xs ${SUBTITLES_MUTED_CLASS}`}>
                        {subtitleLanguage?.nativeName ?? language.code} · {language.code}
                      </p>
                    </div>
                    <div className="flex flex-wrap gap-3">
                      <Toggle
                        id={selectorId("settings-subtitles-language", language.code, "hearing-impaired")}
                        checked={language.hearingImpaired}
                        onChange={(value) =>
                          update({
                            languages: settings.languages.map((entry) =>
                              entry.code === language.code
                                ? { ...entry, hearingImpaired: value }
                                : entry,
                            ),
                          })
                        }
                        label={t("settings.sub.hiPreference")}
                        disabled={disabled}
                      />
                      <Toggle
                        id={selectorId("settings-subtitles-language", language.code, "forced")}
                        checked={language.forced}
                        onChange={(value) =>
                          update({
                            languages: settings.languages.map((entry) =>
                              entry.code === language.code
                                ? { ...entry, forced: value }
                                : entry,
                            ),
                          })
                        }
                        label={t("settings.sub.forcedOnly")}
                        disabled={disabled}
                      />
                    </div>
                  </div>
                );
              })}
            </div>
          ) : null}
        </div>

        {/* Score Thresholds & Search */}
        <div className={`${SUBTITLES_INSET_CLASS} space-y-3`}>
          <div className="grid grid-cols-1 gap-4 sm:grid-cols-2">
            <div className="space-y-1">
              <Label className={SUBTITLES_LABEL_CLASS}>{t("settings.sub.minScoreSeries")}</Label>
              <BlurIntegerInput
                id="settings-subtitles-min-score-series"
                value={settings.minimumScoreSeries}
                onCommit={(v) => update({ minimumScoreSeries: v })}
                max={100}
                disabled={disabled}
              />
            </div>
            <div className="space-y-1">
              <Label className={SUBTITLES_LABEL_CLASS}>{t("settings.sub.minScoreMovie")}</Label>
              <BlurIntegerInput
                id="settings-subtitles-min-score-movie"
                value={settings.minimumScoreMovie}
                onCommit={(v) => update({ minimumScoreMovie: v })}
                max={100}
                disabled={disabled}
              />
            </div>
            <div className="space-y-1">
              <Label className={SUBTITLES_LABEL_CLASS}>{t("settings.sub.searchInterval")}</Label>
              <BlurIntegerInput
                id="settings-subtitles-search-interval-hours"
                value={settings.searchIntervalHours}
                onCommit={(v) => update({ searchIntervalHours: v })}
                disabled={disabled}
              />
            </div>
          </div>
          <p className={`text-xs ${SUBTITLES_MUTED_CLASS}`}>{t("settings.sub.minScoreHelp")}</p>
        </div>

        {/* Toggles */}
        <div className={`${SUBTITLES_INSET_CLASS} space-y-3`}>
          <Toggle id="settings-subtitles-auto-download-on-import" checked={settings.autoDownloadOnImport} onChange={(v) => update({ autoDownloadOnImport: v })} label={t("settings.sub.autoDownload")} disabled={disabled} />
          <Toggle id="settings-subtitles-exclude-ai-translated" checked={!settings.includeAiTranslated} onChange={(v) => update({ includeAiTranslated: !v })} label={t("settings.sub.excludeAi")} disabled={disabled} />
          <Toggle id="settings-subtitles-exclude-machine-translated" checked={!settings.includeMachineTranslated} onChange={(v) => update({ includeMachineTranslated: !v })} label={t("settings.sub.excludeMachine")} disabled={disabled} />
        </div>
      </div>

      {/* Sync */}
      {syncPluginActive ? (
        <div className={`${SUBTITLES_INSET_CLASS} space-y-3 ${disabled ? "pointer-events-none select-none opacity-40" : ""}`}>
          <Toggle id="settings-subtitles-sync-enabled" checked={settings.syncEnabled} onChange={(v) => update({ syncEnabled: v })} label={t("settings.sub.syncEnabled")} disabled={disabled} />
          <p className={`text-xs ${SUBTITLES_MUTED_CLASS}`}>{t("settings.sub.syncEnabledHelp")}</p>
          <div className="grid grid-cols-1 gap-4 sm:grid-cols-3">
            <div className="space-y-1">
              <Label className={SUBTITLES_LABEL_CLASS}>{t("settings.sub.syncThresholdSeries")}</Label>
              <BlurIntegerInput
                id="settings-subtitles-sync-threshold-series"
                value={settings.syncThresholdSeries}
                onCommit={(v) => update({ syncThresholdSeries: v })}
                max={100}
                disabled={syncDisabled}
              />
            </div>
            <div className="space-y-1">
              <Label className={SUBTITLES_LABEL_CLASS}>{t("settings.sub.syncThresholdMovie")}</Label>
              <BlurIntegerInput
                id="settings-subtitles-sync-threshold-movie"
                value={settings.syncThresholdMovie}
                onCommit={(v) => update({ syncThresholdMovie: v })}
                max={100}
                disabled={syncDisabled}
              />
            </div>
            <div className="space-y-1">
              <Label className={SUBTITLES_LABEL_CLASS}>{t("settings.sub.syncMaxOffset")}</Label>
              <BlurIntegerInput
                id="settings-subtitles-sync-max-offset-seconds"
                value={settings.syncMaxOffsetSeconds}
                onCommit={(v) => update({ syncMaxOffsetSeconds: v })}
                disabled={syncDisabled}
              />
            </div>
          </div>
          <p className={`text-xs ${SUBTITLES_MUTED_CLASS}`}>{t("settings.sub.syncThresholdHelp")}</p>
          <p className={`text-xs ${SUBTITLES_MUTED_CLASS}`}>{t("settings.sub.syncMaxOffsetHelp")}</p>
        </div>
      ) : (
        <div id="settings-subtitles-sync-plugin-required" className={SUBTITLES_INSET_CLASS}>
          <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
            <div className="space-y-1">
              <p className="text-sm font-medium text-[var(--scry-ink2)]">{t("settings.sub.syncPluginRequiredTitle")}</p>
              <p className={`text-xs ${SUBTITLES_MUTED_CLASS}`}>
                {syncPluginDescription}
              </p>
              {syncPluginError ? (
                <p id="settings-subtitles-sync-plugin-error" className="text-xs text-destructive">
                  {syncPluginError}
                </p>
              ) : null}
              {syncPluginProgress ? (
                <PluginInstallProgressBar
                  progress={syncPluginProgress}
                  id="settings-subtitles-sync-plugin-progress"
                />
              ) : null}
            </div>
            <Button
              id="settings-subtitles-install-sync-plugin"
              type="button"
              size="sm"
              disabled={
                syncPluginInstalling ||
                syncPluginLoading ||
                !syncPluginAvailable ||
                Boolean(syncPluginBlockedReason)
              }
              onClick={onInstallSyncPlugin}
            >
              {syncPluginInstalling || syncPluginLoading ? (
                <LoadingMark className="h-4 w-4" />
              ) : (
                <Download className="h-4 w-4" />
              )}
              {syncPluginInstalling
                ? t("settings.sub.syncPluginInstalling")
                : t("settings.sub.syncPluginInstall")}
            </Button>
          </div>
        </div>
      )}
    </div>
  );
}
