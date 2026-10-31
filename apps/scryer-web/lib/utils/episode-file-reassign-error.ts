import type { Translate } from "@/components/root/types";
import {
  graphQlErrorExtension,
  userFacingGraphQlErrorMessage,
} from "../graphql/error-message.ts";

export const EPISODE_FILE_REASSIGN_PRIMARY_CONFLICT =
  "EPISODE_FILE_REASSIGN_PRIMARY_CONFLICT";

export const EPISODE_FILE_REASSIGN_REFUSAL_KEYS: Readonly<
  Record<string, string>
> = {
  EPISODE_FILE_REASSIGN_NO_EPISODES: "episodeAssign.refusal.noEpisodes",
  EPISODE_FILE_REASSIGN_EPISODE_OUTSIDE_TITLE:
    "episodeAssign.refusal.outsideTitle",
  [EPISODE_FILE_REASSIGN_PRIMARY_CONFLICT]:
    "episodeAssign.refusal.primaryConflict",
  EPISODE_FILE_REASSIGN_NOT_EPISODIC_TITLE: "episodeAssign.refusal.notEpisodic",
  EPISODE_FILE_REASSIGN_UNSUPPORTED_FILE:
    "episodeAssign.refusal.unsupportedFile",
  EPISODE_FILE_REASSIGN_DESTINATION_OCCUPIED:
    "episodeAssign.refusal.destinationOccupied",
  EPISODE_FILE_REASSIGN_RENAME_REFUSED: "episodeAssign.refusal.renameRefused",
  EPISODE_FILE_REASSIGN_RENAME_FAILED: "episodeAssign.refusal.renameFailed",
};

/** The refusal reason of a failed reassignment, when the server gave one. */
export function episodeFileReassignRefusalReason(
  error: unknown,
): string | null {
  return graphQlErrorExtension(error, "VALIDATION_ERROR", "reason");
}

/**
 * The file that already holds a target episode, when the reassignment was
 * refused for that reason. The caller can then offer to replace it.
 */
export function episodeFileReassignConflictingFileId(
  error: unknown,
): string | null {
  if (
    episodeFileReassignRefusalReason(error) !==
    EPISODE_FILE_REASSIGN_PRIMARY_CONFLICT
  ) {
    return null;
  }
  return graphQlErrorExtension(error, "VALIDATION_ERROR", "conflictingFileId");
}

export function episodeFileReassignErrorMessage(
  error: unknown,
  t: Translate,
  fallback: string,
): string {
  const reason = episodeFileReassignRefusalReason(error);
  const key =
    reason && Object.hasOwn(EPISODE_FILE_REASSIGN_REFUSAL_KEYS, reason)
      ? EPISODE_FILE_REASSIGN_REFUSAL_KEYS[reason]
      : null;
  return key ? t(key) : userFacingGraphQlErrorMessage(error, fallback, t);
}
