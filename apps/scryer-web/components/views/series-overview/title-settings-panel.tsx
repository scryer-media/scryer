import * as React from "react";
import { useClient } from "urql";
import { Eye, Loader2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { ChangeTitleFolderCard } from "@/components/common/change-title-folder-card";
import { FixTitleMatchSettingsCard } from "@/components/common/fix-title-match-settings-card";
import { MediaRenamePlanPanel } from "@/components/common/media-rename-plan-panel";
import { MoveTitleSettingsCard } from "@/components/common/move-title-settings-card";
import { TitleOptionsSettingsGrid } from "@/components/common/title-options-settings-grid";
import { MoveTitlesDialog } from "@/components/dialogs/move-titles-dialog";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { mediaRenamePreviewQuery } from "@/lib/graphql/queries";
import { renameTitlesMutation } from "@/lib/graphql/mutations";
import { useTranslate } from "@/lib/context/translate-context";
import type { TitleDetail } from "@/components/containers/series-overview-container";
import type { TitleOptionUpdates } from "@/lib/types/title-options";
import type { LibraryRecord, LibraryRootRecord } from "@/lib/types/titles";

type MediaRenamePlanItem = {
  collectionId: string | null;
  seriesMovieLinkIds: string[];
  currentPath: string;
  proposedPath: string | null;
};

type MediaRenamePlan = {
  fingerprint: string;
  total: number;
  renamable: number;
  noop: number;
  conflicts: number;
  errors: number;
  items: MediaRenamePlanItem[];
};

export function TitleSettingsPanel({
  title,
  qualityProfiles,
  defaultRootFolder,
  renameEnabled,
  rootFolders,
  libraries,
  onUpdateTitleOptions,
  onOpenFixMatch,
  onTitleChanged,
}: {
  title: TitleDetail;
  qualityProfiles: { id: string; name: string }[];
  defaultRootFolder: string;
  renameEnabled: boolean;
  rootFolders: LibraryRootRecord[];
  /**
   * Every library the move workflow may offer as a destination. Threaded from
   * the container, which already reads the full list; an empty list falls back
   * to the title's own library so the panel still works standalone.
   */
  libraries?: LibraryRecord[];
  onUpdateTitleOptions: (options: TitleOptionUpdates) => Promise<void>;
  onOpenFixMatch?: () => void;
  onTitleChanged?: () => Promise<void> | void;
}) {
  const t = useTranslate();
  const client = useClient();
  const setGlobalStatus = useGlobalStatus();
  const [renamePlan, setRenamePlan] = React.useState<MediaRenamePlan | null>(null);
  const [renamePreviewing, setRenamePreviewing] = React.useState(false);
  const [renameApplying, setRenameApplying] = React.useState(false);
  // The panel's one move entry point (FR-011): the action row opens the move
  // wizard, which asks whether this is a root move or a library transfer
  // before it asks where. No destination is pre-picked here.
  const [moveOpen, setMoveOpen] = React.useState(false);
  // Every library, not just the title's own: a destination in another library
  // is a cross-library transfer (FR-055/FR-056), and the move dialog owns the
  // rules for which destinations are pickable.
  const moveLibraries = React.useMemo(
    () =>
      libraries && libraries.length > 0
        ? libraries.map((entry) => ({
            id: entry.id,
            name:
              entry.name?.trim() ||
              (entry.id === title.libraryId
                ? title.libraryName?.trim() || entry.id
                : entry.id),
            roots: entry.roots,
          }))
        : [
            {
              id: title.libraryId,
              name: title.libraryName?.trim() || title.libraryId,
              roots: rootFolders,
            },
          ],
    [libraries, rootFolders, title.libraryId, title.libraryName],
  );

  React.useEffect(() => {
    if (!renameEnabled) {
      setRenamePlan(null);
    }
  }, [renameEnabled]);

  React.useEffect(() => {
    setRenamePlan(null);
  }, [title.id, title.facet]);

  const handlePreviewRename = async () => {
    setRenamePreviewing(true);
    try {
      const { data, error } = await client.query(mediaRenamePreviewQuery, {
        input: {
          facet: title.facet,
          titleId: title.id,
          dryRun: true,
        },
      }).toPromise();
      if (error) throw error;
      const plan = data.mediaRenamePreview as MediaRenamePlan;
      setRenamePlan(plan);
      setGlobalStatus(
        t("status.renamePreviewGenerated", {
          total: plan.total,
          renamable: plan.renamable,
        }),
      );
    } catch (error: unknown) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.apiError"));
      setRenamePlan(null);
    } finally {
      setRenamePreviewing(false);
    }
  };

  const handleApplyRename = async () => {
    if (!renamePlan) return;
    setRenameApplying(true);
    try {
      const { data, error } = await client.mutation(renameTitlesMutation, {
        input: {
          facet: title.facet,
          titleIds: [title.id],
        },
      }).toPromise();
      if (error) throw error;
      const accepted =
        (data?.renameTitles?.acceptedTitleIds as string[] | undefined)?.length ??
        0;
      if (accepted === 0) {
        throw new Error(t("status.bulkRenameFailed"));
      }
      setGlobalStatus(t("status.renameQueued"));
      setRenamePlan(null);
      await onTitleChanged?.();
    } catch (error: unknown) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.apiError"));
    } finally {
      setRenameApplying(false);
    }
  };

  return (
    <div id="series-overview-title-settings" className="p-4">
      <TitleOptionsSettingsGrid
        title={title}
        qualityProfiles={qualityProfiles}
        defaultRootFolder={defaultRootFolder}
        rootFolders={rootFolders}
        onUpdateTitleOptions={onUpdateTitleOptions}
        onTitleChanged={onTitleChanged}
        idPrefix="series-overview-settings"
        currentLibraryName={title.libraryName ?? null}
        rootFolderReadOnly
      />

      <MoveTitleSettingsCard
        idPrefix="series-overview-settings"
        onOpen={() => setMoveOpen(true)}
      />

      <MoveTitlesDialog
        open={moveOpen}
        onOpenChange={setMoveOpen}
        titles={[
          {
            id: title.id,
            name: title.name,
            libraryId: title.libraryId,
            libraryName: title.libraryName ?? null,
            rootFolderId: title.rootFolderId ?? null,
            rootFolderPath: title.rootFolderPath ?? null,
          },
        ]}
        libraries={moveLibraries}
        initialRootId={null}
      />

      {onOpenFixMatch ? (
        <FixTitleMatchSettingsCard
          facet={title.facet}
          idPrefix="series-overview-settings"
          onOpen={onOpenFixMatch}
        />
      ) : null}

      <ChangeTitleFolderCard
        title={{
          id: title.id,
          name: title.name,
          libraryId: title.libraryId,
          libraryName: title.libraryName ?? null,
          rootFolderId: title.rootFolderId ?? null,
          rootFolderPath: title.rootFolderPath ?? null,
        }}
        roots={rootFolders}
        idPrefix="series-overview-settings"
        onTitleChanged={onTitleChanged}
      />

      {renameEnabled ? (
        <div className={`${onOpenFixMatch ? "mt-3" : "mt-5"} rounded-lg border border-border/70 bg-muted/20 px-3 py-3`}>
          <div className="flex justify-end">
            <Button
              id="series-overview-rename-preview"
              data-ui="series-overview-rename-preview"
              type="button"
              variant="primary"
              size="sm"
              className="w-full shrink-0 justify-center gap-2 rounded-md border border-transparent !bg-primary px-3 font-semibold !text-primary-foreground shadow-sm hover:!bg-primary/90 focus-visible:ring-[var(--scry-accent-ring)] sm:w-auto"
              onClick={() => void handlePreviewRename()}
              disabled={renamePreviewing || renameApplying}
            >
              {renamePreviewing ? (
                <Loader2 className="h-4 w-4 animate-spin" />
              ) : (
                <Eye className="h-4 w-4" />
              )}
              {renamePreviewing ? t("rename.previewing") : t("rename.previewButton")}
            </Button>
          </div>

          {renamePlan ? (
            <MediaRenamePlanPanel
              plan={renamePlan}
              applying={renameApplying}
              applyDisabled={renameApplying || renamePreviewing || renamePlan.renamable === 0}
              applyButtonId="series-overview-rename-apply"
              onApply={() => void handleApplyRename()}
            />
          ) : null}
        </div>
      ) : null}
    </div>
  );
}
