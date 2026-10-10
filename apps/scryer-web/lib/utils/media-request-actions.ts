import type { MediaRequestRecord } from "@/lib/types/titles";

export type MediaRequestViewMode = "admin" | "mine";

export type MediaRequestRowActions = {
  /// Approve or dismiss a request waiting in the queue.
  resolve: boolean;
  /// Put a dismissed request back into the queue as pending.
  reopen: boolean;
  /// Modify or cancel the reader's own pending request.
  editOwn: boolean;
};

// The admin view only lists requests in libraries the reader manages, so the
// mode stands in for the permission check; the server enforces it again.
export function mediaRequestRowActions(
  mode: MediaRequestViewMode,
  status: MediaRequestRecord["status"],
): MediaRequestRowActions {
  return {
    resolve: mode === "admin" && status === "PENDING",
    reopen: mode === "admin" && status === "REJECTED",
    editOwn: mode === "mine" && status === "PENDING",
  };
}
