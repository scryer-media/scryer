import * as React from "react";
import { Eye, Pencil, RefreshCw, Trash2 } from "lucide-react";

import { ConfirmDialog } from "@/components/common/confirm-dialog";
import {
  TITLE_TABLE_HEADER_CELL_CLASS,
  TITLE_TABLE_HEADER_ROW_CLASS,
  TITLE_TABLE_ROW_CLASS,
} from "@/components/views/media-content/title-table-shared";
import { LabeledFieldset } from "@/components/common/labeled-fieldset";
import { LoadingMark } from "@/components/common/loading-mark";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Sheet, SheetContent, SheetDescription, SheetHeader, SheetTitle } from "@/components/ui/sheet";
import { Checkbox } from "@/components/ui/checkbox";
import { TextActionButton } from "@/components/ui/text-action-button";
import { ActionTooltip } from "@/components/ui/tooltip";
import {
  Table,
  TableBody,
  TableCell,
  TableCodeCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { useTranslate } from "@/lib/context/translate-context";
import { useUiDateTimeFormat } from "@/lib/context/ui-settings-context";
import type { ListMembership, ListPreview, ListProviderManifest, ListSubscription } from "@/lib/types/lists";
import { formatUiDateTime } from "@/lib/utils/date-format";
import {
  listIntervalParts,
  listKindLabelKey,
  listMembershipRowId,
  listMembershipTitleHref,
  listMembershipReasonKey,
  listMembershipStateLabelKey,
  listMembershipStateTone,
  listModeLabelKey,
  listOnLeaveLabelKey,
  listSyncRunOutcomeLabelKey,
  listSyncRunOutcomeTone,
  listSyncStateLabelKey,
  listSyncStateTone,
} from "@/lib/utils/lists";

import { ListCoverageBar } from "./list-coverage-bar";
import { ListPreviewSummary } from "./list-preview-summary";
import { ProviderTile } from "./provider-tile";
import type { ListDetailState } from "./lists-view";

type ListDetailPanelProps = {
  detail: ListDetailState | null;
  fallback: ListSubscription | null;
  provider: ListProviderManifest | null;
  canManageLists: boolean;
  busy: boolean;
  membershipPageSize: number;
  onClose: () => void;
  onPage: (id: string, offset: number) => void;
  onShowAllRuns: (id: string) => void;
  onPreview: (id: string) => Promise<ListPreview | null>;
  onEdit: (subscription: ListSubscription) => void;
  onSetEnabled: (subscription: ListSubscription, enabled: boolean) => void;
  onSyncNow: (subscription: ListSubscription) => void;
  onUnsubscribe: (subscription: ListSubscription) => Promise<boolean>;
};

export function ListDetailPanel({
  detail,
  fallback,
  provider,
  canManageLists,
  busy,
  membershipPageSize,
  onClose,
  onPage,
  onShowAllRuns,
  onPreview,
  onEdit,
  onSetEnabled,
  onSyncNow,
  onUnsubscribe,
}: ListDetailPanelProps) {
  const t = useTranslate();
  const dateTimeFormat = useUiDateTimeFormat();
  const subscription = detail?.subscription ?? fallback;
  const [confirmUnfollow, setConfirmUnfollow] = React.useState(false);
  const [preview, setPreview] = React.useState<ListPreview | null>(null);
  const [previewing, setPreviewing] = React.useState(false);
  const detailId = detail?.id ?? null;

  React.useEffect(() => {
    setPreview(null);
    setConfirmUnfollow(false);
  }, [detailId]);

  const formatDate = (value: string | null) => (value ? formatUiDateTime(value, dateTimeFormat) : "—");

  const runPreview = async () => {
    if (!subscription) return;
    setPreviewing(true);
    try {
      setPreview(await onPreview(subscription.id));
    } finally {
      setPreviewing(false);
    }
  };

  const memberships = detail?.memberships ?? null;
  const offset = detail?.membershipOffset ?? 0;
  const interval = subscription ? listIntervalParts(subscription.intervalSeconds) : null;
  const state = subscription ? (subscription.enabled ? subscription.sync.state : "OFF") : "NEW";

  return (
    <Sheet open={detail !== null} onOpenChange={(open) => (!open ? onClose() : undefined)}>
      <SheetContent
        id="list-detail-panel"
        side="right"
        className="w-full overflow-y-auto border-l border-[var(--scry-border)] bg-[var(--scry-surf)] text-[var(--scry-ink2)] sm:max-w-2xl"
      >
        <SheetHeader className="border-b border-[var(--scry-border3)] pr-12">
          <div className="flex items-center gap-3">
            <ProviderTile provider={provider} />
            <div className="min-w-0">
              <SheetTitle className="truncate font-display text-[19px] text-[var(--scry-ink)]">
                {subscription?.name ?? t("label.loading")}
              </SheetTitle>
              <SheetDescription className="truncate">
                {[provider?.name ?? subscription?.source.provider, subscription?.source.sourceType]
                  .filter(Boolean)
                  .join(" · ")}
              </SheetDescription>
            </div>
          </div>
        </SheetHeader>

        {!subscription ? (
          <div className="flex items-center gap-2 px-4 text-sm text-[var(--scry-muted)]">
            {detail?.error ? detail.error : <LoadingMark className="h-4 w-4" />}
          </div>
        ) : (
          <div className="space-y-6 px-4 pb-8">
            <div className="flex flex-wrap items-center gap-2">
              <Badge tone={listSyncStateTone(state)}>{t(listSyncStateLabelKey(state, subscription.sync.lastAt))}</Badge>
              {subscription.kinds.map((kind) => (
                <Badge key={kind} tone="outline">
                  {t(listKindLabelKey(kind))}
                </Badge>
              ))}
              {canManageLists ? (
                <div className="ml-auto flex flex-wrap items-center gap-2">
                  <label className="flex shrink-0 items-center gap-3">
                    <Checkbox
                      id="list-detail-enabled"
                      size="large"
                      checked={subscription.enabled}
                      disabled={busy}
                      onCheckedChange={(checked) => onSetEnabled(subscription, checked === true)}
                    />
                    <span className="text-sm font-medium">{t("label.enabled")}</span>
                  </label>
                  <TextActionButton
                    id="list-detail-sync"
                    tone="accent"
                    disabled={busy || !subscription.enabled}
                    onClick={() => onSyncNow(subscription)}
                    leadingIcon={<RefreshCw className="h-4 w-4" />}
                  >
                    {t("lists.action.syncNow")}
                  </TextActionButton>
                  <TextActionButton
                    id="list-detail-edit"
                    tone="edit"
                    disabled={busy}
                    onClick={() => onEdit(subscription)}
                    leadingIcon={<Pencil className="h-4 w-4" />}
                  >
                    {t("label.edit")}
                  </TextActionButton>
                  <TextActionButton
                    id="list-detail-unfollow"
                    tone="delete"
                    disabled={busy}
                    onClick={() => setConfirmUnfollow(true)}
                    leadingIcon={<Trash2 className="h-4 w-4" />}
                  >
                    {t("lists.action.unfollow")}
                  </TextActionButton>
                </div>
              ) : null}
            </div>

            {state === "FAIL" && subscription.sync.errorMessage ? (
              <p
                id="list-detail-error"
                className="rounded-[10px] border border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)] px-3 py-2 text-[13px] text-[var(--scry-danger-text)]"
              >
                {subscription.sync.errorMessage}
                {subscription.sync.errorAt ? (
                  <span className="block text-[11.5px] opacity-80">{formatDate(subscription.sync.errorAt)}</span>
                ) : null}
              </p>
            ) : null}
            {subscription.sync.pausedUntil ? (
              <p className="text-[12.5px] text-[var(--scry-warning-text)]">
                {t("lists.detail.pausedUntil", { time: formatDate(subscription.sync.pausedUntil) })}
              </p>
            ) : null}

            <LabeledFieldset label={t("lists.detail.coverage")}>
              <ListCoverageBar counts={subscription.counts} legend="inline" />
            </LabeledFieldset>

            <LabeledFieldset label={t("lists.detail.settings")}>
              <dl className="grid grid-cols-2 gap-x-4 gap-y-2 text-[13px] sm:grid-cols-3">
                {(
                  [
                    ["lists.table.mode", t(listModeLabelKey(subscription.mode))],
                    ["lists.follow.onLeave", t(listOnLeaveLabelKey(subscription.onLeave))],
                    [
                      "lists.follow.maxPerSync",
                      subscription.maxPerSync === null ? t("lists.detail.noCap") : String(subscription.maxPerSync),
                    ],
                    ["lists.detail.interval", interval ? t(interval.key, { count: interval.count }) : "—"],
                    ["lists.table.lastSync", formatDate(subscription.sync.lastAt)],
                    ["lists.detail.nextSync", formatDate(subscription.sync.nextAt)],
                  ] as Array<[string, string]>
                ).map(([key, value]) => (
                  <div key={key} className="min-w-0">
                    <dt className="text-[11.5px] text-[var(--scry-muted)]">{t(key)}</dt>
                    <dd className="truncate text-[var(--scry-ink2)]">{value}</dd>
                  </div>
                ))}
              </dl>
              {subscription.providerUrl ? (
                <a
                  href={subscription.providerUrl}
                  target="_blank"
                  rel="noreferrer noopener"
                  className="inline-block text-[12.5px] text-[var(--scry-accent-text)] hover:underline"
                >
                  {t("lists.detail.openAtProvider")}
                </a>
              ) : null}
            </LabeledFieldset>

            <LabeledFieldset label={t("lists.preview.nextSyncHeading")}>
              <Button
                id="list-detail-preview"
                type="button"
                size="xs"
                variant="outline"
                onClick={() => void runPreview()}
                disabled={previewing}
              >
                {previewing ? <LoadingMark className="h-3.5 w-3.5" /> : <Eye className="h-3.5 w-3.5" />}
                {t("lists.preview.run")}
              </Button>
              {preview ? <ListPreviewSummary preview={preview} idPrefix="list-detail" /> : null}
            </LabeledFieldset>

            <LabeledFieldset label={t("lists.detail.titles", { count: memberships?.totalCount ?? 0 })}>
              {memberships && memberships.items.length > 0 ? (
                <>
                  <Table id="list-detail-memberships" density="dense" wrapperClassName="rounded-[12px] border border-[var(--scry-border3)]">
                    <TableHeader>
                      <TableRow className={TITLE_TABLE_HEADER_ROW_CLASS}>
                        <TableHead className={`w-12 text-center ${TITLE_TABLE_HEADER_CELL_CLASS}`}>#</TableHead>
                        <TableHead className={TITLE_TABLE_HEADER_CELL_CLASS}>{t("lists.detail.title")}</TableHead>
                        <TableHead className={TITLE_TABLE_HEADER_CELL_CLASS}>{t("lists.detail.state")}</TableHead>
                      </TableRow>
                    </TableHeader>
                    <TableBody>
                      {memberships.items.map((membership) => (
                        <TableRow
                          key={membership.itemKey}
                          id={listMembershipRowId(subscription.id, membership.itemKey)}
                          data-item-key={membership.itemKey}
                          data-title-id={membership.titleId ?? undefined}
                          data-membership-state={membership.state}
                          className={TITLE_TABLE_ROW_CLASS}
                        >
                          <TableCodeCell className="text-center text-[12px] text-[var(--scry-muted)]">{membership.rank ?? "—"}</TableCodeCell>
                          <TableCell>
                            <div className="text-[13px] font-medium text-[var(--scry-ink)]">
                              {listMembershipTitleHref(membership.kind, membership.titleId, membership.seriesMovieLinkId) ? (
                                <a
                                  href={listMembershipTitleHref(membership.kind, membership.titleId, membership.seriesMovieLinkId)!}
                                  className="rounded-sm hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--scry-focus)]"
                                >
                                  {membership.displayTitle ?? membership.itemKey}
                                </a>
                              ) : membership.displayTitle ?? membership.itemKey}
                              {membership.year ? (
                                <span className="font-normal text-[var(--scry-muted)]"> ({membership.year})</span>
                              ) : null}
                            </div>
                          </TableCell>
                          <TableCell>
                            <MembershipStateBadge membership={membership} />
                          </TableCell>
                        </TableRow>
                      ))}
                    </TableBody>
                  </Table>
                  {memberships.totalCount > membershipPageSize ? (
                    <div className="flex items-center justify-between text-[12px] text-[var(--scry-muted)]">
                      <span>
                        {t("lists.detail.pageRange", {
                          from: offset + 1,
                          to: Math.min(offset + membershipPageSize, memberships.totalCount),
                          total: memberships.totalCount,
                        })}
                      </span>
                      <div className="flex gap-2">
                        <Button
                          type="button"
                          size="xs"
                          variant="outline"
                          disabled={offset === 0 || detail?.loading}
                          onClick={() => onPage(subscription.id, Math.max(0, offset - membershipPageSize))}
                        >
                          {t("lists.detail.previousPage")}
                        </Button>
                        <Button
                          type="button"
                          size="xs"
                          variant="outline"
                          disabled={offset + membershipPageSize >= memberships.totalCount || detail?.loading}
                          onClick={() => onPage(subscription.id, offset + membershipPageSize)}
                        >
                          {t("lists.detail.nextPage")}
                        </Button>
                      </div>
                    </div>
                  ) : null}
                </>
              ) : (
                <p className="text-[12.5px] text-[var(--scry-muted)]">
                  {detail?.loading ? t("label.loading") : t("lists.detail.noTitles")}
                </p>
              )}
            </LabeledFieldset>

            <LabeledFieldset label={t("lists.detail.history")}>
              {detail && detail.runs.length > 0 ? (
                <ul id="list-detail-runs" className="space-y-1.5">
                  {detail.runs.map((run) => (
                    <li
                      key={run.id}
                      className="flex flex-wrap items-center gap-x-3 gap-y-1 rounded-[8px] border border-[var(--scry-border3)] px-3 py-2 text-[12.5px]"
                    >
                      <Badge tone={listSyncRunOutcomeTone(run.outcome)}>{t(listSyncRunOutcomeLabelKey(run.outcome))}</Badge>
                      <span className="text-[var(--scry-ink2)]">{formatDate(run.startedAt)}</span>
                      <span className="text-[var(--scry-muted)]">
                        {t("lists.detail.runCounts", {
                          total: run.counts.total,
                          added: run.counts.added,
                          held: run.counts.held,
                        })}
                      </span>
                      {run.errorMessage ? (
                        <span
                          className={
                            run.outcome === "FAILED"
                              ? "basis-full text-[var(--scry-danger-text)]"
                              : "basis-full text-[var(--scry-warning-text)]"
                          }
                        >
                          {run.errorMessage}
                        </span>
                      ) : null}
                    </li>
                  ))}
                </ul>
              ) : (
                <p className="text-[12.5px] text-[var(--scry-muted)]">{t("lists.detail.noRuns")}</p>
              )}
              {detail?.moreRuns ? (
                <Button
                  type="button"
                  id="list-detail-runs-more"
                  size="xs"
                  variant="outline"
                  disabled={detail.loading}
                  onClick={() => onShowAllRuns(detail.id)}
                >
                  {t("lists.detail.olderRuns")}
                </Button>
              ) : null}
            </LabeledFieldset>
          </div>
        )}

        {subscription ? (
          <ConfirmDialog
            open={confirmUnfollow}
            title={t("lists.unfollow.title", { name: subscription.name })}
            description={t("lists.unfollow.description")}
            confirmLabel={t("lists.action.unfollow")}
            cancelLabel={t("label.cancel")}
            contentId="list-unfollow-dialog"
            confirmButtonId="list-unfollow-confirm"
            confirmButtonVariant="destructive"
            isBusy={busy}
            onCancel={() => setConfirmUnfollow(false)}
            onConfirm={async () => {
              const ok = await onUnsubscribe(subscription);
              if (ok) setConfirmUnfollow(false);
            }}
          />
        ) : null}
      </SheetContent>
    </Sheet>
  );
}

/** A list entry's state. When the server recorded why, hovering or focusing the badge says it. */
function MembershipStateBadge({ membership }: { membership: Pick<ListMembership, "state" | "stateReason"> }) {
  const t = useTranslate();
  const badge = <Badge tone={listMembershipStateTone(membership.state)}>{t(listMembershipStateLabelKey(membership.state))}</Badge>;
  if (!membership.stateReason) return badge;
  const reasonKey = listMembershipReasonKey(membership.stateReason);
  return (
    <ActionTooltip
      // A reason this client has no sentence for is shown as the server wrote it.
      content={reasonKey ? t(reasonKey) : membership.stateReason}
      wrapperClassName="cursor-help"
      wrapperTabIndex={0}
    >
      {badge}
    </ActionTooltip>
  );
}
