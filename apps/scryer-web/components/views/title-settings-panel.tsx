import * as React from "react";
import { useClient } from "urql";

import { ChangeTitleFolderCard } from "@/components/common/change-title-folder-card";
import { FixTitleMatchSettingsCard } from "@/components/common/fix-title-match-settings-card";
import {
  TitleOptionsSettingsGrid,
  type InlineTitleSettingsTitle,
} from "@/components/common/title-options-settings-grid";
import { MoveTitlesDialog } from "@/components/dialogs/move-titles-dialog";
import { useTitleSettingsOptions } from "@/lib/hooks/use-title-settings-options";
import { movieAvailabilityQuery } from "@/lib/graphql/queries";
import type { TitleOptionUpdates } from "@/lib/types/title-options";
import type { LibraryRecord, MovieAvailabilityRecord } from "@/lib/types/titles";

export type TitleSettingsPanelTitle = InlineTitleSettingsTitle & {
  name: string;
  libraryId: string;
  libraryName?: string | null;
  rootFolderId?: string | null;
  rootFolderPath?: string | null;
};

/**
 * One title's settings, shared by the movie panel and the series and anime
 * overview. It loads the choices it offers itself, so both overviews always
 * offer the same ones.
 */
export function TitleSettingsPanel({
  id,
  idPrefix,
  title,
  libraries,
  onUpdateTitleOptions,
  onTitleChanged,
  onOpenFixMatch,
  footerActions,
  footerContent,
  experimentalFeaturesEnabled = false,
}: {
  id?: string;
  /** Prefix for every control's id, so each overview keeps its own. */
  idPrefix: string;
  title: TitleSettingsPanelTitle;
  /**
   * Every library the move workflow may offer as a destination. The title's
   * own library also supplies its root folders; an empty list falls back to
   * the title's library alone.
   */
  libraries: LibraryRecord[];
  onUpdateTitleOptions: (options: TitleOptionUpdates) => Promise<void>;
  onTitleChanged?: () => Promise<void> | void;
  onOpenFixMatch?: () => void;
  /** Extra buttons beside the panel's own footer actions. */
  footerActions?: React.ReactNode;
  /** Content shown under the footer actions. */
  footerContent?: React.ReactNode;
  /**
   * Whether the move entry point is offered on this instance. The flag is read
   * once by the view that owns this panel and passed down, so the panel keeps
   * a module graph that server-renders.
   */
  experimentalFeaturesEnabled?: boolean;
}) {
  const client = useClient();
  const [movieAvailability, setMovieAvailability] = React.useState<MovieAvailabilityRecord | null>(
    title.movieAvailability ?? null,
  );
  React.useEffect(() => {
    let current = true;
    setMovieAvailability(title.movieAvailability ?? null);
    if (title.facet !== "MOVIE") {
      return () => {
        current = false;
      };
    }
    void client
      .query<{ title?: { movieAvailability?: MovieAvailabilityRecord | null } | null }>(
        movieAvailabilityQuery,
        { id: title.id },
        { requestPolicy: "network-only" },
      )
      .toPromise()
      .then(({ data }) => {
        if (current) setMovieAvailability(data?.title?.movieAvailability ?? null);
      });
    return () => {
      current = false;
    };
  }, [client, title.facet, title.id, title.minAvailability, title.movieAvailability]);

  const { qualityProfiles, defaultRootFolder } = useTitleSettingsOptions(
    title.facet,
  );
  const library = React.useMemo(
    () => libraries.find((entry) => entry.id === title.libraryId) ?? null,
    [libraries, title.libraryId],
  );
  const rootFolders = React.useMemo(() => library?.roots ?? [], [library]);
  const currentLibraryName =
    library?.name?.trim() || title.libraryName?.trim() || null;
  // The panel's one move entry point (FR-011): the action row opens the move
  // wizard, which asks whether this is a root move or a library transfer
  // before it asks where. No destination is pre-picked here.
  const [moveOpen, setMoveOpen] = React.useState(false);
  // Every library, not just the title's own: a destination in another library
  // is a cross-library transfer (FR-055/FR-056), and the move dialog owns the
  // rules for which destinations are pickable.
  const moveLibraries = React.useMemo(
    () =>
      libraries.length > 0
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
              name: currentLibraryName || title.libraryId,
              roots: rootFolders,
            },
          ],
    [
      currentLibraryName,
      libraries,
      rootFolders,
      title.libraryId,
      title.libraryName,
    ],
  );
  const movedTitle = {
    id: title.id,
    name: title.name,
    libraryId: title.libraryId,
    libraryName: currentLibraryName,
    rootFolderId: title.rootFolderId ?? null,
    rootFolderPath: title.rootFolderPath ?? null,
  };

  return (
    <div id={id} className="p-4">
      <TitleOptionsSettingsGrid
        title={{ ...title, movieAvailability }}
        qualityProfiles={qualityProfiles}
        defaultRootFolder={defaultRootFolder}
        rootFolders={rootFolders}
        onUpdateTitleOptions={onUpdateTitleOptions}
        onTitleChanged={onTitleChanged}
        idPrefix={idPrefix}
        currentLibraryName={currentLibraryName}
        rootFolderReadOnly
        onOpenMove={experimentalFeaturesEnabled ? () => setMoveOpen(true) : undefined}
        footer={
          <>
            <div className="flex flex-wrap items-center justify-end gap-2 px-3 py-3">
              {onOpenFixMatch ? (
                <FixTitleMatchSettingsCard
                  facet={title.facet}
                  idPrefix={idPrefix}
                  onOpen={onOpenFixMatch}
                  compact
                />
              ) : null}
              <ChangeTitleFolderCard
                title={movedTitle}
                roots={rootFolders}
                idPrefix={idPrefix}
                onTitleChanged={onTitleChanged}
                compact
              />
              {footerActions}
            </div>
            {footerContent ? <div className="px-3 pb-3">{footerContent}</div> : null}
          </>
        }
      />
      {experimentalFeaturesEnabled ? (
        <MoveTitlesDialog
          open={moveOpen}
          onOpenChange={setMoveOpen}
          titles={[movedTitle]}
          libraries={moveLibraries}
          initialRootId={null}
        />
      ) : null}
    </div>
  );
}
