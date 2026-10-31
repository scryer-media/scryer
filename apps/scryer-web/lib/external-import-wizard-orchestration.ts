import { userFacingGraphQlErrorMessage } from "./graphql/error-message.ts";
import type { ExternalImportMonitorWarmupStatus } from "@/lib/types/external-import";

export function continueExternalImportFromConnect(
  loadPreview: () => Promise<void>,
  navigate: () => void,
): void {
  void loadPreview();
  navigate();
}

export function isProwlarrDiscoveryReady(
  hasConnectedProwlarr: boolean,
  sessionId: string | null,
  status: ExternalImportMonitorWarmupStatus | null,
): boolean {
  return (
    !hasConnectedProwlarr || (Boolean(sessionId) && status === "COMPLETED")
  );
}

export function canRetryProwlarrDiscovery(
  status: ExternalImportMonitorWarmupStatus | null,
): boolean {
  return status === "FAILED" || status === "CANCELED";
}

// ── Finalize (background apply) ─────────────────────────────────────────────
// `finalizeExternalImport` validates synchronously and hands back a session id
// for a background apply. Everything the Summary step shows while that apply
// runs is derived here so it can be exercised without a browser.

/** Percentage shown on the Summary step's finalize bar. */
export function finalizeProgressPercent(
  completed: number,
  total: number,
  completedStatus: boolean,
): number {
  if (completedStatus) return 100;
  if (!Number.isFinite(total) || total <= 0) return 0;
  const pct = Math.round((completed / total) * 100);
  return Math.max(0, Math.min(100, pct));
}

/** A finalize session in one of these states no longer needs polling. */
export function isFinalizeSettled(
  status: ExternalImportMonitorWarmupStatus | null,
): boolean {
  return status === "COMPLETED" || status === "FAILED" || status === "CANCELED";
}

export interface FinalizeStatusSample {
  status: ExternalImportMonitorWarmupStatus;
  completed: number;
  total: number;
  errorMessage: string | null;
}

export interface FinalizePollDeps {
  /** Read the tracked apply session; `null` means the read itself failed. */
  fetchStatus: (sessionId: string) => Promise<{
    sample: FinalizeStatusSample | null;
    error: string | null;
    sessionGone?: boolean;
  }>;
  /** Called for every successful read so the view can redraw. */
  onSample: (sample: FinalizeStatusSample) => void;
  /** Injected so tests don't wait on real timers. */
  wait: (ms: number) => Promise<void>;
  /** Stops the loop when the caller unmounts or restarts finalize. */
  isStopped?: () => boolean;
}

export const FINALIZE_POLL_INTERVAL_MS = 1000;
export const FINALIZE_POLL_MAX_INTERVAL_MS = 5000;

/**
 * Poll one finalize apply session until it settles. Transient read failures
 * back off instead of giving up, so a blip never strands the Summary step;
 * a lost session (the orchestrator is in-memory) is terminal and reported.
 */
export async function pollExternalImportFinalize(
  sessionId: string,
  deps: FinalizePollDeps,
): Promise<{ ok: boolean; error: string | null; sessionGone?: boolean }> {
  let errorWaitMs = FINALIZE_POLL_INTERVAL_MS;
  for (;;) {
    if (deps.isStopped?.()) return { ok: false, error: null };
    const { sample, error, sessionGone } = await deps.fetchStatus(sessionId);
    if (deps.isStopped?.()) return { ok: false, error: null };
    if (!sample) {
      const message = error ?? "Failed to load import progress";
      if (sessionGone) {
        return { ok: false, error: message };
      }
      await deps.wait(errorWaitMs);
      errorWaitMs = Math.min(errorWaitMs * 2, FINALIZE_POLL_MAX_INTERVAL_MS);
      continue;
    }
    errorWaitMs = FINALIZE_POLL_INTERVAL_MS;
    deps.onSample(sample);
    if (sample.status === "COMPLETED") return { ok: true, error: null };
    if (sample.status === "FAILED" || sample.status === "CANCELED") {
      return {
        ok: false,
        error:
          sample.errorMessage ??
          (sample.status === "CANCELED"
            ? "The import was canceled."
            : "The import failed."),
      };
    }
    await deps.wait(FINALIZE_POLL_INTERVAL_MS);
  }
}

/**
 * Start a finalize and guarantee a failure is reported even when the start
 * throws. Without this a rejection inside the start sequence would leave the
 * wizard's Finish button disabled with nothing on screen to explain it.
 */
export async function runFinalizeStart(
  start: () => Promise<{
    ok: boolean;
    error: string | null;
    sessionGone?: boolean;
  }>,
  onFailure: (message: string) => void,
): Promise<{ ok: boolean; error: string | null; sessionGone?: boolean }> {
  try {
    const outcome = await start();
    if (!outcome.ok) {
      onFailure(outcome.error ?? "Failed to finalize import");
    }
    return outcome;
  } catch (error) {
    const message =
      userFacingGraphQlErrorMessage(error, String(error)) ||
      "Failed to finalize import";
    onFailure(message);
    return { ok: false, error: message };
  }
}

/** Whether the Summary step's Finish button must stay disabled. */
export function isFinalizeBlocked(state: {
  warmupComplete: boolean;
  previewSettled: boolean;
  mappingReady: boolean;
  finalizing: boolean;
}): boolean {
  return (
    !state.warmupComplete ||
    !state.previewSettled ||
    !state.mappingReady ||
    state.finalizing
  );
}
