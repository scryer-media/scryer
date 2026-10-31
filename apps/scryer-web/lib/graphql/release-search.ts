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
  continueInteractiveReleaseSearchMutation,
  startInteractiveReleaseSearchMutation,
} from "./mutations";
import { interactiveReleaseSearchQuery } from "./queries";
import type { InteractiveReleaseSearchInput } from "./release-search-input";
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
  /**
   * The indexer asked Scryer to slow down rather than failing. It resumes on
   * its own, so the row reads as a cooldown rather than an error.
   */
  rateLimited: boolean;
  /** Why a skipped indexer was not asked; null when it was not skipped. */
  skipReason?: InteractiveSearchSkipReason | null;
  /** When a skip that lifts on its own ends, if known. */
  skippedUntil?: string | null;
  /** How the capabilities used for this indexer were obtained; never changes its search outcome. */
  capsSource?: IndexerCapsSource | null;
  /** Stable reason code for a capability fallback; an open set, so unknown codes are expected. */
  capsReason?: string | null;
  /** Seconds the provider asked Scryer to wait before discovering capabilities again. */
  capsRetryAfterSeconds?: number | null;
};

/** How Scryer obtained the capabilities it used to search an indexer. */
export type IndexerCapsSource =
  | "FRESH"
  | "MANAGED_METADATA"
  | "REFRESHED"
  | "STALE"
  | "PERMISSIVE"
  | "PROVIDER_DECLARATIONS";

export type InteractiveSearchSkipReason =
  "INDEXER_DISABLED" | "TEMPORARILY_DISABLED" | "BACKED_OFF" | "NO_TEXT_SEARCH";

export type InteractiveSearchProgress = {
  /**
   * Job the snapshot came from. A grab names its release by
   * `(searchId, downloadUrl)`, and a retry mints a second job, so callers that
   * grab from a merged view must remember which job each row arrived on.
   */
  searchId: string;
  releases: Release[];
  indexers: InteractiveSearchIndexerProgress[];
  state: "RUNNING" | "COMPLETED" | "CANCELLED";
  startedAt?: string;
  completedAt?: string | null;
  /** Opaque cursor for the following page of a query search; null when there is none. */
  nextCursor?: string | null;
  /** The page was returned at its time bound while some indexers were still searching. */
  deadlineReached?: boolean;
  /** Server sentence for a failed query search, shown only for an unknown code. */
  failureReason?: string | null;
  /** Stable failure condition of a query search. */
  failureReasonCode?: string | null;
};

export {
  titleReleaseSearchInput,
  type InteractiveReleaseSearchInput,
  type TitleReleaseSearchScope,
} from "./release-search-input";
type InteractiveReleaseSearchJobPayload = {
  id: string;
  state: InteractiveSearchProgress["state"];
  results: Release[] | null;
  indexers: InteractiveSearchIndexerProgress[] | null;
  startedAt?: string;
  completedAt?: string | null;
  nextCursor?: string | null;
  deadlineReached?: boolean;
  failureReason?: string | null;
  failureReasonCode?: string | null;
};

const POLL_INTERVAL_MS = 1_000;
// Raw pages answer within 75 seconds; this leaves room for the final poll.
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
    /** `nextCursor` of the previous page; fetches the following page instead of starting a search. */
    after?: string;
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
      searchId: job.id,
      releases,
      indexers: job.indexers ?? [],
      state: job.state,
      startedAt: job.startedAt,
      completedAt: job.completedAt,
      nextCursor: job.nextCursor,
      deadlineReached: job.deadlineReached,
      failureReason: job.failureReason,
      failureReasonCode: job.failureReasonCode,
    });
  };

  // Start-phase failures REJECT: nothing has begun, there are no partials to
  // lose, and every call site's existing catch -> status-toast path relies on
  // the rejection to surface the error (a silently resolved [] would render as
  // "no releases found"). Only the polling phase below is resolve-never-reject.
  let job: InteractiveReleaseSearchJobPayload | undefined;
  try {
    const { data, error } = await client
      .mutation(
        options.after
          ? continueInteractiveReleaseSearchMutation
          : startInteractiveReleaseSearchMutation,
        options.after
          ? { after: options.after, first: input.aggregatePageSize }
          : { input },
        fetchOptions,
      )
      .toPromise();
    if (error) throw error;
    job = (
      options.after
        ? data?.continueInteractiveReleaseSearch
        : data?.startInteractiveReleaseSearch
    ) as InteractiveReleaseSearchJobPayload | undefined;
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
        const snapshot = (pollData?.interactiveReleaseSearch ??
          null) as InteractiveReleaseSearchJobPayload | null;
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
