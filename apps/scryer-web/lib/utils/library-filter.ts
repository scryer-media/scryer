import type { LibraryRecord } from "@/lib/types";

export function normalizeLibraryFilterSelection(
  selectedLibraryIds: string[],
  libraries: LibraryRecord[],
): string[] {
  if (libraries.length === 0) {
    return [];
  }

  const selectedSet = new Set(
    selectedLibraryIds
      .map((libraryId) => libraryId.trim())
      .filter((libraryId) => libraryId.length > 0),
  );
  const normalized = libraries
    .map((library) => library.id)
    .filter((libraryId) => selectedSet.has(libraryId));

  return normalized.length === 0 || normalized.length === libraries.length
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
