import type { Translate } from "@/components/root/types";
import { graphQlErrorExtension, userFacingGraphQlErrorMessage } from "../graphql/error-message.ts";

export const DELETE_REFUSAL_KEYS: Readonly<Record<string, string>> = {
  DELETE_PREVIEW_STALE: "titleDelete.refusal.preview",
  DELETE_TYPED_CONFIRMATION_REQUIRED: "titleDelete.refusal.confirmation",
  DELETE_ROOT_UNAVAILABLE: "titleDelete.refusal.unavailable",
  DELETE_PATH_OUTSIDE_ROOTS: "titleDelete.refusal.outsideRoots",
};

export function deleteErrorMessage(error: unknown, t: Translate, fallback: string): string {
  const reason = graphQlErrorExtension(error, "VALIDATION_ERROR", "reason");
  const key = reason && Object.hasOwn(DELETE_REFUSAL_KEYS, reason)
    ? DELETE_REFUSAL_KEYS[reason] : null;
  return key ? t(key) : userFacingGraphQlErrorMessage(error, fallback);
}
