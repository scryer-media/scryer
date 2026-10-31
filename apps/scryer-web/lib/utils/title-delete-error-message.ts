import type { Translate } from "@/components/root/types";
import {
  graphQlErrorExtension,
  userFacingGraphQlErrorMessage,
} from "../graphql/error-message.ts";

export const TITLE_DELETE_REFUSAL_KEYS: Readonly<Record<string, string>> = {
  TITLE_DELETE_EMPTY_REQUEST: "titleDelete.refusal.selection",
  TITLE_DELETE_DUPLICATE_TITLE: "titleDelete.refusal.selection",
  TITLE_DELETE_JOB_ALREADY_RUNNING: "titleDelete.refusal.busy",
  TITLE_DELETE_IN_PROGRESS: "titleDelete.refusal.busy",
  TITLE_DELETE_CONFIRMATION_REQUIRED: "titleDelete.refusal.preview",
  TITLE_DELETE_PREVIEW_STALE: "titleDelete.refusal.preview",
  TITLE_DELETE_TYPED_CONFIRMATION_REQUIRED: "titleDelete.refusal.confirmation",
  TITLE_DELETE_FOLDER_IS_ROOT: "titleDelete.refusal.boundary",
  TITLE_DELETE_FOLDER_OUTSIDE_ROOTS: "titleDelete.refusal.outsideRoots",
  TITLE_DELETE_FOLDER_OVERLAPS_TITLE: "titleDelete.refusal.ownership",
  TITLE_DELETE_FOREIGN_MEDIA: "titleDelete.refusal.ownership",
  TITLE_DELETE_FOLDER_CLAIM_LOST: "titleDelete.refusal.ownership",
  TITLE_DELETE_FOLDER_IS_SYMLINK: "titleDelete.refusal.layout",
  TITLE_DELETE_FOLDER_NOT_DIRECTORY: "titleDelete.refusal.layout",
  TITLE_DELETE_NO_VERIFIED_MEDIA: "titleDelete.refusal.layout",
  TITLE_DELETE_MIXED_LAYOUT: "titleDelete.refusal.layout",
  TITLE_DELETE_ROOT_UNAVAILABLE: "titleDelete.refusal.unavailable",
  TITLE_DELETE_RECYCLE_PURGE_INCOMPLETE: "titleDelete.refusal.recycle",
  TITLE_DELETE_RECYCLE_ENTRY_BUSY: "titleDelete.refusal.recycle",
  TITLE_DELETE_CATALOG_CLEANUP_FAILED: "titleDelete.refusal.retry",
  title_folder_unknown: "titleDelete.refusal.ownership",
};

export function titleDeleteErrorMessage(
  error: unknown,
  t: Translate,
  fallback: string,
): string {
  const reason =
    graphQlErrorExtension(error, "VALIDATION_ERROR", "reason") ??
    graphQlErrorExtension(error, "INTERNAL_ERROR", "reason");
  const key =
    reason && Object.hasOwn(TITLE_DELETE_REFUSAL_KEYS, reason)
      ? TITLE_DELETE_REFUSAL_KEYS[reason]
      : null;
  return key ? t(key) : userFacingGraphQlErrorMessage(error, fallback, t);
}
