import {
  SUBTITLE_LANGUAGES,
  getSubtitleLanguage,
  type SubtitleLanguage,
} from "./subtitle-languages.ts";

export const ORIGINAL_AUDIO_LANGUAGE_CODE = "original";

export function audioLanguageOptions(
  originalLanguageLabel: string,
): SubtitleLanguage[] {
  return [
    {
      code: ORIGINAL_AUDIO_LANGUAGE_CODE,
      name: originalLanguageLabel,
      nativeName: originalLanguageLabel,
    },
    ...SUBTITLE_LANGUAGES,
  ];
}

export function audioLanguageLabel(
  code: string,
  originalLanguageLabel: string,
): string {
  if (code.trim().toLowerCase() === ORIGINAL_AUDIO_LANGUAGE_CODE) {
    return originalLanguageLabel;
  }
  return getSubtitleLanguage(code)?.name ?? code;
}

export function formatAudioLanguageLabels(
  codes: string[],
  originalLanguageLabel: string,
): string {
  return codes
    .map((code) => audioLanguageLabel(code, originalLanguageLabel))
    .join(", ");
}
