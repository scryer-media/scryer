import * as React from "react";
import { useClient, useQuery } from "urql";
import {
  ChevronDown,
  ChevronRight,
  Loader2,
  TriangleAlert,
  X,
} from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Progress } from "@/components/ui/progress";
import { TitlePosterSlot } from "@/components/title-poster-slot";
import {
  MoveTitlesDialog,
  type MoveDestinationLibrary,
  type MoveTitleRef,
} from "@/components/dialogs/move-titles-dialog";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useTranslate } from "@/lib/context/translate-context";
import { userFacingGraphQlErrorMessage } from "@/lib/graphql/error-message";
import {
  abandonLocationOperationMutation,
  cancelLocationOperationMutation,
  resumeLocationOperationMutation,
} from "@/lib/graphql/mutations";
import {
  librariesQuery,
  locationOperationRetrySelectionQuery,
  locationTransferTitleDetailQuery,
} from "@/lib/graphql/queries";
import { useLocationTransfer } from "@/lib/hooks/use-location-transfer";
import {
  canAbandonOperation,
  canCancelOperation,
  canPlanAgainOperation,
  canResumeOperation,
  canRetryOperation,
  checkpointStateLabelKey,
  isTerminalOperationState,
  operationGuidanceKey,
  operationStateLabelKey,
  retrySelectionTitleRefs,
  toCount,
  type LocationRetrySelection,
} from "@/lib/location-operations";
import {
  splitTransferPath,
  transferArtworkRequest,
  transferOperationProgress,
  transferTitleProgress,
  type TransferTitle,
  detailLines,
} from "@/lib/location-transfers";
import { formatByteCount } from "@/lib/utils/activity-utils";
import { selectPosterVariantUrl } from "@/lib/utils/poster-images";

type Props = { operationId: string; onDismiss?: () => void };
export function LocationOperationPanel(props: Props) {
  return <OperationPanel key={props.operationId} {...props} />;
}

function OperationPanel({ operationId, onDismiss }: Props) {
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
  const artworkRequest = transferArtworkRequest(
    titles.snapshot?.titles.map((row) => row.titleId) ?? [],
  );
  const [artwork] = useQuery<
    Record<string, { id: string; posterUrl: string | null } | null>
  >({
    ...artworkRequest,
    pause: !open || !titles.snapshot?.titles.length,
    requestPolicy: "cache-first",
  });
  const posters = new Map(
    Object.values(artwork.data ?? {})
      .filter((title) => title?.id)
      .map((title) => [title!.id, title!.posterUrl]),
  );
  React.useEffect(() => {
    setNow(Date.now());
  }, [summary.snapshot]);

  // Plan again needs the stored plan joined with the checkpoints, which is a
  // read of its own rather than part of every progress snapshot. It is only
  // worth asking for once the operation has stopped.
  const retryable = canRetryOperation(operation);
  const [retrySelection, setRetrySelection] =
    React.useState<LocationRetrySelection | null>(null);
  React.useEffect(() => {
    if (!retryable) {
      setRetrySelection(null);
      return;
    }
    let active = true;
    client
      .query<{ locationOperationRetrySelection: LocationRetrySelection | null }>(
        locationOperationRetrySelectionQuery,
        { id: operationId },
        { requestPolicy: "network-only" },
      )
      .toPromise()
      .then((result) => {
        if (!active) return;
        setRetrySelection(result.data?.locationOperationRetrySelection ?? null);
      })
      .catch(() => {
        if (active) setRetrySelection(null);
      });
    return () => {
      active = false;
    };
  }, [client, operationId, retryable, refresh]);
  const planAgainTitleIds = React.useMemo(
    () => new Set(retrySelection?.titles.map((title) => title.titleId) ?? []),
    [retrySelection],
  );
  // The operation's own code is the most consequential one across its
  // titles; a title's own code is what says why *it* stopped.
  const titleGuidanceKeys = React.useMemo(
    () =>
      new Map(
        (operation?.titleCheckpoints ?? []).flatMap((checkpoint) => {
          const key = operationGuidanceKey(checkpoint.reasonCode);
          return key ? [[checkpoint.titleId, key] as const] : [];
        }),
      ),
    [operation?.titleCheckpoints],
  );
  const [planAgain, setPlanAgain] = React.useState<{
    titles: MoveTitleRef[];
    libraries: MoveDestinationLibrary[];
  } | null>(null);

  // The ordinary move dialog, opened on this operation's leftovers: the
  // destination it had, and the titles it did not finish (or the one whose
  // row the action was taken from). The preview does the rest.
  async function openPlanAgain(titleId: string | null = null) {
    const titles = retrySelectionTitleRefs(retrySelection, titleId);
    if (!titles.length) return;
    setBusy(true);
    try {
      const result = await client
        .query<{ libraries: MoveDestinationLibrary[] }>(
          librariesQuery,
          { facet: null, permission: "MANAGE_TITLES" },
          { requestPolicy: "network-only" },
        )
        .toPromise();
      if (result.error) throw result.error;
      const libraries = (result.data?.libraries ?? []).map((library) => ({
        id: library.id,
        name: library.name,
        roots: library.roots,
      }));
      const rootPaths = new Map(
        libraries.flatMap((library) =>
          library.roots.map((root) => [root.id, root.path] as const),
        ),
      );
      setPlanAgain({
        titles: titles.map((title) => ({
          ...title,
          rootFolderPath: title.rootFolderId
            ? (rootPaths.get(title.rootFolderId) ?? null)
            : null,
        })),
        libraries,
      });
    } catch (error) {
      setGlobalStatus(
        userFacingGraphQlErrorMessage(error, t("move.planAgainFailed")),
      );
    } finally {
      setBusy(false);
    }
  }

  // Resume and retry ride one mutation: the server reopens a failed or
  // canceled operation and walks it from the last verified checkpoint. Only
  // the words differ, because "resume" implies the run was interrupted and a
  // terminal run was not. Abandon is the opposite direction: it marks a dead
  // run failed and releases what it owns, so Retry can pick it up later.
  type OperationAction = "resume" | "retry" | "cancel" | "abandon";
  async function act(action: OperationAction) {
    setBusy(true);
    try {
      const outcome = await requestAction(action);
      setGlobalStatus(
        outcome.done
          ? t(`move.${action}Requested`)
          : (outcome.detail ?? t(`move.${action}NotPossible`)),
      );
      setRefresh((value) => value + 1);
    } catch (error) {
      setGlobalStatus(
        userFacingGraphQlErrorMessage(error, t(`move.${action}Failed`)),
      );
    } finally {
      setBusy(false);
    }
  }

  async function requestAction(
    action: OperationAction,
  ): Promise<{ done: boolean; detail: string | null }> {
    const variables = { id: operationId };
    if (action === "cancel") {
      const result = await client
        .mutation(cancelLocationOperationMutation, variables)
        .toPromise();
      if (result.error) throw result.error;
      return {
        done: result.data?.cancelLocationOperation?.cancelRequested === true,
        detail: null,
      };
    }
    if (action === "abandon") {
      const result = await client
        .mutation(abandonLocationOperationMutation, variables)
        .toPromise();
      if (result.error) throw result.error;
      const payload = result.data?.abandonLocationOperation;
      return {
        done: payload?.abandoned === true,
        detail: payload?.detail ?? null,
      };
    }
    const result = await client
      .mutation(resumeLocationOperationMutation, variables)
      .toPromise();
    if (result.error) throw result.error;
    const payload = result.data?.resumeLocationOperation;
    return {
      done: payload?.resumed === true,
      detail: payload?.detail ?? null,
    };
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
  // Non-terminal, and nothing has written to it in a while: whatever was
  // running it is gone. The state badge alone would still say "Moving".
  const stalled = canResumeOperation(operation, now);
  const guidanceKey = operationGuidanceKey(operation.reasonCode);
  const counters = operation.counters;
  const progress = transferOperationProgress(summary.snapshot);
  const eta =
    summary.connected && operation.state !== "QUEUED" && !terminal
      ? summary.snapshot.etaSeconds
      : null;
  const total = toCount(
    titles.snapshot?.totalCount ?? summary.snapshot.totalCount,
  );
  const rows = titles.snapshot?.titles ?? [];

  return (
    <Card
      id="location-operation-panel"
      className="overflow-hidden rounded-[14px] border-[var(--scry-border2)] bg-[var(--scry-surfC)] shadow-none"
    >
      <CardContent className="space-y-4 p-4 sm:p-5">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div className="flex flex-wrap items-center gap-3 text-base font-semibold text-[var(--scry-ink3)]">
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
            {stalled && (
              <Badge id="location-operation-stalled" tone="warning">
                {t("move.stalledNoRunner")}
              </Badge>
            )}
          </div>
          <div className="flex gap-2">
            {terminal && onDismiss && (
              <Button
                id="location-operation-dismiss"
                variant="default"
                size="default"
                className="min-w-28 gap-2 font-semibold"
                onClick={onDismiss}
              >
                <X className="h-4 w-4" aria-hidden="true" />
                {t("move.transferDismiss")}
              </Button>
            )}
            {canCancelOperation(operation) && (
              <Button
                id="location-operation-cancel"
                variant="destructive"
                size="sm"
                disabled={busy}
                onClick={() => void act("cancel")}
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
                onClick={() => void act("resume")}
              >
                {t("move.resumeAction")}
              </Button>
            )}
            {canAbandonOperation(operation, now) && (
              <Button
                id="location-operation-abandon"
                variant="destructive"
                size="sm"
                disabled={busy}
                onClick={() => void act("abandon")}
              >
                {t("move.abandonAction")}
              </Button>
            )}
            {canRetryOperation(operation) && (
              <Button
                id="location-operation-retry"
                variant="outline"
                size="sm"
                disabled={busy}
                onClick={() => void act("retry")}
              >
                {t("move.retryAction")}
              </Button>
            )}
            {canPlanAgainOperation(operation, retrySelection) && (
              <Button
                id="location-operation-plan-again"
                variant="outline"
                size="sm"
                disabled={busy}
                onClick={() => void openPlanAgain()}
              >
                {t("move.planAgainAction")}
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
            aria-valuetext={`${progress.toFixed(1)}%`}
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
              <Table
                layout="fixed"
                density="dense"
                className="min-w-[800px]"
                wrapperClassName="rounded-[14px] border border-[var(--scry-border2)] bg-[var(--scry-surfC)]"
              >
                <TableHeader>
                  <TableRow>
                    {[
                      "Title",
                      "Status",
                      "Files",
                      "CurrentFile",
                      "Progress",
                    ].map((column, index) => (
                      <TableHead
                        key={column}
                        className={
                          index === 3
                            ? "w-[30%]"
                            : index === 2 || index === 4
                              ? "w-[12%]"
                              : "w-[23%]"
                        }
                      >
                        {t(`move.transferColumn${column}`)}
                      </TableHead>
                    ))}
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {rows.map((row) => (
                    <TitleRow
                      key={row.titleId}
                      operationId={operationId}
                      row={row}
                      posterUrl={posters.get(row.titleId)}
                      guidanceKey={titleGuidanceKeys.get(row.titleId) ?? null}
                      onPlanAgain={
                        !busy && planAgainTitleIds.has(row.titleId)
                          ? () => void openPlanAgain(row.titleId)
                          : null
                      }
                    />
                  ))}
                </TableBody>
              </Table>
              {!titles.snapshot && (
                <p className="text-xs text-muted-foreground">
                  {t("move.operationLoading")}
                </p>
              )}
              {total > 50 && (
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
              )}
            </div>
          )}
        </div>
        {(operation.detail || guidanceKey || stalled) && (
          <div
            id="location-operation-detail"
            className="flex items-start gap-2 rounded-lg border border-[var(--scry-warning-border)] p-3 text-sm"
          >
            <TriangleAlert className="h-4 w-4 shrink-0" />
            <div className="space-y-1">
              {stalled && (
                <p id="location-operation-stalled-guidance">
                  {t("move.stalledGuidance")}
                </p>
              )}
              {operation.detail &&
                detailLines(operation.detail).map((line, index) => (
                  <p key={index}>{line}</p>
                ))}
              {guidanceKey && (
                <p
                  id="location-operation-guidance"
                  className="text-muted-foreground"
                  data-reason-code={operation.reasonCode ?? undefined}
                >
                  {t(guidanceKey)}
                </p>
              )}
              {retrySelection &&
                retrySelection.catalogBlockedTitleIds.length > 0 && (
                  <p
                    id="location-operation-plan-again-blocked"
                    className="text-muted-foreground"
                  >
                    {t("move.planAgainCatalogBlocked", {
                      count: retrySelection.catalogBlockedTitleIds.length,
                    })}
                  </p>
                )}
            </div>
          </div>
        )}
        {summary.error && (
          <p className="text-xs text-[var(--scry-danger-text)]">
            {t("move.operationLoadFailed")}
          </p>
        )}
      </CardContent>
      <MoveTitlesDialog
        key={planAgain?.titles.map((title) => title.id).join(",") ?? "closed"}
        open={planAgain !== null}
        onOpenChange={(nextOpen) => {
          if (!nextOpen) setPlanAgain(null);
        }}
        titles={planAgain?.titles ?? []}
        libraries={planAgain?.libraries ?? []}
        initialRootId={retrySelection?.destinationRootId ?? null}
      />
    </Card>
  );
}

function TitleFiles({
  operationId,
  titleId,
}: {
  operationId: string;
  titleId: string;
}) {
  const t = useTranslate();
  const [page, setPage] = React.useState(0);
  const { snapshot, error, connected } = useLocationTransfer(
    operationId,
    page,
    true,
    0,
    titleId,
  );
  const files = snapshot?.files ?? [];
  const total = toCount(snapshot?.totalCount);
  return (
    <div className="space-y-2">
      {error && <p role="alert">{t("move.operationLoadFailed")}</p>}
      {!snapshot ? (
        <p>{t("move.operationLoading")}</p>
      ) : (
        <>
          {!connected &&
            !isTerminalOperationState(snapshot.operation.state) && (
              <p>{t("move.transferDisconnected")}</p>
            )}
          <Table
            density="dense"
            layout="fixed"
            wrapperClassName="rounded-lg border border-border"
          >
            <TableHeader>
              <TableRow>
                <TableHead className="w-[45%]">
                  {t("move.transferFilePath")}
                </TableHead>
                <TableHead>{t("move.transferColumnStatus")}</TableHead>
                <TableHead>{t("move.transferCopyProgress")}</TableHead>
                <TableHead>{t("move.transferVerifyProgress")}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {files.map((file) => (
                <React.Fragment key={file.destinationPath || file.sourcePath}>
                  <TableRow>
                    <TableCell>
                      <FilePath
                        path={file.destinationPath || file.sourcePath}
                      />
                      <span className="text-muted-foreground">
                        {file.state === "DUPLICATE"
                          ? "—"
                          : formatByteCount(toCount(file.sizeBytes))}
                      </span>
                    </TableCell>
                    <TableCell>
                      <Badge
                        tone={
                          file.state === "FAILED" || file.state === "BLOCKED"
                            ? "warning"
                            : "neutral"
                        }
                      >
                        {t(`move.transferFileState.${file.state}`)}
                      </Badge>
                    </TableCell>
                    <TableCell>
                      {file.state === "DUPLICATE" ? (
                        "—"
                      ) : file.state === "COMPARING" ? (
                        <span className="text-muted-foreground">
                          {t("move.transferPlacementAwaitingComparison")}
                        </span>
                      ) : (
                        <FileBytes
                          done={toCount(file.copyBytes)}
                          total={toCount(file.sizeBytes)}
                          complete={file.state === "DONE"}
                          label={t("move.transferCopyProgress")}
                        />
                      )}
                    </TableCell>
                    <TableCell>
                      {file.state === "VERIFYING" || file.state === "COMPARING" ? (
                        <FileBytes
                          done={toCount(file.verificationBytes)}
                          total={toCount(file.verificationTotalBytes ?? file.sizeBytes)}
                          complete={false}
                          label={t("move.transferVerifyProgress")}
                          showBytes={false}
                        />
                      ) : file.state === "DONE" ? (
                        t("move.transferFileState.DONE")
                      ) : (
                        "—"
                      )}
                    </TableCell>
                  </TableRow>
                  {file.detail && (
                    <TableRow>
                      <TableCell
                        colSpan={4}
                        className="break-words text-muted-foreground"
                      >
                        {file.detail}
                      </TableCell>
                    </TableRow>
                  )}
                </React.Fragment>
              ))}
            </TableBody>
          </Table>
          {!files.length && <p>{t("move.transferNoFiles")}</p>}
          {total > 50 && (
            <div className="flex items-center justify-between">
              <span>
                {t("move.transferRange", {
                  from: files.length ? page * 50 + 1 : 0,
                  to: files.length ? page * 50 + files.length : 0,
                  total,
                })}
              </span>
              <div className="flex gap-2">
                <Button
                  size="sm"
                  variant="outline"
                  disabled={page === 0}
                  onClick={() => setPage(page - 1)}
                >
                  {t("move.transferPrevious")}
                </Button>
                <Button
                  size="sm"
                  variant="outline"
                  disabled={!snapshot.hasMore}
                  onClick={() => setPage(page + 1)}
                >
                  {t("move.transferNext")}
                </Button>
              </div>
            </div>
          )}
        </>
      )}
    </div>
  );
}

function FileBytes({
  done,
  total,
  complete,
  label,
  showBytes = true,
}: {
  done: number;
  total: number;
  complete: boolean;
  label: string;
  showBytes?: boolean;
}) {
  const progress = complete
    ? 100
    : total > 0
      ? Math.min(100, (done / total) * 100)
      : 0;
  return (
    <div className="space-y-1 tabular-nums">
      {showBytes && (
        <span>
          {formatByteCount(done)} / {formatByteCount(total)}
        </span>
      )}
      <Progress
        value={progress}
        className="h-1.5"
        aria-label={label}
        aria-valuetext={`${progress.toFixed(1)}%`}
      />
    </div>
  );
}

function FilePath({ path }: { path: string }) {
  const parts = splitTransferPath(path);
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <span
          tabIndex={0}
          aria-label={path}
          className="flex min-w-0 font-[var(--font-code)]"
        >
          <span
            aria-hidden="true"
            className="min-w-0 truncate"
            style={{ direction: "rtl", textAlign: "left" }}
          >
            {parts.directory}
          </span>
          <span aria-hidden="true" className="max-w-full shrink-0 truncate">
            {parts.filename}
          </span>
        </span>
      </TooltipTrigger>
      <TooltipContent className="max-w-xl break-all">{path}</TooltipContent>
    </Tooltip>
  );
}

function TitleRow({
  operationId,
  row,
  posterUrl,
  guidanceKey,
  onPlanAgain,
}: {
  operationId: string;
  row: TransferTitle;
  posterUrl?: string | null;
  /** Translation key for why this title stopped; null when it has no code. */
  guidanceKey?: string | null;
  /** Plan this one title again; null when the title is not up for it. */
  onPlanAgain?: (() => void) | null;
}) {
  const t = useTranslate();
  const client = useClient();
  const [expanded, setExpanded] = React.useState(false);
  const [detail, setDetail] = React.useState<string | null>(null);
  const [failed, setFailed] = React.useState(false);
  // A completed title may still have something to say — the merge the plan
  // promised, a companion kept under a new name — so its detail is fetched
  // too, and shown as a statement rather than as a warning.
  const showsDetail = row.hasException || row.state === "COMPLETED";
  React.useEffect(() => {
    if (!expanded || !showsDetail) return;
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
  }, [client, expanded, operationId, row.titleId, row.state, showsDetail]);
  const path = row.currentFile ? splitTransferPath(row.currentFile) : null;
  return (
    <React.Fragment>
      <TableRow
        id={`location-operation-checkpoint-${row.titleId}`}
        className="align-top"
      >
        <TableCell>
          <div className="flex items-start gap-1">
            {
              <button
                aria-label={t("move.transferFilesForTitle", {
                  title: row.name,
                })}
                className="shrink-0 rounded p-1 hover:bg-muted"
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
            }
            <span className="flex min-w-0 items-center gap-3">
              <span className="h-[54px] w-9 shrink-0 overflow-hidden rounded-[6px] border border-[var(--scry-border2)] bg-[var(--scry-soft)]">
                <TitlePosterSlot
                  src={selectPosterVariantUrl(posterUrl, "w70")}
                  alt=""
                  className="h-full w-full object-cover"
                  emptyLabel={t("label.noArt")}
                  fallbackTitle={row.name}
                  fallbackShowText={false}
                  loading="lazy"
                />
              </span>
              <span className="min-w-0 break-words font-medium text-[var(--scry-ink3)]">
                {row.name}
              </span>
            </span>
          </div>
        </TableCell>
        <TableCell>
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
        </TableCell>
        <TableCell className="tabular-nums">
          {toCount(row.filesDone)}/{toCount(row.filesTotal)}
        </TableCell>
        <TableCell className="min-w-0">
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
        </TableCell>
        <TableCell className="tabular-nums">
          {transferTitleProgress(row).toFixed(1)}%
          <Progress
            className="my-1 h-1.5"
            value={transferTitleProgress(row)}
            aria-label={row.name}
          />
        </TableCell>
      </TableRow>
      {expanded && (
        <TableRow id={`transfer-detail-${row.titleId}`}>
          <TableCell
            colSpan={5}
            className="break-words bg-muted/20 p-3 text-xs"
          >
            {row.hasException && (
              <div className="mb-3 space-y-1 text-[var(--scry-warning-text)]">
                {detail ? (
                  detailLines(detail).map((line, index) => (
                    <p key={index}>{line}</p>
                  ))
                ) : (
                  <p>
                    {t(
                      failed
                        ? "move.operationLoadFailed"
                        : "move.transferDetailPending",
                    )}
                  </p>
                )}
              </div>
            )}
            {!row.hasException && detail && (
              <div className="mb-3 space-y-1 text-muted-foreground">
                {detailLines(detail).map((line, index) => (
                  <p key={index}>{line}</p>
                ))}
              </div>
            )}
            {guidanceKey && (
              <p
                id={`transfer-guidance-${row.titleId}`}
                className="mb-3 text-muted-foreground"
              >
                {t(guidanceKey)}
              </p>
            )}
            {onPlanAgain && (
              <Button
                id={`location-operation-plan-again-${row.titleId}`}
                variant="outline"
                size="sm"
                className="mb-3"
                onClick={onPlanAgain}
              >
                {t("move.planAgainTitleAction")}
              </Button>
            )}
            <TitleFiles operationId={operationId} titleId={row.titleId} />
          </TableCell>
        </TableRow>
      )}
    </React.Fragment>
  );
}
