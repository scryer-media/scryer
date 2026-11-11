import type { Facet, LibraryRecord } from "@/lib/types";
import { FACET_REGISTRY } from "../facets/registry.ts";

type LibrariesByFacet = Record<Facet, LibraryRecord[]>;
type NavigableLibraryAccess = {
  viewable: LibrariesByFacet;
  requestable: LibrariesByFacet;
  manageable: LibrariesByFacet;
};

export function selectableLibraries(
  libraries: LibraryRecord[],
): LibraryRecord[] {
  return libraries.filter((library) => library.enabled !== false);
}

export function defaultLibrariesInFacetOrder(
  libraries: LibraryRecord[],
): LibraryRecord[] {
  return FACET_REGISTRY.flatMap((facet) =>
    libraries.filter(
      (library) => library.isDefault && library.facet === facet.id,
    ),
  );
}

export function facetHasEnabledLibrary(
  facet: Facet,
  access: NavigableLibraryAccess,
): boolean {
  return [access.viewable, access.requestable, access.manageable].some(
    (librariesByFacet) =>
      librariesByFacet[facet].some((library) => library.enabled !== false),
  );
}

export function normalizeLibraryFilterSelection(
  selectedLibraryIds: string[],
  libraries: LibraryRecord[],
): string[] {
  const selectable = selectableLibraries(libraries);
  if (selectable.length === 0) {
    return [];
  }

  const selectedSet = new Set(
    selectedLibraryIds
      .map((libraryId) => libraryId.trim())
      .filter((libraryId) => libraryId.length > 0),
  );
  const normalized = selectable
    .map((library) => library.id)
    .filter((libraryId) => selectedSet.has(libraryId));

  return normalized.length === 0 || normalized.length === selectable.length
    ? []
    : normalized;
}

export function selectedLibraryIdsToQueryValue(
  selectedLibraryIds: string[],
): string[] | null {
  return selectedLibraryIds.length > 0 ? selectedLibraryIds : null;
}

export function singleSelectedLibraryId(
  selectedLibraryIds: string[],
): string | null {
  return selectedLibraryIds.length === 1 ? selectedLibraryIds[0] : null;
}
