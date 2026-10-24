import { ArrowDown, ArrowUp } from "lucide-react";
import { AudioLanguagePicker } from "@/components/common/audio-language-picker";
import { Button } from "@/components/ui/button";
import { audioLanguageLabel } from "@/lib/constants/audio-languages";
import { useTranslate } from "@/lib/context/translate-context";

function moveItem(values: string[], index: number, offset: number): string[] {
  const target = index + offset;
  if (target < 0 || target >= values.length) {
    return values;
  }
  const next = [...values];
  [next[index], next[target]] = [next[target], next[index]];
  return next;
}

/**
 * The ordered preferred audio languages: a picker plus up/down controls,
 * because the first language a release carries sets its rank.
 */
export function PreferredAudioLanguageList({
  value,
  onChange,
  disabled,
}: {
  value: string[];
  onChange: (languages: string[]) => void;
  disabled: boolean;
}) {
  const t = useTranslate();
  const originalLabel = t("title.originalAudioLanguagePerTitle");
  return (
    <>
      <AudioLanguagePicker
        value={value}
        onChange={onChange}
        disabled={disabled}
        ariaLabel={t("settings.audioPreferredLanguages")}
      />
      {value.length > 1 ? (
        <ol className="space-y-1">
          {value.map((code, index) => {
            const language = audioLanguageLabel(code, originalLabel);
            return (
              <li
                key={code}
                className="flex items-center gap-2 rounded-[9px] border border-[var(--scry-border2)] bg-[var(--scry-bg)] px-3 py-1.5 text-sm"
              >
                <span className="w-5 text-[var(--scry-muted3)]">{index + 1}.</span>
                <span className="flex-1">{language}</span>
                <Button
                  type="button"
                  variant="ghost"
                  size="icon"
                  disabled={disabled || index === 0}
                  aria-label={t("settings.audioPreferredMoveUp", { language })}
                  onClick={() => onChange(moveItem(value, index, -1))}
                >
                  <ArrowUp className="h-4 w-4" />
                </Button>
                <Button
                  type="button"
                  variant="ghost"
                  size="icon"
                  disabled={disabled || index === value.length - 1}
                  aria-label={t("settings.audioPreferredMoveDown", { language })}
                  onClick={() => onChange(moveItem(value, index, 1))}
                >
                  <ArrowDown className="h-4 w-4" />
                </Button>
              </li>
            );
          })}
        </ol>
      ) : null}
    </>
  );
}
