import * as React from "react";
import { FolderOpen, Plus, Trash2 } from "lucide-react";
import { FolderBrowserDialog } from "@/components/setup/folder-browser-dialog";
import { IconButton } from "@/components/ui/icon-button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import type { Translate } from "@/components/root/types";
import { TranslateContext } from "@/lib/context/translate-context";
import { selectorId } from "@/lib/utils/dom-ids";
import {
  isLocalPathFormatValidForStyle,
  type LocalPathStyle,
} from "@/lib/utils/local-path-style";
import {
  emptyPathMappingRow as emptyRow,
  parsePathMappings,
  removePathMappingRow,
  serializePathMappings,
  type PathMappingDirection,
  type PathMappingRow,
} from "@/lib/utils/path-mappings";

type PathMappingRowErrors = {
  localPath?: string;
  remotePath?: string;
};

export type LocalRemotePathMappingsFieldProps = {
  fieldKey: string;
  label: string;
  value: string;
  helpText?: string | null;
  required?: boolean;
  maxRows?: number;
  direction?: PathMappingDirection;
  localPathStyle?: LocalPathStyle;
  translate?: Translate;
  onChange: (key: string, value: string) => void;
  onValidityChange?: (isValid: boolean) => void;
};

const DEFAULT_MAX_ROWS = 10;

export function LocalRemotePathMappingsField({
  fieldKey,
  label,
  value,
  helpText,
  required = false,
  maxRows = DEFAULT_MAX_ROWS,
  direction = "local-to-remote",
  localPathStyle,
  translate,
  onChange,
  onValidityChange,
}: LocalRemotePathMappingsFieldProps) {
  const translateFromContext = React.useContext(TranslateContext);
  const t = React.useMemo<Translate>(
    () => translate ?? translateFromContext ?? ((key: string) => key),
    [translate, translateFromContext],
  );
  const [rows, setRows] = React.useState<PathMappingRow[]>(() =>
    parsePathMappings(value, direction),
  );
  const [browseRowIndex, setBrowseRowIndex] = React.useState<number | null>(
    null,
  );

  React.useEffect(() => {
    setRows((currentRows) =>
      serializePathMappings(currentRows, direction) === value
        ? currentRows
        : parsePathMappings(value, direction),
    );
  }, [direction, value]);

  const writeRows = React.useCallback(
    (nextRows: PathMappingRow[]) => {
      const normalizedRows = nextRows.length > 0 ? nextRows : [emptyRow()];
      setRows(normalizedRows);
      onChange(fieldKey, serializePathMappings(normalizedRows, direction));
    },
    [direction, fieldKey, onChange],
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
      writeRows(removePathMappingRow(rows, index));
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
    browseRowIndex == null
      ? "/"
      : rows[browseRowIndex]?.localPath.trim() || "/";
  const isEmpty = serializePathMappings(rows, direction).trim().length === 0;
  const localPathFirst = direction === "local-to-remote";
  const rowErrors = React.useMemo<PathMappingRowErrors[]>(
    () =>
      rows.map((row) => {
        const localPath = row.localPath.trim();
        const remotePath = row.remotePath.trim();
        const errors: PathMappingRowErrors = {};
        if (!localPath && remotePath) {
          errors.localPath = t(
            "settings.downloadClientRemotePathMappingsLocalRequired",
          );
        } else if (localPath && !remotePath) {
          errors.remotePath = t(
            "settings.downloadClientRemotePathMappingsRemoteRequired",
          );
        } else if (
          localPath &&
          !isLocalPathFormatValidForStyle(localPath, localPathStyle)
        ) {
          errors.localPath = t(
            "settings.downloadClientRemotePathMappingsLocalAbsolute",
          );
        }
        return errors;
      }),
    [localPathStyle, rows, t],
  );
  const isValid = React.useMemo(
    () =>
      rowErrors.every(
        (rowError) => rowError.localPath == null && rowError.remotePath == null,
      ),
    [rowErrors],
  );

  React.useEffect(() => {
    onValidityChange?.(isValid);
  }, [isValid, onValidityChange]);

  return (
    <label className="block">
      <Label className="mb-2 block">{label}</Label>
      <div className="space-y-2">
        {rows.map((row, index) => {
          const rowError = rowErrors[index] ?? {};
          const localInputId = selectorId(
            "path-mapping",
            fieldKey,
            index + 1,
            "local",
          );
          const remoteInputId = selectorId(
            "path-mapping",
            fieldKey,
            index + 1,
            "remote",
          );
          const browseButtonId = selectorId(
            "path-mapping",
            fieldKey,
            index + 1,
            "browse",
          );
          const removeButtonId = selectorId(
            "path-mapping",
            fieldKey,
            index + 1,
            "remove",
          );
          const firstInputId = localPathFirst ? localInputId : remoteInputId;
          const secondInputId = localPathFirst ? remoteInputId : localInputId;
          return (
            <div key={index} className="space-y-1">
              <div className="grid min-w-0 grid-cols-[minmax(0,1fr)_auto_minmax(0,1fr)_auto] items-center gap-2">
                <div className="relative min-w-0">
                  <Input
                    id={firstInputId}
                    value={localPathFirst ? row.localPath : row.remotePath}
                    readOnly={localPathFirst}
                    onClick={
                      localPathFirst
                        ? () => setBrowseRowIndex(index)
                        : undefined
                    }
                    onChange={
                      localPathFirst
                        ? undefined
                        : (event) =>
                            updateRow(index, { remotePath: event.target.value })
                    }
                    required={required && isEmpty && index === 0}
                    className={`font-[var(--font-code)] text-sm${localPathFirst ? " cursor-pointer pr-10" : ""}`}
                    aria-invalid={
                      (
                        localPathFirst
                          ? rowError.localPath
                          : rowError.remotePath
                      )
                        ? true
                        : undefined
                    }
                    aria-label={`${label} ${index + 1}`}
                  />
                  {localPathFirst ? (
                    <IconButton
                      id={browseButtonId}
                      label={t("setup.browse")}
                      appearance="ghost"
                      className="absolute right-1 top-1/2 h-7 w-7 -translate-y-1/2"
                      onClick={() => setBrowseRowIndex(index)}
                    >
                      <FolderOpen className="h-4 w-4" />
                    </IconButton>
                  ) : null}
                </div>

                <span
                  aria-hidden="true"
                  className="shrink-0 px-1 text-sm text-muted-foreground [font-variant-ligatures:none]"
                >
                  =&gt;
                </span>

                <div className="relative min-w-0">
                  <Input
                    id={secondInputId}
                    value={localPathFirst ? row.remotePath : row.localPath}
                    readOnly={!localPathFirst}
                    onClick={
                      !localPathFirst
                        ? () => setBrowseRowIndex(index)
                        : undefined
                    }
                    onChange={
                      localPathFirst
                        ? (event) =>
                            updateRow(index, { remotePath: event.target.value })
                        : undefined
                    }
                    required={required && isEmpty && index === 0}
                    className={`font-[var(--font-code)] text-sm${!localPathFirst ? " cursor-pointer pr-10" : ""}`}
                    aria-invalid={
                      (
                        localPathFirst
                          ? rowError.remotePath
                          : rowError.localPath
                      )
                        ? true
                        : undefined
                    }
                  />
                  {!localPathFirst ? (
                    <IconButton
                      id={browseButtonId}
                      label={t("setup.browse")}
                      appearance="ghost"
                      className="absolute right-1 top-1/2 h-7 w-7 -translate-y-1/2"
                      onClick={() => setBrowseRowIndex(index)}
                    >
                      <FolderOpen className="h-4 w-4" />
                    </IconButton>
                  ) : null}
                </div>

                {rows.length > 1 ||
                Boolean(row.localPath.trim() || row.remotePath.trim()) ? (
                  <IconButton
                    id={removeButtonId}
                    label={t("label.remove")}
                    appearance="ghost"
                    className="h-9 w-9 shrink-0"
                    onClick={() => removeRow(index)}
                  >
                    <Trash2 className="h-4 w-4" />
                  </IconButton>
                ) : (
                  <div className="h-9 w-9 shrink-0" aria-hidden="true" />
                )}
              </div>
              {rowError.remotePath ? (
                <p className="text-xs text-destructive">
                  {rowError.remotePath}
                </p>
              ) : null}
              {rowError.localPath ? (
                <p className="text-xs text-destructive">{rowError.localPath}</p>
              ) : null}
            </div>
          );
        })}

        <IconButton
          id={selectorId("path-mapping", fieldKey, "add")}
          label={t("label.add")}
          tone="install"
          className="h-10 w-10"
          onClick={addRow}
          disabled={rows.length >= maxRows}
        >
          <Plus className="h-5 w-5" />
        </IconButton>

        {helpText ? (
          <p className="text-xs text-muted-foreground">{helpText}</p>
        ) : null}
      </div>

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
      />
    </label>
  );
}
