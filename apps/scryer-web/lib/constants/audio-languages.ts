import {
  SUBTITLE_LANGUAGES,
  type SubtitleLanguage,
} from "./subtitle-languages.ts";

export const ORIGINAL_AUDIO_LANGUAGE_CODE = "original";

/**
 * The spellings audio has to use, keyed by the ones the shared table uses.
 *
 * ISO 639-2 gives twenty-odd languages two codes, and Scryer's halves picked
 * different ones: {@link SUBTITLE_LANGUAGES} is bibliographic (`ger`, `fre`,
 * `arm`) because subtitle providers speak that, while the backend canonicalizes
 * every audio requirement to the terminological code before storing it, so
 * `ger` comes back as `deu` and `arm` as `hye`.
 *
 * The picker matches an option to a stored value by exact code. Untranslated,
 * a language picked here came back unrecognised: it rendered as a bare code in
 * the summary, its row in the list stayed unticked, and unticking sent the
 * bibliographic spelling the stored value no longer carried -- so the entry
 * could not be removed at all. Only codes that actually differ appear here;
 * `eng`, `ara` and the rest are the same in both sets, which is why the
 * breakage looked arbitrary.
 *
 * `scc` is the retired Serbian code, which the backend resolves to `srp`, and
 * `zht`/`pob` are subtitle-only distinctions it folds into `zho`/`por`.
 */
const AUDIO_CODE_ALIASES: Record<string, string> = {
  alb: "sqi",
  arm: "hye",
  baq: "eus",
  chi: "zho",
  cze: "ces",
  dut: "nld",
  fre: "fra",
  geo: "kat",
  ger: "deu",
  gre: "ell",
  ice: "isl",
  mac: "mkd",
  may: "msa",
  per: "fas",
  pob: "por",
  rum: "ron",
  scc: "srp",
  slo: "slk",
  zht: "zho",
};

/**
 * Names for codes two subtitle entries collapse onto. Audio cannot express the
 * script a Chinese track is subtitled in, so the surviving entry must not claim
 * to be one of them.
 */
const AUDIO_LANGUAGE_NAMES: Record<
  string,
  Pick<SubtitleLanguage, "name" | "nativeName">
> = {
  zho: { name: "Chinese", nativeName: "中文" },
};

/**
 * The shared table under audio's spelling, first entry winning wherever two
 * collapse onto one code -- two rows sharing a stored code would tick and untick
 * together, and one of them could never be reached.
 */
export const AUDIO_LANGUAGES: SubtitleLanguage[] = (() => {
  const byCode = new Map<string, SubtitleLanguage>();
  for (const language of SUBTITLE_LANGUAGES) {
    const code = AUDIO_CODE_ALIASES[language.code] ?? language.code;
    if (byCode.has(code)) {
      continue;
    }
    byCode.set(code, { ...language, ...AUDIO_LANGUAGE_NAMES[code], code });
  }
  return Array.from(byCode.values());
})();

/**
 * Lookup for display, which is deliberately more forgiving than the option
 * list: a bibliographic code still names its language, so a value stored before
 * the two halves were reconciled reads as a language rather than as a code.
 */
const audioLanguageByCode = new Map<string, SubtitleLanguage>(
  AUDIO_LANGUAGES.map((language) => [language.code, language]),
);
for (const [alias, code] of Object.entries(AUDIO_CODE_ALIASES)) {
  const language = audioLanguageByCode.get(code);
  if (language) {
    audioLanguageByCode.set(alias, language);
  }
}

export function audioLanguageOptions(
  originalLanguageLabel: string,
): SubtitleLanguage[] {
  return [
    {
      code: ORIGINAL_AUDIO_LANGUAGE_CODE,
      name: originalLanguageLabel,
      nativeName: originalLanguageLabel,
    },
    ...AUDIO_LANGUAGES,
  ];
}

export function audioLanguageLabel(
  code: string,
  originalLanguageLabel: string,
): string {
  const normalized = code.trim().toLowerCase();
  if (normalized === ORIGINAL_AUDIO_LANGUAGE_CODE) {
    return originalLanguageLabel;
  }
  return audioLanguageByCode.get(normalized)?.name ?? code;
}

export function formatAudioLanguageLabels(
  codes: string[],
  originalLanguageLabel: string,
): string {
  return codes
    .map((code) => audioLanguageLabel(code, originalLanguageLabel))
    .join(", ");
}
