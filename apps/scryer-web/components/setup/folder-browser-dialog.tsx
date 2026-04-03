import { useState, useCallback, useEffect } from "react";
import { useClient } from "urql";
import { Folder, FolderOpen, ChevronRight, ArrowUp, Loader2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogFooter,
} from "@/components/ui/dialog";
import { browsePathQuery } from "@/lib/graphql/queries";

interface DirectoryEntry {
  name: string;
  path: string;
}

interface FolderBrowserDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onSelect: (path: string) => void;
  initialPath?: string;
  title?: string;
}

export function FolderBrowserDialog({
  open,
  onOpenChange,
  onSelect,
  initialPath = "/",
  title = "Select folder",
}: FolderBrowserDialogProps) {
  const client = useClient();
  const [currentPath, setCurrentPath] = useState(initialPath || "/");
  const [entries, setEntries] = useState<DirectoryEntry[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const browse = useCallback(
    async (
      path: string,
      options?: { fallbackToRootOnError?: boolean },
    ) => {
      const nextPath = path.trim() || "/";
      setCurrentPath(nextPath);
      setLoading(true);
      setError(null);
      const { data, error: gqlError } = await client
        .query(browsePathQuery, { path: nextPath })
        .toPromise();
      setLoading(false);
      if (gqlError) {
        if (options?.fallbackToRootOnError && nextPath !== "/") {
          await browse("/", { fallbackToRootOnError: false });
          return;
        }
        setError(gqlError.message);
        return;
      }
      setEntries(data?.browsePath ?? []);
    },
    [client],
  );

  useEffect(() => {
    if (open) {
      browse(initialPath || "/", { fallbackToRootOnError: true });
    }
  }, [open, initialPath, browse]);

  const parentPath = currentPath === "/" ? null : currentPath.replace(/\/[^/]+\/?$/, "") || "/";

  const pathSegments = currentPath.split("/").filter(Boolean);

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
        </DialogHeader>

        {/* Breadcrumb path */}
        <div className="flex items-center gap-1 overflow-x-auto text-sm">
          <button
            type="button"
            onClick={() => browse("/")}
            className="shrink-0 rounded px-1.5 py-0.5 text-muted-foreground hover:bg-muted hover:text-foreground"
          >
            /
          </button>
          {pathSegments.map((segment, i) => {
            const segPath = "/" + pathSegments.slice(0, i + 1).join("/");
            const isLast = i === pathSegments.length - 1;
            return (
              <span key={segPath} className="flex items-center gap-1">
                <ChevronRight className="h-3 w-3 shrink-0 text-muted-foreground" />
                <button
                  type="button"
                  onClick={() => browse(segPath)}
                  className={`shrink-0 rounded px-1.5 py-0.5 ${
                    isLast
                      ? "font-medium text-foreground"
                      : "text-muted-foreground hover:bg-muted hover:text-foreground"
                  }`}
                >
                  {segment}
                </button>
              </span>
            );
          })}
        </div>

        {/* Manual path input */}
        <div className="flex gap-2">
          <Input
            value={currentPath}
            onChange={(e) => setCurrentPath(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") browse(currentPath);
            }}
            className="font-mono text-sm"
          />
          <Button variant="outline" size="sm" onClick={() => browse(currentPath)}>
            Go
          </Button>
        </div>

        {/* Directory listing */}
        <div className="h-64 overflow-y-auto rounded-md border border-border">
          {loading ? (
            <div className="flex h-full items-center justify-center">
              <Loader2 className="h-5 w-5 animate-spin text-muted-foreground" />
            </div>
          ) : error ? (
            <div className="flex h-full items-center justify-center px-4 text-center text-sm text-destructive">
              {error}
            </div>
          ) : (
            <div className="divide-y divide-border">
              {parentPath !== null && (
                <button
                  type="button"
                  onClick={() => browse(parentPath)}
                  className="flex w-full items-center gap-2.5 px-3 py-2 text-left text-sm text-muted-foreground transition-colors hover:bg-muted"
                >
                  <ArrowUp className="h-4 w-4 shrink-0" />
                  <span>..</span>
                </button>
              )}
              {entries.length === 0 && !loading && (
                <div className="px-3 py-6 text-center text-sm text-muted-foreground">
                  No subdirectories
                </div>
              )}
              {entries.map((entry) => (
                <button
                  key={entry.path}
                  type="button"
                  onClick={() => browse(entry.path)}
                  className="flex w-full items-center gap-2.5 px-3 py-2 text-left text-sm transition-colors hover:bg-muted"
                >
                  <Folder className="h-4 w-4 shrink-0 text-muted-foreground" />
                  <span className="truncate">{entry.name}</span>
                </button>
              ))}
            </div>
          )}
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button
            onClick={() => {
              onSelect(currentPath);
              onOpenChange(false);
            }}
          >
            <FolderOpen className="mr-1.5 h-4 w-4" />
            Select: {currentPath}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
