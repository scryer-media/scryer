import { Fragment, useEffect, useMemo, useRef, useState } from "react";
import { Link } from "react-router";
import { Trash2, Undo2 } from "lucide-react";
import { RecycleBinHistory } from "./recycle-bin-history";
import { LibraryMultiSelect } from "@/components/common/library-multi-select";
import { PathPickerButton } from "@/components/common/path-picker-button";
import { SettingsToggleSwitch } from "@/components/common/settings-toggle-switch";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { IconButton } from "@/components/ui/icon-button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { TableDisclosureButton } from "@/components/ui/table-disclosure-button";
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
import { buildCalendarEventHref } from "@/lib/utils/calendar-event-href";
import type { LibraryRecord } from "@/lib/types";
import type { UiDateTimeFormat } from "@/lib/types/settings";
import { formatUiDateTime } from "@/lib/utils/date-format";
import {
  RECYCLE_BIN_MAX_RETENTION_DAYS,
  RECYCLE_BIN_MIN_RETENTION_DAYS,
  groupRecycleBinItems,
  parseRecycleBinRetentionDays,
  recycleBinLocationChanges,
  type RecycleBinRelocation,
  type RecycleBinSettingsChanges,
} from "@/lib/utils/recycle-bin";
import { LoadingMark } from "@/components/common/loading-mark";

export type RecycledItem = {
  id: string;
  originalPath: string;
  fileName: string;
  sizeBytes: number;
  titleId: string | null;
  titleName: string | null;
  titleFacet: string | null;
  episodeId: string | null;
  reason: string;
  recycledAt: string;
  scheduledDeletionAt: string;
  mediaRoot: string;
  libraryId: string;
  libraryName: string;
};

type Props = {
  initialHistoryItemId?: string | null;
  enabled: boolean;
  path: string | null;
  retentionDays: number;
  effectivePaths: string[];
  validationError: string | null;
  relocation: RecycleBinRelocation | null;
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
  onLocationSave: (changes: RecycleBinSettingsChanges) => void;
  onSelectedLibraryIdsChange: (libraryIds: string[]) => void;
  onSelectedItemIdsChange: (itemIds: string[]) => void;
  onRestoreItems: (items: RecycledItem[]) => void;
  onDeleteItems: (items: RecycledItem[]) => void;
  onEmptyAll: () => void;
};

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024)
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

function formatDate(iso: string, dateTimeFormat: UiDateTimeFormat): string {
  return formatUiDateTime(iso, dateTimeFormat, { fallback: iso });
}

const REASON_LABELS: Record<string, { label: string; className: string }> = {
  upgrade_replaced: {
    label: "Upgrade",
    className: "bg-[var(--scry-info-bg-strong)] text-[var(--scry-info-text)]",
  },
  restore_replaced: {
    label: "Replaced",
    className: "bg-[var(--scry-info-bg-strong)] text-[var(--scry-info-text)]",
  },
  file_deleted: {
    label: "Deleted",
    className: "bg-[var(--scry-danger-bg)] text-[var(--scry-danger-text)]",
  },
  invalid_file: {
    label: "Invalid",
    className:
      "bg-[var(--scry-warning-bg-strong)] text-[var(--scry-warning-text)]",
  },
  language_mismatch: {
    label: "Language",
    className:
      "bg-[var(--scry-warning-bg-strong)] text-[var(--scry-warning-text)]",
  },
  post_download_rule_blocked: {
    label: "Rule blocked",
    className:
      "bg-[rgba(var(--scry-accent-rgb),0.2)] text-[var(--scry-accent-text)]",
  },
};

function ReasonBadge({ reason }: { reason: string }) {
  const info = REASON_LABELS[reason] ?? {
    label: reason,
    className: "bg-muted text-muted-foreground",
  };
  return (
    <span className={`rounded px-1.5 py-0.5 text-xs ${info.className}`}>
      {info.label}
    </span>
  );
}

type LocationFormProps = {
  enabled: boolean;
  path: string | null;
  retentionDays: number;
  effectivePaths: string[];
  validationError: string | null;
  canManageConfig: boolean;
  saving: boolean;
  onSave: (changes: RecycleBinSettingsChanges) => void;
};

function RecycleBinLocationForm({
  enabled,
  path,
  retentionDays,
  effectivePaths,
  validationError,
  canManageConfig,
  saving,
  onSave,
}: LocationFormProps) {
  const t = useTranslate();
  const [pathDraft, setPathDraft] = useState(path ?? "");
  const [retentionDraft, setRetentionDraft] = useState(String(retentionDays));
  const parsedRetention = parseRecycleBinRetentionDays(retentionDraft);
  const dirty =
    pathDraft.trim() !== (path ?? "") || parsedRetention !== retentionDays;
  const disabled = !canManageConfig || saving;

  return (
    <form
      id="settings-recycle-bin-location"
      className="space-y-3 border-b border-border pb-4"
      onSubmit={(event) => {
        event.preventDefault();
        if (parsedRetention !== null && dirty) {
          onSave(
            recycleBinLocationChanges(
              { path, retentionDays },
              { path: pathDraft, retentionDays: parsedRetention },
            ),
          );
        }
      }}
    >
      <details className="space-y-3">
        <summary className="cursor-pointer text-sm font-medium">
          {t("indexerSearch.advanced")}
        </summary>
        <div className="space-y-1">
          <Label htmlFor="settings-recycle-bin-path">
            {t("settings.recycleBinPath")}
          </Label>
          <div className="flex max-w-xl items-center gap-2">
            <PathPickerButton
              id="settings-recycle-bin-path"
              value={pathDraft}
              placeholder={t("settings.recycleBinPathPlaceholder")}
              onSelect={(selectedPath) => {
                if (!disabled) setPathDraft(selectedPath);
              }}
              selectionTypes={["folder"]}
              dialogTitle={t("settings.recycleBinPath")}
              disabled={disabled}
            />
            {pathDraft ? (
              <Button
                type="button"
                variant="ghost"
                disabled={disabled}
                onClick={() => setPathDraft("")}
              >
                {t("label.clear")}
              </Button>
            ) : null}
          </div>
          <p className="text-xs text-muted-foreground">
            {t("settings.recycleBinPathHelp")}
          </p>
          <p className="text-xs text-[var(--scry-warning-text)]">
            {t("settings.recycleBinPathFilesystemWarning")}
          </p>
          <p className="text-xs text-muted-foreground">
            {t("settings.recycleBinPathExistingItems")}
          </p>
        </div>
        <div className="space-y-1">
          <Label htmlFor="settings-recycle-bin-retention-days">
            {t("settings.recycleBinRetentionDays")}
          </Label>
          <Input
            id="settings-recycle-bin-retention-days"
            type="number"
            inputMode="numeric"
            min={RECYCLE_BIN_MIN_RETENTION_DAYS}
            max={RECYCLE_BIN_MAX_RETENTION_DAYS}
            step={1}
            value={retentionDraft}
            onChange={(event) => setRetentionDraft(event.target.value)}
            disabled={disabled}
            aria-invalid={parsedRetention === null}
            className="w-32"
          />
          <p className="text-xs text-muted-foreground">
            {parsedRetention === null
              ? t("settings.recycleBinRetentionDaysInvalid", {
                  min: RECYCLE_BIN_MIN_RETENTION_DAYS,
                  max: RECYCLE_BIN_MAX_RETENTION_DAYS,
                })
              : t("settings.recycleBinRetentionDaysHelp")}
          </p>
        </div>
        {enabled ? (
          <div
            id="settings-recycle-bin-effective-paths"
            className="space-y-1 text-xs"
          >
            <p className="text-muted-foreground">
              {t("settings.recycleBinEffectivePaths")}
            </p>
            {effectivePaths.length === 0 ? (
              <p className="text-muted-foreground">
                {t("settings.recycleBinNoEffectivePaths")}
              </p>
            ) : (
              <ul className="space-y-0.5">
                {effectivePaths.map((effectivePath) => (
                  <li key={effectivePath} className="break-all font-mono">
                    {effectivePath}
                  </li>
                ))}
              </ul>
            )}
            {validationError ? (
              <p role="alert" className="text-[var(--scry-danger-text)]">
                {t("settings.recycleBinValidationError", {
                  error: validationError,
                })}
              </p>
            ) : null}
          </div>
        ) : null}
        {canManageConfig ? (
          <Button
            id="settings-recycle-bin-location-save"
            type="submit"
            size="sm"
            disabled={disabled || !dirty || parsedRetention === null}
          >
            {saving ? t("label.saving") : t("label.save")}
          </Button>
        ) : null}
      </details>
    </form>
  );
}

function RecycleBinRelocationNotice({
  relocation,
}: {
  relocation: RecycleBinRelocation;
}) {
  const t = useTranslate();
  const failureCount = relocation.failures.length;
  return (
    <div id="settings-recycle-bin-relocation" className="space-y-1 text-xs">
      {relocation.movedCount > 0 ? (
        <p className="text-muted-foreground">
          {t(
            relocation.movedCount === 1
              ? "settings.recycleBinRelocatedOne"
              : "settings.recycleBinRelocatedOther",
            { count: relocation.movedCount },
          )}
        </p>
      ) : null}
      {failureCount > 0 ? (
        <div role="alert" className="space-y-1 text-[var(--scry-danger-text)]">
          <p>
            {t(
              failureCount === 1
                ? "settings.recycleBinRelocationFailedOne"
                : "settings.recycleBinRelocationFailedOther",
              { count: failureCount },
            )}
          </p>
          <ul className="space-y-0.5">
            {relocation.failures.map((failure) => (
              <li
                key={`${failure.fromPath}:${failure.reason}`}
                className="break-all"
              >
                <span className="font-mono">{failure.fromPath}</span>:{" "}
                {failure.reason}
              </li>
            ))}
          </ul>
        </div>
      ) : null}
    </div>
  );
}

export function SettingsRecycleBinSection({
  initialHistoryItemId = null,
  enabled,
  path,
  retentionDays,
  effectivePaths,
  validationError,
  relocation,
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
  onLocationSave,
  onSelectedLibraryIdsChange,
  onSelectedItemIdsChange,
  onRestoreItems,
  onDeleteItems,
  onEmptyAll,
}: Props) {
  const t = useTranslate();
  const dateTimeFormat = useUiDateTimeFormat();
  const [filter, setFilter] = useState("");
  const [historyItemId, setHistoryItemId] = useState<string | null>(
    initialHistoryItemId,
  );
  const initialHistoryScrolled = useRef(false);
  useEffect(() => {
    if (
      !initialHistoryItemId ||
      initialHistoryScrolled.current ||
      settingsLoading ||
      loading ||
      librariesLoading
    )
      return;
    const row = document.getElementById(
      selectorId("settings-recycle-bin-row", initialHistoryItemId),
    );
    if (!row) return;
    row.scrollIntoView({ block: "start" });
    document
      .getElementById(
        selectorId("settings-recycle-bin-history", initialHistoryItemId),
      )
      ?.focus({ preventScroll: true });
    initialHistoryScrolled.current = true;
  }, [initialHistoryItemId, items, librariesLoading, loading, settingsLoading]);
  const toggleHistory = (id: string) =>
    setHistoryItemId((current) => (current === id ? null : id));
  const isBusy = settingsSaving || mutatingId !== null;
  const unassociatedTitleName = t("settings.recycleBinUnassociatedFiles");
  const groups = useMemo(
    () => groupRecycleBinItems(items, filter, unassociatedTitleName),
    [filter, items, unassociatedTitleName],
  );
  const visibleItems = useMemo(
    () => groups.flatMap((group) => group.items),
    [groups],
  );
  const visibleSelectableItems = visibleItems.filter(
    (item) => !pendingItemIds.has(item.id),
  );
  const selectedItems = items.filter((item) => selectedItemIds.has(item.id));
  const selectedActionableItems = selectedItems.filter(
    (item) => !pendingItemIds.has(item.id),
  );
  const selectedVisibleCount = visibleSelectableItems.filter((item) =>
    selectedItemIds.has(item.id),
  ).length;
  const selectAllState =
    selectedVisibleCount === 0
      ? false
      : selectedVisibleCount === visibleSelectableItems.length
        ? true
        : "indeterminate";

  const updateSelection = (next: Set<string>) =>
    onSelectedItemIdsChange(Array.from(next));
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
        <LoadingMark className="h-4 w-4" />
        {t("label.loading")}
      </div>
    );
  }

  return (
    <div
      id="settings-recycle-bin-section"
      className="@container/recycle min-w-0 space-y-4"
    >
      <div className="flex flex-col gap-3 border-b border-border pb-4 @[40rem]/recycle:flex-row @[40rem]/recycle:items-center @[40rem]/recycle:justify-between">
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
            <LoadingMark className="h-4 w-4 text-muted-foreground" />
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

      <RecycleBinLocationForm
        key={JSON.stringify([path, retentionDays])}
        enabled={enabled}
        path={path}
        retentionDays={retentionDays}
        effectivePaths={effectivePaths}
        validationError={validationError}
        canManageConfig={canManageConfig}
        saving={settingsSaving}
        onSave={onLocationSave}
      />

      {relocation ? (
        <RecycleBinRelocationNotice relocation={relocation} />
      ) : null}

      {!enabled ? null : !canManageItems ? (
        <p className="py-2 text-sm text-muted-foreground">
          {t("settings.recycleBinNoManageableLibraries")}
        </p>
      ) : (
        <>
          <div className="flex flex-col gap-3 @[40rem]/recycle:flex-row @[40rem]/recycle:items-end @[40rem]/recycle:justify-between">
            <div className="min-w-0 space-y-1">
              <Label>{t("settings.recycleBinLibraryFilter")}</Label>
              <LibraryMultiSelect
                libraries={libraries}
                selectedLibraryIds={selectedLibraryIds}
                onSelectedLibraryIdsChange={onSelectedLibraryIdsChange}
                disabled={librariesLoading || isBusy}
                triggerId="settings-recycle-bin-library-filter-trigger"
                allLibrariesButtonId="settings-recycle-bin-library-filter-all"
                triggerClassName="w-full min-w-0 @[40rem]/recycle:w-72"
              />
            </div>
            <Button
              id="settings-recycle-bin-empty-all"
              variant="outline"
              size="sm"
              disabled={
                totalCount === 0 ||
                isBusy ||
                librariesLoading ||
                pendingItemIds.size > 0
              }
              onClick={onEmptyAll}
              className="text-[var(--scry-danger-text-soft)] hover:text-[var(--scry-danger-text)]"
            >
              <Trash2 className="mr-2 h-4 w-4" />
              {t("settings.recycleBinEmptyAll")}
            </Button>
          </div>

          <div className="flex min-w-0 flex-col gap-3 rounded-md border border-border bg-muted/30 p-3 @[56rem]/recycle:flex-row @[56rem]/recycle:items-center @[56rem]/recycle:justify-between">
            <Input
              id="settings-recycle-bin-filter"
              value={filter}
              onChange={(event) => setFilter(event.target.value)}
              placeholder={t("settings.recycleBinFilterPlaceholder")}
              aria-label={t("settings.recycleBinFilterAria")}
              className="min-w-0 @[56rem]/recycle:max-w-md @[56rem]/recycle:flex-1"
            />
            {selectedItems.length > 0 ? (
              <div className="flex min-w-0 flex-wrap items-center gap-2 @max-[40rem]/recycle:[&>button]:h-auto @max-[40rem]/recycle:[&>button]:min-h-10 @max-[40rem]/recycle:[&>button]:w-full @max-[40rem]/recycle:[&>button]:whitespace-normal">
                <span className="text-sm text-muted-foreground @max-[40rem]/recycle:w-full">
                  {t("settings.recycleBinSelectedCount", {
                    count: selectedItems.length,
                  })}
                </span>
                <Button
                  size="sm"
                  variant="outline"
                  disabled={selectedActionableItems.length === 0 || isBusy}
                  id="settings-recycle-bin-restore-selected"
                  onClick={() => onRestoreItems(selectedActionableItems)}
                >
                  <Undo2 className="mr-2 h-4 w-4" />
                  {t("settings.recycleBinRestoreSelected")}
                </Button>
                <Button
                  size="sm"
                  variant="outline"
                  disabled={selectedActionableItems.length === 0 || isBusy}
                  id="settings-recycle-bin-delete-selected"
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
                  id="settings-recycle-bin-clear-selection"
                  onClick={() => onSelectedItemIdsChange([])}
                >
                  {t("settings.recycleBinClearSelection")}
                </Button>
              </div>
            ) : null}
          </div>

          <p className="text-sm text-muted-foreground">
            {t("settings.recycleBinSection")}
          </p>

          {loading || librariesLoading ? (
            <div className="flex items-center gap-2 py-4 text-sm text-muted-foreground">
              <LoadingMark className="h-4 w-4" />
              {t("label.loading")}
            </div>
          ) : items.length === 0 ? (
            <p className="py-4 text-sm text-muted-foreground">
              {t("settings.recycleBinEmpty")}
            </p>
          ) : groups.length === 0 ? (
            <p className="py-4 text-sm text-muted-foreground">
              {t("settings.recycleBinNoFilterMatches")}
            </p>
          ) : (
            <Table
              aria-label={t("settings.recycleBinSection")}
              wrapperClassName="rounded-[14px] border border-[var(--scry-border2)] bg-[var(--scry-surfC)]"
              className="@max-[72rem]/recycle:block"
            >
              <TableHeader className="@max-[72rem]/recycle:block">
                <TableRow className="@max-[72rem]/recycle:grid @max-[72rem]/recycle:grid-cols-[3rem_minmax(0,1fr)]">
                  <TableCheckboxHead className="@max-[72rem]/recycle:flex @max-[72rem]/recycle:h-auto @max-[72rem]/recycle:min-h-11 @max-[72rem]/recycle:items-center @max-[72rem]/recycle:justify-center">
                    <Checkbox
                      id="settings-recycle-bin-select-all"
                      size="table"
                      className="@max-[72rem]/recycle:size-6"
                      checked={selectAllState}
                      disabled={visibleSelectableItems.length === 0 || isBusy}
                      aria-label={t("settings.recycleBinSelectAllFiltered")}
                      onCheckedChange={(checked) =>
                        toggleItems(visibleSelectableItems, checked === true)
                      }
                    />
                  </TableCheckboxHead>
                  <TableHead className="@max-[72rem]/recycle:flex @max-[72rem]/recycle:h-auto @max-[72rem]/recycle:min-h-11 @max-[72rem]/recycle:items-center @max-[72rem]/recycle:py-2">
                    <span className="@max-[72rem]/recycle:hidden">
                      {t("label.name")}
                    </span>
                    <Label
                      htmlFor="settings-recycle-bin-select-all"
                      className="hidden cursor-pointer @max-[72rem]/recycle:block"
                    >
                      {t("settings.recycleBinSelectAllFiltered")}
                    </Label>
                  </TableHead>
                  <TableHead className="@max-[72rem]/recycle:hidden">
                    {t("nav.library")}
                  </TableHead>
                  <TableHead className="@max-[72rem]/recycle:hidden">
                    {t("settings.recycleBinReason")}
                  </TableHead>
                  <TableHead className="@max-[72rem]/recycle:hidden">
                    {t("settings.recycleBinSize")}
                  </TableHead>
                  <TableHead className="@max-[72rem]/recycle:hidden">
                    {t("settings.recycleBinRecycled")}
                  </TableHead>
                  <TableHead className="@max-[72rem]/recycle:hidden">
                    {t("settings.recycleBinScheduledDeletion")}
                  </TableHead>
                  <TableHead className="text-right @max-[72rem]/recycle:hidden">
                    {t("label.actions")}
                  </TableHead>
                </TableRow>
              </TableHeader>
              <TableBody className="@max-[72rem]/recycle:block">
                {groups.flatMap((group) => {
                  const groupSelectableItems = group.items.filter(
                    (item) => !pendingItemIds.has(item.id),
                  );
                  const groupSelectedCount = groupSelectableItems.filter(
                    (item) => selectedItemIds.has(item.id),
                  ).length;
                  const groupChecked =
                    groupSelectedCount === 0
                      ? false
                      : groupSelectedCount === groupSelectableItems.length
                        ? true
                        : "indeterminate";
                  return [
                    <TableRow
                      key={`${group.id}:heading`}
                      className="bg-muted/40 hover:bg-muted/40 @max-[72rem]/recycle:grid @max-[72rem]/recycle:grid-cols-[3rem_minmax(0,1fr)] @max-[72rem]/recycle:items-center"
                    >
                      <TableCheckboxCell>
                        <Checkbox
                          id={selectorId(
                            "settings-recycle-bin-group",
                            group.id,
                          )}
                          size="table"
                          checked={groupChecked}
                          className="@max-[72rem]/recycle:size-6"
                          disabled={groupSelectableItems.length === 0 || isBusy}
                          aria-label={t("settings.recycleBinSelectTitle", {
                            title: group.titleName,
                          })}
                          onCheckedChange={(checked) =>
                            toggleItems(groupSelectableItems, checked === true)
                          }
                        />
                      </TableCheckboxCell>
                      <TableCell colSpan={7} className="min-w-0">
                        <div className="flex min-w-0 flex-wrap items-baseline gap-x-2 gap-y-1">
                          <span className="font-medium [overflow-wrap:anywhere]">
                            {group.titleName}
                          </span>
                          <span className="min-w-0 text-xs text-muted-foreground [overflow-wrap:anywhere]">
                            {group.libraryName} ·{" "}
                            {t(
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
                      const isHistoryExpanded = historyItemId === item.id;
                      const itemHref = buildCalendarEventHref({
                        id: item.episodeId ?? "",
                        titleId: item.titleId ?? "",
                        titleFacet: item.titleFacet ?? "",
                      });
                      const rowBusy =
                        pendingItemIds.has(item.id) ||
                        mutatingId === "__empty__";
                      return (
                        <Fragment key={item.id}>
                          <TableRow
                            id={selectorId("settings-recycle-bin-row", item.id)}
                            data-ui="settings-table-row"
                            className="scroll-mt-24 cursor-pointer @max-[72rem]/recycle:relative @max-[72rem]/recycle:grid @max-[72rem]/recycle:grid-cols-2 @max-[72rem]/recycle:gap-x-4 @max-[72rem]/recycle:gap-y-3 @max-[72rem]/recycle:p-3 @[40rem]/recycle:grid-cols-3 @max-[72rem]/recycle:[&>td]:min-w-0 @max-[72rem]/recycle:[&>td]:p-0 @max-[72rem]/recycle:[&>td]:[overflow-wrap:anywhere]"
                            onClick={(event) => {
                              if (
                                event.target instanceof Element &&
                                event.target.closest(
                                  "button, a, input, select, textarea, [role='button'], [role='checkbox']",
                                )
                              ) {
                                return;
                              }
                              toggleHistory(item.id);
                            }}
                          >
                            <TableCheckboxCell className="@max-[72rem]/recycle:absolute @max-[72rem]/recycle:left-3 @max-[72rem]/recycle:top-3 @max-[72rem]/recycle:w-6">
                              <Checkbox
                                id={selectorId(
                                  "settings-recycle-bin-select",
                                  item.id,
                                )}
                                size="table"
                                checked={selectedItemIds.has(item.id)}
                                className="@max-[72rem]/recycle:size-6"
                                disabled={rowBusy || isBusy}
                                aria-label={t("settings.recycleBinSelectFile", {
                                  name: item.fileName,
                                })}
                                onCheckedChange={(checked) =>
                                  toggleItem(item, checked === true)
                                }
                              />
                            </TableCheckboxCell>
                            <TableCell className="w-full min-w-64 max-w-0 @max-[72rem]/recycle:col-span-full @max-[72rem]/recycle:w-auto @max-[72rem]/recycle:min-w-0 @max-[72rem]/recycle:max-w-none @max-[72rem]/recycle:pl-8!">
                              <div className="flex items-center gap-3 @max-[72rem]/recycle:items-start">
                                <TableDisclosureButton
                                  id={selectorId(
                                    "settings-recycle-bin-history",
                                    item.id,
                                  )}
                                  className="@max-[72rem]/recycle:h-11 @max-[72rem]/recycle:w-11"
                                  aria-label={t(
                                    "settings.recycleBinHistoryFor",
                                    {
                                      name: item.fileName,
                                    },
                                  )}
                                  expanded={isHistoryExpanded}
                                  aria-controls={selectorId(
                                    "settings-recycle-bin-history-detail",
                                    item.id,
                                  )}
                                  onClick={() => toggleHistory(item.id)}
                                />
                                <div className="min-w-0">
                                  {itemHref ? (
                                    <Link
                                      id={selectorId(
                                        "settings-recycle-bin-item-link",
                                        item.id,
                                      )}
                                      to={itemHref}
                                      className="block truncate font-medium text-foreground transition-colors hover:text-primary hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring @max-[72rem]/recycle:whitespace-normal"
                                      title={item.fileName}
                                    >
                                      {item.fileName}
                                    </Link>
                                  ) : (
                                    <div
                                      className="truncate font-medium @max-[72rem]/recycle:whitespace-normal"
                                      title={item.fileName}
                                    >
                                      {item.fileName}
                                    </div>
                                  )}
                                  <div
                                    className="truncate font-[family-name:var(--font-code)] text-xs text-muted-foreground @max-[72rem]/recycle:whitespace-normal"
                                    title={item.originalPath}
                                  >
                                    {item.originalPath}
                                  </div>
                                </div>
                              </div>
                            </TableCell>
                            <TableCell className="text-sm text-muted-foreground">
                              <span className="mb-1 hidden font-sans text-xs text-muted-foreground @max-[72rem]/recycle:block">
                                {t("nav.library")}
                              </span>
                              <span className="block w-28 [overflow-wrap:anywhere] @max-[72rem]/recycle:w-auto">
                                {item.libraryName}
                              </span>
                            </TableCell>
                            <TableCell>
                              <span className="mb-1 hidden font-sans text-xs text-muted-foreground @max-[72rem]/recycle:block">
                                {t("settings.recycleBinReason")}
                              </span>
                              <ReasonBadge reason={item.reason} />
                            </TableCell>
                            <TableCell className="whitespace-nowrap font-[family-name:var(--font-code)] text-sm text-muted-foreground @max-[72rem]/recycle:whitespace-normal">
                              <span className="mb-1 hidden font-sans text-xs text-muted-foreground @max-[72rem]/recycle:block">
                                {t("settings.recycleBinSize")}
                              </span>
                              {formatSize(item.sizeBytes)}
                            </TableCell>
                            <TableCell className="text-sm text-muted-foreground whitespace-nowrap @max-[72rem]/recycle:whitespace-normal">
                              <span className="mb-1 hidden font-sans text-xs text-muted-foreground @max-[72rem]/recycle:block">
                                {t("settings.recycleBinRecycled")}
                              </span>
                              <time dateTime={item.recycledAt}>
                                {formatDate(item.recycledAt, dateTimeFormat)}
                              </time>
                            </TableCell>
                            <TableCell className="text-sm text-muted-foreground whitespace-nowrap @max-[72rem]/recycle:whitespace-normal">
                              <span className="mb-1 hidden font-sans text-xs text-muted-foreground @max-[72rem]/recycle:block">
                                {t("settings.recycleBinScheduledDeletion")}
                              </span>
                              <time dateTime={item.scheduledDeletionAt}>
                                {formatDate(
                                  item.scheduledDeletionAt,
                                  dateTimeFormat,
                                )}
                              </time>
                            </TableCell>
                            <TableCell className="text-right @max-[72rem]/recycle:self-end">
                              <span className="mb-1 hidden font-sans text-xs text-muted-foreground @max-[72rem]/recycle:block">
                                {t("label.actions")}
                              </span>
                              <div className="flex items-center justify-end gap-1">
                                <IconButton
                                  id={selectorId(
                                    "settings-recycle-bin-restore",
                                    item.id,
                                  )}
                                  label={t("settings.recycleBinRestore")}
                                  className="@max-[72rem]/recycle:h-11 @max-[72rem]/recycle:w-11"
                                  tone="enabled"
                                  disabled={rowBusy || isBusy}
                                  onClick={() => onRestoreItems([item])}
                                >
                                  <Undo2 className="h-4 w-4" />
                                </IconButton>
                                <IconButton
                                  id={selectorId(
                                    "settings-recycle-bin-delete",
                                    item.id,
                                  )}
                                  label={t("settings.recycleBinDelete")}
                                  className="@max-[72rem]/recycle:h-11 @max-[72rem]/recycle:w-11"
                                  tone="delete"
                                  disabled={rowBusy || isBusy}
                                  onClick={() => onDeleteItems([item])}
                                >
                                  <Trash2 className="h-4 w-4" />
                                </IconButton>
                              </div>
                            </TableCell>
                          </TableRow>
                          {isHistoryExpanded ? (
                            <TableRow
                              id={selectorId(
                                "settings-recycle-bin-history-detail",
                                item.id,
                              )}
                              className="@max-[72rem]/recycle:block"
                            >
                              <TableCell
                                colSpan={8}
                                className="max-w-0 bg-muted/20 p-0 @max-[72rem]/recycle:block @max-[72rem]/recycle:w-full @max-[72rem]/recycle:max-w-none"
                              >
                                <RecycleBinHistory key={item.id} id={item.id} />
                              </TableCell>
                            </TableRow>
                          ) : null}
                        </Fragment>
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
