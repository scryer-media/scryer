import * as React from "react";
import { useClient } from "urql";
import { useNavigate } from "react-router";
import { CircleCheck, Loader2 } from "lucide-react";

import { Button } from "@/components/ui/button";
import { useTranslate } from "@/lib/context/translate-context";
import { userFacingGraphQlErrorMessage } from "@/lib/graphql/error-message";
import { startLocationOperationMutation } from "@/lib/graphql/mutations";
import {
  recognizeStartRefusal,
  refusalMessageKey,
  refusalNeedsFreshPreview,
} from "@/lib/location-operations";

/**
 * The confirm half of every location dialog, in one place.
 *
 * `startLocationOperation` is one mutation with several destination forms, and
 * every dialog that confirms one runs the same three-outcome state machine
 * around it: it started, it was refused with something to read, or the plan
 * moved under the user and the answer is a fresh preview (FR-081). Only the
 * mutation input and what "a fresh preview" means differ per dialog, so those
 * are the arguments.
 */
export type LocationOperationStart = {
  /** True while the mutation is in flight; the footer disables on it. */
  starting: boolean;
  /** A refusal the user has to read, already translated. */
  startError: string | null;
  /** True when the plan moved and the dialog owes the user a fresh one. */
  planChanged: boolean;
  /** The accepted operation, which is what turns the dialog into a receipt. */
  startedOperationId: string | null;
  /** Confirm the plan in hand. The input is the whole mutation input. */
  start: (input: Record<string, unknown>) => Promise<void>;
  /** Forget the last refusal, leaving the stale-plan notice standing. */
  clearStartError: () => void;
  /** Forget the last refusal and the stale-plan notice. */
  reset: () => void;
  /** The above, plus the accepted operation: reopening starts from nothing. */
  resetAll: () => void;
  /** Dismiss the stale-plan notice, for a dialog that re-previews on demand. */
  clearPlanChanged: () => void;
};

export function useLocationOperationStart(options: {
  /** The sentence a start failure falls back to, already translated. */
  failedMessage: string;
  /**
   * A refusal vocabulary the caller owns — the root-scoped codes, which are
   * coded rather than recognized from a sentence. Return the translated
   * sentence, or null to fall through to the shared handling.
   */
  recognizeOwnRefusal?: (error: unknown) => string | null;
  /**
   * What "the plan moved, take a fresh one" means for this dialog: dropping the
   * preview it holds, or asking its preview effect to run again.
   */
  onNeedsFreshPreview?: () => void;
  /** Fires with the accepted operation id, for the caller's own bookkeeping. */
  onStarted?: (operationId: string) => void;
}): LocationOperationStart {
  const client = useClient();
  const t = useTranslate();
  const {
    failedMessage,
    recognizeOwnRefusal,
    onNeedsFreshPreview,
    onStarted,
  } = options;

  const [starting, setStarting] = React.useState(false);
  const [startError, setStartError] = React.useState<string | null>(null);
  const [planChanged, setPlanChanged] = React.useState(false);
  const [startedOperationId, setStartedOperationId] = React.useState<
    string | null
  >(null);

  const start = React.useCallback(
    async (input: Record<string, unknown>) => {
      setStarting(true);
      setStartError(null);
      try {
        const { data, error } = await client
          .mutation(startLocationOperationMutation, { input })
          .toPromise();
        if (error) {
          throw error;
        }
        const started = data?.startLocationOperation as
          | { operation: { id: string } }
          | undefined;
        if (!started?.operation?.id) {
          throw new Error(failedMessage);
        }
        // The dialog stays open on success: nothing lists location operations
        // yet, so this is where the user picks the operation up in Activity.
        setStartedOperationId(started.operation.id);
        onStarted?.(started.operation.id);
      } catch (error: unknown) {
        const own = recognizeOwnRefusal?.(error) ?? null;
        if (own) {
          setStartError(own);
          return;
        }
        const message = userFacingGraphQlErrorMessage(error, failedMessage);
        // A refused confirmation is nearly always "the plan moved under you",
        // or a title that became blocked between preview and confirm. Either
        // way the answer is a fresh plan, not a backend sentence about
        // fingerprints.
        const refusal = recognizeStartRefusal(error, message);
        if (refusalNeedsFreshPreview(refusal)) {
          setPlanChanged(true);
          setStartError(null);
          onNeedsFreshPreview?.();
        } else {
          // A refusal Scryer has its own words for says them; anything else
          // shows the server's sentence rather than a guess.
          const key = refusalMessageKey(refusal);
          setStartError(key ? t(key) : message);
        }
      } finally {
        setStarting(false);
      }
    },
    [
      client,
      failedMessage,
      onNeedsFreshPreview,
      onStarted,
      recognizeOwnRefusal,
      t,
    ],
  );

  const clearStartError = React.useCallback(() => setStartError(null), []);

  const reset = React.useCallback(() => {
    setStartError(null);
    setPlanChanged(false);
  }, []);

  const resetAll = React.useCallback(() => {
    setStartError(null);
    setPlanChanged(false);
    setStartedOperationId(null);
  }, []);

  const clearPlanChanged = React.useCallback(() => setPlanChanged(false), []);

  return {
    starting,
    startError,
    planChanged,
    startedOperationId,
    start,
    clearStartError,
    reset,
    resetAll,
    clearPlanChanged,
  };
}

/**
 * Router-dependent by design, and mounted only after a start succeeds: the
 * dialogs themselves must render outside a router (the title settings panels
 * are server-rendered in tests without one).
 */
function ViewOperationButton({
  id,
  operationId,
  label,
  onNavigated,
}: {
  id: string;
  operationId: string;
  label: string;
  onNavigated: () => void;
}) {
  const navigate = useNavigate();
  return (
    <Button
      type="button"
      variant="primary"
      id={id}
      onClick={() => {
        onNavigated();
        void navigate(`/activity?operation=${encodeURIComponent(operationId)}`);
      }}
    >
      {label}
    </Button>
  );
}

/**
 * What a location dialog becomes once its operation is accepted: the heading,
 * the operation id, and the way into Activity.
 */
export function LocationOperationStartedPanel({
  idPrefix,
  operationId,
  heading,
  viewLabel,
  onNavigated,
}: {
  /** `root-change` or `move-titles`; every id below is built from it. */
  idPrefix: string;
  operationId: string;
  heading: string;
  viewLabel: string;
  onNavigated: () => void;
}) {
  return (
    <div id={`${idPrefix}-started`} className="space-y-3">
      <p className="flex items-start gap-2 rounded-lg border border-[var(--scry-success-border)] bg-[var(--scry-success-bg)] px-3 py-3 text-sm text-[var(--scry-success-text)]">
        <CircleCheck className="mt-0.5 h-4 w-4 shrink-0" />
        <span>{heading}</span>
      </p>
      <p className="font-[var(--font-code)] text-xs break-all text-muted-foreground">
        {operationId}
      </p>
      <ViewOperationButton
        id={`${idPrefix}-view-operation`}
        operationId={operationId}
        label={viewLabel}
        onNavigated={onNavigated}
      />
    </div>
  );
}

/** A refusal the user has to read, in the shared danger block. */
export function LocationOperationErrorNotice({
  id,
  message,
}: {
  id: string;
  message: string | null;
}) {
  if (!message) {
    return null;
  }
  return (
    <p
      id={id}
      className="rounded-lg border border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)] px-3 py-3 text-sm text-[var(--scry-danger-text)]"
    >
      {message}
    </p>
  );
}

/** The dismiss control every location dialog closes with. */
export function LocationDialogDismissButton({
  id,
  label,
  disabled,
  onDismiss,
}: {
  id: string;
  label: string;
  disabled: boolean;
  onDismiss: () => void;
}) {
  return (
    <Button
      type="button"
      variant="outline"
      id={id}
      onClick={onDismiss}
      disabled={disabled}
    >
      {label}
    </Button>
  );
}

/** A primary footer action that spins while its work is in flight. */
export function LocationDialogPrimaryButton({
  id,
  label,
  busy,
  disabled,
  onClick,
}: {
  id: string;
  label: string;
  busy: boolean;
  disabled: boolean;
  onClick: () => void;
}) {
  return (
    <Button
      type="button"
      variant="primary"
      id={id}
      onClick={onClick}
      disabled={disabled}
    >
      {busy ? <Loader2 className="mr-2 h-4 w-4 animate-spin" /> : null}
      {label}
    </Button>
  );
}
