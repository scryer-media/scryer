import * as React from "react";
import { createPortal } from "react-dom";
import { useBeforeUnload, useBlocker } from "react-router";
import {
  FileText,
  Folder,
  FolderOpen,
  FolderPlus,
  HardDrive,
  Import as ImportIcon,
  Pencil,
  Plus,
  RefreshCw,
  Save,
  Send,
  SlidersVertical,
  Trash2,
} from "lucide-react";
import { AddNewButton } from "@/components/common/add-new-button";
import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { ChangeRootDialog } from "@/components/dialogs/change-root-dialog";
import { AudioLanguagePicker } from "@/components/common/audio-language-picker";
import { FolderBrowserDialog } from "@/components/setup/folder-browser-dialog";
import { Button } from "@/components/ui/button";
import { IconButton } from "@/components/ui/icon-button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { useTranslate } from "@/lib/context/translate-context";
import { useExperimentalFeaturesEnabled } from "@/lib/context/instance-features-context";
import { SCORING_PERSONA_CHOICES } from "@/lib/constants/quality-profiles";
import { formatAudioLanguageLabels } from "@/lib/constants/audio-languages";
import { AVAILABLE_LANGUAGES } from "@/lib/i18n";
import { cn } from "@/lib/utils";
import { selectorId } from "@/lib/utils/dom-ids";
import { DownloadClientRoutingPanel } from "@/components/views/media-content/download-client-routing-panel";
import {
  LIBRARY_FOOTER_SLOT_ID,
  LIBRARY_HEADER_ACTIONS_SLOT_ID,
  LIBRARY_SECONDARY_NAV_SLOT_ID,
} from "@/components/views/media-content/facet-settings-section";
import type {
  DownloadClientRecord,
  DownloadClientRoutingEntry,
  LibraryRecord,
  LibraryScanSummary,
  LibrarySettingsDraft,
  LibrarySettingsRecord,
  ParsedQualityProfile,
  RootFolderOption,
  ScoringPersonaId,
} from "@/lib/types";
import type { ImportMode } from "@/lib/types/settings";
import type {
  DownloadClientRoutingSettings,
  DownloadClientRoutingSettingsByClient,
} from "@/lib/types/download-clients";
import {
  buildDownloadClientRoutingState,
  disabledDownloadClientRoutingSettings,
  serializeDownloadClientRoutingEntries,
} from "@/lib/utils/download-client-routing";
import {
  areNzbgetRoutingMapsEqual,
  areRoutingOrdersEqual,
} from "@/lib/utils/media-content";
import {
  isLocalPathFormatValidForStyle,
  type LocalPathStyle,
} from "@/lib/utils/local-path-style";
import {
  findConflictingLibraryNamesByRootPath,
  normalizeComparableLibraryRootPath,
  normalizeLibraryRootDrafts,
} from "@/lib/utils/library-root-validation";
import {
  FILE_CHMOD_PRESETS,
  FOLDER_CHMOD_PRESETS,
  formatChmodMode,
  isChmodPresetValue,
} from "@/lib/constants/chmod";

const INHERIT_VALUE = "__inherit__";
const BOOLEAN_TRUE_VALUE = "true";
const BOOLEAN_FALSE_VALUE = "false";
const FILLER_POLICY_OPTIONS = [
  { value: "DOWNLOAD_ALL", labelKey: "settings.fillerPolicyDownloadAll" },
  { value: "SKIP_FILLER", labelKey: "settings.fillerPolicySkipFiller" },
] as const;
const RECAP_POLICY_OPTIONS = [
  { value: "DOWNLOAD_ALL", labelKey: "settings.recapPolicyDownloadAll" },
  { value: "SKIP_RECAP", labelKey: "settings.recapPolicySkipRecap" },
] as const;
const BOOLEAN_OVERRIDE_OPTIONS = [
  { value: INHERIT_VALUE, labelKey: "settings.libraryInheritFacet" },
  { value: BOOLEAN_TRUE_VALUE, labelKey: "label.enabled" },
  { value: BOOLEAN_FALSE_VALUE, labelKey: "label.disabled" },
] as const;
const IMPORT_MODE_OPTIONS = [
  { value: INHERIT_VALUE, labelKey: "settings.libraryInheritFacet" },
  { value: "HARDLINK_OR_COPY", labelKey: "settings.importModeHardlinkCopy" },
  { value: "MOVE", labelKey: "settings.importModeMove" },
] as const;

type LibraryMutationInput = {
  name: string;
  roots: RootFolderOption[];
  settings?: LibrarySettingsDraft;
};

type MediaLibrarySettingsPanelProps = {
  facet: LibraryRecord["facet"];
  settingsTitle: string;
  libraries: LibraryRecord[];
  librariesLoading: boolean;
  rootValidationLibraries: LibraryRecord[];
  rootValidationLibrariesLoading: boolean;
  rootValidationUnavailable: boolean;
  invalidRootPathsByLibraryId: Record<string, string[]>;
  preferredLibraryId: string;
  allLibrariesValue: string;
  loading: boolean;
  saving: boolean;
  scanLoading: boolean;
  scanNotice?: string | null;
  scanSummary: LibraryScanSummary | null;
  localPathStyle: LocalPathStyle | undefined;
  qualityProfiles: ParsedQualityProfile[];
  downloadClients: DownloadClientRecord[];
  downloadClientsLoading: boolean;
  canCreateLibrary: boolean;
  canManageDownloadClientRouting: boolean;
  loadLibrarySettings: (libraryId: string) => Promise<LibrarySettingsRecord | null>;
  loadFacetDownloadClientRouting: (
    scopeId: LibraryRecord["facet"],
  ) => Promise<DownloadClientRoutingEntry[]>;
  onCreateLibrary: (input: LibraryMutationInput) => Promise<LibraryRecord | null | void> | LibraryRecord | null | void;
  onUpdateLibrary: (libraryId: string, input: LibraryMutationInput) => Promise<LibraryRecord | null | void> | LibraryRecord | null | void;
  onDeleteLibrary: (libraryId: string) => Promise<boolean | void> | boolean | void;
  onScan: (libraryId: string) => Promise<void> | void;
  /** Reports whether the content column should widen (dense routing table). */
  onWideLayoutChange?: (wide: boolean) => void;
  /** Reports the active library name for the page breadcrumb (null when creating). */
  onActiveLibraryNameChange?: (name: string | null) => void;
};

const NEW_LIBRARY_VALUE = "__new_library__";

type SectionIcon = React.ComponentType<{ className?: string }>;

/** Titled settings card matching the Library settings design (icon + heading,
 * optional description, then content). Used to group each section. */
function LibrarySettingsSection({
  icon: Icon,
  title,
  description,
  children,
}: {
  icon: SectionIcon;
  title: string;
  description?: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <section className="rounded-[16px] border border-[var(--scry-border)] bg-[var(--scry-surf)] p-5 sm:p-6">
      <div className="flex items-center gap-2.5">
        <Icon className="h-[17px] w-[17px] text-[var(--scry-accent-text)]" />
        <h2 className="text-[16px] font-bold text-[var(--scry-ink2)]">{title}</h2>
      </div>
      {description ? (
        <p className="mt-1.5 text-[12.5px] leading-relaxed text-[var(--scry-muted3)]">
          {description}
        </p>
      ) : null}
      <div className="mt-5 space-y-5">{children}</div>
    </section>
  );
}

/** Small "Effective <value>" chip shown beneath each inherit/override control. */
function EffectiveChip({ children }: { children: React.ReactNode }) {
  return (
    <span className="inline-flex items-center gap-1.5 rounded-[7px] border border-[var(--scry-border2)] bg-[var(--scry-chip)] px-2.5 py-[3px] text-[11px] font-semibold text-[var(--scry-text2)]">
      {children}
    </span>
  );
}

function rootsFromLibrary(library: LibraryRecord | null): RootFolderOption[] {
  return (library?.roots ?? []).map((root) => ({
    id: root.id,
    path: root.path,
    isDefault: root.isDefault,
  }));
}

function normalizeRoots(
  roots: RootFolderOption[],
  pathStyle?: LocalPathStyle,
): RootFolderOption[] {
  return normalizeLibraryRootDrafts(roots, pathStyle);
}

function rootsEqual(
  left: RootFolderOption[],
  right: RootFolderOption[],
  pathStyle?: LocalPathStyle,
): boolean {
  const normalizedLeft = normalizeRoots(left, pathStyle);
  const normalizedRight = normalizeRoots(right, pathStyle);
  if (normalizedLeft.length !== normalizedRight.length) {
    return false;
  }
  return normalizedLeft.every((root, index) => {
    const other = normalizedRight[index];
    return other && root.path === other.path && root.isDefault === other.isDefault;
  });
}

function booleanOverrideSelectValue(value: boolean | null | undefined): string {
  if (value == null) {
    return INHERIT_VALUE;
  }

  return value ? BOOLEAN_TRUE_VALUE : BOOLEAN_FALSE_VALUE;
}

function booleanOverrideFromSelectValue(value: string): boolean | null {
  if (value === INHERIT_VALUE) {
    return null;
  }

  return value === BOOLEAN_TRUE_VALUE;
}

function fillerPolicyLabelKey(value: string | null | undefined): string {
  return value === "SKIP_FILLER"
    ? "settings.fillerPolicySkipFiller"
    : "settings.fillerPolicyDownloadAll";
}

function recapPolicyLabelKey(value: string | null | undefined): string {
  return value === "SKIP_RECAP"
    ? "settings.recapPolicySkipRecap"
    : "settings.recapPolicyDownloadAll";
}

function importModeLabelKey(value: ImportMode | null | undefined): string {
  return value === "MOVE"
    ? "settings.importModeMove"
    : "settings.importModeHardlinkCopy";
}

export const MediaLibrarySettingsPanel = React.memo(function MediaLibrarySettingsPanel({
  facet,
  settingsTitle,
  libraries,
  librariesLoading,
  rootValidationLibraries,
  rootValidationLibrariesLoading,
  rootValidationUnavailable,
  invalidRootPathsByLibraryId,
  preferredLibraryId,
  allLibrariesValue,
  loading,
  saving,
  scanLoading,
  scanNotice,
  scanSummary,
  localPathStyle,
  qualityProfiles,
  downloadClients,
  downloadClientsLoading,
  canCreateLibrary,
  canManageDownloadClientRouting,
  loadLibrarySettings,
  loadFacetDownloadClientRouting,
  onCreateLibrary,
  onUpdateLibrary,
  onDeleteLibrary,
  onScan,
  onWideLayoutChange,
  onActiveLibraryNameChange,
}: MediaLibrarySettingsPanelProps) {
  const t = useTranslate();
  // Root changes are still being finished, so the row action and its dialog
  // only exist when the instance has opted in.
  const experimentalFeaturesEnabled = useExperimentalFeaturesEnabled();
  const [mode, setMode] = React.useState<"existing" | "new">("existing");
  const [deleteLibraryOpen, setDeleteLibraryOpen] = React.useState(false);
  const [pendingLibrarySelection, setPendingLibrarySelection] = React.useState<
    string | null
  >(null);
  const [activeLibraryId, setActiveLibraryId] = React.useState<string | null>(null);
  const [draftName, setDraftName] = React.useState("");
  const [draftRoots, setDraftRoots] = React.useState<RootFolderOption[]>([]);
  const [settingsLoading, setSettingsLoading] = React.useState(false);
  const [settingsError, setSettingsError] = React.useState<string | null>(null);
  const [draftRequiredAudioLanguages, setDraftRequiredAudioLanguages] = React.useState<string[]>([]);
  const [draftMetadataLanguage, setDraftMetadataLanguage] = React.useState(INHERIT_VALUE);
  const [draftUseSeasonFolders, setDraftUseSeasonFolders] = React.useState(INHERIT_VALUE);
  const [draftQualityProfileId, setDraftQualityProfileId] = React.useState(INHERIT_VALUE);
  const [draftRequestQualityProfileIds, setDraftRequestQualityProfileIds] = React.useState<string[]>([]);
  const [draftScoringPersona, setDraftScoringPersona] = React.useState(INHERIT_VALUE);
  const [draftFillerPolicy, setDraftFillerPolicy] = React.useState(INHERIT_VALUE);
  const [draftRecapPolicy, setDraftRecapPolicy] = React.useState(INHERIT_VALUE);
  const [draftMonitorSpecials, setDraftMonitorSpecials] = React.useState(INHERIT_VALUE);
  const [draftInterSeasonMovies, setDraftInterSeasonMovies] = React.useState(INHERIT_VALUE);
  const [draftMonitorFillerMovies, setDraftMonitorFillerMovies] = React.useState(INHERIT_VALUE);
  const [draftNfoWriteOnImport, setDraftNfoWriteOnImport] = React.useState(INHERIT_VALUE);
  const [draftPlexmatchWriteOnImport, setDraftPlexmatchWriteOnImport] = React.useState(INHERIT_VALUE);
  const [draftImportMode, setDraftImportMode] = React.useState(INHERIT_VALUE);
  const [draftSetPermissionsLinux, setDraftSetPermissionsLinux] = React.useState(INHERIT_VALUE);
  const [draftFileChmod, setDraftFileChmod] = React.useState("");
  const [draftFolderChmod, setDraftFolderChmod] = React.useState("");
  const [draftChownGroup, setDraftChownGroup] = React.useState("");
  const [draftDownloadClientRoutingMode, setDraftDownloadClientRoutingMode] =
    React.useState<"inherit" | "custom">("inherit");
  const [draftDownloadClientRouting, setDraftDownloadClientRouting] =
    React.useState<DownloadClientRoutingSettingsByClient>({});
  const [draftDownloadClientRoutingOrder, setDraftDownloadClientRoutingOrder] =
    React.useState<string[]>([]);
  const [draftDownloadClientRoutingLoading, setDraftDownloadClientRoutingLoading] =
    React.useState(false);
  const [savedSettings, setSavedSettings] = React.useState<LibrarySettingsRecord | null>(null);
  const [browserOpen, setBrowserOpen] = React.useState(false);
  const [editingIndex, setEditingIndex] = React.useState<number | null>(null);
  // FR-020's single action, opened from one root row. Held by root id rather
  // than by draft index: the dialog talks to the server about a configured
  // root, and a draft row that was never saved is not one.
  const [changeRootId, setChangeRootId] = React.useState<string | null>(null);
  const lastHydratedRoutingKeyRef = React.useRef<string | null>(null);
  const [secondaryNavTarget, setSecondaryNavTarget] =
    React.useState<HTMLElement | null>(null);
  const [headerActionsTarget, setHeaderActionsTarget] =
    React.useState<HTMLElement | null>(null);
  const [footerTarget, setFooterTarget] = React.useState<HTMLElement | null>(
    null,
  );
  React.useEffect(() => {
    setSecondaryNavTarget(document.getElementById(LIBRARY_SECONDARY_NAV_SLOT_ID));
    setHeaderActionsTarget(
      document.getElementById(LIBRARY_HEADER_ACTIONS_SLOT_ID),
    );
    setFooterTarget(document.getElementById(LIBRARY_FOOTER_SLOT_ID));
  }, []);
  React.useEffect(() => {
    onWideLayoutChange?.(draftDownloadClientRoutingMode === "custom");
  }, [draftDownloadClientRoutingMode, onWideLayoutChange]);

  const activeLibrary = React.useMemo(
    () => libraries.find((library) => library.id === activeLibraryId) ?? null,
    [activeLibraryId, libraries],
  );
  React.useEffect(() => {
    onActiveLibraryNameChange?.(activeLibrary?.name ?? null);
  }, [activeLibrary, onActiveLibraryNameChange]);
  React.useEffect(() => {
    if (canCreateLibrary || mode !== "new") {
      return;
    }
    setMode("existing");
    setActiveLibraryId(libraries[0]?.id ?? null);
  }, [canCreateLibrary, libraries, mode]);
  const currentFacet = activeLibrary?.facet ?? facet;
  const isAnimeFacet = currentFacet === "ANIME";
  const isEpisodicFacet = currentFacet === "SERIES" || currentFacet === "ANIME";
  const showPlexmatch = currentFacet === "SERIES" || currentFacet === "ANIME";
  const savedDownloadClientRoutingEntries =
    savedSettings?.downloadClientRoutingOverride ?? null;
  const savedDownloadClientRoutingState = React.useMemo(
    () =>
      buildDownloadClientRoutingState(
        downloadClients,
        savedDownloadClientRoutingEntries ?? [],
        disabledDownloadClientRoutingSettings(),
      ),
    [downloadClients, savedDownloadClientRoutingEntries],
  );

  const hydrateSavedSettings = React.useCallback(
    (settings: LibrarySettingsRecord | null) => {
      setSavedSettings(settings);
      setDraftRequiredAudioLanguages(settings?.requiredAudioLanguagesOverride ?? []);
      setDraftMetadataLanguage(settings?.metadataLanguageOverride ?? INHERIT_VALUE);
      setDraftUseSeasonFolders(
        booleanOverrideSelectValue(settings?.useSeasonFoldersOverride),
      );
      setDraftQualityProfileId(settings?.qualityProfileIdOverride ?? INHERIT_VALUE);
      setDraftRequestQualityProfileIds(settings?.requestQualityProfileIdsOverride ?? []);
      setDraftScoringPersona(settings?.scoringPersonaOverride ?? INHERIT_VALUE);
      setDraftFillerPolicy(settings?.fillerPolicyOverride ?? INHERIT_VALUE);
      setDraftRecapPolicy(settings?.recapPolicyOverride ?? INHERIT_VALUE);
      setDraftMonitorSpecials(booleanOverrideSelectValue(settings?.monitorSpecialsOverride));
      setDraftInterSeasonMovies(
        booleanOverrideSelectValue(settings?.interSeasonMoviesOverride),
      );
      setDraftMonitorFillerMovies(
        booleanOverrideSelectValue(settings?.monitorFillerMoviesOverride),
      );
      setDraftNfoWriteOnImport(
        booleanOverrideSelectValue(settings?.nfoWriteOnImportOverride),
      );
      setDraftPlexmatchWriteOnImport(
        booleanOverrideSelectValue(settings?.plexmatchWriteOnImportOverride),
      );
      setDraftImportMode(settings?.importModeOverride ?? INHERIT_VALUE);
      setDraftSetPermissionsLinux(
        booleanOverrideSelectValue(settings?.setPermissionsLinuxOverride),
      );
      setDraftFileChmod(settings?.fileChmodOverride ?? "");
      setDraftFolderChmod(settings?.folderChmodOverride ?? "");
      setDraftChownGroup(settings?.chownGroupOverride ?? "");
    },
    [],
  );

  React.useEffect(() => {
    if (mode === "new") {
      return;
    }
    if (libraries.length === 0) {
      setActiveLibraryId(null);
      return;
    }
    const preferred =
      preferredLibraryId !== allLibrariesValue
        ? libraries.find((library) => library.id === preferredLibraryId) ?? null
        : null;
    setActiveLibraryId((current) => {
      if (preferred) {
        return preferred.id;
      }
      if (current && libraries.some((library) => library.id === current)) {
        return current;
      }
      return libraries[0]?.id ?? null;
    });
  }, [allLibrariesValue, libraries, mode, preferredLibraryId]);

  React.useEffect(() => {
    if (mode === "new") {
      setSavedSettings(null);
      setDraftRequiredAudioLanguages([]);
      setDraftMetadataLanguage(INHERIT_VALUE);
      setDraftUseSeasonFolders(INHERIT_VALUE);
      setDraftQualityProfileId(INHERIT_VALUE);
      setDraftRequestQualityProfileIds([]);
      setDraftScoringPersona(INHERIT_VALUE);
      setDraftFillerPolicy(INHERIT_VALUE);
      setDraftRecapPolicy(INHERIT_VALUE);
      setDraftMonitorSpecials(INHERIT_VALUE);
      setDraftInterSeasonMovies(INHERIT_VALUE);
      setDraftMonitorFillerMovies(INHERIT_VALUE);
      setDraftNfoWriteOnImport(INHERIT_VALUE);
      setDraftPlexmatchWriteOnImport(INHERIT_VALUE);
      setDraftImportMode(INHERIT_VALUE);
      setDraftSetPermissionsLinux(INHERIT_VALUE);
      setDraftFileChmod("");
      setDraftFolderChmod("");
      setDraftChownGroup("");
      setDraftDownloadClientRoutingMode("inherit");
      setDraftDownloadClientRouting({});
      setDraftDownloadClientRoutingOrder([]);
      setDraftDownloadClientRoutingLoading(false);
      return;
    }
    setDraftName(activeLibrary?.name ?? "");
    setDraftRoots(rootsFromLibrary(activeLibrary));
  }, [activeLibrary, mode]);

  React.useEffect(() => {
    let cancelled = false;
    if (!activeLibrary || mode === "new") {
      return () => {
        cancelled = true;
      };
    }

    setSettingsLoading(true);
    setSettingsError(null);
    void loadLibrarySettings(activeLibrary.id)
      .then((settings) => {
        if (cancelled) {
          return;
        }
        hydrateSavedSettings(settings);
      })
      .catch((error) => {
        if (!cancelled) {
          setSettingsError(error instanceof Error ? error.message : t("settings.librarySettingsLoadFailed"));
        }
      })
      .finally(() => {
        if (!cancelled) {
          setSettingsLoading(false);
        }
      });

    return () => {
      cancelled = true;
    };
  }, [activeLibrary, hydrateSavedSettings, loadLibrarySettings, mode, t]);

  React.useEffect(() => {
    const routingHydrationKey =
      mode === "new"
        ? `new:${currentFacet}:${downloadClients.map((client) => client.id).join(",")}`
        : `library:${activeLibrary?.id ?? "none"}:${savedDownloadClientRoutingEntries ? "custom" : "inherit"}:${(savedDownloadClientRoutingEntries ?? []).map((entry) => entry.clientId).join(",")}:${downloadClients.map((client) => client.id).join(",")}`;

    if (lastHydratedRoutingKeyRef.current === routingHydrationKey) {
      return;
    }
    lastHydratedRoutingKeyRef.current = routingHydrationKey;

    if (mode === "new") {
      setDraftDownloadClientRoutingMode("inherit");
      setDraftDownloadClientRouting({});
      setDraftDownloadClientRoutingOrder([]);
      return;
    }

    setDraftDownloadClientRoutingMode(
      savedDownloadClientRoutingEntries ? "custom" : "inherit",
    );
    setDraftDownloadClientRouting(savedDownloadClientRoutingState.routing);
    setDraftDownloadClientRoutingOrder(savedDownloadClientRoutingState.order);
  }, [
    activeLibrary?.id,
    currentFacet,
    downloadClients,
    mode,
    savedDownloadClientRoutingEntries,
    savedDownloadClientRoutingState,
  ]);

  const normalizedDraftRoots = React.useMemo(
    () => normalizeRoots(draftRoots, localPathStyle),
    [draftRoots, localPathStyle],
  );
  const conflictingLibraryNamesByRootPath = React.useMemo(() => {
    const currentLibraryId = mode === "existing" ? activeLibrary?.id ?? null : null;
    return findConflictingLibraryNamesByRootPath(
      normalizedDraftRoots,
      rootValidationLibraries,
      currentLibraryId,
      localPathStyle,
    );
  }, [
    activeLibrary?.id,
    localPathStyle,
    mode,
    normalizedDraftRoots,
    rootValidationLibraries,
  ]);
  const sortedFolders = React.useMemo(
    () =>
      normalizedDraftRoots
        .map((rf, i) => ({ rf, originalIndex: i }))
        .sort((a, b) => (a.rf.isDefault === b.rf.isDefault ? 0 : a.rf.isDefault ? -1 : 1)),
    [normalizedDraftRoots],
  );
  const hasRootFolderConflicts = conflictingLibraryNamesByRootPath.size > 0;
  const invalidRootFolderPaths = React.useMemo(() => {
    const invalidPaths = new Set<string>();
    normalizedDraftRoots.forEach((root) => {
      if (!isLocalPathFormatValidForStyle(root.path, localPathStyle)) {
        invalidPaths.add(root.path);
      }
    });
    return invalidPaths;
  }, [localPathStyle, normalizedDraftRoots]);
  const validatedInvalidRootPathKeys = React.useMemo(() => {
    if (!activeLibrary?.id) {
      return new Set<string>();
    }
    return new Set(
      (invalidRootPathsByLibraryId[activeLibrary.id] ?? []).map(
        (path) => normalizeComparableLibraryRootPath(path, localPathStyle),
      ),
    );
  }, [activeLibrary?.id, invalidRootPathsByLibraryId, localPathStyle]);
  const hasInvalidRootFolderPaths = invalidRootFolderPaths.size > 0;
  const actionBusy = loading || librariesLoading || rootValidationLibrariesLoading || saving;
  const settingsBusy = actionBusy || settingsLoading;
  const showUnixPermissions = localPathStyle !== "windows";
  const effectiveDraftSetPermissionsLinux =
    draftSetPermissionsLinux === INHERIT_VALUE
      ? (savedSettings?.setPermissionsLinux ?? false)
      : draftSetPermissionsLinux === BOOLEAN_TRUE_VALUE;
  const permissionFieldsDisabled =
    settingsBusy || !effectiveDraftSetPermissionsLinux;
  const draftFileChmodSelectValue = draftFileChmod.trim() || INHERIT_VALUE;
  const draftFolderChmodSelectValue = draftFolderChmod.trim() || INHERIT_VALUE;
  const customFileChmod =
    draftFileChmodSelectValue !== INHERIT_VALUE &&
    !isChmodPresetValue(FILE_CHMOD_PRESETS, draftFileChmodSelectValue)
      ? draftFileChmodSelectValue
      : null;
  const customFolderChmod =
    draftFolderChmodSelectValue !== INHERIT_VALUE &&
    !isChmodPresetValue(FOLDER_CHMOD_PRESETS, draftFolderChmodSelectValue)
      ? draftFolderChmodSelectValue
      : null;
  const downloadClientRoutingBusy =
    downloadClientsLoading || draftDownloadClientRoutingLoading;
  const savedRoots = React.useMemo(() => rootsFromLibrary(activeLibrary), [activeLibrary]);
  const draftDownloadClientRoutingEntries = React.useMemo(
    () =>
      draftDownloadClientRoutingMode === "custom"
        ? serializeDownloadClientRoutingEntries(
            downloadClients,
            draftDownloadClientRouting,
            draftDownloadClientRoutingOrder,
          )
        : null,
    [
      downloadClients,
      draftDownloadClientRouting,
      draftDownloadClientRoutingMode,
      draftDownloadClientRoutingOrder,
    ],
  );
  const settingsDraft = React.useMemo<LibrarySettingsDraft>(
    () => {
      const draft: LibrarySettingsDraft = {
        requiredAudioLanguages:
          draftRequiredAudioLanguages.length > 0 ? draftRequiredAudioLanguages : null,
        metadataLanguage:
          draftMetadataLanguage === INHERIT_VALUE ? null : draftMetadataLanguage,
        useSeasonFolders: isEpisodicFacet
          ? booleanOverrideFromSelectValue(draftUseSeasonFolders)
          : null,
        qualityProfileId:
          draftQualityProfileId === INHERIT_VALUE ? null : draftQualityProfileId,
        requestQualityProfileIds:
          draftRequestQualityProfileIds.length > 0
            ? draftRequestQualityProfileIds
            : null,
        scoringPersona:
          draftScoringPersona === INHERIT_VALUE
            ? null
            : (draftScoringPersona as ScoringPersonaId),
        fillerPolicy:
          isAnimeFacet && draftFillerPolicy !== INHERIT_VALUE ? (draftFillerPolicy as 'DOWNLOAD_ALL' | 'SKIP_FILLER') : null,
        recapPolicy:
          isAnimeFacet && draftRecapPolicy !== INHERIT_VALUE ? (draftRecapPolicy as 'DOWNLOAD_ALL' | 'SKIP_RECAP') : null,
        monitorSpecials:
          isAnimeFacet ? booleanOverrideFromSelectValue(draftMonitorSpecials) : null,
        interSeasonMovies:
          isAnimeFacet ? booleanOverrideFromSelectValue(draftInterSeasonMovies) : null,
        monitorFillerMovies:
          isAnimeFacet ? booleanOverrideFromSelectValue(draftMonitorFillerMovies) : null,
        nfoWriteOnImport: booleanOverrideFromSelectValue(draftNfoWriteOnImport),
        plexmatchWriteOnImport: showPlexmatch
          ? booleanOverrideFromSelectValue(draftPlexmatchWriteOnImport)
          : null,
        importMode:
          draftImportMode === INHERIT_VALUE ? null : (draftImportMode as ImportMode),
        setPermissionsLinux: booleanOverrideFromSelectValue(draftSetPermissionsLinux),
        fileChmod: draftFileChmod.trim() === "" ? null : draftFileChmod.trim(),
        folderChmod: draftFolderChmod.trim() === "" ? null : draftFolderChmod.trim(),
        chownGroup: draftChownGroup.trim() === "" ? null : draftChownGroup.trim(),
        indexerRouting: savedSettings?.indexerRoutingOverride ?? null,
      };
      if (canManageDownloadClientRouting) {
        draft.downloadClientRouting = draftDownloadClientRoutingEntries;
      }
      return draft;
    },
    [
      canManageDownloadClientRouting,
      draftDownloadClientRoutingEntries,
      draftFillerPolicy,
      draftMetadataLanguage,
      draftChownGroup,
      draftFileChmod,
      draftFolderChmod,
      draftImportMode,
      draftInterSeasonMovies,
      draftMonitorFillerMovies,
      draftMonitorSpecials,
      draftNfoWriteOnImport,
      draftPlexmatchWriteOnImport,
      draftQualityProfileId,
      draftRequestQualityProfileIds,
      draftRecapPolicy,
      draftRequiredAudioLanguages,
      draftScoringPersona,
      draftSetPermissionsLinux,
      draftUseSeasonFolders,
      isAnimeFacet,
      isEpisodicFacet,
      savedSettings,
      showPlexmatch,
    ],
  );
  const hasSettingsChanges =
    mode === "new" ||
    (savedSettings !== null &&
      (draftRequiredAudioLanguages.join("\n") !==
        (savedSettings.requiredAudioLanguagesOverride ?? []).join("\n") ||
        settingsDraft.metadataLanguage !== savedSettings.metadataLanguageOverride ||
        settingsDraft.useSeasonFolders !== savedSettings.useSeasonFoldersOverride ||
        settingsDraft.qualityProfileId !== savedSettings.qualityProfileIdOverride ||
        (settingsDraft.requestQualityProfileIds ?? []).join("\n") !==
          (savedSettings.requestQualityProfileIdsOverride ?? []).join("\n") ||
        settingsDraft.scoringPersona !== savedSettings.scoringPersonaOverride ||
        settingsDraft.fillerPolicy !== savedSettings.fillerPolicyOverride ||
        settingsDraft.recapPolicy !== savedSettings.recapPolicyOverride ||
        settingsDraft.monitorSpecials !== savedSettings.monitorSpecialsOverride ||
      settingsDraft.interSeasonMovies !== savedSettings.interSeasonMoviesOverride ||
        settingsDraft.monitorFillerMovies !==
          savedSettings.monitorFillerMoviesOverride ||
        settingsDraft.nfoWriteOnImport !== savedSettings.nfoWriteOnImportOverride ||
        settingsDraft.plexmatchWriteOnImport !==
          savedSettings.plexmatchWriteOnImportOverride ||
        settingsDraft.importMode !== savedSettings.importModeOverride ||
        settingsDraft.setPermissionsLinux !== savedSettings.setPermissionsLinuxOverride ||
        settingsDraft.fileChmod !== savedSettings.fileChmodOverride ||
        settingsDraft.folderChmod !== savedSettings.folderChmodOverride ||
        settingsDraft.chownGroup !== savedSettings.chownGroupOverride ||
        (canManageDownloadClientRouting &&
          ((draftDownloadClientRoutingMode === "custom") !==
            Boolean(savedDownloadClientRoutingEntries) ||
            (draftDownloadClientRoutingMode === "custom" &&
              (!areNzbgetRoutingMapsEqual(
                draftDownloadClientRouting,
                savedDownloadClientRoutingState.routing,
              ) ||
                !areRoutingOrdersEqual(
                  draftDownloadClientRoutingOrder,
                  savedDownloadClientRoutingState.order,
                )))))));
  const hasDraftChanges =
    mode === "new" ||
    draftName.trim() !== (activeLibrary?.name ?? "") ||
    !rootsEqual(draftRoots, savedRoots, localPathStyle) ||
    hasSettingsChanges;
  const shouldBlockNavigation = hasDraftChanges && !saving;
  // A root change is planned against the *stored* configuration, so it is
  // offered only while the panel has nothing unsaved to contradict it.
  const canChangeRoot =
    experimentalFeaturesEnabled &&
    mode !== "new" &&
    !!activeLibrary &&
    !hasDraftChanges &&
    !actionBusy;
  const changeRootTarget = React.useMemo(
    () => savedRoots.find((candidate) => candidate.id === changeRootId) ?? null,
    [changeRootId, savedRoots],
  );
  const changeRootOtherRoots = React.useMemo(
    () =>
      savedRoots.filter(
        (candidate) => !!candidate.id && candidate.id !== changeRootId,
      ),
    [changeRootId, savedRoots],
  );
  const libraryNavigationBlocker = useBlocker(shouldBlockNavigation);

  useBeforeUnload(
    React.useCallback(
      (event: BeforeUnloadEvent) => {
        if (!shouldBlockNavigation) {
          return;
        }
        event.preventDefault();
        event.returnValue = "";
      },
      [shouldBlockNavigation],
    ),
  );
  const selectedValue = mode === "new" ? NEW_LIBRARY_VALUE : activeLibraryId ?? "";

  const applyLibrarySelection = React.useCallback((value: string) => {
    if (value === NEW_LIBRARY_VALUE) {
      if (!canCreateLibrary) {
        return;
      }
      setMode("new");
      setActiveLibraryId(null);
      setDraftName("");
      setDraftRoots([]);
      setSavedSettings(null);
      setDraftRequiredAudioLanguages([]);
      setDraftQualityProfileId(INHERIT_VALUE);
      setDraftRequestQualityProfileIds([]);
      setDraftScoringPersona(INHERIT_VALUE);
      setDraftFillerPolicy(INHERIT_VALUE);
      setDraftRecapPolicy(INHERIT_VALUE);
      setDraftMonitorSpecials(INHERIT_VALUE);
      setDraftInterSeasonMovies(INHERIT_VALUE);
      setDraftMonitorFillerMovies(INHERIT_VALUE);
      setDraftNfoWriteOnImport(INHERIT_VALUE);
      setDraftPlexmatchWriteOnImport(INHERIT_VALUE);
      setDraftImportMode(INHERIT_VALUE);
      setDraftSetPermissionsLinux(INHERIT_VALUE);
      setDraftFileChmod("");
      setDraftFolderChmod("");
      setDraftChownGroup("");
      setDraftDownloadClientRoutingMode("inherit");
      setDraftDownloadClientRouting({});
      setDraftDownloadClientRoutingOrder([]);
      setDraftDownloadClientRoutingLoading(false);
      return;
    }
    setMode("existing");
    setActiveLibraryId(value);
  }, [canCreateLibrary]);

  const handleSelectLibrary = (value: string) => {
    if (value === selectedValue) {
      return;
    }
    if (shouldBlockNavigation) {
      setPendingLibrarySelection(value);
      return;
    }
    applyLibrarySelection(value);
  };

  const handleAddPath = (path: string) => {
    const trimmed = path.trim();
    if (!trimmed) return;
    setDraftRoots((current) => {
      if (current.some((rf) => rf.path === trimmed)) {
        return current;
      }
      return normalizeRoots(
        [...current, { path: trimmed, isDefault: current.length === 0 }],
        localPathStyle,
      );
    });
  };

  const handleEditPath = (index: number, path: string) => {
    const trimmed = path.trim();
    if (!trimmed) return;
    setDraftRoots((current) => {
      if (current.some((rf, i) => rf.path === trimmed && i !== index)) {
        return current;
      }
      return normalizeRoots(
        current.map((rf, i) => (i === index ? { ...rf, path: trimmed } : rf)),
        localPathStyle,
      );
    });
  };

  const handleRemovePath = (index: number) => {
    setDraftRoots((current) =>
      normalizeRoots(current.filter((_, i) => i !== index), localPathStyle),
    );
  };

  const handleSetDefault = (index: number) => {
    setDraftRoots((current) =>
      normalizeRoots(
        current.map((rf, i) => ({ ...rf, isDefault: i === index })),
        localPathStyle,
      ),
    );
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

  const handleNewLibrary = () => {
    handleSelectLibrary(NEW_LIBRARY_VALUE);
  };

  const handleDownloadClientRoutingModeChange = React.useCallback(
    async (nextMode: "inherit" | "custom") => {
      if (nextMode === "inherit") {
        setDraftDownloadClientRoutingMode("inherit");
        return;
      }

      if (draftDownloadClientRoutingMode === "custom") {
        setDraftDownloadClientRoutingMode("custom");
        return;
      }

      if (savedDownloadClientRoutingEntries) {
        setDraftDownloadClientRoutingMode("custom");
        return;
      }

      setSettingsError(null);
      setDraftDownloadClientRoutingLoading(true);
      try {
        const entries = await loadFacetDownloadClientRouting(currentFacet);
        const nextState = buildDownloadClientRoutingState(downloadClients, entries);
        setDraftDownloadClientRoutingMode("custom");
        setDraftDownloadClientRouting(nextState.routing);
        setDraftDownloadClientRoutingOrder(nextState.order);
      } catch (error) {
        setSettingsError(
          error instanceof Error ? error.message : t("status.failedToLoad"),
        );
      } finally {
        setDraftDownloadClientRoutingLoading(false);
      }
    },
    [
      currentFacet,
      downloadClients,
      draftDownloadClientRoutingMode,
      loadFacetDownloadClientRouting,
      savedDownloadClientRoutingEntries,
      t,
    ],
  );

  const updateDownloadClientRoutingDraft = React.useCallback(
    (
      clientId: string,
      nextValue: Partial<DownloadClientRoutingSettings>,
    ) => {
      setDraftDownloadClientRouting((current) => ({
        ...current,
        [clientId]: {
          ...(current[clientId] ?? disabledDownloadClientRoutingSettings()),
          ...nextValue,
        },
      }));
      setDraftDownloadClientRoutingOrder((current) =>
        current.includes(clientId) ? current : [...current, clientId],
      );
    },
    [],
  );

  const moveDownloadClientRoutingDraft = React.useCallback(
    (clientId: string, direction: "up" | "down") => {
      setDraftDownloadClientRoutingOrder((current) => {
        const index = current.indexOf(clientId);
        if (index < 0) {
          return current;
        }

        const nextIndex = direction === "up" ? index - 1 : index + 1;
        if (nextIndex < 0 || nextIndex >= current.length) {
          return current;
        }

        const next = [...current];
        [next[index], next[nextIndex]] = [next[nextIndex], next[index]];
        return next;
      });
    },
    [],
  );

  const handleSaveLibrary = async () => {
    if (downloadClientRoutingBusy) {
      return null;
    }
    const name = draftName.trim();
    if (!name) {
      return null;
    }
    const roots = normalizeRoots(draftRoots, localPathStyle);
    setDraftRoots(roots);
    if (mode === "new") {
      if (!canCreateLibrary) {
        return null;
      }
      const created = await onCreateLibrary({ name, roots, settings: settingsDraft });
      if (created?.id) {
        try {
          const refreshedSettings = await loadLibrarySettings(created.id);
          hydrateSavedSettings(refreshedSettings);
          setSettingsError(null);
        } catch (error) {
          setSettingsError(
            error instanceof Error ? error.message : t("settings.librarySettingsLoadFailed"),
          );
        }
        setMode("existing");
        setActiveLibraryId(created.id);
      }
      return created ?? null;
    }
    if (activeLibrary) {
      const updatedLibrary =
        (await onUpdateLibrary(activeLibrary.id, {
          name,
          roots,
          settings: settingsDraft,
        })) ?? activeLibrary;
      try {
        const refreshedSettings = await loadLibrarySettings(updatedLibrary.id);
        hydrateSavedSettings(refreshedSettings);
        setSettingsError(null);
      } catch (error) {
        setSettingsError(
          error instanceof Error ? error.message : t("settings.librarySettingsLoadFailed"),
        );
      }
      return updatedLibrary;
    }
    return null;
  };

  const handleSaveAndScanLibrary = async () => {
    const savedLibrary = await handleSaveLibrary();
    const libraryId = savedLibrary?.id ?? (mode === "existing" ? activeLibrary?.id : null);
    if (!libraryId) {
      return;
    }
    void onScan(libraryId);
  };

  const handleDeleteLibrary = () => {
    if (!activeLibrary || activeLibrary.isDefault) {
      return;
    }
    setDeleteLibraryOpen(true);
  };

  const handleConfirmDeleteLibrary = async () => {
    setDeleteLibraryOpen(false);
    if (!activeLibrary || activeLibrary.isDefault) {
      return;
    }
    await onDeleteLibrary(activeLibrary.id);
  };

  const handleConfirmDiscardLibraryChanges = React.useCallback(() => {
    if (libraryNavigationBlocker.state === "blocked") {
      libraryNavigationBlocker.proceed();
      return;
    }
    if (pendingLibrarySelection !== null) {
      const nextSelection = pendingLibrarySelection;
      setPendingLibrarySelection(null);
      applyLibrarySelection(nextSelection);
    }
  }, [applyLibrarySelection, libraryNavigationBlocker, pendingLibrarySelection]);

  const handleCancelDiscardLibraryChanges = React.useCallback(() => {
    if (libraryNavigationBlocker.state === "blocked") {
      libraryNavigationBlocker.reset();
    }
    setPendingLibrarySelection(null);
  }, [libraryNavigationBlocker]);

  const handleScan = () => {
    if (!activeLibrary || mode === "new") {
      return;
    }
    void onScan(activeLibrary.id);
  };

  const browserInitialPath = editingIndex !== null
    ? normalizedDraftRoots[editingIndex]?.path ?? "/"
    : "/";

  const browserTitle = editingIndex !== null
    ? t("settings.rootFolderEdit")
    : t("settings.rootFolderAdd");
  const libraryScanDisabled =
    scanLoading || actionBusy || mode === "new" || !activeLibrary;
  const libraryScanSummaryText = scanSummary
    ? t("settings.libraryScanSummary", {
        imported: scanSummary.imported,
        skipped: scanSummary.skipped,
        unmatched: scanSummary.unmatched,
      })
    : null;

  return (
    <>
      {secondaryNavTarget
        ? createPortal(
            <div>
              <div className="mb-2 px-1 text-[11px] font-bold uppercase tracking-[0.05em] text-[var(--scry-faint2)]">
                {t("settings.librariesLabel")}
              </div>
              <ul className="space-y-1">
                {libraries.map((library) => {
                  const active = mode !== "new" && selectedValue === library.id;
                  return (
                    <li key={library.id}>
                      <button
                        id={selectorId("media-library-list-item", library.id)}
                        type="button"
                        onClick={() => handleSelectLibrary(library.id)}
                        disabled={actionBusy}
                        aria-current={active ? "true" : undefined}
                        className={cn(
                          "flex w-full items-center gap-2 rounded-[10px] border border-transparent px-3 py-2 text-left text-[13px] font-medium text-[var(--scry-muted)] transition hover:bg-[var(--scry-hover)] hover:text-[var(--scry-ink2)] disabled:opacity-60",
                          active &&
                            "border-[var(--scry-baccent)] bg-[rgba(var(--scry-accent-rgb),0.12)] text-[var(--scry-ink2)]",
                        )}
                      >
                        <Folder
                          className={cn(
                            "h-4 w-4 shrink-0 text-[var(--scry-faint)]",
                            active && "text-[var(--scry-accent-text)]",
                          )}
                        />
                        <span className="min-w-0 flex-1 truncate">
                          {library.name}
                        </span>
                        {library.isDefault ? (
                          <span className="shrink-0 rounded-[6px] border border-[var(--scry-baccent)] bg-[rgba(var(--scry-accent-rgb),0.12)] px-1.5 py-0.5 text-[9px] font-bold uppercase tracking-[0.05em] text-[var(--scry-accent-text)]">
                            {t("label.default")}
                          </span>
                        ) : null}
                      </button>
                    </li>
                  );
                })}
              </ul>
              {canCreateLibrary ? (
                <AddNewButton
                  id="media-library-new"
                  icon={Plus}
                  label={t("settings.libraryNewButton")}
                  onClick={handleNewLibrary}
                  disabled={actionBusy}
                  aria-current={mode === "new" ? "true" : undefined}
                  className={cn(
                    "mt-2 w-full justify-start",
                    mode === "new" &&
                      "bg-[rgba(var(--scry-accent-rgb),0.16)]",
                  )}
                />
              ) : null}
            </div>,
            secondaryNavTarget,
          )
        : null}
      {headerActionsTarget && activeLibrary && mode !== "new"
        ? createPortal(
            <Button
              type="button"
              variant="outline"
              size="sm"
              onClick={handleScan}
              disabled={libraryScanDisabled}
            >
              <RefreshCw
                className={`mr-1.5 h-4 w-4${scanLoading ? " animate-spin" : ""}`}
              />
              {scanLoading
                ? t("settings.libraryScanRunning")
                : t("settings.libraryScanButton")}
            </Button>,
            headerActionsTarget,
          )
        : null}
      <div id="media-library-settings-panel" className="space-y-[18px]">
          {scanSummary ? (
            <p className="text-xs text-muted-foreground">{libraryScanSummaryText}</p>
          ) : null}
          {scanNotice ? (
            <p className="text-xs text-destructive">{scanNotice}</p>
          ) : null}

          {libraries.length === 0 && !librariesLoading && mode !== "new" ? (
            <p id="media-library-empty" className="text-sm text-muted-foreground">
              {t("settings.libraryEmpty")}
            </p>
          ) : null}

          {mode === "new" || activeLibrary ? (
            <div className="space-y-[18px]">
              <LibrarySettingsSection
                icon={HardDrive}
                title={t("settings.identityStorageTitle")}
              >
              <div className="space-y-2">
                <Label htmlFor="media-library-name">{t("settings.libraryNameLabel")}</Label>
                <Input
                  id="media-library-name"
                  value={draftName}
                  onChange={(event) => setDraftName(event.target.value)}
                  placeholder={t("settings.libraryNamePlaceholder")}
                  disabled={actionBusy}
                />
              </div>

              <div className="space-y-3">
                <Label className="block">{t("settings.rootFoldersLabel")}</Label>
                {rootValidationUnavailable ? (
                  <p className="text-xs text-[var(--scry-warning-text)]">
                    {t("settings.rootFolderValidationUnavailable")}
                  </p>
                ) : null}
                {normalizedDraftRoots.length === 0 && !loading ? (
                  <p className="text-xs text-muted-foreground">{t("settings.rootFoldersEmpty")}</p>
                ) : null}
                <ul className="space-y-2">
                  {sortedFolders.map(({ rf, originalIndex: index }) => {
                    const conflictingLibraryNames =
                      conflictingLibraryNamesByRootPath.get(rf.path) ?? null;
                    const pathFormatIsInvalid = invalidRootFolderPaths.has(rf.path);
                    const pathValidationIsInvalid =
                      validatedInvalidRootPathKeys.has(
                        normalizeComparableLibraryRootPath(
                          rf.path,
                          localPathStyle,
                        ),
                      );
                    const pathIsInvalid =
                      pathFormatIsInvalid || pathValidationIsInvalid;
                    const invalidRootTooltip = pathFormatIsInvalid
                      ? t("settings.downloadClientRemotePathMappingsLocalRequired")
                      : t("settings.rootFolderInvalidTooltip");

                    return (
                      <li
                        key={`${rf.path}-${index}`}
                        id={selectorId("media-library-root-row", rf.path)}
                        className="space-y-1"
                      >
                        <div className="flex items-center gap-2.5 rounded-[11px] border border-[var(--scry-border2)] bg-[var(--scry-bg)] py-1.5 pl-3.5 pr-2">
                          <FolderOpen className="h-4 w-4 shrink-0 text-[var(--scry-faint)]" />
                          <span
                            className="flex-1 truncate font-[var(--font-code)] text-[13.5px] text-[var(--scry-text2)]"
                            title={rf.path}
                          >
                            {rf.path}
                          </span>
                          {pathIsInvalid ? (
                            <span
                              className="shrink-0 rounded-[7px] border border-destructive/40 bg-destructive/10 px-2 py-0.5 text-[10.5px] font-bold uppercase tracking-[0.06em] text-destructive"
                              title={invalidRootTooltip}
                            >
                              {t("settings.rootFolderInvalid")}
                            </span>
                          ) : null}
                          {rf.isDefault ? (
                            <span className="shrink-0 rounded-[7px] border border-[var(--scry-baccent)] bg-[rgba(var(--scry-accent-rgb),0.12)] px-2 py-0.5 text-[10.5px] font-bold uppercase tracking-[0.06em] text-[var(--scry-accent-text)]">
                              {t("label.default")}
                            </span>
                          ) : (
                            <Button
                              id={selectorId("media-library-root-set-default", rf.path)}
                              type="button"
                              variant="ghost"
                              size="sm"
                              className="h-8 shrink-0 px-2 text-xs text-muted-foreground hover:text-foreground hover:underline"
                              onClick={() => handleSetDefault(index)}
                              disabled={actionBusy}
                            >
                              {t("settings.rootFolderSetDefault")}
                            </Button>
                          )}
                          {rf.id && experimentalFeaturesEnabled ? (
                            <Button
                              id={selectorId("media-library-root-change", rf.path)}
                              type="button"
                              variant="ghost"
                              size="sm"
                              className="h-8 shrink-0 px-2 text-xs text-muted-foreground hover:text-foreground hover:underline"
                              onClick={() => setChangeRootId(rf.id ?? null)}
                              disabled={!canChangeRoot}
                              title={
                                canChangeRoot
                                  ? undefined
                                  : t("rootChange.unavailableWhileUnsaved")
                              }
                            >
                              <HardDrive className="mr-1 h-3.5 w-3.5" />
                              {t("rootChange.rowAction")}
                            </Button>
                          ) : null}
                          <IconButton
                            id={selectorId("media-library-root-edit", rf.path)}
                            label={t("label.edit")}
                            tone="edit"
                            onClick={() => openEdit(index)}
                            disabled={actionBusy}
                          >
                            <Pencil className="h-4 w-4" />
                          </IconButton>
                          <IconButton
                            id={selectorId("media-library-root-delete", rf.path)}
                            label={t("label.delete")}
                            tone="delete"
                            onClick={() => handleRemovePath(index)}
                            disabled={actionBusy}
                          >
                            <Trash2 className="h-4 w-4" />
                          </IconButton>
                        </div>
                        {conflictingLibraryNames ? (
                          <p className="text-xs text-destructive">
                            {t("settings.rootFolderConflict", {
                              libraries: conflictingLibraryNames.join(", "),
                            })}
                          </p>
                        ) : null}
                      </li>
                    );
                  })}
                </ul>
                <AddNewButton
                  id="media-library-add-root"
                  icon={FolderPlus}
                  label={t("settings.rootFolderAdd")}
                  onClick={openAdd}
                  disabled={actionBusy}
                />
                <p className="text-xs text-muted-foreground">
                  {loading ? t("label.loading") : t("settings.rootFoldersHelp")}
                </p>
              </div>
              </LibrarySettingsSection>

              <LibrarySettingsSection
                icon={SlidersVertical}
                title={t("settings.mediaProfilesTitle")}
                description={t("settings.mediaProfilesHelp")}
              >
              <div className="grid gap-3 md:grid-cols-3">
                <div className="space-y-2">
                  <Label>{t("settings.libraryRequiredAudioLabel")}</Label>
                  <AudioLanguagePicker
                    value={draftRequiredAudioLanguages}
                    onChange={setDraftRequiredAudioLanguages}
                    disabled={settingsBusy}
                  />
                  {savedSettings ? (
                    <EffectiveChip>
                      {t("settings.libraryEffectiveAudio", {
                        value:
                          formatAudioLanguageLabels(
                            savedSettings.requiredAudioLanguages,
                            t("title.originalAudioLanguagePerTitle"),
                          ) || t("label.none"),
                      })}
                    </EffectiveChip>
                  ) : null}
                </div>
                <div className="space-y-2">
                  <Label>{t("settings.libraryMetadataLanguageLabel")}</Label>
                  <Select
                    value={draftMetadataLanguage}
                    onValueChange={setDraftMetadataLanguage}
                    disabled={settingsBusy}
                  >
                    <SelectTrigger>
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value={INHERIT_VALUE}>
                        {t("settings.libraryInheritGlobal")}
                      </SelectItem>
                      {AVAILABLE_LANGUAGES.map((language) => (
                        <SelectItem key={language.code} value={language.code}>
                          {language.label}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                  {savedSettings ? (
                    <EffectiveChip>
                      {t("settings.libraryEffectiveMetadataLanguage", {
                        value: savedSettings.metadataLanguage,
                      })}
                    </EffectiveChip>
                  ) : null}
                </div>
                {isEpisodicFacet ? (
                  <div className="space-y-2">
                    <Label>{t("settings.librarySeasonFoldersLabel")}</Label>
                    <Select
                      value={draftUseSeasonFolders}
                      onValueChange={setDraftUseSeasonFolders}
                      disabled={settingsBusy}
                    >
                      <SelectTrigger>
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectItem value={INHERIT_VALUE}>
                          {t("settings.libraryInheritFacet")}
                        </SelectItem>
                        <SelectItem value={BOOLEAN_TRUE_VALUE}>{t("label.enabled")}</SelectItem>
                        <SelectItem value={BOOLEAN_FALSE_VALUE}>{t("label.disabled")}</SelectItem>
                      </SelectContent>
                    </Select>
                    {savedSettings ? (
                      <EffectiveChip>
                        {t("settings.libraryEffectiveSeasonFolders", {
                          value: savedSettings.useSeasonFolders
                            ? t("label.enabled")
                            : t("label.disabled"),
                        })}
                      </EffectiveChip>
                    ) : null}
                  </div>
                ) : null}
                <div className="space-y-2">
                  <Label>{t("settings.libraryQualityProfileLabel")}</Label>
                  <Select
                    value={draftQualityProfileId}
                    onValueChange={setDraftQualityProfileId}
                    disabled={settingsBusy}
                  >
                    <SelectTrigger>
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value={INHERIT_VALUE}>
                        {t("settings.libraryInheritFacet")}
                      </SelectItem>
                      {qualityProfiles.map((profile) => (
                        <SelectItem key={profile.id} value={profile.id}>
                          {profile.name}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                  {savedSettings ? (
                    <EffectiveChip>
                      {t("settings.libraryEffectiveProfile", {
                        value:
                          qualityProfiles.find(
                            (profile) => profile.id === savedSettings.qualityProfileId,
                          )?.name ?? savedSettings.qualityProfileId,
                      })}
                    </EffectiveChip>
                  ) : null}
                </div>
                <div className="space-y-2">
                  <Label>{t("settings.libraryScoringPersonaLabel")}</Label>
                  <Select
                    value={draftScoringPersona}
                    onValueChange={setDraftScoringPersona}
                    disabled={settingsBusy}
                  >
                    <SelectTrigger>
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value={INHERIT_VALUE}>
                        {t("settings.libraryInheritFacet")}
                      </SelectItem>
                      {SCORING_PERSONA_CHOICES.map((choice) => (
                        <SelectItem key={choice.value} value={choice.value}>
                          {t(choice.labelKey)}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                  {savedSettings ? (
                    <EffectiveChip>
                      {t("settings.libraryEffectivePersona", {
                        value: t(
                          SCORING_PERSONA_CHOICES.find(
                            (choice) => choice.value === savedSettings.scoringPersona,
                          )?.labelKey ?? "qualityProfile.personaBalanced",
                        ),
                      })}
                    </EffectiveChip>
                  ) : null}
                </div>
              </div>

              <div className="rounded-[12px] border border-[var(--scry-border)] bg-[var(--scry-card2)] p-4">
                <div className="mb-3 flex items-center gap-2">
                  <Send className="h-[15px] w-[15px] text-[var(--scry-accent-text)]" />
                  <span className="text-[13.5px] font-semibold text-[var(--scry-body)]">
                    {t("settings.libraryRequestQualityProfilesLabel")}
                  </span>
                </div>
                <div className="flex flex-wrap gap-2.5">
                  {qualityProfiles.map((profile) => {
                    const checked = draftRequestQualityProfileIds.includes(profile.id);
                    return (
                      <label
                        key={profile.id}
                        className={cn(
                          "flex h-[42px] cursor-pointer items-center gap-2.5 rounded-[10px] border px-3.5 text-[13px] font-semibold transition",
                          checked
                            ? "border-[var(--scry-baccent)] bg-[rgba(var(--scry-accent-rgb),0.1)] text-[var(--scry-ink2)]"
                            : "border-[var(--scry-border2)] bg-[var(--scry-bg)] text-[var(--scry-text2)]",
                        )}
                      >
                        <Checkbox
                          checked={checked}
                          disabled={settingsBusy}
                          onCheckedChange={(value) => {
                            setDraftRequestQualityProfileIds((current) =>
                              value
                                ? [...current, profile.id]
                                : current.filter((profileId) => profileId !== profile.id),
                            );
                          }}
                        />
                        <span>{profile.name}</span>
                      </label>
                    );
                  })}
                </div>
                <p className="mt-3 text-xs text-[var(--scry-muted3)]">
                  {t("settings.libraryRequestQualityProfilesHelp")}
                </p>
              </div>
              </LibrarySettingsSection>

              <LibrarySettingsSection
                icon={ImportIcon}
                title={t("settings.importBehaviorTitle")}
              >
              <div
                className={cn(
                  "grid gap-5",
                  canManageDownloadClientRouting && "md:grid-cols-2",
                )}
              >
                <div className="space-y-2">
                  <Label>{t("settings.importModeLabel")}</Label>
                  <Select
                    value={draftImportMode}
                    onValueChange={setDraftImportMode}
                    disabled={settingsBusy}
                  >
                    <SelectTrigger id="media-library-import-mode-trigger">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {IMPORT_MODE_OPTIONS.map((option) => (
                        <SelectItem
                          id={selectorId("media-library-import-mode-option", option.value)}
                          key={option.value}
                          value={option.value}
                        >
                          {t(option.labelKey)}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                  {savedSettings ? (
                    <EffectiveChip>
                      {t("settings.libraryEffectiveProfile", {
                        value: t(importModeLabelKey(savedSettings.importMode)),
                      })}
                    </EffectiveChip>
                  ) : null}
                  {showUnixPermissions ? (
                    <div className="grid gap-3 pt-2 sm:grid-cols-2">
                      <div className="space-y-2">
                        <Label>{t("settings.setPermissionsLinuxLabel")}</Label>
                        <Select
                          value={draftSetPermissionsLinux}
                          onValueChange={setDraftSetPermissionsLinux}
                          disabled={settingsBusy}
                        >
                          <SelectTrigger>
                            <SelectValue />
                          </SelectTrigger>
                          <SelectContent>
                            {BOOLEAN_OVERRIDE_OPTIONS.map((option) => (
                              <SelectItem key={option.value} value={option.value}>
                                {t(option.labelKey)}
                              </SelectItem>
                            ))}
                          </SelectContent>
                        </Select>
                        {savedSettings ? (
                          <EffectiveChip>
                            {t("settings.libraryEffectiveProfile", {
                              value: savedSettings.setPermissionsLinux
                                ? t("label.enabled")
                                : t("label.disabled"),
                            })}
                          </EffectiveChip>
                        ) : null}
                      </div>
                      <div className="space-y-2">
                        <Label>{t("settings.fileChmodLabel")}</Label>
                        <Select
                          value={draftFileChmodSelectValue}
                          onValueChange={(value) =>
                            setDraftFileChmod(
                              value === INHERIT_VALUE ? "" : value,
                            )
                          }
                          disabled={permissionFieldsDisabled}
                        >
                          <SelectTrigger className="w-full">
                            <SelectValue />
                          </SelectTrigger>
                          <SelectContent>
                            <SelectItem value={INHERIT_VALUE}>
                              {t("settings.libraryInheritFacet")}
                            </SelectItem>
                            {FILE_CHMOD_PRESETS.map((option) => (
                              <SelectItem key={option.value} value={option.value}>
                                <span className="flex w-full items-center justify-between gap-4">
                                  <span>
                                    {option.value} - {t(option.labelKey)}
                                  </span>
                                  <span className="font-[var(--font-code)] text-xs text-muted-foreground">
                                    {formatChmodMode(option.value, "file")}
                                  </span>
                                </span>
                              </SelectItem>
                            ))}
                            {customFileChmod ? (
                              <SelectItem value={customFileChmod}>
                                <span className="flex w-full items-center justify-between gap-4">
                                  <span>
                                    {customFileChmod} -{" "}
                                    {t("settings.chmodPresetCustom")}
                                  </span>
                                  <span className="font-[var(--font-code)] text-xs text-muted-foreground">
                                    {formatChmodMode(customFileChmod, "file")}
                                  </span>
                                </span>
                              </SelectItem>
                            ) : null}
                          </SelectContent>
                        </Select>
                        {savedSettings ? (
                          <EffectiveChip>
                            {t("settings.libraryEffectiveProfile", {
                              value: savedSettings.fileChmod ?? t("label.none"),
                            })}
                          </EffectiveChip>
                        ) : null}
                      </div>
                      <div className="space-y-2">
                        <Label>{t("settings.folderChmodLabel")}</Label>
                        <Select
                          value={draftFolderChmodSelectValue}
                          onValueChange={(value) =>
                            setDraftFolderChmod(
                              value === INHERIT_VALUE ? "" : value,
                            )
                          }
                          disabled={permissionFieldsDisabled}
                        >
                          <SelectTrigger className="w-full">
                            <SelectValue />
                          </SelectTrigger>
                          <SelectContent>
                            <SelectItem value={INHERIT_VALUE}>
                              {t("settings.libraryInheritFacet")}
                            </SelectItem>
                            {FOLDER_CHMOD_PRESETS.map((option) => (
                              <SelectItem key={option.value} value={option.value}>
                                <span className="flex w-full items-center justify-between gap-4">
                                  <span>
                                    {option.value} - {t(option.labelKey)}
                                  </span>
                                  <span className="font-[var(--font-code)] text-xs text-muted-foreground">
                                    {formatChmodMode(option.value, "folder")}
                                  </span>
                                </span>
                              </SelectItem>
                            ))}
                            {customFolderChmod ? (
                              <SelectItem value={customFolderChmod}>
                                <span className="flex w-full items-center justify-between gap-4">
                                  <span>
                                    {customFolderChmod} -{" "}
                                    {t("settings.chmodPresetCustom")}
                                  </span>
                                  <span className="font-[var(--font-code)] text-xs text-muted-foreground">
                                    {formatChmodMode(customFolderChmod, "folder")}
                                  </span>
                                </span>
                              </SelectItem>
                            ) : null}
                          </SelectContent>
                        </Select>
                        {savedSettings ? (
                          <EffectiveChip>
                            {t("settings.libraryEffectiveProfile", {
                              value: savedSettings.folderChmod ?? t("label.none"),
                            })}
                          </EffectiveChip>
                        ) : null}
                      </div>
                      <div className="space-y-2">
                        <Label>{t("settings.chownGroupLabel")}</Label>
                        <Input
                          value={draftChownGroup}
                          onChange={(event) =>
                            setDraftChownGroup(event.target.value)
                          }
                          disabled={permissionFieldsDisabled}
                          placeholder={
                            savedSettings?.chownGroup ??
                            t("settings.libraryInheritFacet")
                          }
                        />
                        {savedSettings ? (
                          <EffectiveChip>
                            {t("settings.libraryEffectiveProfile", {
                              value: savedSettings.chownGroup ?? t("label.none"),
                            })}
                          </EffectiveChip>
                        ) : null}
                      </div>
                    </div>
                  ) : null}
                </div>
                {canManageDownloadClientRouting ? (
                  <div className="space-y-2">
                    <Label>{t("settings.downloadClientRouting")}</Label>
                    <Select
                      value={draftDownloadClientRoutingMode}
                      onValueChange={(value) => {
                        void handleDownloadClientRoutingModeChange(
                          value as "inherit" | "custom",
                        );
                      }}
                      disabled={settingsBusy || downloadClientRoutingBusy}
                    >
                      <SelectTrigger>
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectItem value="inherit">
                          {t("settings.libraryInheritFacet")}
                        </SelectItem>
                        <SelectItem value="custom">
                          {t("settings.libraryCustomRouting")}
                        </SelectItem>
                      </SelectContent>
                    </Select>
                  </div>
                ) : null}
              </div>
              {canManageDownloadClientRouting &&
              draftDownloadClientRoutingMode === "custom" ? (
                <DownloadClientRoutingPanel
                  scopeLabel={activeLibrary?.name ?? settingsTitle}
                  downloadClients={downloadClients}
                  activeScopeRouting={draftDownloadClientRouting}
                  activeScopeRoutingOrder={draftDownloadClientRoutingOrder}
                  downloadClientRoutingLoading={downloadClientRoutingBusy}
                  downloadClientRoutingSaving={saving}
                  updateDownloadClientRoutingForScope={
                    updateDownloadClientRoutingDraft
                  }
                  moveDownloadClientInScope={moveDownloadClientRoutingDraft}
                />
              ) : null}
              </LibrarySettingsSection>

              <LibrarySettingsSection
                icon={FileText}
                title={t("settings.sidecarFilesTitle")}
              >
              <div>
                <div
                  className={`grid gap-3 ${showPlexmatch ? "md:grid-cols-2" : "md:grid-cols-1"}`}
                >
                  <div className="space-y-2">
                    <Label>{t("settings.nfoWriteOnImportLabel")}</Label>
                    <Select
                      value={draftNfoWriteOnImport}
                      onValueChange={setDraftNfoWriteOnImport}
                      disabled={settingsBusy}
                    >
                      <SelectTrigger>
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        {BOOLEAN_OVERRIDE_OPTIONS.map((option) => (
                          <SelectItem key={option.value} value={option.value}>
                            {t(option.labelKey)}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                    <p className="text-xs text-muted-foreground">
                      {t("settings.nfoWriteOnImportDescription")}
                    </p>
                    {savedSettings ? (
                      <EffectiveChip>
                        {t("settings.libraryEffectiveProfile", {
                          value: t(
                            savedSettings.nfoWriteOnImport
                              ? "label.enabled"
                              : "label.disabled",
                          ),
                        })}
                      </EffectiveChip>
                    ) : null}
                  </div>
                  {showPlexmatch ? (
                    <div className="space-y-2">
                      <Label>{t("settings.plexmatchWriteOnImportLabel")}</Label>
                      <Select
                        value={draftPlexmatchWriteOnImport}
                        onValueChange={setDraftPlexmatchWriteOnImport}
                        disabled={settingsBusy}
                      >
                        <SelectTrigger>
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          {BOOLEAN_OVERRIDE_OPTIONS.map((option) => (
                            <SelectItem key={option.value} value={option.value}>
                              {t(option.labelKey)}
                            </SelectItem>
                          ))}
                        </SelectContent>
                      </Select>
                      <p className="text-xs text-muted-foreground">
                        {t("settings.plexmatchWriteOnImportDescription")}
                      </p>
                      {savedSettings?.plexmatchWriteOnImport != null ? (
                        <EffectiveChip>
                          {t("settings.libraryEffectiveProfile", {
                            value: t(
                              savedSettings.plexmatchWriteOnImport
                                ? "label.enabled"
                                : "label.disabled",
                            ),
                          })}
                        </EffectiveChip>
                      ) : null}
                    </div>
                  ) : null}
                </div>
              </div>
              </LibrarySettingsSection>

              {isAnimeFacet ? (
                <LibrarySettingsSection
                  icon={SlidersVertical}
                  title={t("settings.animeSettings")}
                >
                  <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-3">
                    <div className="space-y-2">
                      <Label>{t("settings.fillerPolicyLabel")}</Label>
                      <Select
                        value={draftFillerPolicy}
                        onValueChange={setDraftFillerPolicy}
                        disabled={settingsBusy}
                      >
                        <SelectTrigger>
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          <SelectItem value={INHERIT_VALUE}>
                            {t("settings.libraryInheritFacet")}
                          </SelectItem>
                          {FILLER_POLICY_OPTIONS.map((option) => (
                            <SelectItem key={option.value} value={option.value}>
                              {t(option.labelKey)}
                            </SelectItem>
                          ))}
                        </SelectContent>
                      </Select>
                      {savedSettings?.fillerPolicy ? (
                        <p className="text-xs text-muted-foreground">
                          {t("settings.libraryEffectiveProfile", {
                            value: t(fillerPolicyLabelKey(savedSettings.fillerPolicy)),
                          })}
                        </p>
                      ) : null}
                    </div>
                    <div className="space-y-2">
                      <Label>{t("settings.recapPolicyLabel")}</Label>
                      <Select
                        value={draftRecapPolicy}
                        onValueChange={setDraftRecapPolicy}
                        disabled={settingsBusy}
                      >
                        <SelectTrigger>
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          <SelectItem value={INHERIT_VALUE}>
                            {t("settings.libraryInheritFacet")}
                          </SelectItem>
                          {RECAP_POLICY_OPTIONS.map((option) => (
                            <SelectItem key={option.value} value={option.value}>
                              {t(option.labelKey)}
                            </SelectItem>
                          ))}
                        </SelectContent>
                      </Select>
                      {savedSettings?.recapPolicy ? (
                        <p className="text-xs text-muted-foreground">
                          {t("settings.libraryEffectiveProfile", {
                            value: t(recapPolicyLabelKey(savedSettings.recapPolicy)),
                          })}
                        </p>
                      ) : null}
                    </div>
                    <div className="space-y-2">
                      <Label>{t("settings.monitorSpecialsLabel")}</Label>
                      <Select
                        value={draftMonitorSpecials}
                        onValueChange={setDraftMonitorSpecials}
                        disabled={settingsBusy}
                      >
                        <SelectTrigger>
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          {BOOLEAN_OVERRIDE_OPTIONS.map((option) => (
                            <SelectItem key={option.value} value={option.value}>
                              {t(option.labelKey)}
                            </SelectItem>
                          ))}
                        </SelectContent>
                      </Select>
                      {savedSettings?.monitorSpecials != null ? (
                        <p className="text-xs text-muted-foreground">
                          {t("settings.libraryEffectiveProfile", {
                            value: t(
                              savedSettings.monitorSpecials
                                ? "label.enabled"
                                : "label.disabled",
                            ),
                          })}
                        </p>
                      ) : null}
                    </div>
                    <div className="space-y-2">
                      <Label>{t("settings.interSeasonMoviesLabel")}</Label>
                      <Select
                        value={draftInterSeasonMovies}
                        onValueChange={setDraftInterSeasonMovies}
                        disabled={settingsBusy}
                      >
                        <SelectTrigger>
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          {BOOLEAN_OVERRIDE_OPTIONS.map((option) => (
                            <SelectItem key={option.value} value={option.value}>
                              {t(option.labelKey)}
                            </SelectItem>
                          ))}
                        </SelectContent>
                      </Select>
                      {savedSettings?.interSeasonMovies != null ? (
                        <p className="text-xs text-muted-foreground">
                          {t("settings.libraryEffectiveProfile", {
                            value: t(
                              savedSettings.interSeasonMovies
                                ? "label.enabled"
                                : "label.disabled",
                            ),
                          })}
                        </p>
                      ) : null}
                    </div>
                    <div className="space-y-2">
                      <Label>{t("settings.monitorFillerMoviesLabel")}</Label>
                      <Select
                        value={draftMonitorFillerMovies}
                        onValueChange={setDraftMonitorFillerMovies}
                        disabled={settingsBusy}
                      >
                        <SelectTrigger>
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          {BOOLEAN_OVERRIDE_OPTIONS.map((option) => (
                            <SelectItem key={option.value} value={option.value}>
                              {t(option.labelKey)}
                            </SelectItem>
                          ))}
                        </SelectContent>
                      </Select>
                      {savedSettings?.monitorFillerMovies != null ? (
                        <p className="text-xs text-muted-foreground">
                          {t("settings.libraryEffectiveProfile", {
                            value: t(
                              savedSettings.monitorFillerMovies
                                ? "label.enabled"
                                : "label.disabled",
                            ),
                          })}
                        </p>
                      ) : null}
                    </div>
                  </div>
                </LibrarySettingsSection>
              ) : null}
              {settingsError ? (
                <p className="text-xs text-destructive">{settingsError}</p>
              ) : null}

            </div>
          ) : null}
          {footerTarget && (mode === "new" || activeLibrary)
            ? createPortal(
                <div className="border-t border-[var(--scry-border2)] bg-[var(--scry-surf)]">
                  <div
                    className={cn(
                      "mx-auto flex w-full flex-wrap items-center gap-2 px-4 pb-[max(1.5rem,calc(0.875rem+env(safe-area-inset-bottom)))] pt-3.5 sm:px-6 sm:pb-3.5 md:px-[30px]",
                      draftDownloadClientRoutingMode === "custom"
                        ? "max-w-[1280px]"
                        : "max-w-[920px]",
                    )}
                  >
                    {mode !== "new" &&
                    activeLibrary &&
                    !activeLibrary.isDefault ? (
                      <Button
                        type="button"
                        variant="destructive"
                        onClick={handleDeleteLibrary}
                        disabled={actionBusy}
                      >
                        <Trash2 className="mr-1.5 h-4 w-4" />
                        {t("settings.libraryDeleteButton")}
                      </Button>
                    ) : null}
                    {hasDraftChanges ? (
                      <span className="text-xs text-[var(--scry-muted3)]">
                        {t("settings.libraryUnsavedChanges")}
                      </span>
                    ) : null}
                    <Button
                      id="media-library-save"
                      type="button"
                      variant="outline"
                      onClick={handleSaveLibrary}
                      disabled={
                        settingsBusy ||
                        downloadClientRoutingBusy ||
                        !draftName.trim() ||
                        !hasDraftChanges ||
                        hasRootFolderConflicts ||
                        hasInvalidRootFolderPaths
                      }
                      className="ml-auto"
                    >
                      {t("settings.librarySaveOnlyButton")}
                    </Button>
                    <Button
                      id="media-library-save-scan"
                      type="button"
                      variant="primary"
                      onClick={handleSaveAndScanLibrary}
                      disabled={
                        settingsBusy ||
                        downloadClientRoutingBusy ||
                        !draftName.trim() ||
                        !hasDraftChanges ||
                        hasRootFolderConflicts ||
                        hasInvalidRootFolderPaths
                      }
                    >
                      <Save className="mr-1.5 h-4 w-4" />
                      {t("settings.librarySaveAndScanButton")}
                    </Button>
                  </div>
                </div>,
                footerTarget,
              )
            : null}
      </div>
      <FolderBrowserDialog
        open={browserOpen}
        onOpenChange={setBrowserOpen}
        onSelect={handleBrowserSelect}
        selectionTypes={["folder"]}
        initialPath={browserInitialPath}
        title={browserTitle}
      />
      {experimentalFeaturesEnabled && activeLibrary && changeRootTarget?.id ? (
        <ChangeRootDialog
          open
          onOpenChange={(next) => {
            if (!next) {
              setChangeRootId(null);
            }
          }}
          libraryId={activeLibrary.id}
          root={{
            id: changeRootTarget.id,
            path: changeRootTarget.path,
            isDefault: changeRootTarget.isDefault,
          }}
          otherRoots={changeRootOtherRoots.map((candidate) => ({
            id: candidate.id ?? "",
            path: candidate.path,
            isDefault: candidate.isDefault,
          }))}
        />
      ) : null}
      <ConfirmDialog
        open={deleteLibraryOpen}
        title={t("settings.libraryDeleteButton")}
        description={
          activeLibrary
            ? t("settings.libraryDeleteConfirm", { name: activeLibrary.name })
            : ""
        }
        confirmLabel={t("label.delete")}
        cancelLabel={t("label.cancel")}
        onConfirm={handleConfirmDeleteLibrary}
        onCancel={() => setDeleteLibraryOpen(false)}
      />
      <ConfirmDialog
        open={
          libraryNavigationBlocker.state === "blocked" ||
          pendingLibrarySelection !== null
        }
        title={t("settings.unsavedLibraryChangesTitle")}
        description={t("settings.unsavedLibraryChangesConfirm")}
        confirmLabel={t("label.discard")}
        cancelLabel={t("label.cancel")}
        onConfirm={handleConfirmDiscardLibraryChanges}
        onCancel={handleCancelDiscardLibraryChanges}
      />
    </>
  );
});
