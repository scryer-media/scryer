import * as React from "react";
import { useClient } from "urql";
import {
  ChevronDown,
  ChevronRight,
  Loader2,
  TriangleAlert,
} from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Progress } from "@/components/ui/progress";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useTranslate } from "@/lib/context/translate-context";
import { userFacingGraphQlErrorMessage } from "@/lib/graphql/error-message";
import {
  cancelLocationOperationMutation,
  resumeLocationOperationMutation,
} from "@/lib/graphql/mutations";
import { locationTransferTitleDetailQuery } from "@/lib/graphql/queries";
import { useLocationTransfer } from "@/lib/hooks/use-location-transfer";
import {
  canCancelOperation,
  canResumeOperation,
  checkpointStateLabelKey,
  isTerminalOperationState,
  operationStateLabelKey,
  toCount,
} from "@/lib/location-operations";
import {
  splitTransferPath,
  transferTitleProgress,
  type TransferTitle,
} from "@/lib/location-transfers";
import { formatByteCount } from "@/lib/utils/activity-utils";

type Props = { operationId: string; onDismiss?: () => void };
export function LocationOperationPanel(props: Props) {
  return <OperationPanel key={props.operationId} {...props} />;
}

function OperationPanel({ operationId }: Props) {
  const client = useClient();
  const t = useTranslate();
  const setGlobalStatus = useGlobalStatus();
  const [open, setOpen] = React.useState(false);
  const [page, setPage] = React.useState(0);
  const [busy, setBusy] = React.useState(false);
  const [refresh, setRefresh] = React.useState(0);
  const [now, setNow] = React.useState(() => Date.now());
  const summary = useLocationTransfer(operationId, null, true, refresh);
  const titles = useLocationTransfer(operationId, page, open, refresh);
  const operation = summary.snapshot?.operation;
  React.useEffect(() => {
    setNow(Date.now());
  }, [summary.snapshot]);

  async function act(resume: boolean) {
    setBusy(true);
    try {
      const result = await client
        .mutation(
          resume
            ? resumeLocationOperationMutation
            : cancelLocationOperationMutation,
          { id: operationId },
        )
        .toPromise();
      if (result.error) throw result.error;
      const payload = resume
        ? result.data?.resumeLocationOperation
        : result.data?.cancelLocationOperation;
      setGlobalStatus(
        resume
          ? payload?.resumed
            ? t("move.resumeRequested")
            : (payload?.detail ?? t("move.resumeNotPossible"))
          : payload?.cancelRequested
            ? t("move.cancelRequested")
            : t("move.cancelNotPossible"),
      );
      setRefresh((value) => value + 1);
    } catch (error) {
      setGlobalStatus(
        userFacingGraphQlErrorMessage(
          error,
          t(resume ? "move.resumeFailed" : "move.cancelFailed"),
        ),
      );
    } finally {
      setBusy(false);
    }
  }

  if (!operation || !summary.snapshot)
    return (
      <Card id="location-operation-panel">
        <CardContent className="flex items-center gap-2 py-6 text-sm text-muted-foreground">
          {!summary.error && <Loader2 className="h-4 w-4 animate-spin" />}
          {t(
            summary.error === "missing"
              ? "move.operationMissing"
              : summary.error
                ? "move.operationLoadFailed"
                : "move.operationLoading",
          )}
        </CardContent>
      </Card>
    );

  const terminal = isTerminalOperationState(operation.state);
  const counters = operation.counters;
  const progress = Math.floor(summary.snapshot.progressBasisPoints / 10) / 10;
  const eta =
    summary.connected && operation.state !== "QUEUED" && !terminal
      ? summary.snapshot.etaSeconds
      : null;
  const total = toCount(
    titles.snapshot?.totalCount ?? summary.snapshot.totalCount,
  );
  const rows = titles.snapshot?.titles ?? [];
  const countKeys = [
    "merges",
    "dedups",
    "renames",
    "noOps",
    "unresolved",
    "titlesBlocked",
  ] as const;
  const countLabels = [
    "Merges",
    "Dedups",
    "Renames",
    "NoOps",
    "Unresolved",
    "Blocked",
  ];

  return (
    <Card id="location-operation-panel">
      <CardContent className="space-y-4 py-4">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div className="flex flex-wrap items-center gap-2 text-sm font-medium">
            <span
              id="location-operation-type"
              data-operation-type={operation.operationType}
            >
              {t(`move.operationType.${operation.operationType}`)}
            </span>
            <Badge
              id="location-operation-state"
              tone={
                operation.state === "FAILED"
                  ? "negative"
                  : operation.state === "COMPLETED"
                    ? "positive"
                    : "neutral"
              }
            >
              {t(operationStateLabelKey(operation.state))}
            </Badge>
            {operation.cancelRequested && !terminal && (
              <Badge tone="warning">{t("move.cancelDraining")}</Badge>
            )}
          </div>
          <div className="flex gap-2">
            {canCancelOperation(operation) && (
              <Button
                id="location-operation-cancel"
                variant="destructive"
                size="sm"
                disabled={busy}
                onClick={() => void act(false)}
              >
                {t("move.cancelAction")}
              </Button>
            )}
            {canResumeOperation(operation, now) && (
              <Button
                id="location-operation-resume"
                variant="outline"
                size="sm"
                disabled={busy}
                onClick={() => void act(true)}
              >
                {t("move.resumeAction")}
              </Button>
            )}
          </div>
        </div>
        <div className="space-y-2">
          <div className="flex justify-between text-xs text-muted-foreground">
            <span>{progress.toFixed(1)}%</span>
            <span>
              {terminal || operation.state === "QUEUED"
                ? ""
                : !summary.connected
                  ? t("move.transferDisconnected")
                  : eta === null
                    ? t("move.transferEstimating")
                    : t("move.transferEta", {
                        minutes: Math.max(1, Math.ceil(toCount(eta) / 60)),
                      })}
            </span>
          </div>
          <Progress
            value={progress}
            aria-label={t("move.transferOverallProgress")}
          />
          <p
            id="location-operation-progress"
            className="text-xs text-muted-foreground"
          >
            {t("move.operationProgress", {
              titles: `${toCount(counters.titlesProcessed)}/${toCount(counters.titlesTotal)}`,
              files: `${toCount(counters.filesProcessed)}/${toCount(counters.filesTotal)}`,
              bytes: `${formatByteCount(toCount(counters.bytesProcessed))} / ${formatByteCount(toCount(counters.bytesTotal))}`,
            })}
          </p>
        </div>
        <div>
          <Button
            variant="ghost"
            size="sm"
            className="px-0"
            aria-expanded={open}
            aria-controls="location-transfer-titles"
            onClick={() => setOpen(!open)}
          >
            {open ? (
              <ChevronDown className="mr-1 h-4 w-4" />
            ) : (
              <ChevronRight className="mr-1 h-4 w-4" />
            )}
            {t(open ? "move.transferShowLess" : "move.transferShowMore")}
          </Button>
          {open && (
            <div id="location-transfer-titles" className="space-y-2">
              {titles.error && (
                <p className="text-xs text-muted-foreground">
                  {t("move.operationLoadFailed")}
                </p>
              )}
              <div className="overflow-x-auto">
                <table className="w-full table-fixed text-left text-xs">
                  <thead>
                    <tr className="border-b border-border">
                      {[
                        "Title",
                        "Status",
                        "Files",
                        "CurrentFile",
                        "Progress",
                      ].map((column, index) => (
                        <th
                          key={column}
                          className={`p-2 ${index === 3 ? "w-[30%]" : index === 2 || index === 4 ? "w-[12%]" : "w-[23%]"}`}
                        >
                          {t(`move.transferColumn${column}`)}
                        </th>
                      ))}
                    </tr>
                  </thead>
                  <tbody>
                    {rows.map((row) => (
                      <TitleRow
                        key={row.titleId}
                        operationId={operationId}
                        row={row}
                      />
                    ))}
                  </tbody>
                </table>
              </div>
              {!titles.snapshot && (
                <p className="text-xs text-muted-foreground">
                  {t("move.operationLoading")}
                </p>
              )}
              <div className="flex items-center justify-between gap-2 text-xs text-muted-foreground">
                <span>
                  {t("move.transferRange", {
                    from: rows.length ? page * 50 + 1 : 0,
                    to: rows.length ? page * 50 + rows.length : 0,
                    total,
                  })}
                </span>
                <div className="flex gap-2">
                  <Button
                    variant="outline"
                    size="sm"
                    disabled={page === 0}
                    onClick={() => setPage(page - 1)}
                  >
                    {t("move.transferPrevious")}
                  </Button>
                  <Button
                    variant="outline"
                    size="sm"
                    disabled={!titles.snapshot?.hasMore}
                    onClick={() => setPage(page + 1)}
                  >
                    {t("move.transferNext")}
                  </Button>
                </div>
              </div>
            </div>
          )}
        </div>
        <dl className="grid grid-cols-2 gap-2 sm:grid-cols-3 lg:grid-cols-6">
          {countKeys.map((key, i) => (
            <div
              key={key}
              className="rounded-lg border border-border px-2 py-1"
            >
              <dt className="text-xs text-muted-foreground">
                {t(`move.counter${countLabels[i]}`)}
              </dt>
              <dd className="text-sm">{toCount(counters[key])}</dd>
            </div>
          ))}
        </dl>
        {operation.detail && (
          <p className="flex items-start gap-2 rounded-lg border border-[var(--scry-warning-border)] p-3 text-sm">
            <TriangleAlert className="h-4 w-4 shrink-0" />
            <span>{operation.detail}</span>
          </p>
        )}
        {summary.error && (
          <p className="text-xs text-[var(--scry-danger-text)]">
            {t("move.operationLoadFailed")}
          </p>
        )}
      </CardContent>
    </Card>
  );
}

function TitleRow({
  operationId,
  row,
}: {
  operationId: string;
  row: TransferTitle;
}) {
  const client = useClient();
  const t = useTranslate();
  const [expanded, setExpanded] = React.useState(false);
  const [detail, setDetail] = React.useState<string | null>(null);
  const [failed, setFailed] = React.useState(false);
  React.useEffect(() => {
    if (!expanded || !row.hasException) return;
    let active = true;
    client
      .query(
        locationTransferTitleDetailQuery,
        { id: operationId, titleId: row.titleId },
        { requestPolicy: "network-only" },
      )
      .toPromise()
      .then((result) => {
        if (!active) return;
        setFailed(Boolean(result.error));
        setDetail(result.data?.locationTransferTitleDetail ?? null);
      })
      .catch(() => {
        if (active) setFailed(true);
      });
    return () => {
      active = false;
    };
  }, [client, expanded, operationId, row.titleId, row.state, row.hasException]);
  const path = row.currentFile ? splitTransferPath(row.currentFile) : null;
  return (
    <React.Fragment>
      <tr
        id={`location-operation-checkpoint-${row.titleId}`}
        className="border-b border-border align-top"
      >
        <td className="p-2">
          <div className="flex items-start gap-1">
            {row.hasException && (
              <button
                aria-label={t("move.transferException", { title: row.name })}
                aria-expanded={expanded}
                aria-controls={`transfer-detail-${row.titleId}`}
                onClick={() => setExpanded(!expanded)}
              >
                {expanded ? (
                  <ChevronDown className="h-4 w-4" />
                ) : (
                  <ChevronRight className="h-4 w-4" />
                )}
              </button>
            )}
            <span className="break-words">{row.name}</span>
          </div>
        </td>
        <td className="p-2">
          <Badge tone={row.hasException ? "warning" : "neutral"}>
            {t(
              checkpointStateLabelKey(
                row.verifying > 0 ? "VERIFYING" : row.state,
              ),
            )}
          </Badge>
          {(row.copying > 0 || row.verifying > 0) && (
            <p className="mt-1 text-muted-foreground">
              {t("move.transferActive", {
                copying: row.copying,
                verifying: row.verifying,
              })}
            </p>
          )}
        </td>
        <td className="p-2 tabular-nums">
          {toCount(row.filesDone)}/{toCount(row.filesTotal)}
        </td>
        <td className="min-w-0 p-2">
          {path && row.currentFile ? (
            <Tooltip>
              <TooltipTrigger asChild>
                <span
                  tabIndex={0}
                  aria-label={row.currentFile}
                  className="flex min-w-0 font-[var(--font-code)]"
                >
                  <span
                    aria-hidden="true"
                    className="min-w-0 truncate"
                    style={{ direction: "rtl", textAlign: "left" }}
                  >
                    {path.directory}
                  </span>
                  <span
                    aria-hidden="true"
                    className="max-w-full shrink-0 truncate"
                  >
                    {path.filename}
                  </span>
                </span>
              </TooltipTrigger>
              <TooltipContent className="max-w-xl break-all">
                {row.currentFile}
              </TooltipContent>
            </Tooltip>
          ) : (
            "—"
          )}
        </td>
        <td className="p-2 tabular-nums">
          {Math.floor(transferTitleProgress(row))}%
          {row.verifying > 0 && (
            <p className="text-muted-foreground">
              {formatByteCount(toCount(row.verificationBytes))}
            </p>
          )}
        </td>
      </tr>
      {expanded && row.hasException && (
        <tr id={`transfer-detail-${row.titleId}`}>
          <td colSpan={5} className="break-words bg-muted/20 p-3 text-xs">
            {detail ??
              t(
                failed
                  ? "move.operationLoadFailed"
                  : "move.transferDetailPending",
              )}
          </td>
        </tr>
      )}
    </React.Fragment>
  );
}
