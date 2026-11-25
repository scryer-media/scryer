import { useState } from "react";
import { Ban, RefreshCw } from "lucide-react";
import { useClient, useQuery } from "urql";

import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { DeletePreviewSummary } from "@/components/common/delete-preview-summary";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/dialog";
import { useUiDateTimeFormat } from "@/lib/context/ui-settings-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useTranslate } from "@/lib/context/translate-context";
import {
  approveMediaIssueMutation,
  approveMediaIssueRecycleSearchMutation,
  clearLibraryContentBlockMutation,
  rejectMediaIssueMutation,
} from "@/lib/graphql/mutations";
import {
  deleteMediaFilePreviewQuery,
  libraryContentBlocksQuery,
  mediaIssueRecycleSettingsQuery,
  mediaIssuesQuery,
  mediaIssueTitleQuery,
} from "@/lib/graphql/queries";
import {
  graphQlErrorExtension,
  hasGraphQlErrorReason,
  userFacingGraphQlErrorMessage,
} from "@/lib/graphql/error-message";
import { useDeletePreview } from "@/lib/hooks/use-delete-preview";
import type {
  LibraryContentBlock,
  MediaIssue,
  MediaIssueExecutionStatus,
  MediaIssueRemediation,
  MediaIssueStatus,
} from "@/lib/types/media-issues";
import type { LibraryRecord } from "@/lib/types/titles";
import { formatUiDateTime } from "@/lib/utils/date-format";
import { deleteErrorMessage } from "@/lib/utils/delete-error-message";
import {
  confirmedContentBlockRemoval,
  mediaIssueRecycleApprovalVariables,
} from "@/lib/utils/media-issue-review";
import { formatMediaFileSize } from "@/lib/utils/media-info-format";

const PAGE_SIZE = 25;
const BLOCK_LIMIT = 100;
const selectClass =
  "h-9 max-w-full rounded-md border border-input bg-background px-2 text-sm focus-visible:outline-2 focus-visible:outline-ring";

const reviewStatusKeys = {
  PENDING: "mediaIssue.pending",
  APPROVED: "mediaIssue.approved",
  REJECTED: "mediaIssue.rejected",
  CANCELED: "mediaIssue.canceled",
} as const satisfies Record<MediaIssueStatus, string>;

const executionKeys = {
  NONE: "mediaIssue.notStarted",
  BLOCKED: "mediaIssue.blocked",
  RECYCLING: "mediaIssue.recycling",
  RECYCLE_FAILED: "mediaIssue.recycleFailed",
  NO_ELIGIBLE_RELEASE: "mediaIssue.noEligibleRelease",
  DOWNLOAD_QUEUED: "mediaIssue.downloadQueued",
  REPLACEMENT_IMPORTED: "mediaIssue.replacementImported",
  SEARCHING: "mediaIssue.searching",
  COMPLETE: "mediaIssue.complete",
  FAILED: "mediaIssue.failed",
} as const satisfies Record<MediaIssueExecutionStatus, string>;

const remediationKeys = {
  MARK_BAD: "mediaFile.markBadOnly",
  RECYCLE_AND_SEARCH: "mediaFile.markBadRecycleSearch",
} as const satisfies Record<MediaIssueRemediation, string>;

const mediaIssueRefusalKeys: Readonly<Record<string, string>> = {
  MEDIA_REMEDIATION_NOT_AVAILABLE: "mediaIssue.remediationUnavailable",
  MEDIA_REGISTERED_MEDIA_PATH_CHANGED_BEFORE_INSPECTION:
    "mediaIssue.reportedFileChanged",
  MEDIA_SOURCE_CHANGED_DURING_INSPECTION: "mediaIssue.reportedFileChanged",
  MEDIA_RECYCLE_FAILED: "mediaIssue.recycleFailed",
};

function mediaIssueRefusalKey(reason: string | null): string | undefined {
  return reason && Object.hasOwn(mediaIssueRefusalKeys, reason)
    ? mediaIssueRefusalKeys[reason]
    : undefined;
}

type ReviewAction = "approve" | "recycle" | "reject";

export function MediaIssueReview({
  libraries,
}: {
  libraries: Pick<LibraryRecord, "id" | "name">[];
}) {
  const t = useTranslate();
  const [open, setOpen] = useState(false);
  const [selectedLibraryId, setSelectedLibraryId] = useState("");
  const libraryId =
    libraries.find((library) => library.id === selectedLibraryId)?.id ??
    libraries[0]?.id;

  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogTrigger asChild>
        <Button variant="outline">
          <Ban aria-hidden="true" />
          {t("mediaIssue.reviewTitle")}
        </Button>
      </DialogTrigger>
      <DialogContent className="max-h-[calc(100dvh-2rem)] overflow-y-auto sm:max-w-3xl">
        <DialogHeader className="pr-7">
          <DialogTitle className="font-display text-xl">
            {t("mediaIssue.reviewTitle")}
          </DialogTitle>
          <DialogDescription>{t("mediaIssue.reviewHint")}</DialogDescription>
        </DialogHeader>
        {open && libraryId ? (
          <>
            <label className="grid gap-1.5 text-sm font-medium sm:w-fit">
              {t("mediaIssue.library")}
              <select
                className={selectClass}
                value={libraryId}
                onChange={(event) => setSelectedLibraryId(event.target.value)}
              >
                {libraries.map((library) => (
                  <option key={library.id} value={library.id}>
                    {library.name}
                  </option>
                ))}
              </select>
            </label>
            <LibraryIssueReview key={libraryId} libraryId={libraryId} />
          </>
        ) : null}
      </DialogContent>
    </Dialog>
  );
}

function LibraryIssueReview({ libraryId }: { libraryId: string }) {
  const t = useTranslate();
  const dateFormat = useUiDateTimeFormat();
  const [surface, setSurface] = useState<"reports" | "blocks">("reports");
  const [pendingOnly, setPendingOnly] = useState(true);
  const [page, setPage] = useState(0);
  const [selectedReportId, setSelectedReportId] = useState<string | null>(null);
  const [{ data, fetching, error }, refresh] = useQuery<{
    mediaIssues: MediaIssue[];
  }>({
    query: mediaIssuesQuery,
    variables: {
      libraryId,
      status: pendingOnly ? "PENDING" : null,
      limit: PAGE_SIZE + 1,
      offset: page * PAGE_SIZE,
    },
    pause: surface !== "reports",
    requestPolicy: "network-only",
  });
  const issues = data?.mediaIssues?.slice(0, PAGE_SIZE) ?? [];

  function reload() {
    refresh({ requestPolicy: "network-only" });
  }

  return (
    <div className="space-y-3">
      <div className="flex flex-wrap gap-2 border-b border-border pb-3">
        <Button
          size="sm"
          variant={surface === "reports" ? "secondary" : "ghost"}
          aria-pressed={surface === "reports"}
          onClick={() => setSurface("reports")}
        >
          {t("mediaIssue.reports")}
        </Button>
        <Button
          size="sm"
          variant={surface === "blocks" ? "secondary" : "ghost"}
          aria-pressed={surface === "blocks"}
          onClick={() => setSurface("blocks")}
        >
          {t("mediaFile.confirmedContentBlocks")}
        </Button>
      </div>
      {surface === "blocks" ? (
        <LibraryContentBlocks libraryId={libraryId} />
      ) : (
        <>
          <div className="flex flex-wrap items-center gap-2">
            <select
              className={selectClass}
              aria-label={t("mediaIssue.reviewStatus")}
              value={pendingOnly ? "pending" : "all"}
              onChange={(event) => {
                setPendingOnly(event.target.value === "pending");
                setPage(0);
                setSelectedReportId(null);
              }}
            >
              <option value="pending">{t("mediaIssue.pending")}</option>
              <option value="all">{t("mediaIssue.allReports")}</option>
            </select>
            <RefreshButton busy={fetching} onClick={reload} />
          </div>
          {error ? (
            <ReviewError
              message={userFacingGraphQlErrorMessage(
                error,
                t("mediaIssue.reportsFailed"),
                t,
              )}
            />
          ) : null}
          {fetching && issues.length === 0 ? (
            <p role="status" className="text-sm text-muted-foreground">
              {t("label.loading")}
            </p>
          ) : !error && issues.length === 0 ? (
            <p className="text-sm text-muted-foreground">
              {t(pendingOnly ? "mediaIssue.noPending" : "mediaIssue.noReports")}
            </p>
          ) : null}
          <ul className="divide-y divide-border rounded-md border border-border empty:hidden">
            {issues.map((issue) => (
              <li key={issue.id} className="p-3">
                <div className="flex flex-wrap items-center justify-between gap-2">
                  <div className="space-y-1">
                    <p className="text-sm font-semibold">
                      {t(remediationKeys[issue.remediation])}
                    </p>
                    <p className="text-xs text-muted-foreground">
                      {formatUiDateTime(issue.createdAt, dateFormat)}
                      {" · "}
                      {t(reviewStatusKeys[issue.status])}
                      {" · "}
                      {t(executionKeys[issue.executionStatus])}
                    </p>
                  </div>
                  <Button
                    size="sm"
                    variant="outline"
                    aria-expanded={selectedReportId === issue.id}
                    onClick={() =>
                      setSelectedReportId((current) =>
                        current === issue.id ? null : issue.id,
                      )
                    }
                  >
                    {t(
                      selectedReportId === issue.id
                        ? "mediaIssue.hideReport"
                        : "mediaIssue.viewReport",
                    )}
                  </Button>
                </div>
                {selectedReportId === issue.id ? (
                  <ReportDetails
                    issue={issue}
                    onReviewed={() => {
                      if (pendingOnly && issues.length === 1 && page > 0) {
                        setPage(page - 1);
                      } else {
                        reload();
                      }
                    }}
                    onFailed={reload}
                  />
                ) : null}
              </li>
            ))}
          </ul>
          {page > 0 || (data?.mediaIssues?.length ?? 0) > PAGE_SIZE ? (
            <div className="flex justify-between gap-2">
              <Button
                size="sm"
                variant="outline"
                disabled={fetching || page === 0}
                onClick={() => {
                  setSelectedReportId(null);
                  setPage(page - 1);
                }}
              >
                {t("mediaIssue.previousPage")}
              </Button>
              <Button
                size="sm"
                variant="outline"
                disabled={
                  fetching || (data?.mediaIssues?.length ?? 0) <= PAGE_SIZE
                }
                onClick={() => {
                  setSelectedReportId(null);
                  setPage(page + 1);
                }}
              >
                {t("mediaIssue.nextPage")}
              </Button>
            </div>
          ) : null}
        </>
      )}
    </div>
  );
}

function ReportDetails({
  issue,
  onReviewed,
  onFailed,
}: {
  issue: MediaIssue;
  onReviewed: () => void;
  onFailed: () => void;
}) {
  const t = useTranslate();
  const [action, setAction] = useState<ReviewAction | null>(null);
  const executionErrorKey = mediaIssueRefusalKey(issue.executionErrorCode);
  const [{ data }] = useQuery<{ title: { name: string } | null }>({
    query: mediaIssueTitleQuery,
    variables: { titleId: issue.titleId },
  });

  return (
    <div className="mt-3 space-y-3 border-l-2 border-[var(--scry-baccent)] pl-3">
      <div className="space-y-1">
        <p className="break-words text-sm font-semibold">
          {data?.title?.name ?? t("mediaIssue.titleId", { id: issue.titleId })}
        </p>
        <dl className="grid gap-x-4 gap-y-1 text-xs sm:grid-cols-[auto_1fr]">
          <dt className="text-muted-foreground">{t("mediaIssue.file")}</dt>
          <dd className="break-all font-mono">{issue.mediaFileId}</dd>
          <dt className="text-muted-foreground">{t("mediaIssue.episodes")}</dt>
          <dd>
            {issue.affectedEpisodeIds.length > 0 ? (
              <ul className="flex flex-wrap gap-1">
                {issue.affectedEpisodeIds.map((id) => (
                  <li
                    key={id}
                    className="break-all rounded bg-muted px-1.5 py-0.5 font-mono"
                  >
                    {id}
                  </li>
                ))}
              </ul>
            ) : (
              t("mediaIssue.noEpisodeAssociations")
            )}
          </dd>
          {issue.authorizedRemediation ? (
            <>
              <dt className="text-muted-foreground">
                {t("mediaIssue.approvedAction")}
              </dt>
              <dd>{t(remediationKeys[issue.authorizedRemediation])}</dd>
            </>
          ) : null}
          {issue.executionErrorCode ? (
            <>
              <dt className="text-muted-foreground">
                {t("mediaIssue.executionError")}
              </dt>
              <dd className="break-all font-mono text-destructive">
                {executionErrorKey
                  ? t(executionErrorKey)
                  : issue.executionErrorCode}
              </dd>
            </>
          ) : null}
        </dl>
      </div>
      {issue.details ? (
        <p className="whitespace-pre-wrap break-words rounded bg-muted/60 p-2 text-sm">
          {issue.details}
        </p>
      ) : null}
      {issue.status === "PENDING" ? (
        <div className="flex flex-wrap gap-2">
          <Button size="sm" onClick={() => setAction("approve")}>
            {t("mediaIssue.approveMarkOnly")}
          </Button>
          {issue.remediation === "RECYCLE_AND_SEARCH" ? (
            <Button
              size="sm"
              variant="outline"
              onClick={() => setAction("recycle")}
            >
              {t("mediaIssue.reviewRecycle")}
            </Button>
          ) : null}
          <Button size="sm" variant="ghost" onClick={() => setAction("reject")}>
            {t("mediaIssue.reject")}
          </Button>
        </div>
      ) : null}
      {action ? (
        <ReviewConfirmation
          key={action}
          action={action}
          issue={issue}
          onClose={() => setAction(null)}
          onReviewed={onReviewed}
          onFailed={onFailed}
        />
      ) : null}
    </div>
  );
}

function ReviewConfirmation({
  action,
  issue,
  onClose,
  onReviewed,
  onFailed,
}: {
  action: ReviewAction;
  issue: MediaIssue;
  onClose: () => void;
  onReviewed: () => void;
  onFailed: () => void;
}) {
  const t = useTranslate();
  const client = useClient();
  const setGlobalStatus = useGlobalStatus();
  const [typed, setTyped] = useState("");
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState<string | null>(null);
  const recycle = action === "recycle";
  const [{ data: settings, fetching: settingsLoading, error: settingsError }] =
    useQuery<{
      recycleBinSettings: { enabled: boolean; validationError: string | null };
    }>({
      query: mediaIssueRecycleSettingsQuery,
      pause: !recycle,
      requestPolicy: "network-only",
    });
  const canRecycle =
    !settingsLoading &&
    !settingsError &&
    settings?.recycleBinSettings.enabled === true &&
    !settings.recycleBinSettings.validationError;
  const { preview, loading, error } = useDeletePreview(
    deleteMediaFilePreviewQuery,
    "deleteMediaFilePreview",
    recycle ? { fileId: issue.mediaFileId } : null,
    recycle && canRecycle,
  );
  const recycleVariables = mediaIssueRecycleApprovalVariables(
    issue,
    loading || error ? null : preview,
    typed,
    canRecycle,
  );
  const title = t(
    action === "recycle"
      ? "mediaIssue.approveRecycle"
      : action === "approve"
        ? "mediaIssue.approveMarkOnly"
        : "mediaIssue.reject",
  );

  async function confirm() {
    if (busy || issue.status !== "PENDING" || (recycle && !recycleVariables)) {
      return;
    }
    setBusy(true);
    setFailure(null);
    try {
      const mutation =
        action === "recycle"
          ? approveMediaIssueRecycleSearchMutation
          : action === "approve"
            ? approveMediaIssueMutation
            : rejectMediaIssueMutation;
      const { data, error: mutationError } = await client
        .mutation<Record<string, { id: string } | null>>(mutation, {
          ...(recycleVariables ?? { issueId: issue.id }),
        })
        .toPromise();
      const updated =
        data?.approveMediaIssueRecycleSearch ??
        data?.approveMediaIssue ??
        data?.rejectMediaIssue;
      if (mutationError || updated?.id !== issue.id) {
        throw mutationError ?? new Error(t("status.apiError"));
      }
      setGlobalStatus(t("mediaIssue.reviewSaved"), { level: "SUCCESS" });
      onClose();
      onReviewed();
    } catch (mutationError) {
      const key = mediaIssueRefusalKey(
        graphQlErrorExtension(mutationError, "VALIDATION_ERROR", "reason"),
      );
      setFailure(
        key
          ? t(key)
          : deleteErrorMessage(mutationError, t, t("status.apiError")),
      );
      setTyped("");
    } finally {
      setBusy(false);
    }
  }

  return (
    <ConfirmDialog
      open
      title={title}
      description={t(
        action === "recycle"
          ? "mediaIssue.approveRecycleHint"
          : action === "approve"
            ? "mediaIssue.approveMarkOnlyHint"
            : "mediaIssue.rejectHint",
      )}
      cancelLabel={t("label.cancel")}
      confirmLabel={title}
      confirmButtonVariant={action === "approve" ? "default" : "destructive"}
      isBusy={busy}
      confirmDisabled={
        issue.status !== "PENDING" ||
        (recycle && (!recycleVariables || failure !== null))
      }
      onConfirm={confirm}
      onCancel={() => {
        if (busy) return;
        onClose();
        // The request may have recorded a decision before a transport failure.
        if (failure) onFailed();
      }}
    >
      <div className="space-y-3">
        {recycle && settingsLoading ? (
          <p role="status" className="text-sm text-muted-foreground">
            {t("label.loading")}
          </p>
        ) : null}
        {recycle && !settingsLoading && !canRecycle ? (
          <ReviewError
            message={
              settingsError
                ? userFacingGraphQlErrorMessage(
                    settingsError,
                    t("mediaIssue.recycleUnavailable"),
                    t,
                  )
                : t("mediaIssue.recycleUnavailable")
            }
          />
        ) : null}
        {recycle && canRecycle ? (
          <>
            {preview && !loading && !error ? (
              <dl className="rounded-md border border-border bg-muted/50 p-3">
                <dt className="text-xs text-muted-foreground">
                  {t("mediaIssue.previewFingerprint")}
                </dt>
                <dd className="break-all font-mono text-xs">
                  {preview.fingerprint}
                </dd>
              </dl>
            ) : null}
            {preview && !loading && !error && preview.mediaCount !== 1 ? (
              <ReviewError message={t("mediaIssue.unexpectedPreviewScope")} />
            ) : null}
            <DeletePreviewSummary
              preview={
                preview
                  ? {
                      ...preview,
                      requiresTypedConfirmation: true,
                      typedConfirmationPrompt: t("mediaIssue.typeConfirmation"),
                    }
                  : null
              }
              loading={loading || (!preview && !error)}
              error={error}
              typedConfirmation={typed}
              onTypedConfirmationChange={setTyped}
            />
          </>
        ) : null}
        {failure ? <ReviewError message={failure} /> : null}
      </div>
    </ConfirmDialog>
  );
}

function LibraryContentBlocks({ libraryId }: { libraryId: string }) {
  const t = useTranslate();
  const client = useClient();
  const dateFormat = useUiDateTimeFormat();
  const [target, setTarget] = useState<LibraryContentBlock | null>(null);
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState<string | null>(null);
  const [clearedBlockId, setClearedBlockId] = useState<string | null>(null);
  const [{ data, fetching, error }, refresh] = useQuery<{
    libraryContentBlocks: LibraryContentBlock[];
  }>({
    query: libraryContentBlocksQuery,
    variables: { libraryId, limit: 1000 },
    requestPolicy: "network-only",
  });
  const blocks =
    data?.libraryContentBlocks
      ?.filter((block) => block.id !== clearedBlockId)
      .slice(0, 1000) ?? [];

  function reload() {
    refresh({ requestPolicy: "network-only" });
  }

  async function clearBlock() {
    if (!target || busy) return;
    setBusy(true);
    setFailure(null);
    try {
      const result = await client
        .mutation<{ clearLibraryContentBlock?: { id: string } | null }>(
          clearLibraryContentBlockMutation,
          { blockId: target.id },
        )
        .toPromise();
      if (!confirmedContentBlockRemoval(target.id, result)) {
        throw result.error ?? new Error(t("mediaIssue.clearFailed"));
      }
      setClearedBlockId(target.id);
      setTarget(null);
      reload();
    } catch (mutationError) {
      setFailure(
        hasGraphQlErrorReason(
          mutationError,
          "MEDIA_CONTENT_BLOCK_NOT_FOUND",
          "NOT_FOUND",
        )
          ? t("mediaIssue.blockNotFound")
          : userFacingGraphQlErrorMessage(
              mutationError,
              t("mediaIssue.clearFailed"),
              t,
            ),
      );
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="space-y-3">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <p className="max-w-lg text-sm text-muted-foreground">
          {t("mediaIssue.blocksHint")}
        </p>
        <RefreshButton busy={fetching} onClick={reload} />
      </div>
      {error ? (
        <ReviewError
          message={userFacingGraphQlErrorMessage(
            error,
            t("mediaIssue.blocksFailed"),
            t,
          )}
        />
      ) : null}
      {fetching && blocks.length === 0 ? (
        <p role="status" className="text-sm text-muted-foreground">
          {t("label.loading")}
        </p>
      ) : !error && blocks.length === 0 ? (
        <p className="text-sm text-muted-foreground">
          {t("mediaIssue.noBlocks")}
        </p>
      ) : null}
      {(data?.libraryContentBlocks?.length ?? 0) > BLOCK_LIMIT ? (
        <p className="text-xs text-muted-foreground">
          {t("mediaIssue.blockLimit", { count: BLOCK_LIMIT })}
        </p>
      ) : null}
      <ul className="divide-y divide-border rounded-md border border-border empty:hidden">
        {blocks.map((block) => (
          <li
            key={block.id}
            className="flex flex-wrap items-center justify-between gap-2 p-3"
          >
            <div className="min-w-0 space-y-1">
              <p className="break-words text-sm font-medium">
                {!block.reason || block.reason === "bad_media"
                  ? t("mediaFile.badMedia")
                  : block.reason}
              </p>
              <p className="text-xs text-muted-foreground">
                {formatMediaFileSize(block.sizeBytes)}
                {" · "}
                {formatUiDateTime(block.createdAt, dateFormat)}
              </p>
              <p className="break-all font-mono text-xs text-muted-foreground">
                {block.sampleBlake3}
              </p>
            </div>
            <Button
              size="sm"
              variant="outline"
              onClick={() => {
                setFailure(null);
                setTarget(block);
              }}
            >
              {t("mediaFile.clearContentBlock")}
            </Button>
          </li>
        ))}
      </ul>
      <ConfirmDialog
        open={target !== null}
        title={t("mediaFile.clearContentBlock")}
        description={t("mediaIssue.clearHint")}
        confirmLabel={t("mediaFile.clearContentBlock")}
        cancelLabel={t("label.cancel")}
        onConfirm={clearBlock}
        onCancel={() => {
          if (!busy) setTarget(null);
        }}
        isBusy={busy}
      >
        {target ? (
          <p className="break-all font-mono text-xs">{target.sampleBlake3}</p>
        ) : null}
        {failure ? <ReviewError message={failure} /> : null}
      </ConfirmDialog>
    </div>
  );
}

function RefreshButton({
  busy,
  onClick,
}: {
  busy: boolean;
  onClick: () => void;
}) {
  const t = useTranslate();
  return (
    <Button size="sm" variant="outline" disabled={busy} onClick={onClick}>
      <RefreshCw aria-hidden="true" />
      {t(busy ? "label.refreshing" : "label.refresh")}
    </Button>
  );
}

function ReviewError({ message }: { message: string }) {
  return (
    <p
      role="alert"
      className="break-words rounded border border-destructive/40 bg-destructive/10 p-2 text-sm text-destructive"
    >
      {message}
    </p>
  );
}
