import * as React from "react";
import { useClient } from "urql";

import { useJobRunToasts } from "@/components/root/job-run-provider";
import { useTranslate } from "@/lib/context/translate-context";
import { startGrabMutation } from "@/lib/graphql/mutations";
import { grabJobResultQuery } from "@/lib/graphql/queries";
import type { JobRun } from "@/lib/types";
import { isTerminalJobRunStatus, normalizeJobRun } from "@/lib/utils/job-runs";

export type GrabConflict = {
  titleId: string;
  titleName: string;
  downloadClientId: string | null;
  downloadClientType: string;
  downloadClientItemId: string;
  sourceTitle: string | null;
  sourceKind: string | null;
  state: string | null;
  replaceable: boolean;
};

export type GrabQueuedOutcome = {
  status: string;
  jobId: string | null;
  titleId: string;
  titleName: string;
  sourceTitle: string | null;
  sourceKind: string | null;
  conflict: GrabConflict | null;
};

export type GrabStandbyOutcome = {
  grabbed: boolean;
  conflicts: GrabConflict[];
  cancelled: GrabConflict[];
  cancelFailed: GrabConflict[];
};

/** Exactly one field is set: the one matching the grab that was started. */
export type GrabOutcome = {
  queued: GrabQueuedOutcome | null;
  unlinked: {
    downloadId: string;
    clientName: string;
    sourceTitle: string;
  } | null;
  standby: GrabStandbyOutcome | null;
  pending: { id: string; grabbed: boolean } | null;
};

type StandbyChoice = "CHECK" | "QUEUE" | "CANCEL_AND_QUEUE";

/** The one grab to run; each choice takes what its own mutation takes. */
export type GrabRequest =
  | { existing: object }
  | { replacement: object }
  | {
      indexerSearchAssignment: {
        input: object;
        routing: object;
        replacement: boolean;
      };
    }
  | { unlinked: object }
  | {
      standby: {
        id: string;
        choice: StandbyChoice;
        confirmed: Array<{
          clientId: string | null;
          clientType: string;
          itemId: string;
        }>;
      };
    }
  | { pending: string };

/**
 * Grab a release in the background.
 *
 * A grab waits on the indexer and on the download client, so it never holds a
 * request open: the request returns as soon as the job is accepted, the job's
 * end arrives over the job stream, and its outcome is then read back. The
 * promise resolves with what the grab returned and rejects with the error it
 * raised, exactly as the direct grab mutations do.
 */
export function useGrab(): (request: GrabRequest) => Promise<GrabOutcome> {
  const client = useClient();
  const t = useTranslate();
  const { registerInteractiveJobRun } = useJobRunToasts();

  return React.useCallback(
    async (request: GrabRequest) => {
      const started = await client
        .mutation(startGrabMutation, { input: request })
        .toPromise();
      if (started.error) throw started.error;
      const run = normalizeJobRun(started.data?.startGrab?.jobRun);
      if (!run) throw new Error(t("status.queueFailed"));

      // Registered with the provider itself, not through the calling
      // component, so the wait outlives a page change and the grab still
      // reports how it ended. The provider always settles the wait: with the
      // run once it ends, or with its last state when it can no longer be
      // followed.
      const finished = await new Promise<JobRun>((resolve) => {
        registerInteractiveJobRun(run, resolve);
      });
      if (!isTerminalJobRunStatus(finished.status)) {
        throw new Error(t("status.grabOutcomeUnknown"));
      }
      const outcome = await client
        .query(
          grabJobResultQuery,
          { id: run.id },
          { requestPolicy: "network-only" },
        )
        .toPromise();
      if (outcome.error) throw outcome.error;
      const result = outcome.data?.grabJobResult as GrabOutcome | null;
      if (!result) {
        // The server no longer holds the outcome; the job run still says
        // whether the grab ended well.
        throw new Error(
          t(
            finished.status === "FAILED"
              ? "status.grabFailed"
              : "status.grabResultUnavailable",
          ),
        );
      }
      return result;
    },
    [client, registerInteractiveJobRun, t],
  );
}
