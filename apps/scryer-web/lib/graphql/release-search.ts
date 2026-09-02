// Hotfix 0.17.1: client loop for the server-side interactive release-search
// job. Starts the job, polls its snapshot every second, and reports each
// snapshot through `onUpdate` so results stream into the UI as individual
// indexers complete. The returned promise NEVER rejects: on abort or error it
// resolves with the releases accumulated so far, matching the resolve-with-
// array contract the existing one-shot call sites rely on.
import type { Client } from "urql";

import type { Release } from "@/lib/types";
import {
  cancelInteractiveReleaseSearchMutation,
  startInteractiveReleaseSearchMutation,
} from "./mutations";
import { interactiveReleaseSearchQuery } from "./queries";
import { isAbortError, makeAbortableFetch } from "./urql-client";

export type InteractiveSearchIndexerProgress = {
  indexerId: string;
  name: string;
  /** Routing priority of the indexer; 0 when routing states none. */
  priority: number;
  status: "PENDING" | "SEARCHING" | "COMPLETED" | "FAILED" | "SKIPPED";
  resultCount: number;
  /** Wall time of the indexer's own call, or null before it answered. */
  elapsedMs: number | null;
  failureReason: string | null;
};

export type InteractiveSearchProgress = {
  releases: Release[];
  indexers: InteractiveSearchIndexerProgress[];
  state: "RUNNING" | "COMPLETED" | "CANCELLED";
};

/** Search kinds a title-less query subject may take (spec 0002 D2). */
export type InteractiveSearchKind = "MOVIE" | "SERIES" | "ANIME" | "RAW";

/**
 * The job accepts exactly one subject: a catalog title (`titleId`, optionally
 * narrowed to a season/episode) or a raw operator query (`query` + `kind`).
 * `indexerIds` and `categories` restrict either subject.
 */
export type InteractiveReleaseSearchInput = {
  titleId?: string;
  seriesMovieLinkId?: string;
  season?: string;
  episode?: string;
  query?: string;
  kind?: InteractiveSearchKind;
  indexerIds?: string[];
  categories?: string[];
  limit?: number;
};

type InteractiveReleaseSearchJobPayload = {
  id: string;
  state: InteractiveSearchProgress["state"];
  results: Release[] | null;
  indexers: InteractiveSearchIndexerProgress[] | null;
};

const POLL_INTERVAL_MS = 1_000;
// Defensive client-side stop; the server enforces its own 55s job deadline.
const CLIENT_DEADLINE_MS = 90_000;

function waitForNextPoll(signal?: AbortSignal): Promise<void> {
  return new Promise((resolve) => {
    if (signal?.aborted) {
      resolve();
      return;
    }
    const finish = () => {
      window.clearTimeout(timer);
      signal?.removeEventListener("abort", finish);
      resolve();
    };
    const timer = window.setTimeout(finish, POLL_INTERVAL_MS);
    signal?.addEventListener("abort", finish);
  });
}

export async function runIterativeReleaseSearch(
  client: Client,
  input: InteractiveReleaseSearchInput,
  options: {
    signal?: AbortSignal;
    onUpdate?: (snapshot: InteractiveSearchProgress) => void;
  } = {},
): Promise<Release[]> {
  const { signal, onUpdate } = options;
  const fetchOptions = signal ? { fetch: makeAbortableFetch(signal) } : {};
  let releases: Release[] = [];
  let jobId: string | null = null;

  // Best-effort server-side cancel; intentionally NOT bound to the (already
  // aborted) signal so the request still leaves the browser.
  const cancelJob = () => {
    if (jobId === null) {
      return;
    }
    void client
      .mutation(cancelInteractiveReleaseSearchMutation, { id: jobId })
      .toPromise()
      .catch(() => {});
  };

  const applySnapshot = (job: InteractiveReleaseSearchJobPayload) => {
    releases = job.results ?? [];
    onUpdate?.({
      releases,
      indexers: job.indexers ?? [],
      state: job.state,
    });
  };

  // Start-phase failures REJECT: nothing has begun, there are no partials to
  // lose, and every call site's existing catch -> status-toast path relies on
  // the rejection to surface the error (a silently resolved [] would render as
  // "no releases found"). Only the polling phase below is resolve-never-reject.
  let job: InteractiveReleaseSearchJobPayload | undefined;
  try {
    const { data, error } = await client
      .mutation(startInteractiveReleaseSearchMutation, { input }, fetchOptions)
      .toPromise();
    if (error) throw error;
    job = data?.startInteractiveReleaseSearch as
      | InteractiveReleaseSearchJobPayload
      | undefined;
  } catch (startError) {
    if (signal?.aborted || isAbortError(startError)) {
      return releases;
    }
    throw startError;
  }
  if (!job) {
    return releases;
  }
  jobId = job.id;
  applySnapshot(job);
  if (signal?.aborted) {
    cancelJob();
    return releases;
  }
  if (job.state !== "RUNNING") {
    return releases;
  }

  try {
    const deadline = Date.now() + CLIENT_DEADLINE_MS;
    while (Date.now() < deadline) {
      await waitForNextPoll(signal);
      if (signal?.aborted) {
        cancelJob();
        return releases;
      }
      try {
        const { data: pollData, error: pollError } = await client
          .query(
            interactiveReleaseSearchQuery,
            { id: jobId },
            { requestPolicy: "network-only", ...fetchOptions },
          )
          .toPromise();
        if (signal?.aborted) {
          cancelJob();
          return releases;
        }
        if (pollError) throw pollError;
        const snapshot = (pollData?.interactiveReleaseSearch ?? null) as
          | InteractiveReleaseSearchJobPayload
          | null;
        if (!snapshot) {
          // Unknown/evicted job — treat as finished with what we have.
          return releases;
        }
        applySnapshot(snapshot);
        if (snapshot.state !== "RUNNING") {
          return releases;
        }
      } catch (pollFailure) {
        if (signal?.aborted || isAbortError(pollFailure)) {
          cancelJob();
          return releases;
        }
        // Transient poll failure — keep polling until the deadline.
      }
    }
    return releases;
  } catch (error) {
    if (signal?.aborted || isAbortError(error)) {
      cancelJob();
    }
    return releases;
  }
}
