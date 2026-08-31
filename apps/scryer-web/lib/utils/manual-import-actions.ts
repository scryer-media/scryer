export type DirectMovieManualImportCandidate = {
  candidateId: string;
  sizeBytes?: number | null;
};

export function compareManualImportSeasonLabels(
  left: string,
  right: string,
): number {
  const leftNumber = Number.parseInt(left.match(/\d+/)?.[0] ?? "", 10);
  const rightNumber = Number.parseInt(right.match(/\d+/)?.[0] ?? "", 10);
  const leftSortValue = Number.isFinite(leftNumber)
    ? leftNumber
    : Number.MAX_SAFE_INTEGER;
  const rightSortValue = Number.isFinite(rightNumber)
    ? rightNumber
    : Number.MAX_SAFE_INTEGER;

  return leftSortValue - rightSortValue || left.localeCompare(right);
}

/**
 * A movie import lands exactly one file: the primary. The direct (dialog-less)
 * movie action therefore maps only the largest candidate the selection
 * reports, instead of every video in the download. The server picks the
 * primary among whatever is mapped and records the rest as skipped, so this
 * is belt-and-braces: it keeps samples and extras out of the request in the
 * first place. Ties on size resolve to the earliest candidate.
 */
export function directMovieManualImportMappings(
  files: ReadonlyArray<DirectMovieManualImportCandidate>,
): Array<{ candidateId: string }> {
  let primary: DirectMovieManualImportCandidate | null = null;
  let primarySize = -1;
  for (const file of files) {
    const size =
      typeof file.sizeBytes === "number" && Number.isFinite(file.sizeBytes)
        ? file.sizeBytes
        : 0;
    if (primary === null || size > primarySize) {
      primary = file;
      primarySize = size;
    }
  }
  return primary ? [{ candidateId: primary.candidateId }] : [];
}

export function manualImportActions({
  displayState,
  facet,
  hasTitle,
}: {
  displayState: string;
  facet: string | null;
  hasTitle: boolean;
}) {
  const actionable =
    displayState === "IMPORT_BLOCKED" || displayState === "IMPORT_FAILED";
  const normalizedFacet = facet?.trim().toLowerCase() ?? "";

  return {
    interactive:
      hasTitle &&
      actionable &&
      (normalizedFacet === "series" || normalizedFacet === "anime"),
    direct: hasTitle && actionable && normalizedFacet === "movie",
  };
}
