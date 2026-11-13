/**
 * Audio language choices for a title that is being added or requested.
 *
 * Each field starts at "inherit", which sends nothing, so the new title
 * follows its library's and then its facet's languages. Required languages
 * also offer "none": an explicit empty list that requires no language
 * whatever the library asks. Choosing languages without ticking any sends
 * nothing, the same as inheriting.
 */
export type CatalogAudioLanguageMode = "inherit" | "none" | "choose";

export type CatalogAudioLanguageDraft = {
  requiredMode: CatalogAudioLanguageMode;
  required: string[];
  preferredMode: Exclude<CatalogAudioLanguageMode, "none">;
  preferred: string[];
};

export const INHERIT_CATALOG_AUDIO_LANGUAGES: CatalogAudioLanguageDraft = {
  requiredMode: "inherit",
  required: [],
  preferredMode: "inherit",
  preferred: [],
};

export function catalogAudioLanguagesForSubmit(
  draft: CatalogAudioLanguageDraft,
): { required?: string[]; preferred?: string[] } {
  const required =
    draft.requiredMode === "none"
      ? []
      : draft.requiredMode === "choose" && draft.required.length > 0
        ? draft.required
        : undefined;
  const preferred =
    draft.preferredMode === "choose" && draft.preferred.length > 0
      ? draft.preferred
      : undefined;
  return {
    ...(required ? { required } : {}),
    ...(preferred ? { preferred } : {}),
  };
}

/**
 * The inherit option's label: the plain inherit wording, followed by what the
 * title would inherit when the library's settings could be read. `null`
 * inherited languages mean they could not be read.
 */
export function inheritedAudioLanguagesLabel(
  inheritLabel: string,
  inherited: string[] | null | undefined,
  formatLanguages: (codes: string[]) => string,
  noneLabel: string,
): string {
  if (inherited == null) {
    return inheritLabel;
  }
  const languages = inherited.length > 0 ? formatLanguages(inherited) : "";
  return `${inheritLabel} — ${languages || noneLabel}`;
}

/**
 * The choices that stand for stored languages: null inherits, an empty
 * required list requires none, and any other list is a choice.
 */
export function catalogAudioLanguageDraftFrom(
  required: string[] | null | undefined,
  preferred: string[] | null | undefined,
): CatalogAudioLanguageDraft {
  return {
    requiredMode:
      required == null ? "inherit" : required.length === 0 ? "none" : "choose",
    required: required ?? [],
    preferredMode: preferred?.length ? "choose" : "inherit",
    preferred: preferred ?? [],
  };
}

/**
 * What an approval sends for the audio languages, given what the requester
 * asked for and where the approver left the pickers. A field the approver left
 * at the requested value is omitted, which grants what the requester asked.
 * Otherwise `null` inherits the library's languages (an empty choice counts as
 * inheriting), an empty required list requires none, and a list is the
 * approver's own choice.
 */
export function approvalAudioLanguageOverrides(
  requested: {
    required: string[] | null | undefined;
    preferred: string[] | null | undefined;
  },
  draft: CatalogAudioLanguageDraft,
): {
  requiredAudioLanguages?: string[] | null;
  preferredAudioLanguages?: string[] | null;
} {
  const chosen = catalogAudioLanguagesForSubmit(draft);
  const required = chosen.required ?? null;
  const preferred = chosen.preferred ?? null;
  const requestedRequired = requested.required ?? null;
  const requestedPreferred = requested.preferred?.length
    ? requested.preferred
    : null;
  return {
    ...(sameAudioLanguages(required, requestedRequired)
      ? {}
      : { requiredAudioLanguages: required }),
    ...(sameAudioLanguages(preferred, requestedPreferred)
      ? {}
      : { preferredAudioLanguages: preferred }),
  };
}

function sameAudioLanguages(a: string[] | null, b: string[] | null): boolean {
  if (a === null || b === null) return a === b;
  return a.length === b.length && a.every((code, index) => code === b[index]);
}
