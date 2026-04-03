import * as React from "react";
import { FolderOpen, Pencil, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Label } from "@/components/ui/label";
import { FolderBrowserDialog } from "@/components/setup/folder-browser-dialog";
import type { LibraryScanSummary, RootFolderOption } from "@/lib/types";
import { useTranslate } from "@/lib/context/translate-context";

type MediaLibrarySettingsPanelProps = {
  settingsTitle: string;
  rootFolders: RootFolderOption[];
  onSaveRootFolders: (folders: RootFolderOption[]) => void;
  loading: boolean;
  scanLoading: boolean;
  scanDisabled?: boolean;
  scanNotice?: string | null;
  scanSummary: LibraryScanSummary | null;
  onScan: () => void;
};

export const MediaLibrarySettingsPanel = React.memo(function MediaLibrarySettingsPanel({
  settingsTitle,
  rootFolders,
  onSaveRootFolders,
  loading,
  scanLoading,
  scanDisabled = scanLoading,
  scanNotice,
  scanSummary,
  onScan,
}: MediaLibrarySettingsPanelProps) {
  const t = useTranslate();
  const [browserOpen, setBrowserOpen] = React.useState(false);
  const [editingIndex, setEditingIndex] = React.useState<number | null>(null);
  const sortedFolders = React.useMemo(
    () => rootFolders
      .map((rf, i) => ({ rf, originalIndex: i }))
      .sort((a, b) => (a.rf.isDefault === b.rf.isDefault ? 0 : a.rf.isDefault ? -1 : 1)),
    [rootFolders],
  );

  const handleAddPath = (path: string) => {
    const trimmed = path.trim();
    if (!trimmed) return;
    if (rootFolders.some((rf) => rf.path === trimmed)) return;
    const isFirst = rootFolders.length === 0;
    onSaveRootFolders([...rootFolders, { path: trimmed, isDefault: isFirst }]);
  };

  const handleEditPath = (index: number, path: string) => {
    const trimmed = path.trim();
    if (!trimmed) return;
    if (rootFolders.some((rf, i) => rf.path === trimmed && i !== index)) return;
    const next = rootFolders.map((rf, i) =>
      i === index ? { ...rf, path: trimmed } : rf,
    );
    onSaveRootFolders(next);
  };

  const handleRemovePath = (index: number) => {
    const next = rootFolders.filter((_, i) => i !== index);
    if (next.length > 0 && !next.some((rf) => rf.isDefault)) {
      next[0].isDefault = true;
    }
    onSaveRootFolders(next);
  };

  const handleSetDefault = (index: number) => {
    const next = rootFolders.map((rf, i) => ({ ...rf, isDefault: i === index }));
    onSaveRootFolders(next);
  };

  const openAdd = () => {
    setEditingIndex(null);
    setBrowserOpen(true);
  };

  const openEdit = (index: number) => {
    setEditingIndex(index);
    setBrowserOpen(true);
  };

  const handleBrowserSelect = (path: string) => {
    if (editingIndex !== null) {
      handleEditPath(editingIndex, path);
    } else {
      handleAddPath(path);
    }
  };

  const browserInitialPath = editingIndex !== null
    ? rootFolders[editingIndex]?.path ?? "/"
    : "/";

  const browserTitle = editingIndex !== null
    ? t("settings.rootFolderEdit")
    : t("settings.rootFolderAdd");

  return (
    <>
      <Card>
        <CardHeader>
          <CardTitle>{settingsTitle}</CardTitle>
        </CardHeader>
        <CardContent className="space-y-3">
          <Label className="block">{t("settings.rootFoldersLabel")}</Label>
          {rootFolders.length === 0 && !loading ? (
            <p className="text-xs text-muted-foreground">{t("settings.rootFoldersEmpty")}</p>
          ) : null}
          <ul className="space-y-2">
            {sortedFolders.map(({ rf, originalIndex: index }) => (
              <li key={rf.path} className="flex items-center gap-2">
                <code className="flex-1 truncate rounded-md border border-border bg-muted/50 px-3 py-1.5 font-mono text-sm">
                  {rf.path}
                </code>
                {rf.isDefault ? (
                  <span className="shrink-0 rounded-md bg-muted px-2 py-1 text-xs text-muted-foreground">
                    {t("label.default")}
                  </span>
                ) : (
                  <button
                    type="button"
                    className="shrink-0 rounded-md px-2 py-1 text-xs text-muted-foreground hover:text-foreground hover:underline"
                    onClick={() => handleSetDefault(index)}
                    disabled={loading}
                  >
                    {t("settings.rootFolderSetDefault")}
                  </button>
                )}
                <Button
                  type="button"
                  variant="ghost"
                  size="icon"
                  className="h-8 w-8 shrink-0"
                  onClick={() => openEdit(index)}
                  disabled={loading}
                  aria-label={t("label.edit")}
                >
                  <Pencil className="h-4 w-4" />
                </Button>
                <Button
                  type="button"
                  variant="ghost"
                  size="icon"
                  className="h-8 w-8 shrink-0 text-destructive hover:text-destructive"
                  onClick={() => handleRemovePath(index)}
                  disabled={loading}
                  aria-label={t("label.delete")}
                >
                  <Trash2 className="h-4 w-4" />
                </Button>
              </li>
            ))}
          </ul>
          <Button
            type="button"
            variant="outline"
            onClick={openAdd}
            disabled={loading}
          >
            <FolderOpen className="mr-1.5 h-4 w-4" />
            {t("settings.rootFolderAdd")}
          </Button>
          <p className="text-xs text-muted-foreground">
            {loading ? t("label.loading") : t("settings.rootFoldersHelp")}
          </p>
        </CardContent>
      </Card>
      <Card>
        <CardHeader>
          <CardTitle>{t("settings.libraryScanTitle")}</CardTitle>
        </CardHeader>
        <CardContent className="space-y-3">
          <p className="text-sm text-muted-foreground">{t("settings.libraryScanHelp")}</p>
          <div className="flex flex-wrap items-center gap-3">
            <Button
              type="button"
              onClick={onScan}
              disabled={scanDisabled}
            >
              {scanLoading
                ? t("settings.libraryScanRunning")
                : t("settings.libraryScanButton")}
            </Button>
            {scanSummary ? (
              <span className="text-xs text-muted-foreground">
                {t("settings.libraryScanSummary", {
                  imported: scanSummary.imported,
                  skipped: scanSummary.skipped,
                  unmatched: scanSummary.unmatched,
                })}
              </span>
            ) : null}
          </div>
          {scanNotice ? (
            <p className="text-xs text-destructive">{scanNotice}</p>
          ) : null}
        </CardContent>
      </Card>
      <FolderBrowserDialog
        open={browserOpen}
        onOpenChange={setBrowserOpen}
        onSelect={handleBrowserSelect}
        initialPath={browserInitialPath}
        title={browserTitle}
      />
    </>
  );
});
