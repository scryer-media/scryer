import type { TitleOptionUpdates } from "@/lib/types/title-options";

export const UNCHANGED_TITLE_EDIT_VALUE = "__unchanged__";
export const INHERIT_TITLE_EDIT_VALUE = "__inherit__";
export const ENABLED_TITLE_EDIT_VALUE = "enabled";
export const DISABLED_TITLE_EDIT_VALUE = "disabled";

export type TitleEditDraft = {
  metadataLanguage: string;
  qualityProfileId: string;
  rootFolderId: string;
  monitorType: string;
  useSeasonFolders: string;
  monitorSpecials: string;
  interSeasonMovies: string;
  fillerPolicy: string;
  recapPolicy: string;
};

export function initialTitleEditDraft(): TitleEditDraft {
  return {
    metadataLanguage: UNCHANGED_TITLE_EDIT_VALUE,
    qualityProfileId: UNCHANGED_TITLE_EDIT_VALUE,
    rootFolderId: UNCHANGED_TITLE_EDIT_VALUE,
    monitorType: UNCHANGED_TITLE_EDIT_VALUE,
    useSeasonFolders: UNCHANGED_TITLE_EDIT_VALUE,
    monitorSpecials: UNCHANGED_TITLE_EDIT_VALUE,
    interSeasonMovies: UNCHANGED_TITLE_EDIT_VALUE,
    fillerPolicy: UNCHANGED_TITLE_EDIT_VALUE,
    recapPolicy: UNCHANGED_TITLE_EDIT_VALUE,
  };
}

export function hasTitleEditChanges(
  draft: TitleEditDraft,
  initialDraft: TitleEditDraft,
): boolean {
  return (Object.keys(draft) as Array<keyof TitleEditDraft>).some(
    (key) =>
      draft[key] !== initialDraft[key] &&
      draft[key] !== UNCHANGED_TITLE_EDIT_VALUE,
  );
}

export function buildTitleEditChanges(
  draft: TitleEditDraft,
  initialDraft: TitleEditDraft,
): TitleOptionUpdates {
  const changes: TitleOptionUpdates = {};
  const changed = (key: keyof TitleEditDraft) =>
    draft[key] !== initialDraft[key] &&
    draft[key] !== UNCHANGED_TITLE_EDIT_VALUE;

  if (changed("qualityProfileId")) {
    changes.qualityProfileId =
      draft.qualityProfileId === INHERIT_TITLE_EDIT_VALUE
        ? null
        : draft.qualityProfileId;
  }
  if (changed("metadataLanguage")) {
    changes.metadataLanguage =
      draft.metadataLanguage === INHERIT_TITLE_EDIT_VALUE
        ? null
        : draft.metadataLanguage;
  }
  if (changed("rootFolderId")) {
    changes.rootFolderId = draft.rootFolderId;
  }
  if (changed("monitorType")) {
    changes.monitorType = draft.monitorType;
  }
  if (changed("useSeasonFolders")) {
    changes.useSeasonFolders =
      draft.useSeasonFolders === INHERIT_TITLE_EDIT_VALUE
        ? null
        : draft.useSeasonFolders === ENABLED_TITLE_EDIT_VALUE;
  }
  if (changed("monitorSpecials")) {
    changes.monitorSpecials =
      draft.monitorSpecials === ENABLED_TITLE_EDIT_VALUE;
  }
  if (changed("interSeasonMovies")) {
    changes.interSeasonMovies =
      draft.interSeasonMovies === ENABLED_TITLE_EDIT_VALUE;
  }
  if (changed("fillerPolicy")) {
    changes.fillerPolicy =
      draft.fillerPolicy === INHERIT_TITLE_EDIT_VALUE
        ? null
        : draft.fillerPolicy;
  }
  if (changed("recapPolicy")) {
    changes.recapPolicy =
      draft.recapPolicy === INHERIT_TITLE_EDIT_VALUE ? null : draft.recapPolicy;
  }
  return changes;
}

type TitleOptionSnapshot = {
  metadataLanguageOverride?: string | null;
  qualityProfileId?: string | null;
  rootFolderId?: string | null;
  monitorType?: string | null;
  useSeasonFolders?: boolean | null;
  monitorSpecials?: boolean | null;
  interSeasonMovies?: boolean | null;
  fillerPolicy?: string | null;
  recapPolicy?: string | null;
};

function normalizedOptionalString(
  value: string | null | undefined,
): string | null {
  return value?.trim() || null;
}

export function titleMatchesOptionUpdates(
  title: TitleOptionSnapshot,
  changes: TitleOptionUpdates,
): boolean {
  const stringFields = [
    "qualityProfileId",
    "rootFolderId",
    "monitorType",
    "fillerPolicy",
    "recapPolicy",
  ] as const;
  for (const field of stringFields) {
    if (
      changes[field] !== undefined &&
      normalizedOptionalString(title[field]) !==
        normalizedOptionalString(changes[field])
    ) {
      return false;
    }
  }

  if (
    changes.metadataLanguage !== undefined &&
    normalizedOptionalString(title.metadataLanguageOverride) !==
      normalizedOptionalString(changes.metadataLanguage)
  ) {
    return false;
  }

  const booleanFields = [
    "useSeasonFolders",
    "monitorSpecials",
    "interSeasonMovies",
  ] as const;
  for (const field of booleanFields) {
    if (
      changes[field] !== undefined &&
      (title[field] ?? null) !== (changes[field] ?? null)
    ) {
      return false;
    }
  }

  return true;
}
