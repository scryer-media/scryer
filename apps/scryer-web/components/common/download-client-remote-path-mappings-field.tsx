import * as React from "react";
import { FolderOpen, Plus, Trash2 } from "lucide-react";
import { FolderBrowserDialog } from "@/components/setup/folder-browser-dialog";
import type { Translate } from "@/components/root/types";
import { Button } from "@/components/ui/button";
import { IconButton } from "@/components/ui/icon-button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { TranslateContext } from "@/lib/context/translate-context";
import {
  isLocalPathFormatValidForStyle,
  type LocalPathStyle,
} from "@/lib/utils/local-path-style";

type PathMappingRow = {
  localPath: string;
  remotePath: string;
};

type PathMappingRowErrors = {
  localPath?: string;
  remotePath?: string;
};

export type DownloadClientRemotePathMappingsFieldProps = {
  fieldKey: string;
  label: string;
  value: string;
  helpText?: string | null;
  localPathStyle?: LocalPathStyle;
  required?: boolean;
  maxRows?: number;
  translate?: Translate;
  onChange: (key: string, value: string) => void;
  onValidityChange?: (isValid: boolean) => void;
};

const DEFAULT_MAX_ROWS = 10;
const EXAMPLE_MAPPING = "/downloads/tv => /Volumes/media/downloads/tv";

function emptyRow(): PathMappingRow {
  return { localPath: "", remotePath: "" };
}

function parsePathMappings(value: string): PathMappingRow[] {
  return value
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => line.length > 0)
    .map((line) => {
      const [remotePath, localPath = ""] = line.split(/=>/, 2);
      return {
        remotePath: remotePath.trim(),
        localPath: localPath.trim(),
      };
    });
}

function serializePathMappings(rows: PathMappingRow[]): string {
  return rows
    .filter((row) => row.remotePath.trim() || row.localPath.trim())
    .map((row) => `${row.remotePath.trim()} => ${row.localPath.trim()}`)
    .join("\n");
}

function isWindowsDrivePath(path: string): boolean {
  return /^[A-Za-z]:/.test(path);
}

function detectRemotePathStyle(path: string): "unix" | "windows" {
  if (path.includes("\\") || isWindowsDrivePath(path) || path.startsWith("\\\\")) {
    return "windows";
  }

  return "unix";
}

function normalizeUnixRemotePath(path: string): string {
  const isAbsolute = path.startsWith("/");
  const segments = path.split("/").filter(Boolean);

  if (!isAbsolute) {
    return segments.join("/");
  }

  return segments.length === 0 ? "/" : `/${segments.join("/")}`;
}

function normalizeWindowsRemotePath(path: string): string {
  const replaced = path.replaceAll("\\", "/").toLowerCase();

  if (replaced.startsWith("//")) {
    const segments = replaced
      .slice(2)
      .split("/")
      .filter(Boolean);

    if (segments.length === 0) {
      return "//";
    }

    if (segments.length === 1) {
      return `//${segments[0]}`;
    }

    const [server, share, ...tail] = segments;
    return tail.length === 0
      ? `//${server}/${share}`
      : `//${server}/${share}/${tail.join("/")}`;
  }

  if (isWindowsDrivePath(replaced)) {
    const drive = replaced.slice(0, 2);
    const rest = replaced.slice(2).replace(/^\/+/, "");
    const segments = rest.split("/").filter(Boolean);
    return segments.length === 0 ? `${drive}/` : `${drive}/${segments.join("/")}`;
  }

  const isAbsolute = replaced.startsWith("/");
  const segments = replaced.split("/").filter(Boolean);
  if (!isAbsolute) {
    return segments.join("/");
  }

  return segments.length === 0 ? "/" : `/${segments.join("/")}`;
}

function normalizeRemotePathForDuplicateKey(path: string): string {
  const style = detectRemotePathStyle(path);
  const normalized =
    style === "windows"
      ? normalizeWindowsRemotePath(path)
      : normalizeUnixRemotePath(path);
  return `${style}:${normalized}`;
}

function validateRows(
  rows: PathMappingRow[],
  localPathStyle: LocalPathStyle | undefined,
  t: Translate,
): { isValid: boolean; rowErrors: PathMappingRowErrors[] } {
  const rowErrors = rows.map<PathMappingRowErrors>(() => ({}));
  const duplicates = new Map<string, number[]>();

  rows.forEach((row, index) => {
    const remotePath = row.remotePath.trim();
    const localPath = row.localPath.trim();
    const rowIsEmpty = remotePath.length === 0 && localPath.length === 0;
    if (rowIsEmpty) {
      return;
    }

    if (!remotePath) {
      rowErrors[index].remotePath = t(
        "settings.downloadClientRemotePathMappingsRemoteRequired",
      );
    } else {
      const duplicateKey = normalizeRemotePathForDuplicateKey(remotePath);
      duplicates.set(duplicateKey, [...(duplicates.get(duplicateKey) ?? []), index]);
    }

    if (!localPath) {
      rowErrors[index].localPath = t(
        "settings.downloadClientRemotePathMappingsLocalRequired",
      );
    } else if (!isLocalPathFormatValidForStyle(localPath, localPathStyle)) {
      rowErrors[index].localPath = t(
        "settings.downloadClientRemotePathMappingsLocalAbsolute",
      );
    }
  });

  duplicates.forEach((indices) => {
    if (indices.length < 2) {
      return;
    }

    for (const index of indices) {
      rowErrors[index].remotePath ??= t(
        "settings.downloadClientRemotePathMappingsRemoteDuplicate",
      );
    }
  });

  return {
    isValid: rowErrors.every(
      (rowError) => rowError.localPath == null && rowError.remotePath == null,
    ),
    rowErrors,
  };
}

export function DownloadClientRemotePathMappingsField({
  fieldKey,
  label,
  value,
  helpText,
  localPathStyle,
  required = false,
  maxRows = DEFAULT_MAX_ROWS,
  translate,
  onChange,
  onValidityChange,
}: DownloadClientRemotePathMappingsFieldProps) {
  const translateFromContext = React.useContext(TranslateContext);
  const t = React.useMemo<Translate>(
    () => translate ?? translateFromContext ?? ((key: string) => key),
    [translate, translateFromContext],
  );
  const [rows, setRows] = React.useState<PathMappingRow[]>(() => parsePathMappings(value));
  const [browseRowIndex, setBrowseRowIndex] = React.useState<number | null>(null);

  React.useEffect(() => {
    setRows((currentRows) =>
      serializePathMappings(currentRows) === value ? currentRows : parsePathMappings(value),
    );
  }, [value]);

  const validation = React.useMemo(
    () => validateRows(rows, localPathStyle, t),
    [localPathStyle, rows, t],
  );

  React.useEffect(() => {
    onValidityChange?.(validation.isValid);
  }, [onValidityChange, validation.isValid]);

  const writeRows = React.useCallback(
    (nextRows: PathMappingRow[]) => {
      setRows(nextRows);
      onChange(fieldKey, serializePathMappings(nextRows));
    },
    [fieldKey, onChange],
  );

  const updateRow = React.useCallback(
    (index: number, patch: Partial<PathMappingRow>) => {
      writeRows(
        rows.map((row, rowIndex) =>
          rowIndex === index ? { ...row, ...patch } : row,
        ),
      );
    },
    [rows, writeRows],
  );

  const removeRow = React.useCallback(
    (index: number) => {
      writeRows(rows.filter((_, rowIndex) => rowIndex !== index));
    },
    [rows, writeRows],
  );

  const addRow = React.useCallback(() => {
    if (rows.length >= maxRows) {
      return;
    }

    writeRows([...rows, emptyRow()]);
  }, [maxRows, rows, writeRows]);

  const handleFolderSelect = React.useCallback(
    (path: string) => {
      if (browseRowIndex == null) {
        return;
      }

      updateRow(browseRowIndex, { localPath: path });
      setBrowseRowIndex(null);
    },
    [browseRowIndex, updateRow],
  );

  const browseInitialPath =
    browseRowIndex == null ? "/" : (rows[browseRowIndex]?.localPath.trim() || "/");

  return (
    <div
      id={`download-client-remote-path-mappings-${fieldKey}`}
      className="space-y-3"
    >
      <div className="space-y-1">
        <Label className="block">{label}</Label>
        {helpText ? (
          <p className="text-xs text-muted-foreground">{helpText}</p>
        ) : null}
      </div>

      {rows.length === 0 ? (
        <div className="rounded-xl border border-dashed border-border bg-muted/20 p-4">
          <div className="space-y-2">
            <p className="text-sm text-card-foreground">
              {t("settings.downloadClientRemotePathMappingsEmptyState")}
            </p>
            <p className="text-xs text-muted-foreground">
              <span className="font-medium text-card-foreground">
                {t("settings.downloadClientRemotePathMappingsExampleLabel")}:
              </span>{" "}
              <span className="font-[var(--font-code)]">{EXAMPLE_MAPPING}</span>
            </p>
            <Button
              id={`${fieldKey}-add-mapping`}
              type="button"
              variant="outline"
              size="sm"
              className="mt-1"
              onClick={addRow}
              disabled={rows.length >= maxRows}
            >
              <Plus className="mr-2 h-4 w-4" />
              {t("settings.downloadClientRemotePathMappingsAdd")}
            </Button>
          </div>
        </div>
      ) : (
        <div className="space-y-3">
          <div className="hidden md:grid md:grid-cols-[minmax(0,1fr)_minmax(0,1fr)_auto] md:gap-3">
            <div className="space-y-1">
              <Label className="text-xs font-medium text-card-foreground">
                {t("settings.downloadClientRemotePathMappingsRemoteLabel")}
              </Label>
              <p className="text-xs text-muted-foreground">
                {t("settings.downloadClientRemotePathMappingsRemoteHelp")}
              </p>
            </div>
            <div className="space-y-1">
              <Label className="text-xs font-medium text-card-foreground">
                {t("settings.downloadClientRemotePathMappingsLocalLabel")}
              </Label>
              <p className="text-xs text-muted-foreground">
                {t("settings.downloadClientRemotePathMappingsLocalHelp")}
              </p>
            </div>
            <div aria-hidden="true" />
          </div>

          {rows.map((row, index) => {
            const rowErrors = validation.rowErrors[index] ?? {};
            const remoteInputId = `${fieldKey}-remote-${index}`;
            const localInputId = `${fieldKey}-local-${index}`;
            const rowId = `${fieldKey}-row-${index}`;
            const browseButtonId = `${fieldKey}-browse-${index}`;
            const removeButtonId = `${fieldKey}-remove-${index}`;
            const hasRemoteError = rowErrors.remotePath != null;
            const hasLocalError = rowErrors.localPath != null;

            return (
              <div
                key={`${remoteInputId}-${localInputId}`}
                id={rowId}
                className="space-y-2"
              >
                <div className="grid gap-3 md:grid-cols-[minmax(0,1fr)_minmax(0,1fr)_auto]">
                  <div className="space-y-1">
                    <Label htmlFor={remoteInputId} className="text-xs font-medium md:sr-only">
                      {t("settings.downloadClientRemotePathMappingsRemoteLabel")}
                    </Label>
                    <p className="text-xs text-muted-foreground md:hidden">
                      {t("settings.downloadClientRemotePathMappingsRemoteHelp")}
                    </p>
                    <Input
                      id={remoteInputId}
                      value={row.remotePath}
                      onChange={(event) =>
                        updateRow(index, { remotePath: event.target.value })
                      }
                      required={required && rows.length === 1}
                      aria-invalid={hasRemoteError}
                      className="font-[var(--font-code)] text-sm"
                    />
                    {hasRemoteError ? (
                      <p className="text-xs text-destructive">{rowErrors.remotePath}</p>
                    ) : null}
                  </div>

                  <div className="space-y-1">
                    <Label htmlFor={localInputId} className="text-xs font-medium md:sr-only">
                      {t("settings.downloadClientRemotePathMappingsLocalLabel")}
                    </Label>
                    <p className="text-xs text-muted-foreground md:hidden">
                      {t("settings.downloadClientRemotePathMappingsLocalHelp")}
                    </p>
                    <div className="relative">
                      <Input
                        id={localInputId}
                        value={row.localPath}
                        onChange={(event) =>
                          updateRow(index, { localPath: event.target.value })
                        }
                        onFocus={() => setBrowseRowIndex(index)}
                        required={required && rows.length === 1}
                        aria-invalid={hasLocalError}
                        className="pr-10 font-[var(--font-code)] text-sm"
                      />
                      <IconButton
                        id={browseButtonId}
                        label={t("setup.browse")}
                        appearance="ghost"
                        className="absolute right-1 top-1/2 h-7 w-7 -translate-y-1/2"
                        onClick={() => setBrowseRowIndex(index)}
                      >
                        <FolderOpen className="h-4 w-4" />
                      </IconButton>
                    </div>
                    {hasLocalError ? (
                      <p className="text-xs text-destructive">{rowErrors.localPath}</p>
                    ) : null}
                  </div>

                  <div className="flex items-start justify-end pt-5 md:pt-0">
                    <IconButton
                      id={removeButtonId}
                      label={t("label.remove")}
                      appearance="ghost"
                      className="h-9 w-9 shrink-0 text-destructive hover:bg-destructive/10 hover:text-destructive"
                      onClick={() => removeRow(index)}
                    >
                      <Trash2 className="h-4 w-4" />
                    </IconButton>
                  </div>
                </div>
              </div>
            );
          })}

          <div className="flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
            <Button
              id={`${fieldKey}-add-mapping`}
              type="button"
              variant="outline"
              size="sm"
              onClick={addRow}
              disabled={rows.length >= maxRows}
            >
              <Plus className="mr-2 h-4 w-4" />
              {t("settings.downloadClientRemotePathMappingsAdd")}
            </Button>
            <p className="text-xs text-muted-foreground">
              <span className="font-medium text-card-foreground">
                {t("settings.downloadClientRemotePathMappingsExampleLabel")}:
              </span>{" "}
              <span className="font-[var(--font-code)]">{EXAMPLE_MAPPING}</span>
            </p>
          </div>
        </div>
      )}

      <FolderBrowserDialog
        open={browseRowIndex != null}
        onOpenChange={(open) => {
          if (!open) {
            setBrowseRowIndex(null);
          }
        }}
        onSelect={handleFolderSelect}
        selectionTypes={["folder"]}
        initialPath={browseInitialPath}
        title={label}
        preventCloseAutoFocus
      />
    </div>
  );
}
