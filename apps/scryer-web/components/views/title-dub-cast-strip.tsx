import React from "react";

import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { TitleCastStrip } from "@/components/views/title-cast-strip";
import type { TitleCastStripVariant } from "@/components/views/title-cast-strip";
import { useTranslate } from "@/lib/context/translate-context";
import type { TitleCreditRecord } from "@/lib/types/titles";
import {
  titleCastDubCreditsAlignedTo,
  titleCastDubLanguageLabel,
  titleCastDubLanguages,
  titleCastOriginalCredits,
  titleCastPreferredDubLanguage,
} from "@/lib/utils/title-cast";

type Props = {
  credits?: TitleCreditRecord[] | null;
  variant?: TitleCastStripVariant;
};

/**
 * Dub cast rail with a language picker. The options come from the credits the
 * title actually has, so today that is English alone (SMG only harvests ja/en);
 * a German or Spanish dub appears here on its own once SMG's VA languages are
 * widened, with no change to this component.
 */
export function TitleDubCastStrip({ credits, variant = "panel" }: Props) {
  const t = useTranslate();
  const languages = titleCastDubLanguages(credits);
  const [requestedLanguage, setRequestedLanguage] = React.useState<
    string | null
  >(null);
  // Resolved rather than stored, so a title whose dub languages change (or a
  // different title reusing this rail) never renders an empty selected option.
  const selectedLanguage = titleCastPreferredDubLanguage(
    languages,
    requestedLanguage,
  );

  if (!selectedLanguage) {
    return null;
  }

  const picker = (
    <Select
      value={selectedLanguage}
      onValueChange={(value) => setRequestedLanguage(value)}
    >
      <SelectTrigger
        aria-label={t("title.dubCastLanguage")}
        className="h-8 w-[136px] rounded-[10px] border-[var(--scry-border2)] bg-[var(--scry-inset)] text-[12px] text-[var(--scry-body)] shadow-none"
      >
        <SelectValue placeholder={t("title.dubCastLanguage")} />
      </SelectTrigger>
      <SelectContent>
        {languages.map((language) => (
          <SelectItem key={language} value={language}>
            {titleCastDubLanguageLabel(language)}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  );

  return (
    <TitleCastStrip
      credits={titleCastDubCreditsAlignedTo(
        credits,
        selectedLanguage,
        titleCastOriginalCredits(credits),
      )}
      variant={variant}
      titleKey="title.dubCast"
      headerAccessory={picker}
      keepPlaceholders
    />
  );
}
