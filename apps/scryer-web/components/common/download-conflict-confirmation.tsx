import * as React from "react";
import { ConfirmDialog } from "@/components/common/confirm-dialog";
import type { DownloadConflictLike } from "@/lib/utils/download-conflicts";

type PendingConfirmation = {
  conflict: DownloadConflictLike;
  message: string;
};

export function useDownloadConflictConfirmation() {
  const [pending, setPending] = React.useState<PendingConfirmation | null>(
    null,
  );
  const resolverRef = React.useRef<((confirmed: boolean) => void) | null>(null);

  const close = React.useCallback((confirmed: boolean) => {
    resolverRef.current?.(confirmed);
    resolverRef.current = null;
    setPending(null);
  }, []);

  const confirmReplaceConflict = React.useCallback(
    (conflict: DownloadConflictLike, message: string) =>
      new Promise<boolean>((resolve) => {
        resolverRef.current = resolve;
        setPending({ conflict, message });
      }),
    [],
  );

  const current = pending?.conflict.sourceTitle || pending?.conflict.titleName;
  const state = pending?.conflict.state;
  const dialog = (
    <ConfirmDialog
      open={pending !== null}
      title="Replace in-progress download?"
      description={pending?.message ?? ""}
      confirmLabel="Replace download"
      cancelLabel="Keep current"
      onConfirm={() => close(true)}
      onCancel={() => close(false)}
    >
      <div className="space-y-1 text-xs text-muted-foreground">
        {current ? <p>Current download: {current}</p> : null}
        {state ? <p>State: {state}</p> : null}
      </div>
    </ConfirmDialog>
  );

  return { confirmReplaceConflict, replaceConflictDialog: dialog };
}
