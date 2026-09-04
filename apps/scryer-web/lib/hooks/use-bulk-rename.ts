import * as React from "react";
import type { Client } from "urql";
import { renameTitlesMutation } from "@/lib/graphql/mutations";
import {
  mediaRenamePreviewBulkQuery,
  mediaRenamePreviewQuery,
} from "@/lib/graphql/queries";
import type { MediaRenamePlan } from "@/components/common/media-rename-plan-panel";
import type { JobRun, TitleRecord } from "@/lib/types";
import { normalizeJobRun } from "@/lib/utils/job-runs";
import { selectedTitleIdsKey } from "@/lib/utils/title-selection";
import type { Translate } from "@/components/root/types";
import type { SetGlobalStatus } from "@/lib/context/global-status-context";

const BULK_RENAME_PREVIEW_CONCURRENCY = 4;
export const BULK_RENAME_ITEM_SAMPLE_LIMIT = 50;

type UseBulkRenameArgs = {
  selectedTitles: TitleRecord[];
  /// Whether the actor may manage titles in every selected title's library.
  canRenameSelectedTitles: boolean;
  bulkActionBusy: boolean;
  setBulkActionBusy: React.Dispatch<React.SetStateAction<boolean>>;
  client: Client;
  t: Translate;
  setGlobalStatus: SetGlobalStatus;
  recordCriticalCatalogMutation: () => void;
  registerInteractiveJobRun: (run: JobRun) => void;
  setSelectedTitleIds: React.Dispatch<React.SetStateAction<Set<string>>>;
  batchFailureDetail: (error: unknown) => string | null;
  withFailureDetail: (message: string, detail: string | null) => string;
};

export type BulkRenameSummary = {
  total: number;
  renamable: number;
  noop: number;
  conflicts: number;
  errors: number;
};

export function useBulkRename({
  selectedTitles,
  canRenameSelectedTitles,
  bulkActionBusy,
  setBulkActionBusy,
  client,
  t,
  setGlobalStatus,
  recordCriticalCatalogMutation,
  registerInteractiveJobRun,
  setSelectedTitleIds,
  batchFailureDetail,
  withFailureDetail,
}: UseBulkRenameArgs) {
  const [bulkRenameDialogOpen, setBulkRenameDialogOpen] = React.useState(false);
  const [bulkRenamePreviewLoading, setBulkRenamePreviewLoading] =
    React.useState(false);
  const [bulkRenamePreviewError, setBulkRenamePreviewError] = React.useState<
    string | null
  >(null);
  const [bulkRenamePlansByTitleId, setBulkRenamePlansByTitleId] =
    React.useState<Record<string, MediaRenamePlan>>({});

  const closeBulkRenameDialog = React.useCallback(() => {
    setBulkRenameDialogOpen(false);
    setBulkRenamePreviewLoading(false);
    setBulkRenamePreviewError(null);
    setBulkRenamePlansByTitleId({});
  }, []);

  // Catalog refreshes rebuild the titles array without changing the selection,
  // so the preview effect keys on the selected id set and reads the current
  // records through a ref. Depending on the array itself would clear the plans
  // and restart every facet preview on each background scan tick.
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
    if (!bulkRenameDialogOpen) {
      return;
    }

    const targets = [...selectedTitlesRef.current];
    if (targets.length === 0) {
      setBulkRenamePreviewLoading(false);
      setBulkRenamePreviewError(null);
      setBulkRenamePlansByTitleId({});
      return;
    }

    let cancelled = false;
    setBulkRenamePreviewLoading(true);
    setBulkRenamePreviewError(null);
    setBulkRenamePlansByTitleId({});

    const loadPreviews = async () => {
      const nextPlansByTitleId: Record<string, MediaRenamePlan> = {};
      const failedTitles: string[] = [];
      let firstFailureDetail: string | null = null;
      // The dialog only ever shows a sample, so each request asks for what is
      // still missing from it. Plan counts and the fingerprint describe every
      // file regardless of how few items come back.
      let sampledItems = 0;
      const remainingSample = () =>
        Math.max(0, BULK_RENAME_ITEM_SAMPLE_LIMIT - sampledItems);

      const recordPlan = (titleId: string, plan: MediaRenamePlan) => {
        sampledItems += plan.items.length;
        nextPlansByTitleId[titleId] = plan;
      };

      // One request per facet instead of one per title: the batch resolves the
      // rename settings once rather than re-reading them for every title.
      const previewFacet = async (facet: string, facetTitles: TitleRecord[]) => {
        const result = await client
          .query<{ mediaRenamePreviewBulk: MediaRenamePlan[] }>(
            mediaRenamePreviewBulkQuery,
            {
              input: {
                facet,
                titleIds: facetTitles.map((title) => title.id),
                renamableOnly: true,
                maxItems: remainingSample(),
              },
            },
            { requestPolicy: "network-only" },
          )
          .toPromise();
        if (result.error || !result.data?.mediaRenamePreviewBulk) {
          throw result.error ?? new Error("rename preview failed");
        }
        const plans = result.data.mediaRenamePreviewBulk;
        plans.forEach((plan, index) => {
          const titleId = plan.titleId ?? facetTitles[index]?.id;
          if (titleId) {
            recordPlan(titleId, plan);
          }
        });
      };

      // A batch fails as a whole, so fall back to per-title previews to keep a
      // single unreadable title from blanking the rest of the dialog.
      const previewTitlesIndividually = async (facetTitles: TitleRecord[]) => {
        const queue = [...facetTitles];
        const worker = async () => {
          for (;;) {
            const title = queue.shift();
            if (!title || cancelled) {
              return;
            }
            try {
              const result = await client
                .query<{ mediaRenamePreview: MediaRenamePlan }>(
                  mediaRenamePreviewQuery,
                  {
                    input: {
                      facet: title.facet,
                      titleId: title.id,
                      dryRun: true,
                      renamableOnly: true,
                      maxItems: remainingSample(),
                    },
                  },
                  { requestPolicy: "network-only" },
                )
                .toPromise();
              if (result.error || !result.data?.mediaRenamePreview) {
                throw result.error ?? new Error("rename preview failed");
              }
              recordPlan(title.id, result.data.mediaRenamePreview);
            } catch (error) {
              failedTitles.push(title.name || title.id);
              firstFailureDetail ??= batchFailureDetail(error);
            }
          }
        };
        await Promise.all(
          Array.from(
            {
              length: Math.min(
                BULK_RENAME_PREVIEW_CONCURRENCY,
                facetTitles.length,
              ),
            },
            worker,
          ),
        );
      };

      const titlesByFacet = new Map<string, TitleRecord[]>();
      for (const title of targets) {
        const bucket = titlesByFacet.get(title.facet);
        if (bucket) {
          bucket.push(title);
        } else {
          titlesByFacet.set(title.facet, [title]);
        }
      }

      for (const [facet, facetTitles] of titlesByFacet) {
        if (cancelled) {
          return;
        }
        try {
          await previewFacet(facet, facetTitles);
        } catch (error) {
          // Falling back silently would hide a backend that never serves the
          // batch, leaving the dialog quietly back on one request per title.
          console.warn(
            "[bulk-rename] batched preview failed; falling back to per-title previews",
            error,
          );
          await previewTitlesIndividually(facetTitles);
        }
      }
      if (cancelled) {
        return;
      }

      setBulkRenamePlansByTitleId(nextPlansByTitleId);
      if (failedTitles.length > 0) {
        setBulkRenamePreviewError(
          withFailureDetail(
            t("status.bulkRenamePreviewFailed", {
              failed: failedTitles.length,
            }),
            failedTitles.slice(0, 5).join(", ") || firstFailureDetail,
          ),
        );
      } else {
        setBulkRenamePreviewError(null);
      }
      setBulkRenamePreviewLoading(false);
    };

    void loadPreviews();
    return () => {
      cancelled = true;
    };
  }, [
    batchFailureDetail,
    bulkRenameDialogOpen,
    client,
    selectedTitlesKey,
    t,
    withFailureDetail,
  ]);

  const bulkRenameSummary = React.useMemo<BulkRenameSummary | null>(() => {
    const plans = selectedTitles
      .map((title) => bulkRenamePlansByTitleId[title.id])
      .filter(Boolean);
    if (plans.length === 0) {
      return null;
    }
    return plans.reduce<BulkRenameSummary>(
      (summary, plan) => ({
        total: summary.total + plan.total,
        renamable: summary.renamable + plan.renamable,
        noop: summary.noop + plan.noop,
        conflicts: summary.conflicts + plan.conflicts,
        errors: summary.errors + plan.errors,
      }),
      { total: 0, renamable: 0, noop: 0, conflicts: 0, errors: 0 },
    );
  }, [bulkRenamePlansByTitleId, selectedTitles]);

  const bulkRenameConfirmDisabled =
    bulkActionBusy ||
    selectedTitles.length === 0 ||
    bulkRenamePreviewLoading ||
    !bulkRenameSummary ||
    bulkRenameSummary.renamable === 0;

  const confirmBulkRenameTitles = React.useCallback(async () => {
    const targets = selectedTitles.filter((title) => {
      const plan = bulkRenamePlansByTitleId[title.id];
      return plan !== undefined && plan.renamable > 0;
    });
    if (targets.length === 0 || bulkActionBusy) {
      return;
    }

    setBulkActionBusy(true);
    try {
      recordCriticalCatalogMutation();
      // Renaming moves every file of every selected title, so the request only
      // starts the job. Each title stays locked against further renames until
      // the job reaches it, and progress arrives through the job run.
      const byFacet = new Map<string, TitleRecord[]>();
      for (const title of targets) {
        const bucket = byFacet.get(title.facet);
        if (bucket) {
          bucket.push(title);
        } else {
          byFacet.set(title.facet, [title]);
        }
      }

      let accepted = 0;
      let firstFailureDetail: string | null = null;
      for (const [facet, facetTitles] of byFacet) {
        try {
          const result = await client
            .mutation<{
              renameTitles: {
                acceptedTitleIds: string[];
                jobRun?: unknown;
              };
            }>(renameTitlesMutation, {
              input: { facet, titleIds: facetTitles.map((title) => title.id) },
            })
            .toPromise();
          if (result.error) {
            throw result.error;
          }
          accepted += result.data?.renameTitles.acceptedTitleIds.length ?? 0;
          const run = normalizeJobRun(result.data?.renameTitles.jobRun);
          if (run) {
            registerInteractiveJobRun(run);
          }
        } catch (error) {
          firstFailureDetail ??= batchFailureDetail(error);
        }
      }

      setSelectedTitleIds(new Set());
      closeBulkRenameDialog();

      if (accepted === 0) {
        setGlobalStatus(
          withFailureDetail(t("status.bulkRenameFailed"), firstFailureDetail),
        );
        return;
      }
      setGlobalStatus(
        withFailureDetail(
          t("status.bulkRenameQueued", { count: accepted }),
          firstFailureDetail,
        ),
      );
    } catch (error) {
      setGlobalStatus(
        withFailureDetail(
          t("status.bulkRenameFailed"),
          batchFailureDetail(error),
        ),
      );
    } finally {
      setBulkActionBusy(false);
    }
  }, [
    batchFailureDetail,
    bulkActionBusy,
    bulkRenamePlansByTitleId,
    client,
    closeBulkRenameDialog,
    recordCriticalCatalogMutation,
    registerInteractiveJobRun,
    selectedTitles,
    setBulkActionBusy,
    setGlobalStatus,
    setSelectedTitleIds,
    withFailureDetail,
    t,
  ]);

  const openBulkTitleRename = React.useCallback(() => {
    if (selectedTitles.length === 0 || bulkActionBusy) {
      return;
    }
    // The backend refuses too; this keeps the dialog from opening on a
    // selection the actor cannot rename.
    if (!canRenameSelectedTitles) {
      setGlobalStatus(t("status.bulkRenameForbidden"));
      return;
    }
    setBulkRenamePreviewLoading(false);
    setBulkRenamePreviewError(null);
    setBulkRenamePlansByTitleId({});
    setBulkRenameDialogOpen(true);
  }, [
    bulkActionBusy,
    canRenameSelectedTitles,
    selectedTitles.length,
    setGlobalStatus,
    t,
  ]);

  return {
    bulkRenameDialogOpen,
    setBulkRenameDialogOpen,
    bulkRenamePreviewLoading,
    setBulkRenamePreviewLoading,
    bulkRenamePreviewError,
    setBulkRenamePreviewError,
    bulkRenamePlansByTitleId,
    setBulkRenamePlansByTitleId,
    bulkRenameSummary,
    bulkRenameConfirmDisabled,
    closeBulkRenameDialog,
    confirmBulkRenameTitles,
    openBulkTitleRename,
  };
}
