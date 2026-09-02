import * as React from "react";
import { useClient } from "urql";
import {
  BadgeCheck,
  Database,
  Folder,
  Folders,
  Languages,
  Popcorn,
  RotateCcw,
} from "lucide-react";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { AudioLanguagePicker } from "@/components/common/audio-language-picker";
import { formatAudioLanguageLabels } from "@/lib/constants/audio-languages";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useTranslate } from "@/lib/context/translate-context";
import { setTitleRequiredAudioMutation } from "@/lib/graphql/mutations";
import { AVAILABLE_LANGUAGES, getLanguageLabel } from "@/lib/i18n";
import type { TitleOptionUpdates } from "@/lib/types/title-options";
import type { LibraryRootRecord } from "@/lib/types/titles";

const INHERIT_VALUE = "__inherit__";

export type InlineTitleSettingsTitle = {
  id: string;
  facet: string;
  metadataLanguage?: string | null;
  metadataLanguageOverride?: string | null;
  effectiveMetadataLanguage?: string | null;
  qualityProfileId?: string | null;
  qualityTier?: string | null;
  rootFolderId?: string | null;
  useSeasonFoldersOverride?: boolean | null;
  effectiveUseSeasonFolders?: boolean;
  requiredAudioLanguagesOverride?: string[] | null;
  effectiveRequiredAudioLanguages?: string[];
  inheritsRequiredAudioLanguages?: boolean;
  fillerPolicy?: string | null;
  recapPolicy?: string | null;
  effectiveFillerPolicy?: string | null;
  effectiveRecapPolicy?: string | null;
};

type Props = {
  title: InlineTitleSettingsTitle;
  qualityProfiles: { id: string; name: string }[];
  defaultRootFolder: string;
  rootFolders: LibraryRootRecord[];
  onUpdateTitleOptions: (options: TitleOptionUpdates) => Promise<void>;
  onTitleChanged?: () => Promise<void> | void;
  idPrefix: string;
  /**
   * The library this title lives in, shown beside its root wherever the two are
   * stated rather than edited (FR-010).
   */
  currentLibraryName?: string | null;
  /**
   * Where the title's files live is not an editable field: changing it is a
   * move, and a move is previewed and confirmed in the move workflow (FR-011,
   * and the replace-on-write retirement in FR-077). Callers that surface the
   * "Move To…" action pass `true` and the grid states the current library and
   * root instead of offering a dropdown that would rewrite them in place.
   */
  rootFolderReadOnly?: boolean;
};

export function TitleOptionsSettingsGrid({
  title,
  qualityProfiles,
  defaultRootFolder,
  rootFolders,
  onUpdateTitleOptions,
  onTitleChanged,
  idPrefix,
  currentLibraryName,
  rootFolderReadOnly = false,
}: Props) {
  const t = useTranslate();
  const client = useClient();
  const setGlobalStatus = useGlobalStatus();
  const [saving, setSaving] = React.useState(false);
  const [audioSaving, setAudioSaving] = React.useState(false);
  const requiredAudioLanguages = title.effectiveRequiredAudioLanguages ?? [];
  const hasAudioOverride = title.inheritsRequiredAudioLanguages === false;
  const currentProfileId = title.qualityProfileId?.trim() || INHERIT_VALUE;
  const currentRootFolderId = title.rootFolderId?.trim() || "";
  const currentSeasonFolder =
    title.useSeasonFoldersOverride == null
      ? INHERIT_VALUE
      : title.useSeasonFoldersOverride
        ? "enabled"
        : "disabled";
  const effectiveUseSeasonFolders = title.effectiveUseSeasonFolders ?? true;
  const currentMetadataLanguage =
    title.metadataLanguageOverride?.trim() || INHERIT_VALUE;
  const effectiveMetadataLanguage =
    title.effectiveMetadataLanguage?.trim() || title.metadataLanguage?.trim() || "eng";
  const currentFillerPolicy = title.fillerPolicy?.trim() || INHERIT_VALUE;
  const currentRecapPolicy = title.recapPolicy?.trim() || INHERIT_VALUE;
  const sortedRootFolders = React.useMemo(
    () =>
      [...rootFolders].sort((left, right) => {
        if (left.isDefault !== right.isDefault) {
          return left.isDefault ? -1 : 1;
        }
        return left.path.localeCompare(right.path);
      }),
    [rootFolders],
  );
  const rootFolderById = React.useMemo(
    () => new Map(rootFolders.map((root) => [root.id, root])),
    [rootFolders],
  );
  const rootFolderSelectValue = rootFolderById.has(currentRootFolderId)
    ? currentRootFolderId
    : sortedRootFolders[0]?.id ?? "";

  const saveTitleOptions = async (options: TitleOptionUpdates) => {
    setSaving(true);
    try {
      await onUpdateTitleOptions(options);
    } catch {
      setGlobalStatus(t("status.failedToUpdate"));
    } finally {
      setSaving(false);
    }
  };

  const handleRequiredAudioChange = async (languages: string[]) => {
    setAudioSaving(true);
    try {
      const { error } = await client
        .mutation(setTitleRequiredAudioMutation, {
          input: { titleId: title.id, facet: title.facet, languages },
        })
        .toPromise();
      if (error) {
        throw error;
      }
      await onTitleChanged?.();
    } catch {
      setGlobalStatus(t("status.failedToUpdate"));
    } finally {
      setAudioSaving(false);
    }
  };

  const handleResetAudioOverride = async () => {
    setAudioSaving(true);
    try {
      const { error } = await client
        .mutation(setTitleRequiredAudioMutation, {
          input: { titleId: title.id, facet: title.facet, languages: null },
        })
        .toPromise();
      if (error) {
        throw error;
      }
      await onTitleChanged?.();
    } catch {
      setGlobalStatus(t("status.failedToUpdate"));
    } finally {
      setAudioSaving(false);
    }
  };

  const folderLabel = (path: string) =>
    path.split("/").filter(Boolean).pop() ?? path;

  // The read-only statement of where this title's files are. It names the full
  // path rather than the folder name, because that is what the move workflow's
  // destination list names and the two have to be readable against each other.
  const currentRoot =
    rootFolderById.get(currentRootFolderId) ?? sortedRootFolders[0] ?? null;
  const currentRootPath = currentRoot?.path.trim() || defaultRootFolder;
  const currentRootFolderLabel = currentRoot?.isDefault
    ? t("title.defaultRootFolder", { path: currentRootPath })
    : currentRootPath;

  return (
    <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
      <div className="min-w-0">
        <label className="mb-1 flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
          <BadgeCheck aria-hidden="true" className="size-3.5" />
          {t("title.qualityProfile")}
        </label>
        <Select
          value={currentProfileId}
          onValueChange={(value) =>
            void saveTitleOptions({
              qualityProfileId: value === INHERIT_VALUE ? "" : value,
            })
          }
          disabled={saving || qualityProfiles.length === 0}
        >
          <SelectTrigger id={`${idPrefix}-quality-profile`} className="h-9 w-full">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value={INHERIT_VALUE}>{t("title.inheritDefault")}</SelectItem>
            {qualityProfiles.map((profile) => (
              <SelectItem key={profile.id} value={profile.id}>
                {profile.name}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        {currentProfileId === INHERIT_VALUE && title.qualityTier ? (
          <p className="mt-1 text-xs text-muted-foreground">
            {t("settings.libraryEffectiveProfile", { value: title.qualityTier })}
          </p>
        ) : null}
      </div>

      {/* FR-010: where a title lives is stated beside its other settings, but
          it is not one of them — the library and the root only change through
          the move workflow, which previews the transfer first (FR-011/FR-017).
          Everything here is therefore read-only text, not a pinned control
          pretending to be editable. */}
      {rootFolderReadOnly ? (
        <div className="min-w-0">
          <label className="mb-1 flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
            <Database aria-hidden="true" className="size-3.5" />
            {t("title.changeFolderLibrary")}
          </label>
          <p
            id={`${idPrefix}-library`}
            className="flex h-9 items-center truncate text-sm text-foreground"
          >
            {currentLibraryName?.trim() || "—"}
          </p>
        </div>
      ) : null}

      <div className="min-w-0">
        <label className="mb-1 flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
          <Folder aria-hidden="true" className="size-3.5" />
          {t("title.rootFolder")}
        </label>
        {rootFolderReadOnly ? (
          <p
            id={`${idPrefix}-root-folder`}
            className="flex h-9 items-center break-all font-[var(--font-code)] text-sm text-foreground"
          >
            {currentRootFolderLabel}
          </p>
        ) : (
          <Select
            value={rootFolderSelectValue}
            onValueChange={(rootFolderId) =>
              void saveTitleOptions({ rootFolderId })
            }
            disabled={saving || sortedRootFolders.length === 0}
          >
            <SelectTrigger
              id={`${idPrefix}-root-folder`}
              className="h-9 w-full font-[var(--font-code)] text-sm"
            >
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {sortedRootFolders.map((rootFolder) => (
                <SelectItem key={rootFolder.id} value={rootFolder.id}>
                  {rootFolder.isDefault
                    ? t("title.defaultRootFolder", {
                        path: folderLabel(rootFolder.path || defaultRootFolder),
                      })
                    : folderLabel(rootFolder.path)}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        )}
      </div>

      {title.facet !== "MOVIE" ? (
        <div className="min-w-0">
          <label className="mb-1 flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
            <Folders aria-hidden="true" className="size-3.5" />
            {t("search.addConfigSeasonFolder")}
          </label>
          <Select
            value={currentSeasonFolder}
            onValueChange={(value) =>
              void saveTitleOptions({
                useSeasonFolders:
                  value === INHERIT_VALUE ? null : value === "enabled",
              })
            }
            disabled={saving}
          >
            <SelectTrigger id={`${idPrefix}-season-folder`} className="h-9 w-full">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value={INHERIT_VALUE}>{t("title.inheritDefault")}</SelectItem>
              <SelectItem value="enabled">{t("search.seasonFolder.enabled")}</SelectItem>
              <SelectItem value="disabled">{t("search.seasonFolder.disabled")}</SelectItem>
            </SelectContent>
          </Select>
          {currentSeasonFolder === INHERIT_VALUE ? (
            <p className="mt-1 text-xs text-muted-foreground">
              {t("settings.libraryEffectiveSeasonFolders", {
                value: effectiveUseSeasonFolders
                  ? t("search.seasonFolder.enabled")
                  : t("search.seasonFolder.disabled"),
              })}
            </p>
          ) : null}
        </div>
      ) : null}

      <div className="min-w-0 xl:max-w-72">
        <label className="mb-1 flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
          <Languages aria-hidden="true" className="size-3.5" />
          {t("title.requiredAudioLanguages")}
        </label>
        <div id={`${idPrefix}-required-audio-languages`}>
          <AudioLanguagePicker
            value={requiredAudioLanguages}
            onChange={(codes) => void handleRequiredAudioChange(codes)}
            compact
            disabled={audioSaving}
          />
        </div>
        {!hasAudioOverride ? (
          <p className="mt-1 text-xs text-muted-foreground">
            {t("settings.libraryEffectiveAudio", {
              value:
                formatAudioLanguageLabels(
                  requiredAudioLanguages,
                  t("title.originalAudioLanguagePerTitle"),
                ) || t("label.none"),
            })}
          </p>
        ) : null}
        {hasAudioOverride ? (
          <button
            id={`${idPrefix}-required-audio-reset`}
            type="button"
            className="mt-1 text-xs text-primary hover:underline"
            onClick={() => void handleResetAudioOverride()}
            disabled={audioSaving}
          >
            {t("title.requiredAudioResetInherit")}
          </button>
        ) : null}
      </div>

      <div className="min-w-0">
        <label className="mb-1 flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
          <Database aria-hidden="true" className="size-3.5" />
          {t("settings.libraryMetadataLanguageLabel")}
        </label>
        <Select
          value={currentMetadataLanguage}
          onValueChange={(value) =>
            void saveTitleOptions({
              metadataLanguage: value === INHERIT_VALUE ? null : value,
            })
          }
          disabled={saving}
        >
          <SelectTrigger id={`${idPrefix}-metadata-language`} className="h-9 w-full">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value={INHERIT_VALUE}>{t("title.inheritDefault")}</SelectItem>
            {AVAILABLE_LANGUAGES.map((language) => (
              <SelectItem key={language.code} value={language.code}>
                {language.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        {currentMetadataLanguage === INHERIT_VALUE ? (
          <p className="mt-1 text-xs text-muted-foreground">
            {t("settings.libraryEffectiveMetadataLanguage", {
              value: getLanguageLabel(effectiveMetadataLanguage),
            })}
          </p>
        ) : null}
      </div>

      {title.facet === "ANIME" ? (
        <>
          <div className="min-w-0">
            <label className="mb-1 flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
              <Popcorn aria-hidden="true" className="size-3.5" />
              {t("settings.fillerPolicyLabel")}
            </label>
            <Select
              value={currentFillerPolicy}
              onValueChange={(value) =>
                void saveTitleOptions({
                  fillerPolicy: value === INHERIT_VALUE ? "" : value,
                })
              }
              disabled={saving}
            >
              <SelectTrigger id={`${idPrefix}-filler-policy`} className="h-9 w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value={INHERIT_VALUE}>{t("title.inheritDefault")}</SelectItem>
                <SelectItem value="DOWNLOAD_ALL">{t("settings.fillerPolicyDownloadAll")}</SelectItem>
                <SelectItem value="SKIP_FILLER">{t("settings.fillerPolicySkipFiller")}</SelectItem>
              </SelectContent>
            </Select>
            {currentFillerPolicy === INHERIT_VALUE && title.effectiveFillerPolicy ? (
              <p className="mt-1 text-xs text-muted-foreground">
                {t("settings.libraryEffectiveProfile", {
                  value:
                    title.effectiveFillerPolicy === "SKIP_FILLER"
                      ? t("settings.fillerPolicySkipFiller")
                      : t("settings.fillerPolicyDownloadAll"),
                })}
              </p>
            ) : null}
          </div>

          <div className="min-w-0">
            <label className="mb-1 flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
              <RotateCcw aria-hidden="true" className="size-3.5" />
              {t("settings.recapPolicyLabel")}
            </label>
            <Select
              value={currentRecapPolicy}
              onValueChange={(value) =>
                void saveTitleOptions({
                  recapPolicy: value === INHERIT_VALUE ? "" : value,
                })
              }
              disabled={saving}
            >
              <SelectTrigger id={`${idPrefix}-recap-policy`} className="h-9 w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value={INHERIT_VALUE}>{t("title.inheritDefault")}</SelectItem>
                <SelectItem value="DOWNLOAD_ALL">{t("settings.recapPolicyDownloadAll")}</SelectItem>
                <SelectItem value="SKIP_RECAP">{t("settings.recapPolicySkipRecap")}</SelectItem>
              </SelectContent>
            </Select>
            {currentRecapPolicy === INHERIT_VALUE && title.effectiveRecapPolicy ? (
              <p className="mt-1 text-xs text-muted-foreground">
                {t("settings.libraryEffectiveProfile", {
                  value:
                    title.effectiveRecapPolicy === "SKIP_RECAP"
                      ? t("settings.recapPolicySkipRecap")
                      : t("settings.recapPolicyDownloadAll"),
                })}
              </p>
            ) : null}
          </div>
        </>
      ) : null}
    </div>
  );
}
