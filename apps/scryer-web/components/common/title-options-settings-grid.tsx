import * as React from "react";
import { useClient } from "urql";
import {
  Database,
  Folder,
  FolderInput,
  Folders,
  Languages,
  Popcorn,
  RotateCcw,
  SlidersVertical,
  Tag,
} from "lucide-react";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { AudioLanguagePicker } from "@/components/common/audio-language-picker";
import { TitleTagsEditor } from "@/components/common/title-tags-picker";
import { Button } from "@/components/ui/button";
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
  /**
   * The title's raw tag bag: user labels plus reserved `scryer:` settings
   * entries. The picker shows only the user half and patches it by difference.
   */
  tags?: string[] | null;
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
  onOpenMove?: () => void;
  footer?: React.ReactNode;
};

type SettingsRowProps = {
  icon: React.ElementType;
  label: string;
  effective: React.ReactNode;
  children: React.ReactNode;
};

function SettingsRow({ icon: Icon, label, effective, children }: SettingsRowProps) {
  return (
    <tr className="border-b border-border/70 last:border-b-0">
      <th scope="row" className="w-[24%] px-4 py-3 text-left align-middle sm:px-5">
        <span className="flex items-center gap-2 text-sm font-medium text-muted-foreground">
          <Icon aria-hidden="true" className="size-4 shrink-0" />
          {label}
        </span>
      </th>
      <td className="w-[38%] px-4 py-3 align-middle text-sm text-foreground sm:px-5">
        <div className="min-w-0 break-words">{effective}</div>
      </td>
      <td className="w-[38%] px-4 py-3 align-middle sm:px-5">
        <div className="min-w-40">{children}</div>
      </td>
    </tr>
  );
}

function MoveTitleButton({ id, onOpen }: { id: string; onOpen: () => void }) {
  const t = useTranslate();

  return (
    <div className="flex justify-end">
      <Button
        id={id}
        type="button"
        variant="primary"
        size="sm"
        title={t("move.actionButton")}
        onClick={onOpen}
      >
        <FolderInput aria-hidden="true" className="size-4" />
        {t("move.actionButton")}
      </Button>
    </div>
  );
}

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
  onOpenMove,
  footer,
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
  const effectiveQualityProfile =
    qualityProfiles.find((profile) => profile.id === currentProfileId)?.name ??
    title.qualityTier ??
    "—";
  const effectiveAudioLanguages =
    formatAudioLanguageLabels(
      requiredAudioLanguages,
      t("title.originalAudioLanguagePerTitle"),
    ) || t("label.none");
  const effectiveFillerPolicy =
    title.effectiveFillerPolicy === "SKIP_FILLER"
      ? t("settings.fillerPolicySkipFiller")
      : t("settings.fillerPolicyDownloadAll");
  const effectiveRecapPolicy =
    title.effectiveRecapPolicy === "SKIP_RECAP"
      ? t("settings.recapPolicySkipRecap")
      : t("settings.recapPolicyDownloadAll");

  return (
    <div className="overflow-x-auto rounded-xl border border-border bg-card">
      <table className="w-full min-w-[760px] table-fixed border-collapse align-middle">
        <colgroup>
          <col className="w-[24%]" />
          <col className="w-[38%]" />
          <col className="w-[38%]" />
        </colgroup>
        <tbody>
          <SettingsRow
            icon={SlidersVertical}
            label={t("title.qualityProfile")}
            effective={effectiveQualityProfile}
          >
            <Select
              value={currentProfileId}
              onValueChange={(value) =>
                void saveTitleOptions({
                  qualityProfileId: value === INHERIT_VALUE ? null : value,
                })
              }
              disabled={saving || qualityProfiles.length === 0}
            >
              <SelectTrigger id={`${idPrefix}-quality-profile`} className="ml-auto h-9 w-[70%]">
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
          </SettingsRow>

          {rootFolderReadOnly ? (
            <SettingsRow
              icon={Database}
              label={t("title.changeFolderLibrary")}
              effective={<span id={`${idPrefix}-library`}>{currentLibraryName?.trim() || "—"}</span>}
            >
              {onOpenMove ? (
                <MoveTitleButton
                  id={`${idPrefix}-library-move-to`}
                  onOpen={onOpenMove}
                />
              ) : (
                <span className="text-sm text-muted-foreground">—</span>
              )}
            </SettingsRow>
          ) : null}

          <SettingsRow
            icon={Folder}
            label={t("title.rootFolder")}
            effective={
              <span
                id={rootFolderReadOnly ? `${idPrefix}-root-folder` : undefined}
                className="font-[var(--font-code)] text-xs"
              >
                {currentRootFolderLabel}
              </span>
            }
          >
            {rootFolderReadOnly ? (
              onOpenMove ? (
                <MoveTitleButton id={`${idPrefix}-move-to`} onOpen={onOpenMove} />
              ) : (
                <span className="text-sm text-muted-foreground">
                  —
                </span>
              )
            ) : (
              <Select
                value={rootFolderSelectValue}
                onValueChange={(rootFolderId) => void saveTitleOptions({ rootFolderId })}
                disabled={saving || sortedRootFolders.length === 0}
              >
                <SelectTrigger
                  id={`${idPrefix}-root-folder`}
                  className="ml-auto h-9 w-[70%] font-[var(--font-code)] text-sm"
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
          </SettingsRow>

          {title.facet !== "MOVIE" ? (
            <SettingsRow
              icon={Folders}
              label={t("search.addConfigSeasonFolder")}
              effective={
                effectiveUseSeasonFolders
                  ? t("search.seasonFolder.enabled")
                  : t("search.seasonFolder.disabled")
              }
            >
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
                <SelectTrigger id={`${idPrefix}-season-folder`} className="ml-auto h-9 w-[70%]">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value={INHERIT_VALUE}>{t("title.inheritDefault")}</SelectItem>
                  <SelectItem value="enabled">{t("search.seasonFolder.enabled")}</SelectItem>
                  <SelectItem value="disabled">{t("search.seasonFolder.disabled")}</SelectItem>
                </SelectContent>
              </Select>
            </SettingsRow>
          ) : null}

          <SettingsRow
            icon={Languages}
            label={t("title.requiredAudioLanguages")}
            effective={effectiveAudioLanguages}
          >
            <div className="space-y-1">
              <div id={`${idPrefix}-required-audio-languages`} className="flex justify-end">
                <AudioLanguagePicker
                  value={requiredAudioLanguages}
                  onChange={(codes) => void handleRequiredAudioChange(codes)}
                  compact
                  disabled={audioSaving}
                  buttonClassName="w-[70%]"
                />
              </div>
              {hasAudioOverride ? (
                <button
                  id={`${idPrefix}-required-audio-reset`}
                  type="button"
                  className="text-xs text-primary hover:underline"
                  onClick={() => void handleResetAudioOverride()}
                  disabled={audioSaving}
                >
                  {t("title.requiredAudioResetInherit")}
                </button>
              ) : null}
            </div>
          </SettingsRow>

          <SettingsRow
            icon={Languages}
            label={t("settings.libraryMetadataLanguageLabel")}
            effective={getLanguageLabel(effectiveMetadataLanguage)}
          >
            <Select
              value={currentMetadataLanguage}
              onValueChange={(value) =>
                void saveTitleOptions({
                  metadataLanguage: value === INHERIT_VALUE ? null : value,
                })
              }
              disabled={saving}
            >
              <SelectTrigger id={`${idPrefix}-metadata-language`} className="ml-auto h-9 w-[70%]">
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
          </SettingsRow>

          {title.facet === "ANIME" ? (
            <>
              <SettingsRow
                icon={Popcorn}
                label={t("settings.fillerPolicyLabel")}
                effective={effectiveFillerPolicy}
              >
                <Select
                  value={currentFillerPolicy}
                  onValueChange={(value) =>
                    void saveTitleOptions({
                      fillerPolicy: value === INHERIT_VALUE ? null : value,
                    })
                  }
                  disabled={saving}
                >
                  <SelectTrigger id={`${idPrefix}-filler-policy`} className="ml-auto h-9 w-[70%]">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value={INHERIT_VALUE}>{t("title.inheritDefault")}</SelectItem>
                    <SelectItem value="DOWNLOAD_ALL">{t("settings.fillerPolicyDownloadAll")}</SelectItem>
                    <SelectItem value="SKIP_FILLER">{t("settings.fillerPolicySkipFiller")}</SelectItem>
                  </SelectContent>
                </Select>
              </SettingsRow>

              <SettingsRow
                icon={RotateCcw}
                label={t("settings.recapPolicyLabel")}
                effective={effectiveRecapPolicy}
              >
                <Select
                  value={currentRecapPolicy}
                  onValueChange={(value) =>
                    void saveTitleOptions({
                      recapPolicy: value === INHERIT_VALUE ? null : value,
                    })
                  }
                  disabled={saving}
                >
                  <SelectTrigger id={`${idPrefix}-recap-policy`} className="ml-auto h-9 w-[70%]">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value={INHERIT_VALUE}>{t("title.inheritDefault")}</SelectItem>
                    <SelectItem value="DOWNLOAD_ALL">{t("settings.recapPolicyDownloadAll")}</SelectItem>
                    <SelectItem value="SKIP_RECAP">{t("settings.recapPolicySkipRecap")}</SelectItem>
                  </SelectContent>
                </Select>
              </SettingsRow>
            </>
          ) : null}

          <tr className="border-b border-border/70 last:border-b-0">
            <th scope="row" className="w-[24%] px-4 py-3 text-left align-middle sm:px-5">
              <span className="flex items-center gap-2 text-sm font-medium text-muted-foreground">
                <Tag aria-hidden="true" className="size-4 shrink-0" />
                {t("title.tagsLabel")}
              </span>
            </th>
            <TitleTagsEditor
              titleId={title.id}
              tags={title.tags}
              idPrefix={idPrefix}
              onTitleChanged={onTitleChanged}
              disabled={saving}
              layout="table"
              showLabel={false}
            />
          </tr>
        </tbody>
      </table>
      {footer ? <div className="border-t border-border/70">{footer}</div> : null}
    </div>
  );
}
