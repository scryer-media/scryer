import * as React from "react";
import { useClient } from "urql";

import {
  AssignEpisodeFileDialog,
  type AssignEpisodeFileOutcome,
} from "@/components/dialogs/assign-episode-file-dialog";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useTranslate } from "@/lib/context/translate-context";
import { userFacingGraphQlErrorMessage } from "@/lib/graphql/error-message";
import { reassignEpisodeFileMutation } from "@/lib/graphql/mutations";
import { titleEpisodeTargetsQuery } from "@/lib/graphql/queries";
import {
  EPISODE_FILE_REASSIGN_PRIMARY_CONFLICT,
  episodeFileReassignErrorMessage,
  episodeFileReassignRefusalReason,
} from "@/lib/utils/episode-file-reassign-error";
import type { EpisodeTargetEpisode } from "@/lib/utils/episode-target";

/** The file an operator is moving, and the episodes it holds now. */
export type EpisodeFileAssignmentTarget = {
  fileId: string;
  filePath: string;
  originalReleaseName?: string | null;
  episodeIds: string[];
};

/** What a completed reassignment changed. */
export type EpisodeFileReassignment = {
  fileId: string;
  previousEpisodeIds: string[];
  episodeIds: string[];
  demotedFileIds: string[];
  promotedFileIds: string[];
};

type TitleEpisodeTargetsData = {
  title?: {
    collections?: {
      episodes?: {
        id: string;
        seasonNumber: string | null;
        episodeNumber: string | null;
        absoluteNumber: string | null;
        title: string | null;
      }[] | null;
    }[] | null;
  } | null;
};

type ReassignEpisodeFileData = {
  reassignEpisodeFile?: {
    file: { id: string };
    previousEpisodeIds: string[];
    episodeIds: string[];
    demotedFileIds: string[];
    promotedFileIds: string[];
  } | null;
};

/**
 * The "Assign to episode" flow for a library file: it loads the title's
 * episodes, lets the operator pick, and reassigns the file. The server renames
 * the file for its new episodes in the same operation.
 */
export function useEpisodeFileAssignment({
  titleId,
  onAssigned,
}: {
  titleId: string | null;
  onAssigned: (change: EpisodeFileReassignment) => Promise<void> | void;
}) {
  const client = useClient();
  const t = useTranslate();
  const setGlobalStatus = useGlobalStatus();
  const [target, setTarget] = React.useState<EpisodeFileAssignmentTarget | null>(null);
  const [episodes, setEpisodes] = React.useState<EpisodeTargetEpisode[] | null>(null);
  const [loadError, setLoadError] = React.useState<string | null>(null);
  const requestRef = React.useRef(0);

  const openAssignment = React.useCallback(
    (next: EpisodeFileAssignmentTarget) => {
      if (!titleId) return;
      const request = ++requestRef.current;
      setTarget(next);
      setEpisodes(null);
      setLoadError(null);
      void client
        .query<TitleEpisodeTargetsData>(
          titleEpisodeTargetsQuery,
          { id: titleId },
          { requestPolicy: "network-only" },
        )
        .toPromise()
        .then(({ data, error }) => {
          if (error) throw error;
          if (requestRef.current !== request) return;
          setEpisodes(
            (data?.title?.collections ?? []).flatMap((collection) =>
              (collection.episodes ?? []).map((episode) => ({
                id: episode.id,
                seasonNumber: episode.seasonNumber,
                episodeNumber: episode.episodeNumber,
                absoluteNumber: episode.absoluteNumber,
                title: episode.title,
              })),
            ),
          );
        })
        .catch((error: unknown) => {
          if (requestRef.current !== request) return;
          setLoadError(userFacingGraphQlErrorMessage(error, t("episodeAssign.loadFailed")));
        });
    },
    [client, t, titleId],
  );

  const handleOpenChange = React.useCallback((open: boolean) => {
    if (open) return;
    requestRef.current += 1;
    setTarget(null);
  }, []);

  const handleConfirm = React.useCallback(
    async (episodeIds: string[], replace: boolean): Promise<AssignEpisodeFileOutcome> => {
      if (!target) return { kind: "error", message: t("episodeAssign.failed") };
      const { data, error } = await client
        .mutation<ReassignEpisodeFileData>(reassignEpisodeFileMutation, {
          input: { fileId: target.fileId, episodeIds, replace },
        })
        .toPromise();
      if (error) {
        if (!replace && episodeFileReassignRefusalReason(error) === EPISODE_FILE_REASSIGN_PRIMARY_CONFLICT) {
          return { kind: "conflict" };
        }
        return {
          kind: "error",
          message: episodeFileReassignErrorMessage(error, t, t("episodeAssign.failed")),
        };
      }
      const result = data?.reassignEpisodeFile;
      if (!result) return { kind: "error", message: t("episodeAssign.failed") };
      setGlobalStatus(t("episodeAssign.success"), { level: "SUCCESS" });
      await onAssigned({
        fileId: result.file.id,
        previousEpisodeIds: result.previousEpisodeIds,
        episodeIds: result.episodeIds,
        demotedFileIds: result.demotedFileIds,
        promotedFileIds: result.promotedFileIds,
      });
      return { kind: "assigned" };
    },
    [client, onAssigned, setGlobalStatus, t, target],
  );

  const assignmentDialog = (
    <AssignEpisodeFileDialog
      open={target !== null}
      onOpenChange={handleOpenChange}
      idPrefix="episode-file-assign"
      fileName={target?.filePath ?? null}
      originalReleaseName={target?.originalReleaseName}
      episodes={episodes}
      loadError={loadError}
      initialEpisodeIds={target?.episodeIds ?? []}
      requireChange
      replaceSupported
      onConfirm={handleConfirm}
    />
  );

  return { openAssignment, assignmentDialog };
}
