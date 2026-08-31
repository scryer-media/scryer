import * as React from "react";
import {
  SubtitleLanguagePicker,
  type SubtitleLanguagePickerProps,
} from "@/components/common/subtitle-language-picker";
import { audioLanguageOptions } from "@/lib/constants/audio-languages";
import { useTranslate } from "@/lib/context/translate-context";

type AudioLanguagePickerProps = Omit<
  SubtitleLanguagePickerProps,
  "languageOptions"
>;

export const AudioLanguagePicker = React.memo(function AudioLanguagePicker(
  props: AudioLanguagePickerProps,
) {
  const t = useTranslate();
  const languageOptions = React.useMemo(
    () => audioLanguageOptions(t("title.originalAudioLanguagePerTitle")),
    [t],
  );

  return (
    <SubtitleLanguagePicker {...props} languageOptions={languageOptions} />
  );
});
