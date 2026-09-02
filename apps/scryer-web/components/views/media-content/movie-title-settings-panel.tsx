import * as React from "react";
import { useClient } from "urql";

import { ChangeTitleFolderCard } from "@/components/common/change-title-folder-card";
import { FixTitleMatchSettingsCard } from "@/components/common/fix-title-match-settings-card";
import { MoveTitleSettingsCard } from "@/components/common/move-title-settings-card";
import { TitleOptionsSettingsGrid } from "@/components/common/title-options-settings-grid";
import { MoveTitlesDialog } from "@/components/dialogs/move-titles-dialog";
import { DEFAULT_MOVIE_LIBRARY_PATH } from "@/lib/constants/settings";
import { seriesOverviewSettingsInitQuery } from "@/lib/graphql/queries";
import type { TitleOptionUpdates } from "@/lib/types/title-options";
import type { LibraryRecord, TitleRecord } from "@/lib/types/titles";
import { qualityProfileSettingsToEntries } from "@/lib/utils/quality-profiles";

type MovieTitleSettingsPanelProps = {
  title: TitleRecord;
  libraries: LibraryRecord[];
  onUpdateTitleOptions: (options: TitleOptionUpdates) => Promise<void>;
  onTitleChanged: () => Promise<void> | void;
  onOpenFixMatch: () => void;
};

export function MovieTitleSettingsPanel({
  title,
  libraries,
  onUpdateTitleOptions,
  onTitleChanged,
  onOpenFixMatch,
}: MovieTitleSettingsPanelProps) {
  const client = useClient();
  const [qualityProfiles, setQualityProfiles] = React.useState<
    { id: string; name: string }[]
  >([]);
  const [defaultRootFolder, setDefaultRootFolder] = React.useState(
    DEFAULT_MOVIE_LIBRARY_PATH,
  );
  const library = React.useMemo(
    () => libraries.find((entry) => entry.id === title.libraryId) ?? null,
    [libraries, title.libraryId],
  );
  const rootFolders = React.useMemo(() => library?.roots ?? [], [library]);
  const libraryName = library?.name ?? null;
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
              name:
                libraryName?.trim() ||
                title.libraryName?.trim() ||
                title.libraryId,
              roots: rootFolders,
            },
          ],
    [libraries, libraryName, rootFolders, title.libraryId, title.libraryName],
  );

  React.useEffect(() => {
    let cancelled = false;
    const load = async () => {
      try {
        const { data, error } = await client
          .query(
            seriesOverviewSettingsInitQuery,
            { scope: "MOVIE" },
            { requestPolicy: "network-only" },
          )
          .toPromise();
        if (error) {
          throw error;
        }
        if (cancelled) {
          return;
        }
        setQualityProfiles(
          qualityProfileSettingsToEntries(data.qualityProfileSettings).map(
            (profile) => ({ id: profile.id, name: profile.name }),
          ),
        );
        const folder = (data.mediaSettings?.libraryPath ?? "").trim();
        if (folder) {
          setDefaultRootFolder(folder);
        }
      } catch {
        // Settings are optional here; other title overrides remain usable.
      }
    };
    void load();
    return () => {
      cancelled = true;
    };
  }, [client]);

  return (
    <div className="p-4">
      <TitleOptionsSettingsGrid
        title={title}
        qualityProfiles={qualityProfiles}
        defaultRootFolder={defaultRootFolder}
        rootFolders={rootFolders}
        onUpdateTitleOptions={onUpdateTitleOptions}
        onTitleChanged={onTitleChanged}
        idPrefix="title-overview-settings"
        currentLibraryName={libraryName ?? title.libraryName ?? null}
        rootFolderReadOnly
      />
      <MoveTitleSettingsCard
        idPrefix="title-overview-settings"
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
            libraryName: libraryName ?? title.libraryName ?? null,
            rootFolderId: title.rootFolderId ?? null,
            rootFolderPath: title.rootFolderPath ?? null,
          },
        ]}
        libraries={moveLibraries}
        initialRootId={null}
      />
      <FixTitleMatchSettingsCard
        facet={title.facet}
        idPrefix="title-overview-settings"
        onOpen={onOpenFixMatch}
      />
      <ChangeTitleFolderCard
        title={{
          id: title.id,
          name: title.name,
          libraryId: title.libraryId,
          libraryName: libraryName ?? title.libraryName ?? null,
          rootFolderId: title.rootFolderId ?? null,
          rootFolderPath: title.rootFolderPath ?? null,
        }}
        roots={rootFolders}
        idPrefix="title-overview-settings"
        onTitleChanged={onTitleChanged}
      />
    </div>
  );
}
