import { useEffect, useState, type ComponentType, type ReactNode } from "react";
import {
  CalendarClock,
  FileText,
  Import as ImportIcon,
  SlidersVertical,
} from "lucide-react";
import { useTranslate } from "@/lib/context/translate-context";
import { SettingsToggleSwitch } from "@/components/common/settings-toggle-switch";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  FILE_CHMOD_PRESETS,
  FOLDER_CHMOD_PRESETS,
  formatChmodMode,
  isChmodPresetValue,
} from "@/lib/constants/chmod";
import type { ImportMode } from "@/lib/types/settings";
import type { LocalPathStyle } from "@/lib/utils/local-path-style";
import type { ViewCategoryId } from "./indexer-category-picker";
import { MovieAvailabilitySettingsFields } from "./movie-availability-settings";

const FILLER_POLICY_OPTIONS = [
  { value: "DOWNLOAD_ALL", label: "settings.fillerPolicyDownloadAll" },
  { value: "SKIP_FILLER", label: "settings.fillerPolicySkipFiller" },
];

const RECAP_POLICY_OPTIONS = [
  { value: "DOWNLOAD_ALL", label: "settings.recapPolicyDownloadAll" },
  { value: "SKIP_RECAP", label: "settings.recapPolicySkipRecap" },
];

const IMPORT_MODE_OPTIONS: { value: ImportMode; label: string }[] = [
  { value: "HARDLINK_OR_COPY", label: "settings.importModeHardlinkCopy" },
  { value: "MOVE", label: "settings.importModeMove" },
];

const FILE_CHMOD_DERIVED_VALUE = "__derive_from_folder__";

type SectionIcon = ComponentType<{ className?: string }>;

function GeneralSettingsSection({
  icon: Icon,
  title,
  description,
  children,
}: {
  icon: SectionIcon;
  title: string;
  description?: ReactNode;
  children: ReactNode;
}) {
  return (
    <section className="rounded-[16px] border border-[var(--scry-border)] bg-[var(--scry-surf)] p-5 sm:p-6">
      <div className="flex items-center gap-2.5">
        <Icon className="h-[17px] w-[17px] text-[var(--scry-accent-text)]" />
        <h2 className="text-[16px] font-bold text-[var(--scry-ink2)]">
          {title}
        </h2>
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

function ToggleSettingRow({
  label,
  description,
  checked,
  disabled,
  onChange,
}: {
  label: string;
  description: string;
  checked: boolean;
  disabled: boolean;
  onChange: (checked: boolean) => void;
}) {
  return (
    <div className="flex items-center justify-between gap-4 rounded-[12px] border border-[var(--scry-border)] bg-[var(--scry-card2)] p-4">
      <div className="min-w-0">
        <Label className="text-[13.5px] font-semibold text-[var(--scry-body)]">
          {label}
        </Label>
        <p className="mt-1 text-xs leading-relaxed text-[var(--scry-muted3)]">
          {description}
        </p>
      </div>
      <SettingsToggleSwitch
        checked={checked}
        ariaLabel={label}
        disabled={disabled}
        onChange={onChange}
      />
    </div>
  );
}

export function GeneralSettingsPanel({
  activeQualityScopeId,
  mediaSettingsLoading,
  categoryFillerPolicies,
  handleFillerPolicyChange,
  categoryRecapPolicies,
  handleRecapPolicyChange,
  categoryMonitorSpecials,
  handleMonitorSpecialsChange,
  categoryInterSeasonMovies,
  handleInterSeasonMoviesChange,
  categoryMonitorFillerMovies,
  handleMonitorFillerMoviesChange,
  nfoWriteOnImport,
  handleNfoWriteChange,
  plexmatchWriteOnImport,
  handlePlexmatchWriteChange,
  importMode,
  handleImportModeChange,
  localPathStyle,
  setPermissionsLinux,
  handleSetPermissionsLinuxChange,
  fileChmod,
  handleFileChmodChange,
  folderChmod,
  handleFolderChmodChange,
  chownGroup,
  handleChownGroupChange,
}: {
  activeQualityScopeId: ViewCategoryId;
  mediaSettingsLoading: boolean;
  categoryFillerPolicies: Record<ViewCategoryId, string>;
  handleFillerPolicyChange: (value: string) => void;
  categoryRecapPolicies: Record<ViewCategoryId, string>;
  handleRecapPolicyChange: (value: string) => void;
  categoryMonitorSpecials: Record<ViewCategoryId, string>;
  handleMonitorSpecialsChange: (checked: boolean) => void;
  categoryInterSeasonMovies: Record<ViewCategoryId, string>;
  handleInterSeasonMoviesChange: (checked: boolean) => void;
  categoryMonitorFillerMovies: Record<ViewCategoryId, string>;
  handleMonitorFillerMoviesChange: (checked: boolean) => void;
  nfoWriteOnImport: Record<ViewCategoryId, string>;
  handleNfoWriteChange: (checked: boolean) => void;
  plexmatchWriteOnImport: Record<ViewCategoryId, string>;
  handlePlexmatchWriteChange: (checked: boolean) => void;
  importMode: Record<ViewCategoryId, ImportMode>;
  handleImportModeChange: (value: ImportMode) => void;
  localPathStyle: LocalPathStyle | undefined;
  setPermissionsLinux: Record<ViewCategoryId, string>;
  handleSetPermissionsLinuxChange: (checked: boolean) => void;
  fileChmod: Record<ViewCategoryId, string>;
  handleFileChmodChange: (value: string) => void;
  folderChmod: Record<ViewCategoryId, string>;
  handleFolderChmodChange: (value: string) => void;
  chownGroup: Record<ViewCategoryId, string>;
  handleChownGroupChange: (value: string) => void;
}) {
  const t = useTranslate();
  const showAnimePolicies = activeQualityScopeId === "ANIME";
  const showPlexmatch =
    activeQualityScopeId === "SERIES" || activeQualityScopeId === "ANIME";
  const [chownGroupDraft, setChownGroupDraft] = useState(
    chownGroup[activeQualityScopeId] ?? "",
  );
  const permissionsEnabled =
    setPermissionsLinux[activeQualityScopeId] === "true";
  const showUnixPermissions = localPathStyle !== "windows";
  const permissionFieldsDisabled = mediaSettingsLoading || !permissionsEnabled;
  const selectedFileChmod =
    fileChmod[activeQualityScopeId]?.trim() || FILE_CHMOD_DERIVED_VALUE;
  const selectedFolderChmod =
    folderChmod[activeQualityScopeId]?.trim() || "755";
  const customFileChmod =
    selectedFileChmod !== FILE_CHMOD_DERIVED_VALUE &&
    !isChmodPresetValue(FILE_CHMOD_PRESETS, selectedFileChmod)
      ? selectedFileChmod
      : null;
  const customFolderChmod = !isChmodPresetValue(
    FOLDER_CHMOD_PRESETS,
    selectedFolderChmod,
  )
    ? selectedFolderChmod
    : null;

  useEffect(() => {
    setChownGroupDraft(chownGroup[activeQualityScopeId] ?? "");
  }, [activeQualityScopeId, chownGroup]);

  const commitChownGroup = () => {
    const normalized = chownGroupDraft.trim();
    if (normalized !== (chownGroup[activeQualityScopeId] ?? "")) {
      handleChownGroupChange(normalized);
    }
  };

  return (
    <div className="space-y-[18px]">
      <GeneralSettingsSection
        icon={ImportIcon}
        title={t("settings.importBehaviorTitle")}
        description={t("settings.importModeDescription")}
      >
        <div className="max-w-xl space-y-2">
          <Label>{t("settings.importModeLabel")}</Label>
          <Select
            value={importMode[activeQualityScopeId]}
            onValueChange={(value) =>
              handleImportModeChange(value as ImportMode)
            }
            disabled={mediaSettingsLoading}
          >
            <SelectTrigger className="w-full">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {IMPORT_MODE_OPTIONS.map((option) => (
                <SelectItem key={option.value} value={option.value}>
                  {t(option.label)}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
        {showUnixPermissions ? (
          <div className="grid max-w-xl gap-4 sm:grid-cols-2">
            <ToggleSettingRow
              label={t("settings.setPermissionsLinuxLabel")}
              description={t("settings.setPermissionsLinuxDescription")}
              checked={setPermissionsLinux[activeQualityScopeId] === "true"}
              disabled={mediaSettingsLoading}
              onChange={handleSetPermissionsLinuxChange}
            />
            <div className="space-y-2">
              <Label>{t("settings.fileChmodLabel")}</Label>
              <Select
                value={selectedFileChmod}
                onValueChange={(value) =>
                  handleFileChmodChange(
                    value === FILE_CHMOD_DERIVED_VALUE ? "" : value,
                  )
                }
                disabled={permissionFieldsDisabled}
              >
                <SelectTrigger className="w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value={FILE_CHMOD_DERIVED_VALUE}>
                    {t("settings.fileChmodDeriveFromFolder")}
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
                          {customFileChmod} - {t("settings.chmodPresetCustom")}
                        </span>
                        <span className="font-[var(--font-code)] text-xs text-muted-foreground">
                          {formatChmodMode(customFileChmod, "file")}
                        </span>
                      </span>
                    </SelectItem>
                  ) : null}
                </SelectContent>
              </Select>
            </div>
            <div className="space-y-2">
              <Label>{t("settings.folderChmodLabel")}</Label>
              <Select
                value={selectedFolderChmod}
                onValueChange={handleFolderChmodChange}
                disabled={permissionFieldsDisabled}
              >
                <SelectTrigger className="w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
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
            </div>
            <div className="space-y-2">
              <Label>{t("settings.chownGroupLabel")}</Label>
              <Input
                value={chownGroupDraft}
                onChange={(event) => setChownGroupDraft(event.target.value)}
                onBlur={commitChownGroup}
                onKeyDown={(event) => {
                  if (event.key === "Enter") {
                    event.preventDefault();
                    commitChownGroup();
                  }
                }}
                disabled={permissionFieldsDisabled}
                placeholder={t("label.none")}
              />
            </div>
          </div>
        ) : null}
      </GeneralSettingsSection>

      <GeneralSettingsSection
        icon={FileText}
        title={t("settings.sidecarFilesTitle")}
      >
        <div
          className={`grid gap-3 ${showPlexmatch ? "md:grid-cols-2" : "md:grid-cols-1"}`}
        >
          <ToggleSettingRow
            label={t("settings.nfoWriteOnImportLabel")}
            description={t("settings.nfoWriteOnImportDescription")}
            checked={nfoWriteOnImport[activeQualityScopeId] === "true"}
            disabled={mediaSettingsLoading}
            onChange={handleNfoWriteChange}
          />
          {showPlexmatch ? (
            <ToggleSettingRow
              label={t("settings.plexmatchWriteOnImportLabel")}
              description={t("settings.plexmatchWriteOnImportDescription")}
              checked={plexmatchWriteOnImport[activeQualityScopeId] === "true"}
              disabled={mediaSettingsLoading}
              onChange={handlePlexmatchWriteChange}
            />
          ) : null}
        </div>
      </GeneralSettingsSection>

      {activeQualityScopeId === "MOVIE" ? (
        <GeneralSettingsSection
          icon={CalendarClock}
          title={t("settings.movieAvailability")}
          description={t("settings.movieAvailabilityFacetDescription")}
        >
          <MovieAvailabilitySettingsFields />
        </GeneralSettingsSection>
      ) : null}

      {showAnimePolicies ? (
        <GeneralSettingsSection
          icon={SlidersVertical}
          title={t("settings.animeSettings")}
        >
          <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-3">
            <div className="space-y-2">
              <Label>{t("settings.fillerPolicyLabel")}</Label>
              <Select
                value={categoryFillerPolicies[activeQualityScopeId]}
                onValueChange={handleFillerPolicyChange}
                disabled={mediaSettingsLoading}
              >
                <SelectTrigger className="w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {FILLER_POLICY_OPTIONS.map((option) => (
                    <SelectItem key={option.value} value={option.value}>
                      {t(option.label)}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>

            <div className="space-y-2">
              <Label>{t("settings.recapPolicyLabel")}</Label>
              <Select
                value={categoryRecapPolicies[activeQualityScopeId]}
                onValueChange={handleRecapPolicyChange}
                disabled={mediaSettingsLoading}
              >
                <SelectTrigger className="w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {RECAP_POLICY_OPTIONS.map((option) => (
                    <SelectItem key={option.value} value={option.value}>
                      {t(option.label)}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
          </div>

          <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-3">
            <ToggleSettingRow
              label={t("settings.monitorSpecialsLabel")}
              description={t("settings.monitorSpecialsDescription")}
              checked={
                categoryMonitorSpecials[activeQualityScopeId] !== "false"
              }
              disabled={mediaSettingsLoading}
              onChange={handleMonitorSpecialsChange}
            />
            <ToggleSettingRow
              label={t("settings.interSeasonMoviesLabel")}
              description={t("settings.interSeasonMoviesDescription")}
              checked={
                categoryInterSeasonMovies[activeQualityScopeId] !== "false"
              }
              disabled={mediaSettingsLoading}
              onChange={handleInterSeasonMoviesChange}
            />
            <ToggleSettingRow
              label={t("settings.monitorFillerMoviesLabel")}
              description={t("settings.monitorFillerMoviesDescription")}
              checked={
                categoryMonitorFillerMovies[activeQualityScopeId] === "true"
              }
              disabled={mediaSettingsLoading}
              onChange={handleMonitorFillerMoviesChange}
            />
          </div>
        </GeneralSettingsSection>
      ) : null}
    </div>
  );
}
