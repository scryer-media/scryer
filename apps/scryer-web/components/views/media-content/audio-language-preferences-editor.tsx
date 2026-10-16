import { AudioLanguagePicker } from "@/components/common/audio-language-picker";
import { InfoHelp } from "@/components/common/info-help";
import { PreferredAudioLanguageList } from "@/components/common/preferred-audio-language-list";
import { SettingsToggleSwitch } from "@/components/common/settings-toggle-switch";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { AUDIO_LANGUAGE_PRIORITY_OPTIONS } from "@/lib/constants/audio-languages";
import { useTranslate } from "@/lib/context/translate-context";
import type { AudioLanguagePreferences, AudioLanguagePriority } from "@/lib/types";

/** Preferred and optional audio languages of one facet. */
export function AudioLanguagePreferencesEditor({
  value,
  onChange,
  disabled,
}: {
  value: AudioLanguagePreferences;
  onChange: (update: Partial<AudioLanguagePreferences>) => void;
  disabled: boolean;
}) {
  const t = useTranslate();

  return (
    <div className="max-w-xl space-y-5">
      <div className="space-y-2">
        <Label className="inline-flex items-center gap-2">
          {t("settings.audioPreferredLanguages")}
          <InfoHelp
            text={t("settings.audioPreferredLanguagesHint")}
            ariaLabel={t("settings.audioPreferredLanguages")}
          />
        </Label>
        <PreferredAudioLanguageList
          value={value.preferredAudioLanguages}
          onChange={(languages) => onChange({ preferredAudioLanguages: languages })}
          disabled={disabled}
        />
      </div>

      <div className="space-y-2">
        <Label className="inline-flex items-center gap-2">
          {t("settings.audioOptionalLanguages")}
          <InfoHelp
            text={t("settings.audioOptionalLanguagesHint")}
            ariaLabel={t("settings.audioOptionalLanguages")}
          />
        </Label>
        <AudioLanguagePicker
          value={value.optionalAudioLanguages}
          onChange={(languages) => onChange({ optionalAudioLanguages: languages })}
          disabled={disabled}
          ariaLabel={t("settings.audioOptionalLanguages")}
        />
      </div>

      <div className="space-y-2">
        <Label className="inline-flex items-center gap-2">
          {t("settings.audioLanguagePriority")}
          <InfoHelp
            text={t("settings.audioLanguagePriorityHint")}
            ariaLabel={t("settings.audioLanguagePriority")}
          />
        </Label>
        <Select
          value={value.audioLanguagePriority}
          onValueChange={(priority) =>
            onChange({ audioLanguagePriority: priority as AudioLanguagePriority })
          }
          disabled={disabled}
        >
          <SelectTrigger className="w-full">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {AUDIO_LANGUAGE_PRIORITY_OPTIONS.map((option) => (
              <SelectItem key={option.value} value={option.value}>
                {t(option.labelKey)}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>

      <div className="flex items-center justify-between gap-4 rounded-[12px] border border-[var(--scry-border)] bg-[var(--scry-card2)] p-4">
        <div className="min-w-0">
          <Label className="text-[13.5px] font-semibold text-[var(--scry-body)]">
            {t("settings.audioOptionalKeepSearching")}
          </Label>
          <p className="mt-1 text-xs leading-relaxed text-[var(--scry-muted3)]">
            {t("settings.audioOptionalKeepSearchingHint")}
          </p>
        </div>
        <SettingsToggleSwitch
          checked={value.optionalAudioLanguagesKeepSearching}
          ariaLabel={t("settings.audioOptionalKeepSearching")}
          disabled={disabled}
          onChange={(checked) => onChange({ optionalAudioLanguagesKeepSearching: checked })}
        />
      </div>
    </div>
  );
}
