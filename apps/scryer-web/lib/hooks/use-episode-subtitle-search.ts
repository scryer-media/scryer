import * as React from "react";
import { useClient } from "urql";
import { useJobRunToasts } from "@/components/root/job-run-provider";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { JOB_RUN_FIELDS } from "@/lib/graphql/queries";
import { normalizeJobRun } from "@/lib/utils/job-runs";
import {
  completedSubtitleBatchIds,
  subtitleBatchErrorMessage,
  submitEpisodeSubtitleBatch,
} from "@/lib/utils/episode-subtitle-search";

const mutation = `mutation StartEpisodeSubtitleSearch($input: EpisodeSubtitleSearchInput!) {
  startEpisodeSubtitleSearch(input: $input) { ${JOB_RUN_FIELDS} }
}`;

export function useEpisodeSubtitleSearch(
  onRefresh?: () => Promise<void> | void,
) {
  const client = useClient();
  const t = useTranslate();
  const setStatus = useGlobalStatus();
  const { registerInteractiveJobRun, runs } = useJobRunToasts();
  const [pending, setPending] = React.useState(false);
  const submitting = React.useRef(false);
  const refresh = React.useRef(onRefresh);
  React.useEffect(() => {
    refresh.current = onRefresh;
  }, [onRefresh]);
  const terminalIds = [...completedSubtitleBatchIds(runs)].sort().join(",");
  const previous = React.useRef(
    new Set(terminalIds ? terminalIds.split(",") : []),
  );
  React.useEffect(() => {
    const ids = new Set(terminalIds ? terminalIds.split(",") : []);
    const changed = [...ids].some((id) => !previous.current.has(id));
    previous.current = ids;
    if (changed) void refresh.current?.();
  }, [terminalIds]);

  const start = async (
    titleId: string,
    episodeIds: string[],
    clear: () => void,
  ) => {
    if (submitting.current || !episodeIds.length) return;
    submitting.current = true;
    setPending(true);
    try {
      await submitEpisodeSubtitleBatch(
        async () => {
          const result = await client
            .mutation(mutation, { input: { titleId, episodeIds } })
            .toPromise();
          if (result.error) throw result.error;
          const run = normalizeJobRun(result.data?.startEpisodeSubtitleSearch);
          if (!run) throw new Error(t("subtitle.batchFailed"));
          return run;
        },
        (run) => {
          registerInteractiveJobRun(run);
        },
        clear,
      );
      setStatus(t("subtitle.batchQueued"), { level: "SUCCESS" });
    } catch (error) {
      setStatus(subtitleBatchErrorMessage(error, t), { level: "ERROR" });
    } finally {
      submitting.current = false;
      setPending(false);
    }
  };
  return { pending, start };
}
