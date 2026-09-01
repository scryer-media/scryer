import * as React from "react";
import { useClient } from "urql";

import { ChangeTitleFolderCard } from "@/components/common/change-title-folder-card";
import { FixTitleMatchSettingsCard } from "@/components/common/fix-title-match-settings-card";
import { TitleOptionsSettingsGrid } from "@/components/common/title-options-settings-grid";
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
