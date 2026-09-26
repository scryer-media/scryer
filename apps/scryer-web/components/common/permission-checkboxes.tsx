import type * as React from "react";
import { ChevronDown } from "lucide-react";

import { MultiSelectOptionList } from "@/components/ui/multi-select-dropdown";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { selectTriggerClassName } from "@/components/ui/select";
import type { LibraryRecord } from "@/lib/types";
import {
  APP_PERMISSIONS,
  LIBRARY_PERMISSIONS,
  libraryPermissionShadowSource,
  libraryPermissionsWithRequestShadowing,
  type AppPermission,
  type LibraryPermission,
} from "@/lib/utils/permissions";
import { cn } from "@/lib/utils";
import { permissionIdToken } from "@/lib/utils/permissions";

export type LibraryPermissionDrafts = Record<string, string[]>;

type PermissionOption = {
  value: string;
  label: string;
};

type FacetPermissionDropdownProps = {
  facet: LibraryRecord["facet"];
  label: string;
  libraries: LibraryRecord[];
  permissions?: string[];
  selectedByLibrary: LibraryPermissionDrafts;
  disabled?: boolean;
  showSelectAll: boolean;
  idPrefix?: string;
  onLibraryChange: (changes: LibraryPermissionDrafts) => void;
};

export const APP_PERMISSION_OPTIONS: Array<{ value: AppPermission; label: string }> = [
  { value: APP_PERMISSIONS.manageUsers, label: "Manage Users" },
  { value: APP_PERMISSIONS.managePermissions, label: "Manage Permissions" },
  { value: APP_PERMISSIONS.manageSystemSettings, label: "Manage System Settings" },
  { value: APP_PERMISSIONS.manageCatalogSettings, label: "Manage Catalog Settings" },
  { value: APP_PERMISSIONS.manageLists, label: "Manage Lists" },
];

export const LIBRARY_PERMISSION_OPTIONS: Array<{ value: LibraryPermission; label: string }> = [
  { value: LIBRARY_PERMISSIONS.view, label: "View" },
  { value: LIBRARY_PERMISSIONS.manageTitles, label: "Manage Titles" },
  { value: LIBRARY_PERMISSIONS.resolveImports, label: "Resolve Imports" },
  { value: LIBRARY_PERMISSIONS.manageLibrary, label: "Manage Library" },
  { value: LIBRARY_PERMISSIONS.request, label: "Request" },
  { value: LIBRARY_PERMISSIONS.autoApproveRequests, label: "Auto-Approve Requests" },
  { value: LIBRARY_PERMISSIONS.manageSubtitles, label: "Manage Subtitles" },
];

const FACET_OPTIONS: Array<{ value: LibraryRecord["facet"]; label: string }> = [
  { value: "MOVIE", label: "Movie" },
  { value: "SERIES", label: "Series" },
  { value: "ANIME", label: "Anime" },
];

function permissionLabel(permission: string, options: PermissionOption[]) {
  return options.find((option) => option.value === permission)?.label ?? permission;
}

function permissionOptions(permissions: string[] | undefined, options: PermissionOption[]) {
  if (!permissions) {
    return options;
  }

  return permissions.map((permission) => ({
    value: permission,
    label: permissionLabel(permission, options),
  }));
}

export function togglePermissionValue(current: string[], value: string): string[] {
  const next = new Set(current);
  if (next.has(value)) {
    next.delete(value);
  } else {
    next.add(value);
  }
  return Array.from(next);
}

function changedPermissionValue(previous: string[], next: string[]) {
  return (
    next.find((value) => !previous.includes(value)) ??
    previous.find((value) => !next.includes(value)) ??
    ""
  );
}

function shadowTitle(source: string | null): string | undefined {
  return source ? `Included by ${source}` : undefined;
}

function selectedCountLabel(count: number) {
  return count === 1 ? "1 selected" : `${count} selected`;
}

function PermissionDropdownTrigger({
  label,
  count,
  disabled,
  readOnly,
  className,
  ...props
}: React.ButtonHTMLAttributes<HTMLButtonElement> & {
  label: string;
  count: number;
  disabled?: boolean;
  readOnly?: boolean;
}) {
  return (
    <button
      type="button"
      className={selectTriggerClassName({
        className: cn(
          "w-full text-left font-normal",
          count === 0 && "text-[var(--scry-faint)]",
          readOnly && "opacity-60",
          className,
        ),
      })}
      disabled={disabled}
      aria-disabled={disabled || readOnly}
      {...props}
    >
      <span
        className={cn(
          "min-w-0 truncate text-sm",
          count === 0 && "text-[var(--scry-faint)]",
        )}
      >
        {count === 0 ? label : `${label}: ${selectedCountLabel(count)}`}
      </span>
      <ChevronDown className="h-4 w-4 shrink-0 text-[var(--scry-faint)]" />
    </button>
  );
}

function AppPermissionDropdown({
  permissions,
  selected,
  disabled,
  showSelectAll,
  idPrefix,
  onChange,
}: {
  permissions?: string[];
  selected: string[];
  disabled?: boolean;
  showSelectAll: boolean;
  idPrefix?: string;
  onChange: (next: string[]) => void;
}) {
  const options = permissionOptions(permissions, APP_PERMISSION_OPTIONS);
  const allPermissions = options.map((option) => option.value);
  const allSelected =
    allPermissions.length > 0 && allPermissions.every((permission) => selected.includes(permission));

  return (
    <Popover>
      <PopoverTrigger asChild>
        <PermissionDropdownTrigger
          id={idPrefix ? `${idPrefix}-app-trigger` : undefined}
          label="App"
          count={selected.length}
          disabled={options.length === 0}
          readOnly={disabled}
        />
      </PopoverTrigger>
      <PopoverContent
        align="start"
        className="w-max min-w-[var(--radix-popover-trigger-width)] max-w-[calc(100vw-2rem)] p-2"
      >
        <MultiSelectOptionList
          groups={[{
            options: options.map((permission) => ({
              ...permission,
              id: idPrefix
                ? `${idPrefix}-app-${permissionIdToken(permission.value)}`
                : undefined,
            })),
          }]}
          selectedValues={selected}
          onSelectedValuesChange={onChange}
          allOption={showSelectAll ? {
            id: idPrefix ? `${idPrefix}-app-select-all` : undefined,
            label: allSelected ? "Unselect all" : "Select all",
            selected: allSelected,
            onSelect: () => onChange(allSelected ? [] : allPermissions),
          } : undefined}
          allOptionClassName="mb-1 rounded-b-none border-b border-[var(--scry-line2)] pb-2"
          disabled={disabled}
          optionClassName="min-w-max"
          optionLabelClassName="whitespace-nowrap"
        />
      </PopoverContent>
    </Popover>
  );
}

function FacetPermissionDropdown({
  facet,
  label,
  libraries,
  permissions,
  selectedByLibrary,
  disabled,
  showSelectAll,
  idPrefix,
  onLibraryChange,
}: FacetPermissionDropdownProps) {
  const options = permissionOptions(permissions, LIBRARY_PERMISSION_OPTIONS);
  const facetLibraries = libraries.filter((library) => library.facet === facet);
  const allPermissions = options.map((option) => option.value);
  const allSelected =
    allPermissions.length > 0 &&
    facetLibraries.length > 0 &&
    facetLibraries.every((library) => {
      const selected = libraryPermissionsWithRequestShadowing(
        selectedByLibrary[library.id] ?? [],
      );
      return allPermissions.every((permission) => selected.includes(permission));
    });
  const selectedCount = facetLibraries.reduce(
    (count, library) =>
      count +
      libraryPermissionsWithRequestShadowing(selectedByLibrary[library.id] ?? []).length,
    0,
  );

  return (
    <Popover>
      <PopoverTrigger asChild>
        <PermissionDropdownTrigger
          id={idPrefix ? `${idPrefix}-${facet.toLowerCase()}-trigger` : undefined}
          label={label}
          count={selectedCount}
          disabled={facetLibraries.length === 0}
          readOnly={disabled}
        />
      </PopoverTrigger>
      <PopoverContent
        align="start"
        className="w-max min-w-[var(--radix-popover-trigger-width)] max-w-[calc(100vw-2rem)] p-2"
      >
        <div className="flex max-h-80 flex-col gap-3 overflow-y-auto">
          {showSelectAll ? (
            <MultiSelectOptionList
              groups={[]}
              selectedValues={[]}
              onSelectedValuesChange={() => {}}
              allOption={{
                id: idPrefix ? `${idPrefix}-${facet.toLowerCase()}-select-all` : undefined,
                label: allSelected ? "Unselect all" : "Select all",
                selected: allSelected,
                onSelect: () =>
                  onLibraryChange(
                    Object.fromEntries(
                      facetLibraries.map((library) => [
                        library.id,
                        allSelected ? [] : allPermissions,
                      ]),
                    ),
                  ),
              }}
              disabled={disabled || allPermissions.length === 0}
              className="overflow-visible border-b border-[var(--scry-line2)] pb-2"
              maxHeightClassName=""
              optionClassName="min-w-max"
              optionLabelClassName="whitespace-nowrap"
            />
          ) : null}
          {facetLibraries.map((library) => {
            const selected = selectedByLibrary[library.id] ?? [];
            const effectiveSelected = libraryPermissionsWithRequestShadowing(selected);
            return (
              <div key={library.id} className="space-y-1">
                <div className="truncate px-2 text-xs font-semibold uppercase text-muted-foreground">
                  {library.name}
                </div>
                <MultiSelectOptionList
                  groups={[{
                    options: options.map((permission) => {
                      const shadowSource = libraryPermissionShadowSource(
                        selected,
                        permission.value,
                      );
                      return {
                        ...permission,
                        id: idPrefix
                          ? `${idPrefix}-${facet.toLowerCase()}-${library.id}-${permissionIdToken(permission.value)}`
                          : undefined,
                        disabled: shadowSource !== null,
                        title: shadowTitle(shadowSource),
                      };
                    }),
                  }]}
                  selectedValues={effectiveSelected}
                  onSelectedValuesChange={(next) => {
                    const permission = changedPermissionValue(effectiveSelected, next);
                    if (permission) {
                      onLibraryChange({
                        [library.id]: togglePermissionValue(selected, permission),
                      });
                    }
                  }}
                  disabled={disabled}
                  className="overflow-visible"
                  maxHeightClassName=""
                  optionClassName="min-w-max"
                  optionLabelClassName="whitespace-nowrap"
                />
              </div>
            );
          })}
        </div>
      </PopoverContent>
    </Popover>
  );
}

export function PermissionDropdowns({
  libraries,
  appPermissions,
  libraryPermissions,
  selectedAppPermissions,
  selectedLibraryPermissions,
  disabled,
  showSelectAll = false,
  idPrefix,
  onAppChange,
  onLibraryChange,
}: {
  libraries: LibraryRecord[];
  appPermissions?: string[];
  libraryPermissions?: string[];
  selectedAppPermissions: string[];
  selectedLibraryPermissions: LibraryPermissionDrafts;
  disabled?: boolean;
  showSelectAll?: boolean;
  idPrefix?: string;
  onAppChange: (next: string[]) => void;
  onLibraryChange: (changes: LibraryPermissionDrafts) => void;
}) {
  return (
    <div className="grid gap-2 sm:grid-cols-2 xl:grid-cols-4">
      <AppPermissionDropdown
        permissions={appPermissions}
        selected={selectedAppPermissions}
        disabled={disabled}
        showSelectAll={showSelectAll}
        idPrefix={idPrefix}
        onChange={onAppChange}
      />
      {FACET_OPTIONS.map((facet) => (
        <FacetPermissionDropdown
          key={facet.value}
          facet={facet.value}
          label={facet.label}
          libraries={libraries}
          permissions={libraryPermissions}
          selectedByLibrary={selectedLibraryPermissions}
          disabled={disabled}
          showSelectAll={showSelectAll}
          idPrefix={idPrefix}
          onLibraryChange={onLibraryChange}
        />
      ))}
    </div>
  );
}
