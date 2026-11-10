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
import { boundedQuery, QueryTimeoutError } from "./bounded-query";

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
   * Stable code beside `failureReason` for a failure the operator must act
   * on, such as an indexer whose plugin is not loaded; an open set.
   */
  failureReasonCode?: string | null;
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
  /**
   * Releases a title search removed because no enabled download client
   * accepts them, one entry per missing client family. Not part of
   * `resultCount`.
   */
  unroutableResults?: InteractiveSearchUnroutableResults[] | null;
};

/** Releases one indexer returned that no enabled download client can take. */
export type InteractiveSearchUnroutableResults = {
  /** Stable reason code; an open set, so unknown codes are expected. */
  reason: string;
  count: number;
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
// Only a failure bound: reaching it cancels the server job and reports an
// error. It must exceed the server job deadline
// (INTERACTIVE_RELEASE_SEARCH_DEADLINE, which is INDEXER_HTTP_TIMEOUT, 120 s)
// plus room for the final poll, so a slow indexer ends the job on the server
// and its late results still arrive instead of the search failing.
const CLIENT_DEADLINE_MS = 130_000;

/** Why a search ended without a usable server session. */
export type InteractiveReleaseSearchErrorReason =
  /** The start or continue request returned no session. */
  | "NO_SESSION"
  /** The server no longer knows the session while it was being polled. */
  | "SESSION_LOST";

export class InteractiveReleaseSearchError extends Error {
  readonly reason: InteractiveReleaseSearchErrorReason;

  constructor(reason: InteractiveReleaseSearchErrorReason) {
    super(`Interactive release search failed: ${reason}`);
    this.name = "InteractiveReleaseSearchError";
    this.reason = reason;
  }
}

/**
 * True for a failure raised by the search lifecycle itself (a missing or lost
 * session, or the client deadline) rather than by the server. Such errors carry
 * no operator-facing text, so callers show their translated search failure.
 */
export function isInteractiveReleaseSearchLifecycleError(
  error: unknown,
): boolean {
  return (
    error instanceof InteractiveReleaseSearchError ||
    error instanceof QueryTimeoutError
  );
}

function waitForNextPoll(
  signal: AbortSignal,
  timeoutMs: number,
): Promise<void> {
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
    const timer = window.setTimeout(finish, timeoutMs);
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
    /** Polling failures retain partial results but must still be shown to the operator. */
    onError?: (error: unknown) => void;
  } = {},
): Promise<Release[]> {
  const { onUpdate, onError } = options;
  const requestController = new AbortController();
  const signal = options.signal
    ? AbortSignal.any([options.signal, requestController.signal])
    : requestController.signal;
  const fetchOptions = { fetch: makeAbortableFetch(signal) };
  const deadline = Date.now() + CLIENT_DEADLINE_MS;
  const timeRemaining = () => Math.max(1, deadline - Date.now());
  let releases: Release[] = [];
  let jobId: string | null = null;
  let lastJob: InteractiveReleaseSearchJobPayload | undefined;

  // Best-effort server-side cancel; intentionally NOT bound to the (already
  // aborted) signal so the request still leaves the browser.
  const cancelJob = () => {
    if (jobId === null) {
      return;
    }
    void boundedQuery(
      client.mutation(cancelInteractiveReleaseSearchMutation, { id: jobId }),
      new AbortController().signal,
    ).catch((error: unknown) => {
      console.warn("Interactive release search cancellation failed", error);
    });
  };

  const applySnapshot = (job: InteractiveReleaseSearchJobPayload) => {
    lastJob = job;
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
  const endSearch = (error?: unknown) => {
    if (lastJob?.state === "RUNNING") {
      applySnapshot({ ...lastJob, state: "CANCELLED" });
    }
    if (error) {
      if (onError) onError(error);
      else console.warn("Interactive release search could not complete", error);
    }
    return releases;
  };

  // Start-phase failures REJECT: nothing has begun, there are no partials to
  // lose, and every call site's existing catch -> status-toast path relies on
  // the rejection to surface the error (a silently resolved [] would render as
  // "no releases found"). Only the polling phase below is resolve-never-reject.
  try {
    let job: InteractiveReleaseSearchJobPayload | undefined;
    try {
      const { data, error } = await boundedQuery(
        client.mutation(
          options.after
            ? continueInteractiveReleaseSearchMutation
            : startInteractiveReleaseSearchMutation,
          options.after
            ? { after: options.after, first: input.aggregatePageSize }
            : { input },
          fetchOptions,
        ),
        signal,
        timeRemaining(),
      );
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
      throw new InteractiveReleaseSearchError("NO_SESSION");
    }
    jobId = job.id;
    applySnapshot(job);
    if (signal?.aborted) {
      cancelJob();
      return endSearch();
    }
    if (job.state !== "RUNNING") {
      return releases;
    }

    while (Date.now() < deadline) {
      await waitForNextPoll(
        signal,
        Math.min(POLL_INTERVAL_MS, timeRemaining()),
      );
      if (signal?.aborted) {
        cancelJob();
        return endSearch();
      }
      if (Date.now() >= deadline) break;
      let snapshot: InteractiveReleaseSearchJobPayload | null;
      try {
        const { data: pollData, error: pollError } = await boundedQuery(
          client.query(
            interactiveReleaseSearchQuery,
            { id: jobId },
            { requestPolicy: "network-only", ...fetchOptions },
          ),
          signal,
          timeRemaining(),
        );
        if (pollError) throw pollError;
        snapshot = (pollData?.interactiveReleaseSearch ??
          null) as InteractiveReleaseSearchJobPayload | null;
      } catch (pollFailure) {
        if (signal?.aborted || isAbortError(pollFailure)) {
          cancelJob();
          return endSearch();
        }
        if (pollFailure instanceof QueryTimeoutError) {
          cancelJob();
          return endSearch(pollFailure);
        }
        console.warn(
          "Interactive release search poll failed; retrying",
          pollFailure,
        );
        continue;
      }
      if (signal.aborted) {
        cancelJob();
        return endSearch();
      }
      if (!snapshot) {
        return endSearch(new InteractiveReleaseSearchError("SESSION_LOST"));
      }
      applySnapshot(snapshot);
      if (snapshot.state !== "RUNNING") {
        return releases;
      }
    }
    cancelJob();
    return endSearch(new QueryTimeoutError());
  } finally {
    requestController.abort();
  }
}
