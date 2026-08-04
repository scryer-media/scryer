import { useMemo, useState } from "react";
import { Loader2, RotateCcw, Trash2 } from "lucide-react";
import { LibraryMultiSelect } from "@/components/common/library-multi-select";
import { SettingsToggleSwitch } from "@/components/common/settings-toggle-switch";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { IconButton } from "@/components/ui/icon-button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Table,
  TableBody,
  TableCell,
  TableCheckboxCell,
  TableCheckboxHead,
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
import { groupRecycleBinItems } from "@/lib/utils/recycle-bin";

export type RecycledItem = {
  id: string;
  originalPath: string;
  fileName: string;
  sizeBytes: number;
  titleId: string | null;
  titleName: string | null;
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
  pendingItemIds: ReadonlySet<string>;
  selectedItemIds: ReadonlySet<string>;
  onEnabledChange: (enabled: boolean) => void;
  onSelectedLibraryIdsChange: (libraryIds: string[]) => void;
  onSelectedItemIdsChange: (itemIds: string[]) => void;
  onRestoreItems: (items: RecycledItem[]) => void;
  onDeleteItems: (items: RecycledItem[]) => void;
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
  restore_replaced: { label: "Replaced", className: "bg-[var(--scry-info-bg-strong)] text-[var(--scry-info-text)]" },
  file_deleted: { label: "Deleted", className: "bg-[var(--scry-danger-bg)] text-[var(--scry-danger-text)]" },
  invalid_file: { label: "Invalid", className: "bg-[var(--scry-warning-bg-strong)] text-[var(--scry-warning-text)]" },
  language_mismatch: { label: "Language", className: "bg-[var(--scry-warning-bg-strong)] text-[var(--scry-warning-text)]" },
  post_download_rule_blocked: { label: "Rule blocked", className: "bg-[rgba(var(--scry-accent-rgb),0.2)] text-[var(--scry-accent-text)]" },
};

function ReasonBadge({ reason }: { reason: string }) {
  const info = REASON_LABELS[reason] ?? { label: reason, className: "bg-muted text-muted-foreground" };
  return <span className={`rounded px-1.5 py-0.5 text-xs ${info.className}`}>{info.label}</span>;
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
  pendingItemIds,
  selectedItemIds,
  onEnabledChange,
  onSelectedLibraryIdsChange,
  onSelectedItemIdsChange,
  onRestoreItems,
  onDeleteItems,
  onEmptyAll,
}: Props) {
  const t = useTranslate();
  const dateTimeFormat = useUiDateTimeFormat();
  const [filter, setFilter] = useState("");
  const isBusy = settingsSaving || mutatingId !== null;
  const unassociatedTitleName = t("settings.recycleBinUnassociatedFiles");
  const groups = useMemo(
    () => groupRecycleBinItems(items, filter, unassociatedTitleName),
    [filter, items, unassociatedTitleName],
  );
  const visibleItems = useMemo(() => groups.flatMap((group) => group.items), [groups]);
  const visibleSelectableItems = visibleItems.filter((item) => !pendingItemIds.has(item.id));
  const selectedItems = items.filter((item) => selectedItemIds.has(item.id));
  const selectedActionableItems = selectedItems.filter((item) => !pendingItemIds.has(item.id));
  const selectedVisibleCount = visibleSelectableItems.filter((item) => selectedItemIds.has(item.id)).length;
  const selectAllState =
    selectedVisibleCount === 0
      ? false
      : selectedVisibleCount === visibleSelectableItems.length
        ? true
        : "indeterminate";

  const updateSelection = (next: Set<string>) => onSelectedItemIdsChange(Array.from(next));
  const toggleItem = (item: RecycledItem, selected: boolean) => {
    const next = new Set(selectedItemIds);
    if (selected) next.add(item.id);
    else next.delete(item.id);
    updateSelection(next);
  };
  const toggleItems = (groupItems: RecycledItem[], selected: boolean) => {
    const next = new Set(selectedItemIds);
    for (const item of groupItems) {
      if (pendingItemIds.has(item.id)) continue;
      if (selected) next.add(item.id);
      else next.delete(item.id);
    }
    updateSelection(next);
  };

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
          <Label htmlFor="settings-recycle-bin-enabled-toggle">{t("settings.recycleBinEnabled")}</Label>
          <p className="text-xs text-muted-foreground">
            {t(canManageConfig ? "settings.recycleBinEnabledHelp" : "settings.recycleBinEnabledReadonly")}
          </p>
        </div>
        <div className="flex items-center gap-2">
          {settingsSaving ? <Loader2 className="h-4 w-4 animate-spin text-muted-foreground" /> : null}
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
        <p className="py-2 text-sm text-muted-foreground">{t("settings.recycleBinNoManageableLibraries")}</p>
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
              disabled={totalCount === 0 || isBusy || librariesLoading || pendingItemIds.size > 0}
              onClick={onEmptyAll}
              className="text-[var(--scry-danger-text-soft)] hover:text-[var(--scry-danger-text)]"
            >
              <Trash2 className="mr-2 h-4 w-4" />
              {t("settings.recycleBinEmptyAll")}
            </Button>
          </div>

          <div className="flex flex-col gap-3 rounded-md border border-border bg-muted/30 p-3 sm:flex-row sm:items-center sm:justify-between">
            <Input
              id="settings-recycle-bin-filter"
              value={filter}
              onChange={(event) => setFilter(event.target.value)}
              placeholder={t("settings.recycleBinFilterPlaceholder")}
              aria-label={t("settings.recycleBinFilterAria")}
              className="max-w-md"
            />
            <div className="flex flex-wrap items-center gap-2">
              <span className="text-sm text-muted-foreground">
                {t("settings.recycleBinSelectedCount", { count: selectedItems.length })}
              </span>
              <Button
                size="sm"
                variant="outline"
                disabled={selectedActionableItems.length === 0 || isBusy}
                onClick={() => onRestoreItems(selectedActionableItems)}
              >
                <RotateCcw className="mr-2 h-4 w-4" />
                {t("settings.recycleBinRestoreSelected")}
              </Button>
              <Button
                size="sm"
                variant="outline"
                disabled={selectedActionableItems.length === 0 || isBusy}
                onClick={() => onDeleteItems(selectedActionableItems)}
                className="text-[var(--scry-danger-text-soft)] hover:text-[var(--scry-danger-text)]"
              >
                <Trash2 className="mr-2 h-4 w-4" />
                {t("settings.recycleBinDeleteSelected")}
              </Button>
              <Button
                size="sm"
                variant="ghost"
                disabled={selectedItems.length === 0 || isBusy}
                onClick={() => onSelectedItemIdsChange([])}
              >
                {t("settings.recycleBinClearSelection")}
              </Button>
            </div>
          </div>

          <p className="text-sm text-muted-foreground">{t("settings.recycleBinSection")}</p>

          {loading || librariesLoading ? (
            <div className="flex items-center gap-2 py-4 text-sm text-muted-foreground">
              <Loader2 className="h-4 w-4 animate-spin" />
              {t("label.loading")}
            </div>
          ) : items.length === 0 ? (
            <p className="py-4 text-sm text-muted-foreground">{t("settings.recycleBinEmpty")}</p>
          ) : groups.length === 0 ? (
            <p className="py-4 text-sm text-muted-foreground">
              {t("settings.recycleBinNoFilterMatches")}
            </p>
          ) : (
            <Table>
              <TableHeader>
                <TableRow>
                  <TableCheckboxHead>
                    <Checkbox
                      id="settings-recycle-bin-select-all"
                      size="table"
                      checked={selectAllState}
                      disabled={visibleSelectableItems.length === 0 || isBusy}
                      aria-label={t("settings.recycleBinSelectAllFiltered")}
                      onCheckedChange={(checked) => toggleItems(visibleSelectableItems, checked === true)}
                    />
                  </TableCheckboxHead>
                  <TableHead>{t("label.name")}</TableHead>
                  <TableHead>{t("nav.library")}</TableHead>
                  <TableHead>{t("settings.recycleBinReason")}</TableHead>
                  <TableHead>{t("settings.recycleBinSize")}</TableHead>
                  <TableHead>{t("settings.recycleBinRecycled")}</TableHead>
                  <TableHead className="text-right">{t("label.actions")}</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {groups.flatMap((group) => {
                  const groupSelectableItems = group.items.filter((item) => !pendingItemIds.has(item.id));
                  const groupSelectedCount = groupSelectableItems.filter((item) => selectedItemIds.has(item.id)).length;
                  const groupChecked =
                    groupSelectedCount === 0
                      ? false
                      : groupSelectedCount === groupSelectableItems.length
                        ? true
                        : "indeterminate";
                  return [
                    <TableRow key={`${group.id}:heading`} className="bg-muted/40 hover:bg-muted/40">
                      <TableCheckboxCell>
                        <Checkbox
                          id={selectorId("settings-recycle-bin-group", group.id)}
                          size="table"
                          checked={groupChecked}
                          disabled={groupSelectableItems.length === 0 || isBusy}
                          aria-label={t("settings.recycleBinSelectTitle", { title: group.titleName })}
                          onCheckedChange={(checked) => toggleItems(groupSelectableItems, checked === true)}
                        />
                      </TableCheckboxCell>
                      <TableCell colSpan={6}>
                        <div className="flex items-center gap-2">
                          <span className="font-medium">{group.titleName}</span>
                          <span className="text-xs text-muted-foreground">
                            {group.libraryName} · {t(
                              group.items.length === 1
                                ? "settings.recycleBinFileCountOne"
                                : "settings.recycleBinFileCountOther",
                              { count: group.items.length },
                            )}
                          </span>
                        </div>
                      </TableCell>
                    </TableRow>,
                    ...group.items.map((item) => {
                      const rowBusy = pendingItemIds.has(item.id) || mutatingId === "__empty__";
                      return (
                        <TableRow key={item.id} id={selectorId("settings-recycle-bin-row", item.id)}>
                          <TableCheckboxCell>
                            <Checkbox
                              id={selectorId("settings-recycle-bin-select", item.id)}
                              size="table"
                              checked={selectedItemIds.has(item.id)}
                              disabled={rowBusy || isBusy}
                              aria-label={t("settings.recycleBinSelectFile", { name: item.fileName })}
                              onCheckedChange={(checked) => toggleItem(item, checked === true)}
                            />
                          </TableCheckboxCell>
                          <TableCell>
                            <div>
                              <div className="font-medium">{item.fileName}</div>
                              <div className="max-w-[300px] truncate font-[var(--font-code)] text-xs text-muted-foreground" title={item.originalPath}>{item.originalPath}</div>
                            </div>
                          </TableCell>
                          <TableCell className="text-sm text-muted-foreground">{item.libraryName}</TableCell>
                          <TableCell><ReasonBadge reason={item.reason} /></TableCell>
                          <TableCell className="whitespace-nowrap font-[var(--font-code)] text-sm text-muted-foreground">{formatSize(item.sizeBytes)}</TableCell>
                          <TableCell className="text-sm text-muted-foreground whitespace-nowrap">{formatDate(item.recycledAt, dateTimeFormat)}</TableCell>
                          <TableCell className="text-right">
                            <div className="flex items-center justify-end gap-1">
                              <IconButton
                                id={selectorId("settings-recycle-bin-restore", item.id)}
                                label={t("settings.recycleBinRestore")}
                                tone="enabled"
                                disabled={rowBusy || isBusy}
                                onClick={() => onRestoreItems([item])}
                              >
                                <RotateCcw className="h-4 w-4" />
                              </IconButton>
                              <IconButton
                                id={selectorId("settings-recycle-bin-delete", item.id)}
                                label={t("settings.recycleBinDelete")}
                                tone="delete"
                                disabled={rowBusy || isBusy}
                                onClick={() => onDeleteItems([item])}
                              >
                                <Trash2 className="h-4 w-4" />
                              </IconButton>
                            </div>
                          </TableCell>
                        </TableRow>
                      );
                    }),
                  ];
                })}
              </TableBody>
            </Table>
          )}
        </>
      )}
    </div>
  );
}
