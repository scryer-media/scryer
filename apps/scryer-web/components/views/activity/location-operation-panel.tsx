import * as React from "react";
import { useClient } from "urql";
import { Loader2, ShieldCheck, TriangleAlert, X } from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Progress } from "@/components/ui/progress";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useTranslate } from "@/lib/context/translate-context";
import { userFacingGraphQlErrorMessage } from "@/lib/graphql/error-message";
import {
  cancelLocationOperationMutation,
  resumeLocationOperationMutation,
} from "@/lib/graphql/mutations";
import { locationOperationQuery } from "@/lib/graphql/queries";
import {
  canCancelOperation,
  canResumeOperation,
  checkpointNeedsAttention,
  checkpointStateLabelKey,
  classificationLabelKey,
  isTerminalOperationState,
  OPERATION_POLL_INTERVAL_MS,
  operationByteProgress,
  operationStateLabelKey,
  orderedCheckpoints,
  toCount,
  verificationStampText,
  type LocationOperation,
  type LocationTitleCheckpoint,
} from "@/lib/location-operations";
import { formatByteCount } from "@/lib/utils/activity-utils";
import { cn } from "@/lib/utils";

type Props = {
  operationId: string;
  /** Clears `?operation=` and returns the user to the queue. */
  onDismiss?: () => void;
};

/**
 * Activity view for one location operation (FR-091): lifecycle state, volume
 * and outcome counters, the verification depth stamp with its fallback count
 * (FR-043), and every per-title checkpoint with its blocked/failed/warning
 * detail.
 *
 * It polls `locationOperation(id)` because checkpoints do not exist at accept
 * time — they are written as each title enters the run — and because the
 * operation is not yet linked to a job run, so nothing else pushes it.
 */
export function LocationOperationPanel({ operationId, onDismiss }: Props) {
  const client = useClient();
  const t = useTranslate();
  const setGlobalStatus = useGlobalStatus();

  const [operation, setOperation] = React.useState<LocationOperation | null>(
    null,
  );
  const [loading, setLoading] = React.useState(true);
  const [error, setError] = React.useState<string | null>(null);
  const [missing, setMissing] = React.useState(false);
  const [busy, setBusy] = React.useState(false);
  const [expanded, setExpanded] = React.useState<Set<string>>(new Set());
  const [refreshNonce, setRefreshNonce] = React.useState(0);
  const [nowMs, setNowMs] = React.useState(() => Date.now());

  const polling =
    operation === null || !isTerminalOperationState(operation.state);

  React.useEffect(() => {
    setOperation(null);
    setMissing(false);
    setError(null);
    setLoading(true);
    setExpanded(new Set());
  }, [operationId]);

  React.useEffect(() => {
    if (!operationId) {
      return undefined;
    }
    let active = true;
    let timer: ReturnType<typeof setTimeout> | null = null;

    const read = () => {
      client
        .query(
          locationOperationQuery,
          { id: operationId },
          { requestPolicy: "network-only" },
        )
        .toPromise()
        .then(({ data, error: queryError }) => {
          if (!active) {
            return;
          }
          if (queryError) {
            setError(
              userFacingGraphQlErrorMessage(
                queryError,
                t("move.operationLoadFailed"),
              ),
            );
            return;
          }
          const next = data?.locationOperation as LocationOperation | null;
          setError(null);
          setMissing(next == null);
          setOperation(next ?? null);
          setNowMs(Date.now());
        })
        .catch((caught: unknown) => {
          if (active) {
            setError(
              userFacingGraphQlErrorMessage(
                caught,
                t("move.operationLoadFailed"),
              ),
            );
          }
        })
        .finally(() => {
          if (!active) {
            return;
          }
          setLoading(false);
          if (polling) {
            timer = setTimeout(read, OPERATION_POLL_INTERVAL_MS);
          }
        });
    };

    read();
    return () => {
      active = false;
      if (timer) {
        clearTimeout(timer);
      }
    };
  }, [client, operationId, polling, refreshNonce, t]);

  const refresh = React.useCallback(() => {
    setRefreshNonce((current) => current + 1);
  }, []);

  const handleCancel = React.useCallback(async () => {
    setBusy(true);
    try {
      const { data, error: mutationError } = await client
        .mutation(cancelLocationOperationMutation, { id: operationId })
        .toPromise();
      if (mutationError) {
        throw mutationError;
      }
      const requested = Boolean(
        (data?.cancelLocationOperation as { cancelRequested?: boolean })
          ?.cancelRequested,
      );
      setGlobalStatus(
        requested ? t("move.cancelRequested") : t("move.cancelNotPossible"),
      );
      // The cancel payload carries only {id, cancelRequested}; the refreshed
      // row comes from the operation query.
      refresh();
    } catch (caught: unknown) {
      setGlobalStatus(
        userFacingGraphQlErrorMessage(caught, t("move.cancelFailed")),
      );
    } finally {
      setBusy(false);
    }
  }, [client, operationId, refresh, setGlobalStatus, t]);

  const handleResume = React.useCallback(async () => {
    setBusy(true);
    try {
      const { data, error: mutationError } = await client
        .mutation(resumeLocationOperationMutation, { id: operationId })
        .toPromise();
      if (mutationError) {
        throw mutationError;
      }
      const payload = data?.resumeLocationOperation as
        | { resumed?: boolean; detail?: string | null }
        | undefined;
      setGlobalStatus(
        payload?.resumed
          ? t("move.resumeRequested")
          : (payload?.detail ?? t("move.resumeNotPossible")),
      );
      refresh();
    } catch (caught: unknown) {
      setGlobalStatus(
        userFacingGraphQlErrorMessage(caught, t("move.resumeFailed")),
      );
    } finally {
      setBusy(false);
    }
  }, [client, operationId, refresh, setGlobalStatus, t]);

  const toggleCheckpoint = React.useCallback((titleId: string) => {
    setExpanded((current) => {
      const next = new Set(current);
      if (next.has(titleId)) {
        next.delete(titleId);
      } else {
        next.add(titleId);
      }
      return next;
    });
  }, []);

  if (loading && !operation) {
    return (
      <Card id="location-operation-panel">
        <CardContent className="flex items-center gap-2 py-6 text-sm text-muted-foreground">
          <Loader2 className="h-4 w-4 animate-spin" />
          {t("move.operationLoading")}
        </CardContent>
      </Card>
    );
  }

  if (missing || !operation) {
    return (
      <Card id="location-operation-panel">
        <CardContent className="flex items-center justify-between gap-3 py-6 text-sm text-muted-foreground">
          <span>{error ?? t("move.operationMissing")}</span>
          {onDismiss ? (
            <Button type="button" variant="outline" size="sm" onClick={onDismiss}>
              {t("move.operationDismiss")}
            </Button>
          ) : null}
        </CardContent>
      </Card>
    );
  }

  const counters = operation.counters;
  const progress = Math.round(operationByteProgress(counters) * 100);
  const checkpoints = orderedCheckpoints(operation.titleCheckpoints);
  const fallbackCount = toCount(operation.verificationFallbackCount);
  const terminal = isTerminalOperationState(operation.state);

  return (
    <Card id="location-operation-panel">
      <CardContent className="space-y-4 py-4">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div className="min-w-0">
            <p className="flex items-center gap-2 text-sm font-medium text-foreground">
              {t(`move.operationType.${operation.operationType}`)}
              <Badge tone={operationTone(operation)}>
                {t(operationStateLabelKey(operation.state))}
              </Badge>
              {operation.cancelRequested && !terminal ? (
                <Badge tone="warning" id="location-operation-cancel-requested">
                  {t("move.cancelDraining")}
                </Badge>
              ) : null}
            </p>
            <p className="mt-0.5 font-[var(--font-code)] text-xs text-muted-foreground">
              {operation.id}
            </p>
          </div>
          <div className="flex shrink-0 items-center gap-2">
            {canCancelOperation(operation) ? (
              <Button
                type="button"
                variant="outline"
                size="sm"
                id="location-operation-cancel"
                onClick={() => void handleCancel()}
                disabled={busy}
              >
                <X className="mr-1 h-3.5 w-3.5" />
                {t("move.cancelAction")}
              </Button>
            ) : null}
            {canResumeOperation(operation, nowMs) ? (
              <Button
                type="button"
                variant="outline"
                size="sm"
                id="location-operation-resume"
                onClick={() => void handleResume()}
                disabled={busy}
              >
                {t("move.resumeAction")}
              </Button>
            ) : null}
            {onDismiss ? (
              <Button
                type="button"
                variant="outline"
                size="sm"
                id="location-operation-dismiss"
                onClick={onDismiss}
              >
                {t("move.operationDismiss")}
              </Button>
            ) : null}
          </div>
        </div>

        <div className="space-y-1">
          <Progress value={progress} />
          <p className="text-xs text-muted-foreground">
            {t("move.operationProgress", {
              titles: `${toCount(counters.titlesProcessed)}/${toCount(counters.titlesTotal)}`,
              files: `${toCount(counters.filesProcessed)}/${toCount(counters.filesTotal)}`,
              bytes: `${formatByteCount(toCount(counters.bytesProcessed))} / ${formatByteCount(toCount(counters.bytesTotal))}`,
            })}
          </p>
        </div>

        <dl className="grid grid-cols-2 gap-2 sm:grid-cols-5">
          <Counter label={t("move.counterDedups")} value={toCount(counters.dedups)} />
          <Counter label={t("move.counterRenames")} value={toCount(counters.renames)} />
          <Counter label={t("move.counterNoOps")} value={toCount(counters.noOps)} />
          <Counter
            label={t("move.counterUnresolved")}
            value={toCount(counters.unresolved)}
          />
          <Counter
            label={t("move.counterBlocked")}
            value={toCount(counters.titlesBlocked)}
          />
        </dl>

        <p
          id="location-operation-verification"
          className="flex items-start gap-2 rounded-lg border border-border bg-muted/20 px-3 py-2 text-sm text-foreground"
        >
          <ShieldCheck className="mt-0.5 h-4 w-4 shrink-0 text-muted-foreground" />
          <span>
            {verificationStampText(
              operation.verificationDepth,
              fallbackCount,
              t,
            )}
          </span>
        </p>

        {operation.detail ? (
          <p className="flex items-start gap-2 rounded-lg border border-[var(--scry-warning-border)] bg-[var(--scry-warning-bg)] px-3 py-2 text-sm text-[var(--scry-warning-text)]">
            <TriangleAlert className="mt-0.5 h-4 w-4 shrink-0" />
            <span>{operation.detail}</span>
          </p>
        ) : null}

        {error ? (
          <p className="text-xs text-[var(--scry-danger-text)]">{error}</p>
        ) : null}

        <div className="space-y-1">
          <p className="text-sm font-medium text-foreground">
            {t("move.checkpointsHeading")}
          </p>
          {checkpoints.length === 0 ? (
            <p
              id="location-operation-no-checkpoints"
              className="text-xs text-muted-foreground"
            >
              {t("move.checkpointsPending")}
            </p>
          ) : (
            <ul className="space-y-1">
              {checkpoints.map((checkpoint) => (
                <CheckpointRow
                  key={checkpoint.titleId}
                  checkpoint={checkpoint}
                  expanded={expanded.has(checkpoint.titleId)}
                  onToggle={() => toggleCheckpoint(checkpoint.titleId)}
                  t={t}
                />
              ))}
            </ul>
          )}
        </div>
      </CardContent>
    </Card>
  );
}

function operationTone(
  operation: LocationOperation,
): "neutral" | "positive" | "warning" | "negative" | "info" {
  switch (operation.state) {
    case "COMPLETED":
      return "positive";
    case "COMPLETED_WITH_WARNINGS":
      return "warning";
    case "FAILED":
      return "negative";
    case "CANCELED":
      return "neutral";
    default:
      return "info";
  }
}

function Counter({ label, value }: { label: string; value: number }) {
  return (
    <div className="min-w-0 rounded-lg border border-border bg-muted/10 px-2 py-1">
      <dt className="truncate text-xs text-muted-foreground">{label}</dt>
      <dd className="text-sm text-foreground">{value}</dd>
    </div>
  );
}

function CheckpointRow({
  checkpoint,
  expanded,
  onToggle,
  t,
}: {
  checkpoint: LocationTitleCheckpoint;
  expanded: boolean;
  onToggle: () => void;
  t: (key: string, values?: Record<string, string | number>) => string;
}) {
  const attention = checkpointNeedsAttention(checkpoint);
  return (
    <li
      className={cn(
        "rounded-lg border px-2 py-1",
        attention
          ? "border-[var(--scry-warning-border)] bg-[var(--scry-warning-bg)]"
          : "border-border bg-muted/10",
      )}
    >
      <button
        type="button"
        onClick={onToggle}
        className="flex w-full items-center justify-between gap-2 text-left"
      >
        <span className="min-w-0 truncate text-sm text-foreground">
          {checkpoint.destinationFolderPath ??
            checkpoint.sourceFolderPath ??
            checkpoint.titleId}
        </span>
        <Badge tone={attention ? "warning" : "neutral"}>
          {t(checkpointStateLabelKey(checkpoint.state))}
        </Badge>
      </button>
      {expanded ? (
        <dl className="mt-1 space-y-0.5 text-xs text-muted-foreground">
          <div>
            <dt className="inline">{t("move.checkpointClass")}: </dt>
            <dd className="inline">
              {checkpoint.classification
                ? t(classificationLabelKey(checkpoint.classification))
                : "—"}
            </dd>
          </div>
          <div>
            <dt className="inline">{t("move.checkpointFrom")}: </dt>
            <dd className="inline font-[var(--font-code)] break-all">
              {checkpoint.sourceFolderPath ?? "—"}
            </dd>
          </div>
          <div>
            <dt className="inline">{t("move.checkpointTo")}: </dt>
            <dd className="inline font-[var(--font-code)] break-all">
              {checkpoint.destinationFolderPath ?? "—"}
            </dd>
          </div>
          <div>
            <dt className="inline">{t("move.checkpointVerified")}: </dt>
            <dd className="inline">
              {t("move.checkpointVerifiedValue", {
                files: `${toCount(checkpoint.filesVerified)}/${toCount(checkpoint.filesTotal)}`,
                bytes: `${formatByteCount(toCount(checkpoint.bytesVerified))} / ${formatByteCount(toCount(checkpoint.bytesTotal))}`,
              })}
            </dd>
          </div>
          {checkpoint.detail ? (
            <div>
              <dt className="inline">{t("move.checkpointDetail")}: </dt>
              <dd className="inline">{checkpoint.detail}</dd>
            </div>
          ) : null}
        </dl>
      ) : null}
    </li>
  );
}
