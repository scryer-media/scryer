import * as React from "react";
import { Link } from "react-router";
import { useClient } from "urql";
import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { LoadingMark } from "@/components/common/loading-mark";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { SingleSelectField } from "@/components/ui/select";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { useTranslate } from "@/lib/context/translate-context";
import {
  ERROR_REASON_KEYS,
  userFacingGraphQlErrorMessage,
} from "@/lib/graphql/error-message";
import { toCount } from "@/lib/location-operations";
import { formatByteCount } from "@/lib/utils/activity-utils";
import {
  canApplyConflictPreview,
  previewPendingImportConflictsMutation,
  resolvePendingImportConflictsMutation,
  pendingImportConflictTaskQuery,
  type ConflictPolicy,
  type ConflictLeftovers,
  type ConflictPreview,
  type ConflictTask,
} from "@/lib/pending-import-conflicts";

type Props = {
  pendingImportIds: string[];
  initialPolicy: ConflictPolicy;
  onClose: () => void;
  onStarted: (task: ConflictTask) => void;
};

export function PendingImportConflictDialog({
  pendingImportIds,
  initialPolicy,
  onClose,
  onStarted,
}: Props) {
  const client = useClient();
  const t = useTranslate();
  const [policy, setPolicy] = React.useState(initialPolicy);
  const [leftovers, setLeftovers] =
    React.useState<ConflictLeftovers>("PRESERVE_ALL");
  const [preview, setPreview] = React.useState<ConflictPreview | null>(null);
  const [loading, setLoading] = React.useState(false);
  const [applying, setApplying] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const [refresh, setRefresh] = React.useState(0);
  const [permanentConfirm, setPermanentConfirm] = React.useState(false);
  const input = React.useMemo(
    () => ({ pendingImportIds, policy, leftovers }),
    [pendingImportIds, policy, leftovers],
  );

  React.useEffect(() => {
    let active = true;
    setLoading(true);
    setPreview(null);
    setError(null);
    setPermanentConfirm(false);
    client
      .mutation<{ previewPendingImportConflicts: ConflictPreview }>(
        previewPendingImportConflictsMutation,
        { input },
      )
      .toPromise()
      .then((result) => {
        if (!active) return;
        if (result.error) throw result.error;
        if (!result.data?.previewPendingImportConflicts) throw new Error();
        setPreview(result.data.previewPendingImportConflicts);
      })
      .catch((failure) => {
        if (active)
          setError(
            userFacingGraphQlErrorMessage(
              failure,
              t("pendingImports.conflict.previewFailed"),
              t,
            ),
          );
      })
      .finally(() => {
        if (active) setLoading(false);
      });
    return () => {
      active = false;
    };
  }, [client, input, refresh, t]);

  async function apply(confirmPermanentDelete = false) {
    if (!preview || !canApplyConflictPreview(preview, loading, applying))
      return;
    if (
      preview.requiresPermanentDeleteConfirmation &&
      !confirmPermanentDelete
    ) {
      setPermanentConfirm(true);
      return;
    }
    setApplying(true);
    setError(null);
    try {
      const result = await client
        .mutation<{ resolvePendingImportConflicts: ConflictTask }>(
          resolvePendingImportConflictsMutation,
          {
            input: {
              ...input,
              planFingerprint: preview.planFingerprint,
              confirmPermanentDelete,
            },
          },
        )
        .toPromise();
      if (result.error) throw result.error;
      if (!result.data?.resolvePendingImportConflicts) throw new Error();
      onStarted(result.data.resolvePendingImportConflicts);
      onClose();
    } catch (failure) {
      setError(
        userFacingGraphQlErrorMessage(
          failure,
          t("pendingImports.conflict.applyFailed"),
          t,
        ),
      );
      setPermanentConfirm(false);
      setPreview(null);
    } finally {
      setApplying(false);
    }
  }

  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open && !applying) onClose();
      }}
    >
      <DialogContent
        id="pending-import-conflict-dialog"
        className="max-h-[90dvh] overflow-y-auto sm:max-w-5xl"
        aria-describedby={undefined}
        showCloseButton={!applying}
        onEscapeKeyDown={(event) => {
          if (applying) event.preventDefault();
        }}
        onInteractOutside={(event) => {
          if (applying) event.preventDefault();
        }}
      >
        <DialogHeader>
          <DialogTitle className="flex flex-wrap items-center gap-2">
            {t("pendingImports.conflict.title")}
            <Badge
              id="pending-import-conflict-beta"
              tone="warning"
              className="text-[11px]"
            >
              {t("settings.pluginBeta")}
            </Badge>
          </DialogTitle>
        </DialogHeader>
        <div className="grid gap-4 sm:grid-cols-2">
          <SingleSelectField
            id="pending-import-conflict-policy"
            label={t("pendingImports.conflict.policy")}
            value={policy}
            disabled={applying}
            onValueChange={(value) => {
              if (value === "ADOPT_ALL" || value === "KEEP_ONE")
                setPolicy(value);
            }}
            options={[
              {
                value: "ADOPT_ALL",
                label: t("pendingImports.conflict.adoptAll"),
              },
              {
                value: "KEEP_ONE",
                label: t("pendingImports.conflict.keepOne"),
              },
            ]}
          />
          <SingleSelectField
            id="pending-import-conflict-leftovers"
            label={t("pendingImports.conflict.leftovers")}
            value={leftovers}
            disabled={applying}
            onValueChange={(value) => {
              if (value === "PRESERVE_ALL" || value === "MEDIA_ONLY")
                setLeftovers(value);
            }}
            options={[
              {
                value: "PRESERVE_ALL",
                label: t("pendingImports.conflict.preserveAll"),
              },
              {
                value: "MEDIA_ONLY",
                label: t("pendingImports.conflict.mediaOnly"),
              },
            ]}
          />
        </div>
        {loading && (
          <p role="status" className="flex items-center gap-2">
            <LoadingMark className="h-4 w-4" />
            {t("label.loading")}
          </p>
        )}
        {error && (
          <p role="alert" className="text-sm text-destructive">
            {error}
          </p>
        )}
        {preview && (
          <>
            {preview.failedCount > 0 && (
              <p className="text-sm text-muted-foreground">
                {t("pendingImports.conflict.actionableCount", {
                  actionable: preview.actionableCount,
                  failed: preview.failedCount,
                })}
              </p>
            )}
            {preview.sampled && (
              <p className="text-sm">{t("pendingImports.conflict.sampled")}</p>
            )}
            <Table wrapperClassName="overflow-x-auto" className="min-w-[640px]">
              <TableHeader>
                <TableRow>
                  {[
                    "titleColumn",
                    "primary",
                    "moveBytes",
                    "recycleBytes",
                    "leftoverFiles",
                  ].map((column) => (
                    <TableHead key={column}>
                      {t(`pendingImports.conflict.${column}`)}
                    </TableHead>
                  ))}
                </TableRow>
              </TableHeader>
              <TableBody>
                {preview.rows.map((row) => (
                  <TableRow
                    key={row.pendingImportId}
                    id={`pending-import-conflict-row-${row.pendingImportId}`}
                  >
                    <TableCell>
                      {row.titleName}
                      {row.errorReason && (
                        <p role="alert" className="text-destructive">
                          {ERROR_REASON_KEYS[row.errorReason]
                            ? t(ERROR_REASON_KEYS[row.errorReason])
                            : row.errorMessage}
                        </p>
                      )}
                    </TableCell>
                    <TableCell
                      id={`pending-import-conflict-primary-${row.pendingImportId}`}
                      className="max-w-72 break-all font-mono text-xs"
                    >
                      {row.errorReason ? "—" : row.primaryPath}
                    </TableCell>
                    <TableCell>
                      {row.errorReason
                        ? "—"
                        : formatByteCount(toCount(row.moveBytes))}
                    </TableCell>
                    <TableCell>
                      {row.errorReason
                        ? "—"
                        : t("pendingImports.conflict.filesBytes", {
                            count: row.recycleCount,
                            bytes: formatByteCount(toCount(row.recycleBytes)),
                          })}
                      <PathList
                        paths={row.recyclePaths}
                        sampled={row.filesSampled}
                      />
                    </TableCell>
                    <TableCell>
                      {row.errorReason
                        ? "—"
                        : t("pendingImports.conflict.filesBytes", {
                            count: row.leftoverCount,
                            bytes: formatByteCount(toCount(row.leftoverBytes)),
                          })}
                      <PathList
                        paths={row.leftoverPaths}
                        sampled={row.filesSampled}
                      />
                      {row.collisionRenameSamples.length > 0 && (
                        <details className="mt-2 text-xs">
                          <summary>
                            {t("pendingImports.conflict.renamedFiles", {
                              count: row.collisionRenames,
                            })}
                          </summary>
                          <ul className="space-y-2">
                            {row.collisionRenameSamples.map((rename) => (
                              <li
                                key={rename.sourcePath}
                                className="break-all font-mono"
                              >
                                <span>{rename.sourcePath}</span>
                                <span aria-hidden="true"> → </span>
                                <span>{rename.destinationPath}</span>
                              </li>
                            ))}
                          </ul>
                          {row.filesSampled && (
                            <p>{t("pendingImports.conflict.sampledFiles")}</p>
                          )}
                        </details>
                      )}
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </>
        )}
        <DialogFooter>
          <Button variant="outline" disabled={applying} onClick={onClose}>
            {t("label.cancel")}
          </Button>
          <Button
            id="pending-import-conflict-preview"
            variant="outline"
            disabled={loading || applying}
            onClick={() => setRefresh((value) => value + 1)}
          >
            {t("pendingImports.conflict.preview")}
          </Button>
          <Button
            id="pending-import-conflict-apply"
            disabled={!canApplyConflictPreview(preview, loading, applying)}
            onClick={() => void apply()}
          >
            {applying && <LoadingMark className="mr-2 h-4 w-4" />}
            {t("pendingImports.conflict.apply")}
          </Button>
        </DialogFooter>
        <ConfirmDialog
          open={permanentConfirm}
          contentId="pending-import-conflict-permanent-dialog"
          confirmButtonId="pending-import-conflict-permanent-confirm"
          title={t("pendingImports.conflict.permanentTitle")}
          description={t("pendingImports.conflict.permanentDescription", {
            count: preview?.permanentDeleteCount ?? 0,
            bytes: formatByteCount(toCount(preview?.permanentDeleteBytes)),
          })}
          confirmLabel={t("pendingImports.conflict.permanentConfirm")}
          cancelLabel={t("label.cancel")}
          isBusy={applying}
          onConfirm={() => apply(true)}
          onCancel={() => {
            if (!applying) setPermanentConfirm(false);
          }}
        >
          <PathList
            paths={preview?.permanentDeletePaths ?? []}
            sampled={preview?.permanentDeleteSampled ?? false}
          />
        </ConfirmDialog>
      </DialogContent>
    </Dialog>
  );
}

export function PendingImportConflictProgress({
  initialTask,
  onChanged,
  onDismiss,
}: {
  initialTask: ConflictTask;
  onChanged: (task: ConflictTask) => void;
  onDismiss: () => void;
}) {
  const client = useClient();
  const t = useTranslate();
  const [task, setTask] = React.useState(initialTask);
  const [failed, setFailed] = React.useState(false);
  const [missing, setMissing] = React.useState(false);
  React.useEffect(() => {
    let active = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    async function refresh() {
      try {
        const result = await client
          .query<{ pendingImportConflictTask: ConflictTask | null }>(
            pendingImportConflictTaskQuery,
            { jobRunId: initialTask.jobRunId },
            { requestPolicy: "network-only" },
          )
          .toPromise();
        if (!active) return;
        const next = result.data?.pendingImportConflictTask;
        if (!result.error && !next) {
          setMissing(true);
          return;
        }
        if (result.error) {
          setFailed(true);
        } else if (next) {
          setTask(next);
          setFailed(false);
          if (next.completed) {
            onChanged(next);
            return;
          }
        }
      } catch {
        if (active) setFailed(true);
      }
      if (active) timer = setTimeout(() => void refresh(), 2000);
    }
    if (!initialTask.completed) void refresh();
    return () => {
      active = false;
      clearTimeout(timer);
    };
  }, [client, initialTask, onChanged]);
  return (
    <div
      id="pending-import-conflict-progress"
      role="status"
      className="space-y-2 rounded-lg border border-border p-3 text-sm"
    >
      {missing ? (
        <p>{t("pendingImports.conflict.taskMissing")}</p>
      ) : task.totalCount === 0 ? (
        <p>{t("label.loading")}</p>
      ) : (
        <p>
          {t(
            task.completed
              ? "pendingImports.conflict.completed"
              : "pendingImports.conflict.progress",
            {
              settled: task.settledCount,
              total: task.totalCount,
              failed: task.failedCount,
            },
          )}
        </p>
      )}
      {failed && <p>{t("pendingImports.conflict.progressUnavailable")}</p>}
      <Button
        id="pending-import-conflict-progress-dismiss"
        size="sm"
        variant="ghost"
        onClick={onDismiss}
      >
        {t("pendingImports.conflict.hideProgress")}
      </Button>
      {task.operationIds.map((id) => (
        <Link
          key={id}
          to={`/activity?operation=${encodeURIComponent(id)}`}
          className="mr-3 underline"
        >
          {t("pendingImports.conflict.operation", { id })}
        </Link>
      ))}
    </div>
  );
}

function PathList({ paths, sampled }: { paths: string[]; sampled: boolean }) {
  const t = useTranslate();
  if (!paths.length) return null;
  return (
    <details className="mt-2 text-xs">
      <summary>{t("pendingImports.conflict.showFiles")}</summary>
      <ul className="mt-1 space-y-1">
        {paths.map((path) => (
          <li key={path} className="break-all font-mono">
            {path}
          </li>
        ))}
      </ul>
      {sampled && <p>{t("pendingImports.conflict.sampledFiles")}</p>}
    </details>
  );
}
