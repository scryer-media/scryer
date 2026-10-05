import { useCallback, useState } from "react";
import { useMutation } from "urql";
import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useTranslate } from "@/lib/context/translate-context";
import { userFacingGraphQlErrorMessage } from "@/lib/graphql/error-message";
import { releaseHeldImportSourcesMutation } from "@/lib/graphql/mutations";
import type { DownloadQueueItem } from "@/lib/types";

export function canReleaseHeldSources(item: DownloadQueueItem): boolean {
  return Boolean(item.heldImportSources?.importId);
}

/**
 * Asks before releasing the sources a completed import is holding, naming the
 * download and what its client's removal policy will then do with it.
 */
export function useReleaseHeldSources(onReleased: (item: DownloadQueueItem) => Promise<void> | void) {
  const t = useTranslate();
  const setStatus = useGlobalStatus();
  const [, execute] = useMutation(releaseHeldImportSourcesMutation);
  const [target, setTarget] = useState<DownloadQueueItem | null>(null);
  const [busy, setBusy] = useState(false);
  const request = useCallback((item: DownloadQueueItem) => {
    if (canReleaseHeldSources(item)) setTarget(item);
  }, []);
  const held = target?.heldImportSources ?? null;
  const client = target ? target.clientName || target.clientType : "";

  const dialog = (
    <ConfirmDialog
      open={target !== null && held !== null}
      title={t("queue.releaseHeldSourcesConfirmTitle")}
      description={t("queue.releaseHeldSourcesConfirmDescription", {
        download: target?.titleName || target?.downloadClientItemId || "",
      })}
      confirmLabel={t("queue.releaseHeldSources")}
      cancelLabel={t("label.cancel")}
      isBusy={busy}
      onCancel={() => setTarget(null)}
      onConfirm={async () => {
        if (!target || !held) return;
        setBusy(true);
        try {
          const result = await execute({ input: { importId: held.importId } });
          if (result.error) {
            setStatus(userFacingGraphQlErrorMessage(result.error, t("queue.releaseHeldSourcesFailed")), { level: "ERROR" });
            return;
          }
          setStatus(t("queue.releaseHeldSourcesSuccess"));
          setTarget(null);
          await onReleased(target);
        } catch (error: unknown) {
          setStatus(userFacingGraphQlErrorMessage(error, t("queue.releaseHeldSourcesFailed")), { level: "ERROR" });
        } finally {
          setBusy(false);
        }
      }}
    >
      {held ? (
        <p className="text-xs text-muted-foreground">
          {t(held.clientRemovesDownload ? "queue.releaseHeldSourcesClientRemoves" : "queue.releaseHeldSourcesClientKeeps", { client })}
        </p>
      ) : null}
    </ConfirmDialog>
  );

  return { request, dialog };
}
