import * as React from "react";
import { useClient } from "urql";

import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { DeletePreviewSummary } from "@/components/common/delete-preview-summary";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useTranslate } from "@/lib/context/translate-context";
import { userFacingGraphQlErrorMessage } from "@/lib/graphql/error-message";
import { deleteMediaFileMutation } from "@/lib/graphql/mutations";
import { deleteMediaFilePreviewQuery } from "@/lib/graphql/queries";
import { useDeletePreview } from "@/lib/hooks/use-delete-preview";
import { useTrackedJobRuns } from "@/lib/hooks/use-tracked-job-runs";
import type { JobRun } from "@/lib/types/jobs";
import { normalizeJobRun } from "@/lib/utils/job-runs";

export type DeletableMediaFile = {
  id: string;
  filePath?: string | null;
};

/**
 * Delete one media file from disk, confirmed against a preview. Both title
 * overviews use this, so the confirmation, the guard against deleting a file
 * twice, and the progress and outcome reports are the same on each. The file is
 * deleted by a job; the title is refreshed only once that job has finished.
 */
export function useMediaFileDeletion<TFile extends DeletableMediaFile, TContext>({
  captureContext,
  onQueued,
  onBeforeDelete,
  onFinished,
}: {
  /** Read whatever the finish handler needs before the file is gone. */
  captureContext: (file: TFile) => TContext;
  onQueued?: (file: TFile, context: TContext) => void;
  /** Runs just before the delete request is sent. */
  onBeforeDelete?: () => void;
  /** Refreshes the title once the deletion job has finished. */
  onFinished: (run: JobRun, file: TFile, context: TContext) => Promise<void> | void;
}): {
  deletingFileIds: ReadonlySet<string>;
  requestDelete: (file: TFile) => void;
  dialog: React.ReactNode;
} {
  const client = useClient();
  const t = useTranslate();
  const setGlobalStatus = useGlobalStatus();
  const trackJobRun = useTrackedJobRuns();
  const [target, setTarget] = React.useState<TFile | null>(null);
  const [typedConfirmation, setTypedConfirmation] = React.useState("");
  const [loading, setLoading] = React.useState(false);
  const [deletingFileIds, setDeletingFileIds] = React.useState<ReadonlySet<string>>(
    () => new Set(),
  );
  const previewVariables = React.useMemo(
    () => (target ? { fileId: target.id } : null),
    [target],
  );
  const {
    preview,
    loading: previewLoading,
    error: previewError,
  } = useDeletePreview(
    deleteMediaFilePreviewQuery,
    "deleteMediaFilePreview",
    previewVariables,
    target !== null,
  );
  const handlersRef = React.useRef({ captureContext, onQueued, onBeforeDelete, onFinished });
  React.useEffect(() => {
    handlersRef.current = { captureContext, onQueued, onBeforeDelete, onFinished };
  });

  const requestDelete = React.useCallback(
    (file: TFile) => {
      if (deletingFileIds.has(file.id)) {
        return;
      }
      setTarget(file);
      setTypedConfirmation("");
    },
    [deletingFileIds],
  );

  const cancel = React.useCallback(() => {
    if (loading) {
      return;
    }
    setTarget(null);
    setTypedConfirmation("");
  }, [loading]);

  const confirm = React.useCallback(async () => {
    if (!target || !preview) {
      return;
    }
    const file = target;
    const context = handlersRef.current.captureContext(file);
    setLoading(true);
    try {
      handlersRef.current.onBeforeDelete?.();
      const { data, error } = await client
        .mutation<{ deleteMediaFile?: { jobRun?: unknown } }>(deleteMediaFileMutation, {
          input: {
            fileId: file.id,
            deleteFromDisk: true,
            previewFingerprint: preview.fingerprint,
            typedConfirmation: typedConfirmation.trim() || undefined,
          },
        })
        .toPromise();
      if (error) {
        throw error;
      }
      const run = normalizeJobRun(data?.deleteMediaFile?.jobRun);
      if (!run) {
        throw new Error(t("status.apiError"));
      }
      setDeletingFileIds((current) => new Set(current).add(file.id));
      handlersRef.current.onQueued?.(file, context);
      trackJobRun(run, (terminalRun) => {
        setDeletingFileIds((current) => {
          const next = new Set(current);
          next.delete(file.id);
          return next;
        });
        void Promise.resolve()
          .then(() => handlersRef.current.onFinished(terminalRun, file, context))
          .catch((refreshError: unknown) => {
            console.error("[media-file-deletion] refresh after deletion failed:", refreshError);
          })
          .finally(() => {
            setGlobalStatus(
              terminalRun.status === "COMPLETED"
                ? t("status.mediaFileDeleted")
                : (terminalRun.errorText ??
                    terminalRun.summaryText ??
                    t("status.apiError")),
              { level: terminalRun.status === "COMPLETED" ? "SUCCESS" : "ERROR" },
            );
          });
      });
      setGlobalStatus(t("status.mediaFileDeleteQueued"), { level: "SUCCESS" });
      setTarget(null);
      setTypedConfirmation("");
    } catch (error: unknown) {
      setGlobalStatus(userFacingGraphQlErrorMessage(error, t("status.apiError")), { level: "ERROR" });
    } finally {
      setLoading(false);
    }
  }, [client, preview, setGlobalStatus, t, target, trackJobRun, typedConfirmation]);

  const confirmDisabled =
    previewLoading ||
    Boolean(previewError) ||
    !preview ||
    (preview.requiresTypedConfirmation && typedConfirmation.trim() !== "DELETE");

  const dialog = (
    <ConfirmDialog
      open={target !== null}
      title={t("mediaFile.delete")}
      description={target?.filePath ?? t("mediaFile.delete")}
      confirmLabel={t("label.delete")}
      cancelLabel={t("label.cancel")}
      isBusy={loading}
      confirmDisabled={confirmDisabled}
      onConfirm={confirm}
      onCancel={cancel}
    >
      <DeletePreviewSummary
        preview={preview}
        loading={previewLoading}
        error={previewError}
        typedConfirmation={typedConfirmation}
        onTypedConfirmationChange={setTypedConfirmation}
      />
    </ConfirmDialog>
  );

  return { deletingFileIds, requestDelete, dialog };
}
