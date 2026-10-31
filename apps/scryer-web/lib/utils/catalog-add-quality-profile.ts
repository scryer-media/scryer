export const INHERIT_CATALOG_QUALITY_PROFILE_VALUE = "__inherit__";

type QualityProfileOption = {
  id: string;
  name: string;
};

type LibraryQualityProfile = {
  qualityProfileId?: string | null;
};

type RootFolder = {
  id?: string;
  isDefault: boolean;
};

type CatalogAddDraft = {
  libraryId?: string;
  qualityProfileId?: string;
  rootFolderId?: string;
};

function explicitProfileId(
  profileId: string | null | undefined,
): string | undefined {
  const normalized = profileId?.trim();
  return normalized || undefined;
}

export function effectiveCatalogQualityProfileId(
  library: LibraryQualityProfile | null | undefined,
  fallbackProfileId: string,
): string {
  return (
    explicitProfileId(library?.qualityProfileId) ?? fallbackProfileId.trim()
  );
}

export function catalogQualityProfileSelectValue(
  profileId: string | null | undefined,
): string {
  return explicitProfileId(profileId) ?? INHERIT_CATALOG_QUALITY_PROFILE_VALUE;
}

export function inheritedCatalogQualityProfileLabel(
  library: LibraryQualityProfile | null | undefined,
  fallbackProfileId: string,
  profiles: readonly QualityProfileOption[],
  inheritLabel: string,
): string {
  const profileId = effectiveCatalogQualityProfileId(
    library,
    fallbackProfileId,
  );
  const profileName =
    profiles.find((profile) => profile.id === profileId)?.name ?? profileId;
  return profileName ? `${inheritLabel} — ${profileName}` : inheritLabel;
}

export function defaultCatalogRootFolderId(
  rootFolders: readonly RootFolder[],
): string | undefined {
  return (
    rootFolders.find((rootFolder) => rootFolder.isDefault && rootFolder.id)
      ?.id || rootFolders.find((rootFolder) => rootFolder.id)?.id
  );
}

export function draftForCatalogLibrary<T extends CatalogAddDraft>(
  draft: T,
  libraryId: string,
  rootFolders: readonly RootFolder[],
): T {
  return {
    ...draft,
    libraryId,
    rootFolderId: defaultCatalogRootFolderId(rootFolders),
  };
}

export function catalogAddOptionsForSubmit<T extends CatalogAddDraft>(
  draft: T,
): T {
  const qualityProfileId = explicitProfileId(draft.qualityProfileId);
  if (qualityProfileId) {
    return { ...draft, qualityProfileId };
  }

  const { qualityProfileId: _qualityProfileId, ...inherited } = draft;
  return inherited as T;
}

export function catalogAddDraftResetKey(
  facet: string,
  resultId: string,
  defaultLibraryId: string | undefined,
  defaultRootFolderId: string | undefined,
): string {
  return JSON.stringify([
    facet,
    resultId,
    defaultLibraryId ?? null,
    defaultRootFolderId ?? null,
  ]);
}

type CatalogAddSubmitState = {
  catalogConfigLoading: boolean;
  qualityProfileCount: number;
  hasCatalogDestination: boolean;
  libraryRequired: boolean;
  hasSelectedLibrary: boolean;
};

export function canSubmitCatalogAdd(state: CatalogAddSubmitState): boolean {
  return (
    !state.catalogConfigLoading &&
    state.qualityProfileCount > 0 &&
    state.hasCatalogDestination &&
    (!state.libraryRequired || state.hasSelectedLibrary)
  );
}
