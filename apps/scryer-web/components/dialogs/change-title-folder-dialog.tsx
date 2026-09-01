import * as React from "react";
import { useClient } from "urql";
import {
  ArrowLeftRight,
  ArrowUp,
  ChevronRight,
  Folder,
  FolderOpen,
  Loader2,
  ShieldAlert,
  TriangleAlert,
} from "lucide-react";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { useTranslate } from "@/lib/context/translate-context";
import { userFacingGraphQlErrorMessage } from "@/lib/graphql/error-message";
import { applyTitleFolderChangeMutation } from "@/lib/graphql/mutations";
import {
  browsePathQuery,
  changeTitleFolderPreviewQuery,
} from "@/lib/graphql/queries";
import { cn } from "@/lib/utils";
import { selectorId } from "@/lib/utils/dom-ids";
import {
  defaultFolderMatchResolution,
  folderMatchOutcomeMessage,
  normalizeFolderPath,
  parentWithinRoot,
  segmentsWithinRoot,
  type ChangeFolderPreview,
  type ChangeFolderResult,
  type FolderMatchResolution,
  type FolderMatchScan,
} from "@/lib/change-title-folder";
import type { LibraryRootRecord } from "@/lib/types/titles";

export type ChangeFolderTitle = {
  id: string;
  name: string;
  libraryId: string;
  libraryName?: string | null;
  rootFolderId?: string | null;
  rootFolderPath?: string | null;
};

type DirectoryEntry = {
  name: string;
  path: string;
  isDirectory: boolean;
};

type Props = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  title: ChangeFolderTitle | null;
  roots: LibraryRootRecord[];
  onChanged?: (result: ChangeFolderResult) => Promise<void> | void;
};

export function ChangeTitleFolderDialog({
  open,
  onOpenChange,
  title,
  roots,
  onChanged,
}: Props) {
  const client = useClient();
  const t = useTranslate();

  const sortedRoots = React.useMemo(
    () =>
      [...roots].sort((left, right) => {
        if (left.isDefault !== right.isDefault) {
          return left.isDefault ? -1 : 1;
        }
        return left.path.localeCompare(right.path);
      }),
    [roots],
  );

  const titleId = title?.id ?? null;
  const titleRootFolderId = title?.rootFolderId ?? null;
  const initialRootId =
    sortedRoots.find((root) => root.id === titleRootFolderId)?.id ??
    sortedRoots[0]?.id ??
    "";

  const [rootId, setRootId] = React.useState(initialRootId);
  const [browsePath, setBrowsePath] = React.useState("");
  const [entries, setEntries] = React.useState<DirectoryEntry[]>([]);
  const [browseLoading, setBrowseLoading] = React.useState(false);
  const [browseError, setBrowseError] = React.useState<string | null>(null);
  const [selectedFolderPath, setSelectedFolderPath] = React.useState<string | null>(
    null,
  );
  const [preview, setPreview] = React.useState<ChangeFolderPreview | null>(null);
  const [previewLoading, setPreviewLoading] = React.useState(false);
  const [previewError, setPreviewError] = React.useState<string | null>(null);
  const [resolution, setResolution] = React.useState<FolderMatchResolution | null>(
    null,
  );
  const [applying, setApplying] = React.useState(false);
  const [applyError, setApplyError] = React.useState<string | null>(null);
  const [result, setResult] = React.useState<ChangeFolderResult | null>(null);

  const selectedRoot =
    sortedRoots.find((root) => root.id === rootId) ?? sortedRoots[0] ?? null;
  const selectedRootPath = selectedRoot ? normalizeFolderPath(selectedRoot.path) : "";

  // Reset the whole flow whenever the dialog opens on a (possibly new) title.
  React.useEffect(() => {
    if (!open || titleId === null) {
      return;
    }
    setRootId(initialRootId);
    setEntries([]);
    setBrowseError(null);
    setSelectedFolderPath(null);
    setPreview(null);
    setPreviewError(null);
    setResolution(null);
    setApplyError(null);
    setResult(null);
  }, [open, titleId, initialRootId]);

  // Browsing always starts at the chosen root and never leaves it.
  React.useEffect(() => {
    if (!open) {
      return;
    }
    setBrowsePath(selectedRootPath);
  }, [open, selectedRootPath]);

  React.useEffect(() => {
    if (!open || !browsePath || !selectedRootPath) {
      return undefined;
    }
    let active = true;
    setBrowseLoading(true);
    setBrowseError(null);
    client
      .query(
        browsePathQuery,
        { path: browsePath, includeFiles: false },
        { requestPolicy: "network-only" },
      )
      .toPromise()
      .then(({ data, error }) => {
        if (!active) {
          return;
        }
        if (error) {
          setEntries([]);
          setBrowseError(
            userFacingGraphQlErrorMessage(error, t("title.changeFolderBrowseFailed")),
          );
          return;
        }
        const listed = (data?.browsePath ?? []) as DirectoryEntry[];
        setEntries(listed.filter((entry) => entry.isDirectory));
      })
      .catch((error: unknown) => {
        if (!active) {
          return;
        }
        setEntries([]);
        setBrowseError(
          userFacingGraphQlErrorMessage(error, t("title.changeFolderBrowseFailed")),
        );
      })
      .finally(() => {
        if (active) {
          setBrowseLoading(false);
        }
      });
    return () => {
      active = false;
    };
  }, [browsePath, client, open, selectedRootPath, t]);

  // Every selection change re-runs the preview; the ownership state shown is
  // always the one the backend computed for the folder currently selected.
  React.useEffect(() => {
    if (!open || !titleId || !selectedFolderPath) {
      setPreview(null);
      setPreviewError(null);
      setPreviewLoading(false);
      return undefined;
    }
    let active = true;
    setPreviewLoading(true);
    setPreviewError(null);
    setApplyError(null);
    client
      .query(
        changeTitleFolderPreviewQuery,
        { input: { titleId, folderPath: selectedFolderPath } },
        { requestPolicy: "network-only" },
      )
      .toPromise()
      .then(({ data, error }) => {
        if (!active) {
          return;
        }
        if (error) {
          setPreview(null);
          setResolution(null);
          setPreviewError(
            userFacingGraphQlErrorMessage(
              error,
              t("title.changeFolderPreviewFailed"),
            ),
          );
          return;
        }
        const next = data?.changeTitleFolderPreview as
          | ChangeFolderPreview
          | undefined;
        if (!next) {
          setPreview(null);
          setResolution(null);
          setPreviewError(t("title.changeFolderPreviewFailed"));
          return;
        }
        setPreview(next);
        // An unowned folder is assignable straight away; a contested one waits
        // for the user to choose swap or takeover.
        setResolution(defaultFolderMatchResolution(next));
      })
      .catch((error: unknown) => {
        if (!active) {
          return;
        }
        setPreview(null);
        setResolution(null);
        setPreviewError(
          userFacingGraphQlErrorMessage(error, t("title.changeFolderPreviewFailed")),
        );
      })
      .finally(() => {
        if (active) {
          setPreviewLoading(false);
        }
      });
    return () => {
      active = false;
    };
  }, [client, open, selectedFolderPath, t, titleId]);

  const handleApply = React.useCallback(async () => {
    if (!titleId || !preview || !resolution || preview.noOp) {
      return;
    }
    setApplying(true);
    setApplyError(null);
    try {
      const { data, error } = await client
        .mutation(applyTitleFolderChangeMutation, {
          input: {
            titleId,
            folderPath: preview.selectedFolderPath,
            resolution,
          },
        })
        .toPromise();
      if (error) {
        throw error;
      }
      const applied = data?.applyTitleFolderChange as
        | ChangeFolderResult
        | undefined;
      if (!applied) {
        throw new Error(t("title.changeFolderApplyFailed"));
      }
      setResult(applied);
      await onChanged?.(applied);
      // A takeover leaves another title unmatched; keep the dialog open so the
      // repair note is read before it disappears.
      if (!applied.displacedTitle) {
        onOpenChange(false);
      }
    } catch (error: unknown) {
      setApplyError(
        userFacingGraphQlErrorMessage(error, t("title.changeFolderApplyFailed")),
      );
    } finally {
      setApplying(false);
    }
  }, [client, onChanged, onOpenChange, preview, resolution, t, titleId]);

  const currentFolderPath = preview?.title.folderPath ?? null;
  const currentRootPath =
    preview?.currentRootPath ?? title?.rootFolderPath?.trim() ?? null;
  const libraryName =
    preview?.libraryName ?? title?.libraryName?.trim() ?? title?.libraryId ?? "";
  const owner = preview?.currentOwner ?? null;
  const conflicted = preview?.ownership === "OWNED_BY_ANOTHER_TITLE";
  const alreadyOwned = preview?.ownership === "OWNED_BY_THIS_TITLE";
  const parentPath = browsePath
    ? parentWithinRoot(browsePath, selectedRootPath)
    : null;
  const breadcrumbSegments = browsePath
    ? segmentsWithinRoot(browsePath, selectedRootPath)
    : [];
  const canApply = Boolean(
    preview && !preview.noOp && resolution && !applying && !previewLoading,
  );
  const confirmLabel =
    resolution === "SWAP"
      ? t("title.changeFolderSwapAction")
      : resolution === "TAKE_OVER"
        ? t("title.changeFolderTakeOverAction")
        : t("title.changeFolderConfirm");

  const scanSummary = (scan: FolderMatchScan) =>
    t("title.changeFolderScanSummary", {
      scanned: scan.scanned,
      matched: scan.matched,
      imported: scan.imported,
      unmatched: scan.unmatched,
    });

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent id="change-title-folder-dialog" className="sm:max-w-3xl">
        <DialogHeader>
          <DialogTitle>{t("title.changeFolderDialogTitle")}</DialogTitle>
          <DialogDescription>
            {t("title.changeFolderDialogDescription", {
              name: title?.name ?? "",
            })}
          </DialogDescription>
        </DialogHeader>

        {result ? (
          <div className="space-y-3" id="change-title-folder-result">
            <div className="rounded-lg border border-border bg-muted/20 px-3 py-3 text-sm text-foreground">
              {folderMatchOutcomeMessage(result, t)}
            </div>
            {result.detachedMediaFileCount > 0 ? (
              <p
                id="change-title-folder-detached-media"
                className="text-xs text-muted-foreground"
              >
                {t("title.changeFolderDetachedMedia", {
                  count: result.detachedMediaFileCount,
                })}
              </p>
            ) : null}
            {result.scan ? (
              <p
                id="change-title-folder-scan-summary"
                className="text-xs text-muted-foreground"
              >
                {scanSummary(result.scan)}
              </p>
            ) : null}
            {result.swappedTitleScan ? (
              <p
                id="change-title-folder-swapped-scan-summary"
                className="text-xs text-muted-foreground"
              >
                {scanSummary(result.swappedTitleScan)}
              </p>
            ) : null}
            {result.displacedTitle ? (
              <div
                id="change-title-folder-displaced-note"
                className="flex gap-2 rounded-lg border border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)] px-3 py-3 text-sm text-[var(--scry-danger-text)]"
              >
                <TriangleAlert className="mt-0.5 h-4 w-4 shrink-0" />
                <span>
                  {t("title.changeFolderDisplacedNote", {
                    name: result.displacedTitle.name,
                    folder: result.displacedTitle.previousFolderPath,
                  })}
                </span>
              </div>
            ) : null}
          </div>
        ) : (
          <div className="space-y-4">
            <dl className="grid gap-2 rounded-lg border border-border bg-muted/20 px-3 py-3 text-sm sm:grid-cols-2">
              <div className="min-w-0">
                <dt className="text-xs text-muted-foreground">{t("label.title")}</dt>
                <dd className="truncate text-foreground">{title?.name ?? ""}</dd>
              </div>
              <div className="min-w-0">
                <dt className="text-xs text-muted-foreground">
                  {t("title.changeFolderLibrary")}
                </dt>
                <dd className="truncate text-foreground">{libraryName}</dd>
              </div>
              <div className="min-w-0">
                <dt className="text-xs text-muted-foreground">
                  {t("title.changeFolderCurrentFolder")}
                </dt>
                <dd
                  className="truncate font-[var(--font-code)] text-foreground"
                  title={currentFolderPath ?? undefined}
                >
                  {currentFolderPath ?? t("title.changeFolderCurrentFolderUnknown")}
                </dd>
              </div>
              <div className="min-w-0">
                <dt className="text-xs text-muted-foreground">
                  {t("title.changeFolderCurrentRoot")}
                </dt>
                <dd
                  className="truncate font-[var(--font-code)] text-foreground"
                  title={currentRootPath ?? undefined}
                >
                  {currentRootPath ?? t("title.changeFolderCurrentRootUnknown")}
                </dd>
              </div>
            </dl>

            {sortedRoots.length === 0 ? (
              <p
                id="change-title-folder-no-roots"
                className="rounded-lg border border-border px-3 py-6 text-sm text-muted-foreground"
              >
                {t("title.changeFolderNoRoots")}
              </p>
            ) : (
              <div className="space-y-2">
                <label
                  className="text-xs font-medium text-muted-foreground"
                  htmlFor="change-title-folder-root"
                >
                  {t("title.changeFolderRootLabel")}
                </label>
                <Select
                  value={selectedRoot?.id ?? ""}
                  onValueChange={(value) => {
                    setRootId(value);
                    setSelectedFolderPath(null);
                  }}
                  disabled={applying}
                >
                  <SelectTrigger
                    id="change-title-folder-root"
                    className="h-9 w-full font-[var(--font-code)] text-sm"
                  >
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {sortedRoots.map((root) => (
                      <SelectItem key={root.id} value={root.id}>
                        {root.path}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
                <p className="text-xs text-muted-foreground">
                  {t("title.changeFolderRootScopeHelp")}
                </p>

                <div className="rounded-lg border border-border">
                  <div className="flex min-w-0 items-center gap-1 overflow-x-auto border-b border-border px-2 py-1.5 text-xs">
                    <button
                      id="change-title-folder-breadcrumb-root"
                      type="button"
                      className={cn(
                        "shrink-0 rounded px-1.5 py-1 font-[var(--font-code)]",
                        browsePath === selectedRootPath
                          ? "bg-muted text-foreground"
                          : "text-muted-foreground hover:bg-muted/60 hover:text-foreground",
                      )}
                      onClick={() => setBrowsePath(selectedRootPath)}
                      disabled={applying}
                    >
                      {selectedRootPath}
                    </button>
                    {breadcrumbSegments.map((segment, index) => {
                      const segmentPath = `${selectedRootPath}/${breadcrumbSegments
                        .slice(0, index + 1)
                        .join("/")}`;
                      return (
                        <span
                          key={segmentPath}
                          className="flex shrink-0 items-center gap-1"
                        >
                          <ChevronRight className="h-3 w-3 text-muted-foreground" />
                          <button
                            type="button"
                            className="shrink-0 rounded px-1.5 py-1 font-[var(--font-code)] text-muted-foreground hover:bg-muted/60 hover:text-foreground"
                            onClick={() => setBrowsePath(segmentPath)}
                            disabled={applying}
                          >
                            {segment}
                          </button>
                        </span>
                      );
                    })}
                  </div>

                  <div className="max-h-64 overflow-y-auto p-2">
                    {browseLoading ? (
                      <div className="flex items-center gap-2 px-2 py-6 text-sm text-muted-foreground">
                        <Loader2 className="h-4 w-4 animate-spin" />
                        {t("title.changeFolderBrowseLoading")}
                      </div>
                    ) : browseError ? (
                      <p
                        id="change-title-folder-browse-error"
                        className="rounded-md border border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)] px-3 py-2 text-sm text-[var(--scry-danger-text)]"
                      >
                        {browseError}
                      </p>
                    ) : (
                      <>
                        {parentPath ? (
                          <button
                            id="change-title-folder-up"
                            type="button"
                            className="mb-1 flex w-full items-center gap-2 rounded-md px-2 py-2 text-left text-sm text-muted-foreground hover:bg-muted/60"
                            onClick={() => setBrowsePath(parentPath)}
                            disabled={applying}
                          >
                            <ArrowUp className="h-4 w-4 shrink-0" />
                            {t("title.changeFolderUp")}
                          </button>
                        ) : null}
                        {entries.length === 0 ? (
                          <p className="px-2 py-6 text-center text-sm text-muted-foreground">
                            {t("title.changeFolderBrowseEmpty")}
                          </p>
                        ) : null}
                        {entries.map((entry) => {
                          const selected =
                            selectedFolderPath === normalizeFolderPath(entry.path);
                          return (
                            <div
                              key={entry.path}
                              className={cn(
                                "mb-1 flex items-center gap-2 rounded-md border px-2 py-1.5",
                                selected
                                  ? "border-primary bg-primary/5"
                                  : "border-transparent hover:bg-muted/50",
                              )}
                            >
                              <button
                                id={selectorId(
                                  "change-title-folder-select",
                                  entry.path,
                                )}
                                type="button"
                                className="flex min-w-0 flex-1 items-center gap-2 text-left"
                                onClick={() =>
                                  setSelectedFolderPath(normalizeFolderPath(entry.path))
                                }
                                disabled={applying}
                              >
                                <Folder className="h-4 w-4 shrink-0 text-muted-foreground" />
                                <span
                                  className="truncate font-[var(--font-code)] text-sm text-foreground"
                                  title={entry.path}
                                >
                                  {entry.name}
                                </span>
                              </button>
                              <Button
                                id={selectorId(
                                  "change-title-folder-open",
                                  entry.path,
                                )}
                                type="button"
                                variant="ghost"
                                size="sm"
                                className="shrink-0"
                                onClick={() =>
                                  setBrowsePath(normalizeFolderPath(entry.path))
                                }
                                disabled={applying}
                              >
                                <FolderOpen className="mr-1.5 h-4 w-4" />
                                {t("title.changeFolderOpen")}
                              </Button>
                            </div>
                          );
                        })}
                      </>
                    )}
                  </div>
                </div>
              </div>
            )}

            {!selectedFolderPath ? (
              <p className="text-sm text-muted-foreground">
                {t("title.changeFolderSelectPrompt")}
              </p>
            ) : null}

            {previewLoading ? (
              <div className="flex items-center gap-2 rounded-md border border-border px-3 py-4 text-sm text-muted-foreground">
                <Loader2 className="h-4 w-4 animate-spin" />
                {t("title.changeFolderPreviewLoading")}
              </div>
            ) : null}

            {previewError ? (
              <p
                id="change-title-folder-preview-error"
                className="rounded-md border border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)] px-3 py-2 text-sm text-[var(--scry-danger-text)]"
              >
                {previewError}
              </p>
            ) : null}

            {preview && !previewLoading ? (
              <div
                id="change-title-folder-preview"
                className="space-y-3 rounded-lg border border-border bg-muted/20 px-3 py-3"
              >
                <div className="min-w-0">
                  <p className="text-xs text-muted-foreground">
                    {t("title.changeFolderSelectedFolder")}
                  </p>
                  <p
                    className="truncate font-[var(--font-code)] text-sm text-foreground"
                    title={preview.selectedFolderPath}
                  >
                    {preview.selectedFolderPath}
                  </p>
                </div>

                <p
                  id="change-title-folder-ownership"
                  className="text-sm text-foreground"
                >
                  {preview.ownership === "UNOWNED"
                    ? t("title.changeFolderOwnershipUnowned")
                    : alreadyOwned
                      ? t("title.changeFolderOwnershipThisTitle")
                      : t("title.changeFolderOwnershipAnother", {
                          name: owner?.name ?? "",
                        })}
                </p>

                <ul className="space-y-1 text-xs text-muted-foreground">
                  <li>
                    {t("title.changeFolderTrackedMediaCurrent", {
                      count: preview.currentFolderTrackedMediaCount,
                    })}
                  </li>
                  <li>
                    {t("title.changeFolderTrackedMediaSelected", {
                      count: preview.selectedFolderTrackedMediaCount,
                    })}
                  </li>
                </ul>

                <p className="flex items-start gap-2 text-xs text-muted-foreground">
                  <ShieldAlert className="mt-0.5 h-3.5 w-3.5 shrink-0" />
                  {t("title.changeFolderNoFilesMoved")}
                </p>

                {conflicted ? (
                  <div
                    id="change-title-folder-conflict"
                    className="space-y-3 rounded-md border border-border bg-background/60 px-3 py-3"
                  >
                    <p className="text-sm font-medium text-foreground">
                      {t("title.changeFolderConflictHeading")}
                    </p>
                    <div className="flex flex-col gap-2 sm:flex-row">
                      {preview.availableResolutions.includes("SWAP") ? (
                        <Button
                          id="change-title-folder-choose-swap"
                          type="button"
                          variant={resolution === "SWAP" ? "primary" : "outline"}
                          size="sm"
                          onClick={() => setResolution("SWAP")}
                          disabled={applying}
                        >
                          <ArrowLeftRight className="mr-1.5 h-4 w-4" />
                          {t("title.changeFolderSwapAction")}
                        </Button>
                      ) : null}
                      {preview.availableResolutions.includes("TAKE_OVER") ? (
                        <Button
                          id="change-title-folder-choose-take-over"
                          type="button"
                          variant={
                            resolution === "TAKE_OVER" ? "primary" : "outline"
                          }
                          size="sm"
                          onClick={() => setResolution("TAKE_OVER")}
                          disabled={applying}
                        >
                          <TriangleAlert className="mr-1.5 h-4 w-4" />
                          {t("title.changeFolderTakeOverAction")}
                        </Button>
                      ) : null}
                    </div>
                    {resolution === "SWAP" ? (
                      <p className="text-xs text-muted-foreground">
                        {t("title.changeFolderSwapExplanation", {
                          owner: owner?.name ?? "",
                          folder: preview.selectedFolderPath,
                        })}
                      </p>
                    ) : null}
                    {resolution === "TAKE_OVER" ? (
                      <p
                        id="change-title-folder-take-over-warning"
                        className="rounded-md border border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)] px-3 py-2 text-xs text-[var(--scry-danger-text)]"
                      >
                        {t("title.changeFolderTakeOverExplanation", {
                          owner: owner?.name ?? "",
                          folder: preview.selectedFolderPath,
                        })}
                      </p>
                    ) : null}
                    {resolution === null ? (
                      <p className="text-xs text-muted-foreground">
                        {t("title.changeFolderChooseResolution")}
                      </p>
                    ) : null}
                  </div>
                ) : null}

                {alreadyOwned ? (
                  <p
                    id="change-title-folder-no-op"
                    className="rounded-md border border-border bg-background/60 px-3 py-2 text-xs text-muted-foreground"
                  >
                    {t("title.changeFolderNoOpHelp")}
                  </p>
                ) : null}
              </div>
            ) : null}

            {applyError ? (
              <p
                id="change-title-folder-apply-error"
                className="rounded-md border border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)] px-3 py-2 text-sm text-[var(--scry-danger-text)]"
              >
                {applyError}
              </p>
            ) : null}
          </div>
        )}

        <DialogFooter>
          <Button
            id="change-title-folder-cancel"
            type="button"
            variant="ghost"
            onClick={() => onOpenChange(false)}
            disabled={applying}
          >
            {result ? t("label.close") : t("label.cancel")}
          </Button>
          {result ? null : (
            <Button
              id="change-title-folder-apply"
              type="button"
              onClick={() => void handleApply()}
              disabled={!canApply}
            >
              {applying ? (
                <>
                  <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                  {t("title.changeFolderApplying")}
                </>
              ) : (
                confirmLabel
              )}
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
