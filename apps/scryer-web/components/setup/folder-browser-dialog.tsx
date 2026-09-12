import { useState, useCallback, useEffect, useMemo, useRef } from "react";
import { useClient } from "urql";
import { useVirtualizer } from "@tanstack/react-virtual";
import { File, Folder, FolderOpen, ChevronRight, ArrowUp, Loader2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  DialogFooter,
} from "@/components/ui/dialog";
import { useTranslate } from "@/lib/context/translate-context";
import { userFacingGraphQlErrorMessage } from "@/lib/graphql/error-message";
import { browsePathQuery } from "@/lib/graphql/queries";
import { cn } from "@/lib/utils";
import { selectorId } from "@/lib/utils/dom-ids";

interface DirectoryEntry {
  name: string;
  path: string;
  isDirectory: boolean;
}

type BrowserRow =
  | { kind: "parent"; path: string }
  | { kind: "entry"; entry: DirectoryEntry };

// Rows are uniform by construction: a 2rem icon tile, py-2 padding, a hairline
// border and the mb-1 gap. A fixed pitch keeps the scrollbar honest without
// paying for per-row measurement.
const BROWSER_ROW_HEIGHT = 54;

interface FolderBrowserDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onSelect: (path: string) => void;
  selectionTypes: Array<"folder" | "file">;
  initialPath?: string;
  title?: string;
  preventCloseAutoFocus?: boolean;
}

export function FolderBrowserDialog({
  open,
  onOpenChange,
  onSelect,
  selectionTypes,
  initialPath = "/",
  title,
  preventCloseAutoFocus = false,
}: FolderBrowserDialogProps) {
  const client = useClient();
  const t = useTranslate();
  const canSelectFolders = selectionTypes.includes("folder");
  const canSelectFiles = selectionTypes.includes("file");
  const dialogTitle = title ?? t("folderBrowser.selectPath");
  const selectionDescription =
    canSelectFolders && canSelectFiles
      ? t("folderBrowser.descriptionFileOrFolder")
      : canSelectFiles
        ? t("folderBrowser.descriptionFile")
        : t("folderBrowser.descriptionFolder");
  const [currentPath, setCurrentPath] = useState(initialPath || "/");
  const [entries, setEntries] = useState<DirectoryEntry[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [browsedPath, setBrowsedPath] = useState<string | null>(null);
  const breadcrumbRef = useRef<HTMLDivElement | null>(null);
  const listScrollRef = useRef<HTMLDivElement | null>(null);

  const browse = useCallback(
    async (path: string) => {
      const nextPath = path.trim() || "/";
      setCurrentPath(nextPath);
      setLoading(true);
      setError(null);
      const { data, error: gqlError } = await client
        .query(browsePathQuery, { path: nextPath, includeFiles: canSelectFiles })
        .toPromise();
      setLoading(false);
      if (gqlError) {
        setEntries([]);
        setBrowsedPath(null);
        setError(
          userFacingGraphQlErrorMessage(gqlError, t("folderBrowser.error")),
        );
        return;
      }
      setEntries(data?.browsePath ?? []);
      setBrowsedPath(nextPath);
    },
    [canSelectFiles, client, t],
  );

  useEffect(() => {
    if (open) {
      browse(initialPath || "/");
    }
  }, [open, initialPath, browse]);

  // Keep the deepest segment in view; the row scrolls but has no visible
  // affordance on platforms with overlay scrollbars.
  useEffect(() => {
    const row = breadcrumbRef.current;
    if (!row) return;
    row.scrollLeft = row.scrollWidth;
  }, [open, currentPath]);

  const parentPath = currentPath === "/" ? null : currentPath.replace(/\/[^/]+\/?$/, "") || "/";

  const pathSegments = currentPath.split("/").filter(Boolean);
  const canSelect = !loading && error === null && browsedPath === currentPath;

  // A directory can hold hundreds of entries and each row carries its own icon,
  // so render only what the viewport needs. The parent row joins the same list
  // to keep the whole scroll height under the virtualizer.
  const rows = useMemo<BrowserRow[]>(() => {
    const next: BrowserRow[] = [];
    if (parentPath !== null) {
      next.push({ kind: "parent", path: parentPath });
    }
    for (const entry of entries) {
      next.push({ kind: "entry", entry });
    }
    return next;
  }, [entries, parentPath]);

  const rowVirtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => listScrollRef.current,
    getItemKey: (index) => {
      const row = rows[index];
      return row?.kind === "entry" ? row.entry.path : "..";
    },
    estimateSize: () => BROWSER_ROW_HEIGHT,
    overscan: 8,
  });

  const virtualRows = rowVirtualizer.getVirtualItems();
  const totalRowSize = rowVirtualizer.getTotalSize();
  const topSpacerHeight = virtualRows[0]?.start ?? 0;
  const bottomSpacerHeight = virtualRows.length
    ? Math.max(totalRowSize - virtualRows[virtualRows.length - 1].end, 0)
    : 0;

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        id="folder-browser-dialog"
        onCloseAutoFocus={
          preventCloseAutoFocus ? (event) => event.preventDefault() : undefined
        }
        className="w-[calc(100vw-2rem)] overflow-hidden border-[var(--scry-border)] bg-[var(--scry-surf)] p-0 text-[var(--scry-ink2)] shadow-[0_24px_80px_rgba(0,0,0,0.55)] sm:max-w-[42rem]"
      >
        <DialogHeader className="border-b border-[var(--scry-border3)] bg-[linear-gradient(180deg,rgba(255,255,255,0.04),rgba(255,255,255,0))] px-4 py-3 sm:px-5">
          <DialogTitle className="flex items-center gap-2 text-[15px] font-semibold text-[var(--scry-ink2)]">
            <span className="grid h-8 w-8 place-items-center rounded-[10px] border border-[var(--scry-border2)] bg-[var(--scry-card2)] text-[var(--scry-accent-text)]">
              <FolderOpen className="h-4 w-4" />
            </span>
            {dialogTitle}
          </DialogTitle>
          <DialogDescription className="sr-only">
            {selectionDescription}
          </DialogDescription>
        </DialogHeader>

        <div className="min-w-0 max-w-full space-y-3 overflow-hidden p-4 sm:p-5">
          <div className="min-w-0 max-w-full rounded-[12px] border border-[var(--scry-line2)] bg-[var(--scry-card2)] p-3">
            <div
              ref={breadcrumbRef}
              className="flex min-w-0 max-w-full items-center gap-1 overflow-x-auto pb-1 text-sm"
            >
              <button
                type="button"
                onClick={() => browse("/")}
                className={cn(
                  "shrink-0 rounded-[8px] border px-2 py-1 font-[var(--font-code)] transition-colors",
                  pathSegments.length === 0
                    ? "border-[var(--scry-baccent)] bg-[rgba(var(--scry-accent-rgb),0.14)] text-[var(--scry-accent-text)]"
                    : "border-transparent text-[var(--scry-muted3)] hover:border-[var(--scry-border3)] hover:bg-[var(--scry-hover)] hover:text-[var(--scry-ink2)]",
                )}
              >
                /
              </button>
              {pathSegments.map((segment, i) => {
                const segPath = "/" + pathSegments.slice(0, i + 1).join("/");
                const isLast = i === pathSegments.length - 1;
                return (
                  <span key={segPath} className="flex shrink-0 items-center gap-1">
                    <ChevronRight className="h-3 w-3 shrink-0 text-[var(--scry-muted3)]" />
                    <button
                      type="button"
                      onClick={() => browse(segPath)}
                      className={cn(
                        "max-w-[14rem] shrink-0 truncate rounded-[8px] border px-2 py-1 font-[var(--font-code)] transition-colors",
                        isLast
                          ? "border-[var(--scry-baccent)] bg-[rgba(var(--scry-accent-rgb),0.14)] font-medium text-[var(--scry-accent-text)]"
                          : "border-transparent text-[var(--scry-muted3)] hover:border-[var(--scry-border3)] hover:bg-[var(--scry-hover)] hover:text-[var(--scry-ink2)]",
                      )}
                      title={segment}
                    >
                      {segment}
                    </button>
                  </span>
                );
              })}
            </div>
            <div className="mt-2 flex min-w-0 max-w-full gap-2">
              <Input
                id="folder-browser-path-input"
                value={currentPath}
                onChange={(e) => {
                  setCurrentPath(e.target.value);
                  setEntries([]);
                  setError(null);
                  setBrowsedPath(null);
                }}
                onKeyDown={(e) => {
                  if (e.key === "Enter") browse(currentPath);
                }}
                className="h-10 min-w-0 border-[var(--scry-border3)] bg-[var(--scry-inset)] font-[var(--font-code)] text-sm text-[var(--scry-ink2)]"
              />
              <Button
                id="folder-browser-go"
                variant="secondary"
                size="sm"
                className="h-10 shrink-0 border-[var(--scry-border3)]"
                onClick={() => browse(currentPath)}
              >
                {t("folderBrowser.go")}
              </Button>
            </div>
          </div>

          <div className="h-[22rem] min-w-0 max-w-full overflow-hidden rounded-[12px] border border-[var(--scry-line2)] bg-[var(--scry-card2)]">
            {loading ? (
              <div className="flex h-full items-center justify-center">
                <div className="flex items-center gap-3 rounded-[12px] border border-[var(--scry-border2)] bg-[var(--scry-inset)] px-4 py-3 text-sm text-[var(--scry-muted3)]">
                  <Loader2 className="h-5 w-5 animate-spin text-[var(--scry-accent-text)]" />
                  {t("folderBrowser.loading")}
                </div>
              </div>
            ) : error ? (
              <div className="flex h-full flex-col items-center justify-center gap-3 px-6 text-center text-sm">
                <p className="max-w-md break-all rounded-[10px] border border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)] px-3 py-2 text-[var(--scry-danger-text)]">
                  {error}
                </p>
                {currentPath !== "/" ? (
                  <Button
                    id="folder-browser-root-recovery"
                    type="button"
                    variant="secondary"
                    size="sm"
                    onClick={() => browse("/")}
                  >
                    {t("folderBrowser.browseRoot")}
                  </Button>
                ) : null}
              </div>
            ) : (
              <div
                ref={listScrollRef}
                className="h-full min-w-0 max-w-full overflow-x-hidden overflow-y-auto p-2"
              >
                {topSpacerHeight > 0 ? (
                  <div aria-hidden style={{ height: topSpacerHeight }} />
                ) : null}
                {virtualRows.map((virtualRow) => {
                  const row = rows[virtualRow.index];
                  if (!row) {
                    return null;
                  }
                  if (row.kind === "parent") {
                    return (
                      <button
                        id="folder-browser-up"
                        key={virtualRow.key}
                        type="button"
                        onClick={() => browse(row.path)}
                        className="mb-1 grid w-full min-w-0 max-w-full grid-cols-[2rem_minmax(0,1fr)] items-center gap-2.5 overflow-hidden rounded-[10px] border border-transparent px-3 py-2 text-left text-sm text-[var(--scry-muted3)] transition-colors hover:border-[var(--scry-border3)] hover:bg-[var(--scry-hover)] hover:text-[var(--scry-ink2)]"
                      >
                        <span className="grid h-8 w-8 place-items-center rounded-[9px] bg-[var(--scry-inset)]">
                          <ArrowUp className="h-4 w-4 shrink-0" />
                        </span>
                        <span className="min-w-0 truncate">..</span>
                      </button>
                    );
                  }
                  const { entry } = row;
                  return (
                    <button
                      id={selectorId("folder-browser-entry", entry.path)}
                      key={virtualRow.key}
                      type="button"
                      onClick={() => {
                        if (entry.isDirectory) {
                          void browse(entry.path);
                          return;
                        }
                        onSelect(entry.path);
                        onOpenChange(false);
                      }}
                      className="mb-1 grid w-full min-w-0 max-w-full grid-cols-[2rem_minmax(0,1fr)] items-center gap-2.5 overflow-hidden rounded-[10px] border border-transparent px-3 py-2 text-left text-sm transition-colors hover:border-[var(--scry-border3)] hover:bg-[var(--scry-hover)]"
                    >
                      <span className="grid h-8 w-8 shrink-0 place-items-center rounded-[9px] bg-[var(--scry-inset)] text-[var(--scry-accent-text)]">
                        {entry.isDirectory ? (
                          <Folder className="h-4 w-4 shrink-0" />
                        ) : (
                          <File className="h-4 w-4 shrink-0" />
                        )}
                      </span>
                      <span
                        className="block min-w-0 max-w-full overflow-hidden text-ellipsis whitespace-nowrap font-[var(--font-code)] text-[var(--scry-ink2)]"
                        title={entry.name}
                      >
                        {entry.name}
                      </span>
                    </button>
                  );
                })}
                {bottomSpacerHeight > 0 ? (
                  <div aria-hidden style={{ height: bottomSpacerHeight }} />
                ) : null}
                {entries.length === 0 && !loading && (
                  <div className="px-3 py-10 text-center text-sm text-[var(--scry-muted3)]">
                    {canSelectFiles
                      ? t("folderBrowser.emptyFilesAndFolders")
                      : t("folderBrowser.emptyFolders")}
                  </div>
                )}
              </div>
            )}
          </div>
        </div>

        <DialogFooter className="min-w-0 max-w-full border-t border-[var(--scry-border3)] bg-[var(--scry-inset)] px-4 py-3 sm:flex-row sm:items-center sm:justify-between sm:px-5">
          <p
            className="min-w-0 truncate text-left font-[var(--font-code)] text-xs text-[var(--scry-muted3)] sm:mr-auto"
            title={currentPath}
          >
            {currentPath}
          </p>
          <div className="flex min-w-0 shrink-0 flex-wrap justify-end gap-2">
            <Button
              id="folder-browser-cancel"
              variant="outline"
              onClick={() => onOpenChange(false)}
            >
              {t("label.cancel")}
            </Button>
            {canSelectFolders && (
              <Button
                id="folder-browser-select"
                disabled={!canSelect}
                onClick={() => {
                  onSelect(currentPath);
                  onOpenChange(false);
                }}
              >
                <FolderOpen className="mr-1.5 h-4 w-4" />
                {t("folderBrowser.selectFolder")}
              </Button>
            )}
          </div>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
