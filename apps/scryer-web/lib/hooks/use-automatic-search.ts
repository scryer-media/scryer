import * as React from "react";
import { useClient } from "urql";
import { useJobRunToasts } from "@/components/root/job-run-provider";
import { triggerAcquisitionSearchMutation } from "@/lib/graphql/mutations";
import { normalizeJobRun } from "@/lib/utils/job-runs";
import { matchesActiveAutomaticSearch } from "@/lib/utils/automatic-search";
import { useTranslate } from "@/lib/context/translate-context";
import type { JobRun } from "@/lib/types";

export function useAutomaticSearch() {
  const client = useClient();
  const t = useTranslate();
  const { runs, registerInteractiveJobRun } = useJobRunToasts();
  const [pending, setPending] = React.useState<Record<string, boolean>>({});
  const startAutomaticSearch = React.useCallback(
    async (titleId: string, seasonNumber?: number) => {
      const key = `${titleId}:${seasonNumber ?? "all"}`;
      setPending((current) => ({ ...current, [key]: true }));
      try {
        const { data, error } = await client
          .mutation(triggerAcquisitionSearchMutation, {
            input: {
              intent: "AUTOMATIC",
              titleId,
              ...(seasonNumber === undefined ? {} : { seasonNumber }),
            },
          })
          .toPromise();
        if (error) throw error;
        const run = normalizeJobRun(data?.triggerAcquisitionSearch?.jobRun);
        if (!run) throw new Error(t("wanted.searchSnapshotMissing"));
        registerInteractiveJobRun(run);
      } finally {
        setPending((current) => {
          const next = { ...current };
          delete next[key];
          return next;
        });
      }
    },
    [client, registerInteractiveJobRun, t],
  );
  // Search one item (an `episode:` or `series_movie:` scope key) in the
  // background. The request returns as soon as the search is accepted; the
  // promise settles when the search finishes, reported over the job stream.
  // A search waits on its indexers, so it never holds a request open. The
  // wait is held by the job provider, which always settles it: with the run
  // once it ends, or with its last state when it can no longer be followed.
  const searchItem = React.useCallback(
    async (scopeKey: string): Promise<JobRun> => {
      const { data, error } = await client
        .mutation(triggerAcquisitionSearchMutation, {
          input: { wantedItemId: scopeKey },
        })
        .toPromise();
      if (error) throw error;
      const run = normalizeJobRun(data?.triggerAcquisitionSearch?.jobRun);
      if (!run) throw new Error(t("wanted.searchSnapshotMissing"));
      return new Promise<JobRun>((resolve) => {
        registerInteractiveJobRun(run, resolve);
      });
    },
    [client, registerInteractiveJobRun, t],
  );
  const isSearching = React.useCallback(
    (titleId: string, season?: number) =>
      pending[`${titleId}:${season ?? "all"}`] === true ||
      runs.some((run) => matchesActiveAutomaticSearch(run, titleId, season)),
    [pending, runs],
  );
  return { startAutomaticSearch, searchItem, isSearching };
}
