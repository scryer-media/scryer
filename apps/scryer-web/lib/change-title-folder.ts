import type { Translate } from "@/components/root/types";

/** How a candidate folder relates to the title being edited. */
export type FolderMatchOwnership =
  | "UNOWNED"
  | "OWNED_BY_THIS_TITLE"
  | "OWNED_BY_ANOTHER_TITLE";

/** How the user chose to settle a candidate folder's ownership. */
export type FolderMatchResolution = "ASSIGN" | "SWAP" | "TAKE_OVER";

/** What a folder-match correction actually did. */
export type FolderMatchOutcome =
  | "ALREADY_OWNED"
  | "ASSIGNED"
  | "SWAPPED"
  | "TAKEN_OVER";

export type FolderMatchTitleRef = {
  id: string;
  name: string;
  folderPath: string | null;
};

export type FolderMatchScan = {
  scanned: number;
  matched: number;
  imported: number;
  skipped: number;
  unmatched: number;
};

export type ChangeFolderPreview = {
  title: FolderMatchTitleRef;
  facet: string;
  libraryId: string;
  libraryName: string;
  currentRootId: string | null;
  currentRootPath: string | null;
  selectedFolderPath: string;
  selectedRootId: string;
  selectedRootPath: string;
  ownership: FolderMatchOwnership;
  currentOwner: FolderMatchTitleRef | null;
  currentFolderTrackedMediaCount: number;
  selectedFolderTrackedMediaCount: number;
  filesWillMove: boolean;
  noOp: boolean;
  availableResolutions: FolderMatchResolution[];
};

export type ChangeFolderResult = {
  outcome: FolderMatchOutcome;
  title: FolderMatchTitleRef;
  previousFolderPath: string | null;
  detachedMediaFileCount: number;
  scan: FolderMatchScan | null;
  swappedTitle: FolderMatchTitleRef | null;
  swappedTitleScan: FolderMatchScan | null;
  displacedTitle: {
    id: string;
    name: string;
    previousFolderPath: string;
    repairReasonCode: string;
  } | null;
};

export function normalizeFolderPath(path: string): string {
  const trimmed = path.trim();
  if (trimmed.length > 1 && trimmed.endsWith("/")) {
    return trimmed.replace(/\/+$/, "") || "/";
  }
  return trimmed;
}

/** True when `path` is the root itself or lives underneath it. */
export function isInsideRoot(path: string, rootPath: string): boolean {
  const candidate = normalizeFolderPath(path);
  const root = normalizeFolderPath(rootPath);
  if (!candidate || !root) {
    return false;
  }
  return candidate === root || candidate.startsWith(`${root}/`);
}

/**
 * Parent directory of `path`, clamped to `rootPath`; null at (or outside) the
 * root so browsing can never leave the title's library roots.
 */
export function parentWithinRoot(
  path: string,
  rootPath: string,
): string | null {
  const candidate = normalizeFolderPath(path);
  const root = normalizeFolderPath(rootPath);
  if (candidate === root || !isInsideRoot(candidate, root)) {
    return null;
  }
  const parent = candidate.replace(/\/[^/]+$/, "") || "/";
  return isInsideRoot(parent, root) ? parent : root;
}

/** Path segments of `path` below `rootPath`, for breadcrumbs. */
export function segmentsWithinRoot(path: string, rootPath: string): string[] {
  const candidate = normalizeFolderPath(path);
  const root = normalizeFolderPath(rootPath);
  if (!isInsideRoot(candidate, root) || candidate === root) {
    return [];
  }
  return candidate.slice(root.length).split("/").filter(Boolean);
}

/**
 * Resolution to preselect for a preview. Only an unowned folder is claimable
 * without a decision; a contested folder is never resolved for the user.
 */
export function defaultFolderMatchResolution(
  preview: Pick<ChangeFolderPreview, "ownership" | "availableResolutions">,
): FolderMatchResolution | null {
  return preview.ownership === "UNOWNED" &&
    preview.availableResolutions.includes("ASSIGN")
    ? "ASSIGN"
    : null;
}

/** User-facing summary of what a folder-match correction did. */
export function folderMatchOutcomeMessage(
  result: ChangeFolderResult,
  t: Translate,
): string {
  if (result.outcome === "SWAPPED") {
    return t("title.changeFolderOutcomeSwapped", {
      name: result.title.name,
      other: result.swappedTitle?.name ?? "",
    });
  }
  if (result.outcome === "ALREADY_OWNED") {
    return t("title.changeFolderOutcomeAlreadyOwned", {
      name: result.title.name,
    });
  }
  return t("title.changeFolderOutcomeAssigned", {
    name: result.title.name,
    folder: result.title.folderPath ?? "",
  });
}
