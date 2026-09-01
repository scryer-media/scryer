import * as React from "react";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { useTranslate } from "@/lib/context/translate-context";
import type { TitleRecord } from "@/lib/types";
import type { LibraryRootRecord } from "@/lib/types/titles";
import type { ParsedQualityProfile } from "@/lib/types/quality-profiles";
import { AVAILABLE_LANGUAGES } from "@/lib/i18n";
import type { TitleOptionUpdates } from "@/lib/types/title-options";
import {
  DISABLED_TITLE_EDIT_VALUE,
  ENABLED_TITLE_EDIT_VALUE,
  INHERIT_TITLE_EDIT_VALUE,
  UNCHANGED_TITLE_EDIT_VALUE,
  buildTitleEditChanges,
  hasTitleEditChanges,
  initialTitleEditDraft,
  type TitleEditDraft,
} from "@/lib/utils/title-edit-dialog";

const UNCHANGED_VALUE = UNCHANGED_TITLE_EDIT_VALUE;
const INHERIT_VALUE = INHERIT_TITLE_EDIT_VALUE;
const ENABLED_VALUE = ENABLED_TITLE_EDIT_VALUE;
const DISABLED_VALUE = DISABLED_TITLE_EDIT_VALUE;

type BulkTitleEditDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  view: string;
  selectedTitles: TitleRecord[];
  qualityProfiles: ParsedQualityProfile[];
  rootFolders: LibraryRootRecord[];
  busy: boolean;
  onSubmit: (changes: TitleOptionUpdates) => Promise<void> | void;
  /**
   * Destination library shown beside the destination root (FR-010). Null when
   * the selection spans libraries, which is the FR-017 disabled case.
   */
  destinationLibraryName?: string | null;
  /**
   * When set, the root control is a **destination** control: changing it opens
   * the move workflow rather than staging a root rewrite (FR-011).
   */
  onRequestMove?: (rootFolderId: string) => void;
};

function initialDraftState(): TitleEditDraft {
  return initialTitleEditDraft();
}

export function BulkTitleEditDialog({
  open,
  onOpenChange,
  view,
  selectedTitles,
  qualityProfiles,
  rootFolders,
  busy,
  onSubmit,
  destinationLibraryName,
  onRequestMove,
}: BulkTitleEditDialogProps) {
  const t = useTranslate();
  const initialDraft = React.useMemo(
    () => initialDraftState(),
    [],
  );
  const [draft, setDraft] = React.useState<TitleEditDraft>(initialDraft);

  const isMovieView = view === "movies";
  const isAnimeView = view === "anime";
  const hasPendingChange = hasTitleEditChanges(draft, initialDraft);
  const folderLabel = React.useCallback(
    (path: string) => path.split("/").filter(Boolean).pop() ?? path,
    [],
  );
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

  React.useEffect(() => {
    if (!open) {
      return;
    }
    setDraft(initialDraft);
  }, [initialDraft, open]);

  const monitorOptions = React.useMemo(
    () =>
      isMovieView
        ? [
            {
              value: "MONITORED",
              label: t("search.monitorType.monitored"),
            },
            {
              value: "UNMONITORED",
              label: t("search.monitorType.unmonitored"),
            },
          ]
        : [
            {
              value: "FUTURE_EPISODES",
              label: t("search.monitorType.futureEpisodes"),
            },
            {
              value: "MISSING_AND_FUTURE_EPISODES",
              label: t("search.monitorType.missingAndFutureEpisodes"),
            },
            {
              value: "ALL_EPISODES",
              label: t("search.monitorType.allEpisodes"),
            },
            {
              value: "NONE",
              label: t("search.monitorType.none"),
            },
          ],
    [isMovieView, t],
  );

  const buildChanges = React.useCallback(
    () => buildTitleEditChanges(draft, initialDraft),
    [draft, initialDraft],
  );

  const handleSubmit = React.useCallback(() => {
    if (!hasPendingChange || busy) {
      return;
    }
    void Promise.resolve(onSubmit(buildChanges()));
  }, [buildChanges, busy, hasPendingChange, onSubmit]);

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-3xl">
        <DialogHeader>
          <DialogTitle>{t("title.bulkEditTitle")}</DialogTitle>
          <DialogDescription>
            {t("title.bulkEditDescription", { count: selectedTitles.length })}
          </DialogDescription>
        </DialogHeader>

        <div className="grid gap-4 md:grid-cols-2">
          <EditableField label={t("settings.qualityProfileSection")}>
            <Select
              value={draft.qualityProfileId}
              onValueChange={(value) =>
                setDraft((previous) => ({ ...previous, qualityProfileId: value }))
              }
              disabled={busy}
            >
              <SelectTrigger className="h-9 w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value={UNCHANGED_VALUE}>
                  {t("label.unchanged")}
                </SelectItem>
                <SelectItem value={INHERIT_VALUE}>
                  {t("title.inheritDefault")}
                </SelectItem>
                {qualityProfiles.map((profile) => (
                  <SelectItem key={profile.id} value={profile.id}>
                    {profile.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </EditableField>

          {/* FR-010: the destination library sits beside the destination root.
              Cross-library transfer is a later story, so a mixed-library or
              other-library destination is refused with its reason (FR-017). */}
          {onRequestMove ? (
            <EditableField label={t("move.destinationLibrary")}>
              <Select value="__current__" disabled>
                <SelectTrigger
                  id="bulk-title-edit-destination-library"
                  className="h-9 w-full text-sm"
                >
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="__current__">
                    {destinationLibraryName?.trim() ||
                      t("move.destinationMixedSourceLibraries")}
                  </SelectItem>
                </SelectContent>
              </Select>
              <p className="text-xs text-muted-foreground">
                {destinationLibraryName
                  ? t("move.destinationCrossLibraryUnavailable")
                  : t("move.destinationMixedSourceLibraries")}
              </p>
            </EditableField>
          ) : null}

          <EditableField
            label={
              onRequestMove ? t("move.destinationRoot") : t("title.rootFolder")
            }
          >
            <Select
              value={draft.rootFolderId}
              onValueChange={(value) => {
                if (onRequestMove) {
                  // Changing the destination opens the move workflow; the bulk
                  // save never rewrites roots in place (FR-011).
                  if (value !== UNCHANGED_VALUE) {
                    onRequestMove(value);
                  }
                  return;
                }
                setDraft((previous) => ({ ...previous, rootFolderId: value }));
              }}
              disabled={busy || (Boolean(onRequestMove) && sortedRootFolders.length === 0)}
            >
              <SelectTrigger className="h-9 w-full font-[var(--font-code)] text-sm">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value={UNCHANGED_VALUE}>
                  {t("label.unchanged")}
                </SelectItem>
                {sortedRootFolders.map((rootFolder) => (
                  <SelectItem key={rootFolder.id} value={rootFolder.id}>
                    {rootFolder.isDefault
                      ? t("title.defaultRootFolder", {
                          path: folderLabel(rootFolder.path),
                        })
                      : folderLabel(rootFolder.path)}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            {onRequestMove ? (
              <p className="text-xs text-muted-foreground">
                {t("move.destinationRootHelp")}
              </p>
            ) : null}
          </EditableField>

          <EditableField label={t("search.addConfigMonitorType")}>
            <Select
              value={draft.monitorType}
              onValueChange={(value) =>
                setDraft((previous) => ({ ...previous, monitorType: value }))
              }
              disabled={busy}
            >
              <SelectTrigger className="h-9 w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value={UNCHANGED_VALUE}>
                  {t("label.unchanged")}
                </SelectItem>
                {monitorOptions.map((option) => (
                  <SelectItem key={option.value} value={option.value}>
                    {option.label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </EditableField>

          <EditableField label={t("settings.libraryMetadataLanguageLabel")}>
            <Select
              value={draft.metadataLanguage}
              onValueChange={(value) =>
                setDraft((previous) => ({ ...previous, metadataLanguage: value }))
              }
              disabled={busy}
            >
              <SelectTrigger className="h-9 w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value={UNCHANGED_VALUE}>
                  {t("label.unchanged")}
                </SelectItem>
                {AVAILABLE_LANGUAGES.map((language) => (
                  <SelectItem key={language.code} value={language.code}>
                    {language.label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </EditableField>

          {!isMovieView ? (
            <EditableField label={t("search.addConfigSeasonFolder")}>
              <Select
                value={draft.useSeasonFolders}
                onValueChange={(value) =>
                  setDraft((previous) => ({
                    ...previous,
                    useSeasonFolders: value,
                  }))
                }
                disabled={busy}
              >
                <SelectTrigger className="h-9 w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value={UNCHANGED_VALUE}>
                    {t("label.unchanged")}
                  </SelectItem>
                  <SelectItem value={ENABLED_VALUE}>
                    {t("search.seasonFolder.enabled")}
                  </SelectItem>
                  <SelectItem value={DISABLED_VALUE}>
                    {t("search.seasonFolder.disabled")}
                  </SelectItem>
                </SelectContent>
              </Select>
            </EditableField>
          ) : null}

          {isAnimeView ? (
            <EditableField label={t("settings.monitorSpecialsLabel")}>
              <Select
                value={draft.monitorSpecials}
                onValueChange={(value) =>
                  setDraft((previous) => ({
                    ...previous,
                    monitorSpecials: value,
                  }))
                }
                disabled={busy}
              >
                <SelectTrigger className="h-9 w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value={UNCHANGED_VALUE}>
                    {t("label.unchanged")}
                  </SelectItem>
                  <SelectItem value={ENABLED_VALUE}>{t("label.enabled")}</SelectItem>
                  <SelectItem value={DISABLED_VALUE}>{t("label.disabled")}</SelectItem>
                </SelectContent>
              </Select>
            </EditableField>
          ) : null}

          {isAnimeView ? (
            <EditableField label={t("settings.interSeasonMoviesLabel")}>
              <Select
                value={draft.interSeasonMovies}
                onValueChange={(value) =>
                  setDraft((previous) => ({
                    ...previous,
                    interSeasonMovies: value,
                  }))
                }
                disabled={busy}
              >
                <SelectTrigger className="h-9 w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value={UNCHANGED_VALUE}>
                    {t("label.unchanged")}
                  </SelectItem>
                  <SelectItem value={ENABLED_VALUE}>{t("label.enabled")}</SelectItem>
                  <SelectItem value={DISABLED_VALUE}>{t("label.disabled")}</SelectItem>
                </SelectContent>
              </Select>
            </EditableField>
          ) : null}

          {isAnimeView ? (
            <EditableField label={t("settings.fillerPolicyLabel")}>
              <Select
                value={draft.fillerPolicy}
                onValueChange={(value) =>
                  setDraft((previous) => ({ ...previous, fillerPolicy: value }))
                }
                disabled={busy}
              >
                <SelectTrigger className="h-9 w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value={UNCHANGED_VALUE}>
                    {t("label.unchanged")}
                  </SelectItem>
                  <SelectItem value={INHERIT_VALUE}>
                    {t("title.inheritDefault")}
                  </SelectItem>
                  <SelectItem value="DOWNLOAD_ALL">
                    {t("settings.fillerPolicyDownloadAll")}
                  </SelectItem>
                  <SelectItem value="SKIP_FILLER">
                    {t("settings.fillerPolicySkipFiller")}
                  </SelectItem>
                </SelectContent>
              </Select>
            </EditableField>
          ) : null}

          {isAnimeView ? (
            <EditableField label={t("settings.recapPolicyLabel")}>
              <Select
                value={draft.recapPolicy}
                onValueChange={(value) =>
                  setDraft((previous) => ({ ...previous, recapPolicy: value }))
                }
                disabled={busy}
              >
                <SelectTrigger className="h-9 w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value={UNCHANGED_VALUE}>
                    {t("label.unchanged")}
                  </SelectItem>
                  <SelectItem value={INHERIT_VALUE}>
                    {t("title.inheritDefault")}
                  </SelectItem>
                  <SelectItem value="DOWNLOAD_ALL">
                    {t("settings.recapPolicyDownloadAll")}
                  </SelectItem>
                  <SelectItem value="SKIP_RECAP">
                    {t("settings.recapPolicySkipRecap")}
                  </SelectItem>
                </SelectContent>
              </Select>
            </EditableField>
          ) : null}
        </div>

        <DialogFooter>
          <Button
            type="button"
            variant="outline"
            onClick={() => onOpenChange(false)}
            disabled={busy}
          >
            {t("label.cancel")}
          </Button>
          <Button
            type="button"
            variant="primary"
            onClick={handleSubmit}
            disabled={busy || !hasPendingChange}
          >
            {busy ? t("label.saving") : t("label.save")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function EditableField({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div className="space-y-2 rounded-lg border border-border/70 bg-muted/20 p-3">
      <p className="text-sm font-medium text-card-foreground">{label}</p>
      {children}
    </div>
  );
}
