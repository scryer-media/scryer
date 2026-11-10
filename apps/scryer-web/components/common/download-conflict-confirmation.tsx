import { useTranslate } from "@/lib/context/translate-context";
import * as React from "react";
import { ConfirmDialog } from "@/components/common/confirm-dialog";
import type { DownloadConflictLike } from "@/lib/utils/download-conflicts";

type PendingConfirmation = {
  conflict: DownloadConflictLike;
  message: string;
};

export function useDownloadConflictConfirmation() {
  const t = useTranslate();
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
      title={t("uiAttr.Replace_in_progress_download")}
      description={pending?.message ?? ""}
      confirmLabel={t("uiAttr.Replace_download")}
      cancelLabel={t("uiAttr.Keep_current")}
      onConfirm={() => close(true)}
      onCancel={() => close(false)}
    >
      <div className="space-y-1 text-xs text-muted-foreground">
        {current ? (
          <p>
            {t("uiText.currentDownload")} {current}
          </p>
        ) : null}
        {state ? (
          <p>
            {t("uiText.state")} {state}
          </p>
        ) : null}
      </div>
    </ConfirmDialog>
  );

  return { confirmReplaceConflict, replaceConflictDialog: dialog };
}
