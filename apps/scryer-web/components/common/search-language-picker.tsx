import * as React from "react";
import {
  SubtitleLanguagePicker,
  type SubtitleLanguagePickerProps,
} from "@/components/common/subtitle-language-picker";
import { SEARCH_LANGUAGES } from "@/lib/constants/audio-languages";

type SearchLanguagePickerProps = Omit<
  SubtitleLanguagePickerProps,
  "languageOptions"
>;

/**
 * Release-search languages for a library or title. The options are the
 * server's accepted set, so every stored code renders as a ticked option.
 */
export const SearchLanguagePicker = React.memo(function SearchLanguagePicker(
  props: SearchLanguagePickerProps,
) {
  return <SubtitleLanguagePicker {...props} languageOptions={SEARCH_LANGUAGES} />;
});
