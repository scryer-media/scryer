import * as React from "react";
import { useClient } from "urql";
import { useJobRunToasts } from "@/components/root/job-run-provider";
import { triggerAcquisitionSearchMutation } from "@/lib/graphql/mutations";
import { normalizeJobRun } from "@/lib/utils/job-runs";
import { matchesActiveAutomaticSearch } from "@/lib/utils/automatic-search";
import { useTranslate } from "@/lib/context/translate-context";

export function useAutomaticSearch() {
  const client = useClient();
  const t = useTranslate();
  const { runs, registerInteractiveJobRun } = useJobRunToasts();
  const [pending, setPending] = React.useState<Record<string, boolean>>({});
  const startAutomaticSearch = React.useCallback(async (titleId: string, seasonNumber?: number) => {
    const key = `${titleId}:${seasonNumber ?? "all"}`;
    setPending((current) => ({ ...current, [key]: true }));
    try {
      const { data, error } = await client.mutation(triggerAcquisitionSearchMutation, {
        input: { intent: "AUTOMATIC", titleId, ...(seasonNumber === undefined ? {} : { seasonNumber }) },
      }).toPromise();
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
  }, [client, registerInteractiveJobRun, t]);
  const isSearching = React.useCallback((titleId: string, season?: number) =>
    pending[`${titleId}:${season ?? "all"}`] === true ||
    runs.some((run) => matchesActiveAutomaticSearch(run, titleId, season)), [pending, runs]);
  return { startAutomaticSearch, isSearching };
}
