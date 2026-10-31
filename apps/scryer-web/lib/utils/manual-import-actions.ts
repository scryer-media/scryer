import type { DownloadImportActions } from "@/lib/types/download-queue";

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

const NO_IMPORT_ACTIONS: DownloadImportActions = {
  manualImportInteractive: false,
  manualImportDirect: false,
  assignTitle: false,
  ignore: false,
  markFailed: false,
};

/**
 * The import actions a download offers, as the server decided them.
 *
 * Eligibility lives on the server (`derive_download_queue_import_actions`) so
 * that the activity rows, the dashboard rows and the title overviews cannot
 * disagree about the same download, and so the mutation enforces the same rule
 * the button was drawn from. This only reads the answer; a payload from an
 * older server that carries none offers nothing rather than guessing.
 */
export function downloadImportActions(item: {
  importActions?: DownloadImportActions | null;
}): DownloadImportActions {
  return item.importActions ?? NO_IMPORT_ACTIONS;
}

/** Whether the download offers a manual import at all, in either flavour. */
export function allowsManualImport(item: {
  importActions?: DownloadImportActions | null;
}): boolean {
  const actions = downloadImportActions(item);
  return actions.manualImportInteractive || actions.manualImportDirect;
}

/** Series and anime files have to be matched to episodes in the dialog. */
export function manualImportNeedsMapping(
  facet: string | null | undefined,
): boolean {
  const normalizedFacet = facet?.trim().toLowerCase() ?? "";
  return normalizedFacet === "series" || normalizedFacet === "anime";
}

/**
 * A disc image holds several titles, so which one to import is a choice for
 * the dialog rather than the direct movie import.
 */
export function manualImportSelectionNeedsDialog(
  files: ReadonlyArray<{ fileName: string }>,
): boolean {
  return files.some((file) => file.fileName.toLowerCase().endsWith(".iso"));
}
