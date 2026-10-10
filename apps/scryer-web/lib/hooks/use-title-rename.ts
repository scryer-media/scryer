import * as React from "react";
import { useClient } from "urql";

import type { MediaRenamePlan } from "@/components/common/media-rename-plan-panel";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useTranslate } from "@/lib/context/translate-context";
import { renameTitlesMutation } from "@/lib/graphql/mutations";
import { mediaRenamePreviewQuery } from "@/lib/graphql/queries";
import { useTrackedJobRuns } from "@/lib/hooks/use-tracked-job-runs";
import { normalizeJobRun } from "@/lib/utils/job-runs";

export type TitleRenameController = {
  /** False when the viewer may not rename this title or renaming is off. */
  available: boolean;
  plan: MediaRenamePlan | null;
  previewing: boolean;
  applying: boolean;
  preview: () => Promise<void>;
  apply: () => Promise<void>;
  cancel: () => void;
};

/**
 * Preview and apply a rename of one title's files. Both title overviews use
 * this, so they gate, report and follow a rename the same way: the rename runs
 * as a job, and the title is refreshed once that job has finished.
 */
export function useTitleRename<
  TTitle extends { id: string; facet: string; renameEnabled?: boolean },
>({
  title,
  canManageTitle,
  onBeforeApply,
  onApplied,
}: {
  title: TTitle | null;
  canManageTitle: boolean;
  /** Runs just before the rename request is sent. */
  onBeforeApply?: () => void;
  /**
   * Refreshes the renamed title once the rename has run. It receives that
   * title, since the viewer may have moved on by then.
   */
  onApplied?: (title: TTitle) => Promise<void> | void;
}): TitleRenameController {
  const client = useClient();
  const t = useTranslate();
  const setGlobalStatus = useGlobalStatus();
  const trackJobRun = useTrackedJobRuns();
  const titleKey = title ? `${title.facet}:${title.id}` : null;
  // A title whose rename setting has not loaded yet offers no rename: the
  // server refuses one while renaming is off, so the button waits for the
  // answer rather than appearing and failing.
  const available =
    title !== null && canManageTitle && title.renameEnabled === true;
  const [planState, setPlanState] = React.useState<{
    titleKey: string;
    plan: MediaRenamePlan;
  } | null>(null);
  const [previewingKey, setPreviewingKey] = React.useState<string | null>(null);
  const [applyingKey, setApplyingKey] = React.useState<string | null>(null);
  const plan =
    available && planState !== null && planState.titleKey === titleKey
      ? planState.plan
      : null;
  const onAppliedRef = React.useRef(onApplied);
  const onBeforeApplyRef = React.useRef(onBeforeApply);
  React.useEffect(() => {
    onAppliedRef.current = onApplied;
    onBeforeApplyRef.current = onBeforeApply;
  });

  const preview = React.useCallback(async () => {
    if (!title || !available || !titleKey) {
      return;
    }
    const requestedKey = titleKey;
    setPreviewingKey(requestedKey);
    try {
      const { data, error } = await client
        .query<{ mediaRenamePreview: MediaRenamePlan }>(mediaRenamePreviewQuery, {
          input: { facet: title.facet, titleId: title.id, dryRun: true },
        })
        .toPromise();
      if (error) {
        throw error;
      }
      const nextPlan = data?.mediaRenamePreview ?? null;
      if (!nextPlan) {
        throw new Error(t("status.apiError"));
      }
      setPlanState({ titleKey: requestedKey, plan: nextPlan });
      setGlobalStatus(
        t("status.renamePreviewGenerated", {
          total: nextPlan.total,
          renamable: nextPlan.renamable,
        }),
        { level: "SUCCESS" },
      );
    } catch (error: unknown) {
      setPlanState((current) =>
        current?.titleKey === requestedKey ? null : current,
      );
      setGlobalStatus(error instanceof Error ? error.message : t("status.apiError"), { level: "ERROR" });
    } finally {
      setPreviewingKey((current) => (current === requestedKey ? null : current));
    }
  }, [available, client, setGlobalStatus, t, title, titleKey]);

  const apply = React.useCallback(async () => {
    if (!title || !plan || !titleKey) {
      return;
    }
    const requestedKey = titleKey;
    const renamedTitle = title;
    setApplyingKey(requestedKey);
    try {
      onBeforeApplyRef.current?.();
      // One title can be a thousand files, so this starts a job and the title
      // stays locked until the job is done with it.
      const { data, error } = await client
        .mutation<{
          renameTitles?: { acceptedTitleIds?: string[]; jobRun?: unknown };
        }>(renameTitlesMutation, {
          input: { facet: renamedTitle.facet, titleIds: [renamedTitle.id] },
        })
        .toPromise();
      if (error) {
        throw error;
      }
      if ((data?.renameTitles?.acceptedTitleIds?.length ?? 0) === 0) {
        throw new Error(t("status.bulkRenameFailed"));
      }
      // The rename is queued either way; a failed reload only leaves the page
      // showing the old paths until its next refresh.
      const refreshRenamedTitle = () => {
        void Promise.resolve()
          .then(() => onAppliedRef.current?.(renamedTitle))
          .catch((refreshError: unknown) => {
            console.error("[title-rename] refresh after rename failed:", refreshError);
          });
      };
      const run = normalizeJobRun(data?.renameTitles?.jobRun);
      if (run) {
        trackJobRun(run, refreshRenamedTitle);
      } else {
        refreshRenamedTitle();
      }
      setGlobalStatus(t("status.renameQueued"));
      setPlanState((current) =>
        current?.titleKey === requestedKey ? null : current,
      );
    } catch (error: unknown) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.apiError"), { level: "ERROR" });
    } finally {
      setApplyingKey((current) => (current === requestedKey ? null : current));
    }
  }, [client, plan, setGlobalStatus, t, title, titleKey, trackJobRun]);

  const cancel = React.useCallback(() => {
    setPlanState(null);
  }, []);

  return {
    available,
    plan,
    previewing: titleKey !== null && previewingKey === titleKey,
    applying: titleKey !== null && applyingKey === titleKey,
    preview,
    apply,
    cancel,
  };
}
