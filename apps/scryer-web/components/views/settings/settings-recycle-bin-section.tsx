import { Loader2, RotateCcw, Trash2 } from "lucide-react";
import { LibraryMultiSelect } from "@/components/common/library-multi-select";
import { SettingsToggleSwitch } from "@/components/common/settings-toggle-switch";
import { Button } from "@/components/ui/button";
import { IconButton } from "@/components/ui/icon-button";
import { Label } from "@/components/ui/label";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { useTranslate } from "@/lib/context/translate-context";
import { useUiDateTimeFormat } from "@/lib/context/ui-settings-context";
import { selectorId } from "@/lib/utils/dom-ids";
import type { LibraryRecord } from "@/lib/types";
import type { UiDateTimeFormat } from "@/lib/types/settings";
import { formatUiDateTime } from "@/lib/utils/date-format";

export type RecycledItem = {
  id: string;
  originalPath: string;
  fileName: string;
  sizeBytes: number;
  titleId: string | null;
  reason: string;
  recycledAt: string;
  mediaRoot: string;
  libraryId: string;
  libraryName: string;
};

type Props = {
  enabled: boolean;
  settingsLoading: boolean;
  settingsSaving: boolean;
  canManageConfig: boolean;
  canManageItems: boolean;
  libraries: LibraryRecord[];
  librariesLoading: boolean;
  selectedLibraryIds: string[];
  items: RecycledItem[];
  totalCount: number;
  loading: boolean;
  mutatingId: string | null;
  pendingRestoreIds: ReadonlySet<string>;
  onEnabledChange: (enabled: boolean) => void;
  onSelectedLibraryIdsChange: (libraryIds: string[]) => void;
  onRestore: (item: RecycledItem) => void;
  onDelete: (item: RecycledItem) => void;
  onEmptyAll: () => void;
};

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

function formatDate(iso: string, dateTimeFormat: UiDateTimeFormat): string {
  return formatUiDateTime(iso, dateTimeFormat, { fallback: iso });
}

const REASON_LABELS: Record<string, { label: string; className: string }> = {
  upgrade_replaced: { label: "Upgrade", className: "bg-[var(--scry-info-bg-strong)] text-[var(--scry-info-text)]" },
  file_deleted: { label: "Deleted", className: "bg-[var(--scry-danger-bg)] text-[var(--scry-danger-text)]" },
  invalid_file: { label: "Invalid", className: "bg-[var(--scry-warning-bg-strong)] text-[var(--scry-warning-text)]" },
  language_mismatch: { label: "Language", className: "bg-[var(--scry-warning-bg-strong)] text-[var(--scry-warning-text)]" },
  post_download_rule_blocked: { label: "Rule blocked", className: "bg-[rgba(var(--scry-accent-rgb),0.2)] text-[var(--scry-accent-text)]" },
};

function ReasonBadge({ reason }: { reason: string }) {
  const info = REASON_LABELS[reason] ?? { label: reason, className: "bg-muted text-muted-foreground" };
  return (
    <span className={`rounded px-1.5 py-0.5 text-xs ${info.className}`}>
      {info.label}
    </span>
  );
}

export function SettingsRecycleBinSection({
  enabled,
  settingsLoading,
  settingsSaving,
  canManageConfig,
  canManageItems,
  libraries,
  librariesLoading,
  selectedLibraryIds,
  items,
  totalCount,
  loading,
  mutatingId,
  pendingRestoreIds,
  onEnabledChange,
  onSelectedLibraryIdsChange,
  onRestore,
  onDelete,
  onEmptyAll,
}: Props) {
  const t = useTranslate();
  const dateTimeFormat = useUiDateTimeFormat();
  const isBusy = settingsSaving || mutatingId !== null;

  if (settingsLoading) {
    return (
      <div className="flex items-center gap-2 text-sm text-muted-foreground">
        <Loader2 className="h-4 w-4 animate-spin" />
        {t("label.loading")}
      </div>
    );
  }

  return (
    <div id="settings-recycle-bin-section" className="space-y-4">
      <div className="flex flex-col gap-3 border-b border-border pb-4 sm:flex-row sm:items-center sm:justify-between">
        <div className="space-y-1">
          <Label htmlFor="settings-recycle-bin-enabled-toggle">
            {t("settings.recycleBinEnabled")}
          </Label>
          <p className="text-xs text-muted-foreground">
            {t(
              canManageConfig
                ? "settings.recycleBinEnabledHelp"
                : "settings.recycleBinEnabledReadonly",
            )}
          </p>
        </div>
        <div className="flex items-center gap-2">
          {settingsSaving ? (
            <Loader2 className="h-4 w-4 animate-spin text-muted-foreground" />
          ) : null}
          <SettingsToggleSwitch
            id="settings-recycle-bin-enabled-toggle"
            checked={enabled}
            disabled={!canManageConfig || settingsSaving}
            ariaLabel={t("settings.recycleBinEnabled")}
            onChange={onEnabledChange}
          />
        </div>
      </div>

      {!enabled ? null : !canManageItems ? (
        <p className="py-2 text-sm text-muted-foreground">
          {t("settings.recycleBinNoManageableLibraries")}
        </p>
      ) : (
        <>
          <div className="flex flex-col gap-3 sm:flex-row sm:items-end sm:justify-between">
            <div className="space-y-1">
              <Label>{t("settings.recycleBinLibraryFilter")}</Label>
              <LibraryMultiSelect
                libraries={libraries}
                selectedLibraryIds={selectedLibraryIds}
                onSelectedLibraryIdsChange={onSelectedLibraryIdsChange}
                disabled={librariesLoading || isBusy}
                triggerId="settings-recycle-bin-library-filter-trigger"
                allLibrariesButtonId="settings-recycle-bin-library-filter-all"
                triggerClassName="w-full min-w-56 sm:w-72"
              />
            </div>
            <Button
              id="settings-recycle-bin-empty-all"
              variant="outline"
              size="sm"
              disabled={totalCount === 0 || isBusy || librariesLoading}
              onClick={onEmptyAll}
              className="text-[var(--scry-danger-text-soft)] hover:text-[var(--scry-danger-text)]"
            >
              <Trash2 className="mr-2 h-4 w-4" />
              {t("settings.recycleBinEmptyAll")}
            </Button>
          </div>

          <p className="text-sm text-muted-foreground">{t("settings.recycleBinSection")}</p>

          {loading || librariesLoading ? (
            <div className="flex items-center gap-2 py-4 text-sm text-muted-foreground">
              <Loader2 className="h-4 w-4 animate-spin" />
              {t("label.loading")}
            </div>
          ) : items.length === 0 ? (
            <p className="py-4 text-sm text-muted-foreground">{t("settings.recycleBinEmpty")}</p>
          ) : (
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>{t("label.name")}</TableHead>
                  <TableHead>{t("nav.library")}</TableHead>
                  <TableHead>{t("settings.recycleBinReason")}</TableHead>
                  <TableHead>{t("settings.recycleBinSize")}</TableHead>
                  <TableHead>{t("settings.recycleBinRecycled")}</TableHead>
                  <TableHead className="text-right">{t("label.actions")}</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {items.map((item) => {
                  const rowBusy =
                    pendingRestoreIds.has(item.id) ||
                    mutatingId === item.id ||
                    mutatingId === "__empty__";
                  return (
                    <TableRow
                      key={item.id}
                      id={selectorId("settings-recycle-bin-row", item.id)}
                    >
                      <TableCell>
                        <div>
                          <div className="font-medium">{item.fileName}</div>
                          <div className="max-w-[300px] truncate font-[var(--font-code)] text-xs text-muted-foreground" title={item.originalPath}>
                            {item.originalPath}
                          </div>
                        </div>
                      </TableCell>
                      <TableCell className="text-sm text-muted-foreground">
                        {item.libraryName}
                      </TableCell>
                      <TableCell>
                        <ReasonBadge reason={item.reason} />
                      </TableCell>
                      <TableCell className="whitespace-nowrap font-[var(--font-code)] text-sm text-muted-foreground">
                        {formatSize(item.sizeBytes)}
                      </TableCell>
                      <TableCell className="text-sm text-muted-foreground whitespace-nowrap">
                        {formatDate(item.recycledAt, dateTimeFormat)}
                      </TableCell>
                      <TableCell className="text-right">
                        <div className="flex items-center justify-end gap-1">
                          <IconButton
                            id={selectorId("settings-recycle-bin-restore", item.id)}
                            label={t("settings.recycleBinRestore")}
                            tone="enabled"
                            disabled={rowBusy}
                            onClick={() => onRestore(item)}
                          >
                            <RotateCcw className="h-4 w-4" />
                          </IconButton>
                          <IconButton
                            id={selectorId("settings-recycle-bin-delete", item.id)}
                            label={t("settings.recycleBinDelete")}
                            tone="delete"
                            disabled={rowBusy}
                            onClick={() => onDelete(item)}
                          >
                            <Trash2 className="h-4 w-4" />
                          </IconButton>
                        </div>
                      </TableCell>
                    </TableRow>
                  );
                })}
              </TableBody>
            </Table>
          )}
        </>
      )}
    </div>
  );
}
