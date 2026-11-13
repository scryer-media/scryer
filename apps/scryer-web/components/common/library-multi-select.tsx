import * as React from "react";

import {
  MultiSelectDropdown,
  type MultiSelectGroup,
} from "@/components/ui/multi-select-dropdown";
import { localizedFacetLabel } from "@/components/views/overview-localization";
import { useTranslate } from "@/lib/context/translate-context";
import type { LibraryRecord } from "@/lib/types";
import { normalizeLibraryFilterSelection } from "@/lib/utils/library-filter";

const FACET_ORDER: LibraryRecord["facet"][] = ["MOVIE", "SERIES", "ANIME"];
const MAX_INLINE_LIBRARY_LABELS = 2;

type LibraryMultiSelectProps = {
  libraries: LibraryRecord[];
  selectedLibraryIds: string[];
  onSelectedLibraryIdsChange: (libraryIds: string[]) => void;
  disabled?: boolean;
  triggerId?: string;
  allLibrariesButtonId?: string;
  triggerClassName?: string;
  contentClassName?: string;
};

function groupLibrariesByFacet(libraries: LibraryRecord[]) {
  return FACET_ORDER.map((facet) => ({
    facet,
    libraries: libraries.filter((library) => library.facet === facet),
  })).filter((group) => group.libraries.length > 0);
}

export function LibraryMultiSelect({
  libraries,
  selectedLibraryIds,
  onSelectedLibraryIdsChange,
  disabled = false,
  triggerId,
  allLibrariesButtonId,
  triggerClassName,
  contentClassName,
}: LibraryMultiSelectProps) {
  const t = useTranslate();

  const normalizedSelectedLibraryIds = React.useMemo(
    () => normalizeLibraryFilterSelection(selectedLibraryIds, libraries),
    [libraries, selectedLibraryIds],
  );
  const groupedLibraries = React.useMemo(
    () => groupLibrariesByFacet(libraries),
    [libraries],
  );
  const showFacetGroups = groupedLibraries.length > 1;
  const libraryById = React.useMemo(
    () => new Map(libraries.map((library) => [library.id, library])),
    [libraries],
  );
  const selectedLibraries = React.useMemo(
    () =>
      normalizedSelectedLibraryIds
        .map((libraryId) => libraryById.get(libraryId) ?? null)
        .filter((library): library is LibraryRecord => library !== null),
    [libraryById, normalizedSelectedLibraryIds],
  );
  const triggerLabel = React.useMemo(() => {
    if (selectedLibraries.length === 0) {
      return t("libraryFilter.all");
    }

    if (selectedLibraries.length <= MAX_INLINE_LIBRARY_LABELS) {
      return selectedLibraries
        .map((library) =>
          showFacetGroups
            ? `${localizedFacetLabel(t, library.facet)}: ${library.name}`
            : library.name,
        )
        .join(", ");
    }

    return t("libraryFilter.selectedCount", {
      count: selectedLibraries.length,
    });
  }, [selectedLibraries, showFacetGroups, t]);

  const implicitAllSelected =
    libraries.length > 0 && normalizedSelectedLibraryIds.length === 0;
  const allLibraryIds = React.useMemo(
    () => libraries.map((library) => library.id),
    [libraries],
  );
  const effectiveSelectedLibraryIds = implicitAllSelected
    ? allLibraryIds
    : normalizedSelectedLibraryIds;
  const optionGroups = React.useMemo<MultiSelectGroup[]>(
    () =>
      groupedLibraries.map((group) => ({
        label: showFacetGroups
          ? localizedFacetLabel(t, group.facet)
          : undefined,
        options: group.libraries.map((library) => ({
          value: library.id,
          label: library.name,
        })),
      })),
    [groupedLibraries, showFacetGroups, t],
  );

  const selectAllLibraries = React.useCallback(() => {
    onSelectedLibraryIdsChange([]);
  }, [onSelectedLibraryIdsChange]);

  const handleSelectedLibraryIdsChange = React.useCallback(
    (nextSelection: string[]) => {
      onSelectedLibraryIdsChange(
        nextSelection.length === 0 ||
          nextSelection.length === allLibraryIds.length
          ? []
          : nextSelection,
      );
    },
    [allLibraryIds.length, onSelectedLibraryIdsChange],
  );

  return (
    <MultiSelectDropdown
      id={triggerId}
      groups={optionGroups}
      selectedValues={effectiveSelectedLibraryIds}
      onSelectedValuesChange={handleSelectedLibraryIdsChange}
      triggerLabel={triggerLabel}
      disabled={disabled || libraries.length === 0}
      triggerClassName={triggerClassName}
      contentClassName={contentClassName}
      allOption={{
        id: allLibrariesButtonId,
        label: t("libraryFilter.all"),
        selected: implicitAllSelected,
        onSelect: selectAllLibraries,
      }}
    />
  );
}
