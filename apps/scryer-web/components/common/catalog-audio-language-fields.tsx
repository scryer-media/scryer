import { AudioLanguagePicker } from "@/components/common/audio-language-picker";
import { PreferredAudioLanguageList } from "@/components/common/preferred-audio-language-list";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { formatAudioLanguageLabels } from "@/lib/constants/audio-languages";
import { useTranslate } from "@/lib/context/translate-context";
import {
  inheritedAudioLanguagesLabel,
  type CatalogAudioLanguageDraft,
  type CatalogAudioLanguageMode,
} from "@/lib/utils/catalog-audio-languages";
import { selectorId } from "@/lib/utils/dom-ids";

type Props = {
  /** `add-to-catalog` or `request-media`; every id hangs off it. */
  idPrefix: string;
  value: CatalogAudioLanguageDraft;
  onChange: (value: CatalogAudioLanguageDraft) => void;
  /** What the title would inherit, or null when it could not be read. */
  inheritedRequired: string[] | null;
  inheritedPreferred: string[] | null;
  disabled: boolean;
};

/**
 * The required and preferred audio language choices of the add and request
 * dialogs: two cells of the dialog's two-column grid. Each defaults to
 * inheriting, and its label states what would be inherited when known.
 */
export function CatalogAudioLanguageFields({
  idPrefix,
  value,
  onChange,
  inheritedRequired,
  inheritedPreferred,
  disabled,
}: Props) {
  const t = useTranslate();
  const originalLabel = t("title.originalAudioLanguagePerTitle");
  const format = (codes: string[]) =>
    formatAudioLanguageLabels(codes, originalLabel);
  const inheritLabel = t("search.addConfigInheritLibrary");
  const noneLabel = t("label.none");
  const requiredLabelId = selectorId(idPrefix, "required-audio-label");
  const preferredLabelId = selectorId(idPrefix, "preferred-audio-label");

  return (
    <>
      <div className="space-y-1">
        <span
          id={requiredLabelId}
          className="block text-xs font-medium text-card-foreground"
        >
          {t("search.addConfigRequiredAudio")}
        </span>
        <Select
          value={value.requiredMode}
          onValueChange={(mode) =>
            onChange({
              ...value,
              requiredMode: mode as CatalogAudioLanguageMode,
            })
          }
          disabled={disabled}
        >
          <SelectTrigger
            id={selectorId(idPrefix, "required-audio")}
            aria-labelledby={requiredLabelId}
            className="h-12 w-full"
          >
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem
              id={selectorId(idPrefix, "required-audio-option", "inherit")}
              value="inherit"
            >
              {inheritedAudioLanguagesLabel(
                inheritLabel,
                inheritedRequired,
                format,
                noneLabel,
              )}
            </SelectItem>
            <SelectItem
              id={selectorId(idPrefix, "required-audio-option", "none")}
              value="none"
            >
              {t("search.addConfigAudioRequireNone")}
            </SelectItem>
            <SelectItem
              id={selectorId(idPrefix, "required-audio-option", "choose")}
              value="choose"
            >
              {t("search.addConfigAudioChoose")}
            </SelectItem>
          </SelectContent>
        </Select>
        {value.requiredMode === "choose" ? (
          <AudioLanguagePicker
            value={value.required}
            onChange={(required) => onChange({ ...value, required })}
            disabled={disabled}
            triggerId={selectorId(idPrefix, "required-audio-languages")}
            ariaLabel={t("search.addConfigRequiredAudio")}
          />
        ) : null}
      </div>

      <div className="space-y-1">
        <span
          id={preferredLabelId}
          className="block text-xs font-medium text-card-foreground"
        >
          {t("search.addConfigPreferredAudio")}
        </span>
        <Select
          value={value.preferredMode}
          onValueChange={(mode) =>
            onChange({
              ...value,
              preferredMode: mode === "choose" ? "choose" : "inherit",
            })
          }
          disabled={disabled}
        >
          <SelectTrigger
            id={selectorId(idPrefix, "preferred-audio")}
            aria-labelledby={preferredLabelId}
            className="h-12 w-full"
          >
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem
              id={selectorId(idPrefix, "preferred-audio-option", "inherit")}
              value="inherit"
            >
              {inheritedAudioLanguagesLabel(
                inheritLabel,
                inheritedPreferred,
                format,
                noneLabel,
              )}
            </SelectItem>
            <SelectItem
              id={selectorId(idPrefix, "preferred-audio-option", "choose")}
              value="choose"
            >
              {t("search.addConfigAudioChoose")}
            </SelectItem>
          </SelectContent>
        </Select>
        {value.preferredMode === "choose" ? (
          <div className="space-y-2">
            <PreferredAudioLanguageList
              value={value.preferred}
              onChange={(preferred) => onChange({ ...value, preferred })}
              disabled={disabled}
              triggerId={selectorId(idPrefix, "preferred-audio-languages")}
            />
          </div>
        ) : null}
      </div>
    </>
  );
}
