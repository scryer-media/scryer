import { hasGraphQlErrorReason } from "../graphql/error-message.ts";

/** Apply the interactive-session refusal even when keys were already loaded. */
export function applyApiKeyLoadRefusal(
  error: unknown,
  state: {
    setKeys: (keys: []) => void;
    setCanCreate: (allowed: boolean) => void;
    setLoaded: (loaded: boolean) => void;
    setStatus: (status: string | null) => void;
  },
): boolean {
  if (
    !hasGraphQlErrorReason(
      error,
      "INTERFACE_INTERACTIVE_SESSION_REQUIRED",
      "UNAUTHORIZED",
    )
  ) {
    return false;
  }
  state.setKeys([]);
  state.setCanCreate(false);
  state.setLoaded(true);
  state.setStatus(null);
  return true;
}
