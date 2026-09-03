import * as React from "react";
import type { Client } from "urql";
import { deleteTitlesMutation } from "@/lib/graphql/mutations";
import { deleteTitlesPreviewQuery } from "@/lib/graphql/queries";
import type { JobRun, TitleRecord } from "@/lib/types";
import type { DeletePreview, DeleteTitlesPreview } from "@/lib/types/delete-preview";
import { normalizeJobRun } from "@/lib/utils/job-runs";
import { selectedTitleIdsKey } from "@/lib/utils/title-selection";
import type { Translate } from "@/components/root/types";
import type { SetGlobalStatus } from "@/lib/context/global-status-context";

type UseBulkDeleteArgs = {
  selectedTitles: TitleRecord[];
  selectedTitleLibraryIds: string[];
  bulkActionBusy: boolean;
  setBulkActionBusy: React.Dispatch<React.SetStateAction<boolean>>;
  client: Client;
  t: Translate;
  setGlobalStatus: SetGlobalStatus;
  recordCriticalCatalogMutation: () => void;
  registerInteractiveJobRun: (run: JobRun) => void;
  scheduleDeletionJobFallbackChecks: () => void;
  setPendingDeletedTitleIds: React.Dispatch<React.SetStateAction<Set<string>>>;
  setSelectedTitleIds: React.Dispatch<React.SetStateAction<Set<string>>>;
  deletionJobIdsRef: React.MutableRefObject<Set<string>>;
  batchFailureDetail: (error: unknown) => string | null;
  withFailureDetail: (message: string, detail: string | null) => string;
  aggregateDeletePreviews: (previews: DeletePreview[]) => DeletePreview | null;
};

export function useBulkDelete({
  selectedTitles,
  selectedTitleLibraryIds,
  bulkActionBusy,
  setBulkActionBusy,
  client,
  t,
  setGlobalStatus,
  recordCriticalCatalogMutation,
  registerInteractiveJobRun,
  scheduleDeletionJobFallbackChecks,
  setPendingDeletedTitleIds,
  setSelectedTitleIds,
  deletionJobIdsRef,
  batchFailureDetail,
  withFailureDetail,
  aggregateDeletePreviews,
}: UseBulkDeleteArgs) {
  const [bulkDeleteDialogOpen, setBulkDeleteDialogOpen] = React.useState(false);
  const [bulkDeleteFilesOnDisk, setBulkDeleteFilesOnDisk] =
    React.useState(false);
  const [bulkDeleteTypedConfirmation, setBulkDeleteTypedConfirmation] =
    React.useState("");
  const [bulkDeletePreviewLoading, setBulkDeletePreviewLoading] =
    React.useState(false);
  const [bulkDeletePreviewError, setBulkDeletePreviewError] = React.useState<
    string | null
  >(null);
  const [bulkDeletePreviewsByTitleId, setBulkDeletePreviewsByTitleId] =
    React.useState<Record<string, DeletePreview>>({});

  const closeBulkDeleteDialog = React.useCallback(() => {
    setBulkDeleteDialogOpen(false);
    setBulkDeleteFilesOnDisk(false);
    setBulkDeleteTypedConfirmation("");
    setBulkDeletePreviewLoading(false);
    setBulkDeletePreviewError(null);
    setBulkDeletePreviewsByTitleId({});
  }, []);

  React.useEffect(() => {
    if (!bulkDeleteFilesOnDisk) {
      setBulkDeleteTypedConfirmation("");
      setBulkDeletePreviewLoading(false);
      setBulkDeletePreviewError(null);
      setBulkDeletePreviewsByTitleId({});
    }
  }, [bulkDeleteFilesOnDisk]);

  // Catalog refreshes rebuild the titles array without changing the selection,
  // so the preview effect keys on the selected id set and reads the current
  // records through a ref. Depending on the array itself would restart the
  // preview — blanking the summary and remounting the typed-confirmation
  // input — on every background scan tick.
  const selectedTitlesKey = React.useMemo(
    () => selectedTitleIdsKey(selectedTitles),
    [selectedTitles],
  );
  const selectedTitlesRef = React.useRef(selectedTitles);
  // Declared before the preview effect so the ref is already current when the
  // preview effect below runs for a changed id set.
  React.useEffect(() => {
    selectedTitlesRef.current = selectedTitles;
  }, [selectedTitles]);

  React.useEffect(() => {
    if (!bulkDeleteDialogOpen || !bulkDeleteFilesOnDisk) {
      return;
    }

    const targets = [...selectedTitlesRef.current];
    if (targets.length === 0) {
      setBulkDeletePreviewLoading(false);
      setBulkDeletePreviewError(null);
      setBulkDeletePreviewsByTitleId({});
      return;
    }

    let cancelled = false;
    setBulkDeletePreviewLoading(true);
    setBulkDeletePreviewError(null);

    const loadPreviews = async () => {
      try {
        const result = await client
          .query<{ deleteTitlesPreview: DeleteTitlesPreview }>(
            deleteTitlesPreviewQuery,
            { input: { titleIds: targets.map((title) => title.id) } },
            { requestPolicy: "network-only" },
          )
          .toPromise();
        if (cancelled) {
          return;
        }

        if (result.error || !result.data?.deleteTitlesPreview) {
          throw result.error ?? new Error("delete title preview failed");
        }
        const payload = result.data.deleteTitlesPreview;
        const nextPreviewsByTitleId: Record<string, DeletePreview> = {};
        const failedTitles: string[] = [];

        for (const item of payload.items) {
          if (item.preview) {
            nextPreviewsByTitleId[item.titleId] = item.preview;
          } else {
            const title = targets.find((target) => target.id === item.titleId);
            failedTitles.push(title?.name ?? item.titleId);
          }
        }

        setBulkDeletePreviewsByTitleId(nextPreviewsByTitleId);
        if (payload.failedCount > 0) {
          setBulkDeletePreviewError(
            withFailureDetail(
              t("status.bulkDeletePreviewFailed", { failed: payload.failedCount }),
              failedTitles.slice(0, 5).join(", "),
            ),
          );
        } else {
          setBulkDeletePreviewError(null);
        }
      } catch (error) {
        if (cancelled) {
          return;
        }
        setBulkDeletePreviewsByTitleId({});
        setBulkDeletePreviewError(
          withFailureDetail(
            t("status.bulkDeletePreviewFailed", { failed: targets.length }),
            batchFailureDetail(error),
          ),
        );
      } finally {
        if (!cancelled) {
          setBulkDeletePreviewLoading(false);
        }
      }
    };

    void loadPreviews();
    return () => {
      cancelled = true;
    };
  }, [
    batchFailureDetail,
    bulkDeleteDialogOpen,
    bulkDeleteFilesOnDisk,
    client,
    selectedTitlesKey,
    t,
    withFailureDetail,
  ]);

  const bulkDeletePreview = React.useMemo(
    () =>
      aggregateDeletePreviews(
        Object.values(bulkDeletePreviewsByTitleId).filter(Boolean),
      ),
    [aggregateDeletePreviews, bulkDeletePreviewsByTitleId],
  );
  const bulkDeletePreviewMissing =
    bulkDeleteFilesOnDisk &&
    selectedTitles.some((title) => !bulkDeletePreviewsByTitleId[title.id]);
  const bulkDeleteConfirmDisabled =
    bulkActionBusy ||
    selectedTitles.length === 0 ||
    (bulkDeleteFilesOnDisk &&
      (bulkDeletePreviewLoading ||
        !!bulkDeletePreviewError ||
        bulkDeletePreviewMissing ||
        !bulkDeletePreview ||
        (bulkDeletePreview.requiresTypedConfirmation &&
          bulkDeleteTypedConfirmation.trim() !== "DELETE")));

  const confirmBulkDeleteTitles = React.useCallback(async () => {
    const targets = [...selectedTitles];
    if (targets.length === 0 || bulkActionBusy) {
      return;
    }

    setBulkActionBusy(true);
    try {
      const items = targets.map((title) => {
        const preview = bulkDeletePreviewsByTitleId[title.id];
        if (bulkDeleteFilesOnDisk && !preview) {
          throw new Error("Delete preview is not ready yet.");
        }
        return {
          titleId: title.id,
          ...(bulkDeleteFilesOnDisk
            ? { previewFingerprint: preview?.fingerprint }
            : {}),
        };
      });
      const result = await client
        .mutation<{
          deleteTitles?: {
            acceptedTitleIds?: string[];
            jobRun?: unknown;
          };
        }>(deleteTitlesMutation, {
          input: {
            items,
            deleteFilesOnDisk: bulkDeleteFilesOnDisk,
            ...(bulkDeleteFilesOnDisk && bulkDeleteTypedConfirmation.trim()
              ? { typedConfirmation: bulkDeleteTypedConfirmation.trim() }
              : {}),
          },
        })
        .toPromise();
      if (result.error) {
        throw result.error;
      }
      const acceptedIds = result.data?.deleteTitles?.acceptedTitleIds ?? [];
      if (acceptedIds.length > 0) {
        recordCriticalCatalogMutation();
      }
      const run = normalizeJobRun(result.data?.deleteTitles?.jobRun);
      if (run) {
        deletionJobIdsRef.current.add(run.id);
        registerInteractiveJobRun(run);
        scheduleDeletionJobFallbackChecks();
      }
      setPendingDeletedTitleIds((current) => {
        const next = new Set(current);
        for (const id of acceptedIds) {
          next.add(id);
        }
        return next;
      });
      setSelectedTitleIds(new Set());
      closeBulkDeleteDialog();
      setGlobalStatus(
        `Queued deletion for ${acceptedIds.length} title${acceptedIds.length === 1 ? "" : "s"}.`,
      );
    } catch (error) {
      setGlobalStatus(
        withFailureDetail(
          t("status.bulkTitleDeleteFailed"),
          batchFailureDetail(error),
        ),
      );
    } finally {
      setBulkActionBusy(false);
    }
  }, [
    batchFailureDetail,
    bulkActionBusy,
    bulkDeleteFilesOnDisk,
    bulkDeletePreviewsByTitleId,
    bulkDeleteTypedConfirmation,
    client,
    closeBulkDeleteDialog,
    deletionJobIdsRef,
    recordCriticalCatalogMutation,
    registerInteractiveJobRun,
    scheduleDeletionJobFallbackChecks,
    selectedTitles,
    setBulkActionBusy,
    setGlobalStatus,
    setPendingDeletedTitleIds,
    setSelectedTitleIds,
    t,
    withFailureDetail,
  ]);

  const openBulkTitleDelete = React.useCallback(() => {
    if (selectedTitles.length === 0 || bulkActionBusy) {
      return;
    }
    if (selectedTitleLibraryIds.length !== 1) {
      setGlobalStatus("Bulk actions require titles from one library.");
      return;
    }
    setBulkDeleteFilesOnDisk(false);
    setBulkDeleteTypedConfirmation("");
    setBulkDeletePreviewLoading(false);
    setBulkDeletePreviewError(null);
    setBulkDeletePreviewsByTitleId({});
    setBulkDeleteDialogOpen(true);
  }, [bulkActionBusy, selectedTitleLibraryIds.length, selectedTitles.length, setGlobalStatus]);

  return {
    bulkDeleteDialogOpen,
    setBulkDeleteDialogOpen,
    bulkDeleteFilesOnDisk,
    setBulkDeleteFilesOnDisk,
    bulkDeleteTypedConfirmation,
    setBulkDeleteTypedConfirmation,
    bulkDeletePreviewLoading,
    setBulkDeletePreviewLoading,
    bulkDeletePreviewError,
    setBulkDeletePreviewError,
    bulkDeletePreviewsByTitleId,
    setBulkDeletePreviewsByTitleId,
    closeBulkDeleteDialog,
    bulkDeletePreview,
    bulkDeleteConfirmDisabled,
    confirmBulkDeleteTitles,
    openBulkTitleDelete,
  };
}
