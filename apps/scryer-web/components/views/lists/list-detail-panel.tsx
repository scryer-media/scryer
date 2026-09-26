import * as React from "react";
import { Eye, Pencil, RefreshCw, Trash2 } from "lucide-react";

import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { LoadingMark } from "@/components/common/loading-mark";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Sheet, SheetContent, SheetDescription, SheetHeader, SheetTitle } from "@/components/ui/sheet";
import { Switch } from "@/components/ui/switch";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { useTranslate } from "@/lib/context/translate-context";
import { useUiDateTimeFormat } from "@/lib/context/ui-settings-context";
import type { ListPreview, ListProviderManifest, ListSubscription } from "@/lib/types/lists";
import { formatUiDateTime } from "@/lib/utils/date-format";
import {
  listIntervalParts,
  listKindLabelKey,
  listMembershipRowId,
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
  onPreview: (id: string) => Promise<ListPreview | null>;
  onEdit: (subscription: ListSubscription) => void;
  onSetEnabled: (subscription: ListSubscription, enabled: boolean) => void;
  onSyncNow: (subscription: ListSubscription) => void;
  onUnsubscribe: (subscription: ListSubscription) => Promise<boolean>;
};

const SECTION_HEADING = "mb-2 text-[12px] font-semibold uppercase tracking-[0.06em] text-[var(--scry-muted)]";

export function ListDetailPanel({
  detail,
  fallback,
  provider,
  canManageLists,
  busy,
  membershipPageSize,
  onClose,
  onPage,
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
              <Badge tone={listSyncStateTone(state)}>{t(listSyncStateLabelKey(state))}</Badge>
              {subscription.kinds.map((kind) => (
                <Badge key={kind} tone="outline">
                  {t(listKindLabelKey(kind))}
                </Badge>
              ))}
              {canManageLists ? (
                <div className="ml-auto flex flex-wrap items-center gap-2">
                  <Switch
                    id="list-detail-enabled"
                    aria-label={t("lists.action.enabled", { name: subscription.name })}
                    checked={subscription.enabled}
                    disabled={busy}
                    onCheckedChange={(checked) => onSetEnabled(subscription, checked)}
                  />
                  <Button
                    id="list-detail-sync"
                    type="button"
                    size="sm"
                    variant="outline"
                    disabled={busy || !subscription.enabled}
                    onClick={() => onSyncNow(subscription)}
                  >
                    <RefreshCw className="h-4 w-4" />
                    {t("lists.action.syncNow")}
                  </Button>
                  <Button
                    id="list-detail-edit"
                    type="button"
                    size="sm"
                    variant="outline"
                    disabled={busy}
                    onClick={() => onEdit(subscription)}
                  >
                    <Pencil className="h-4 w-4" />
                    {t("label.edit")}
                  </Button>
                  <Button
                    id="list-detail-unfollow"
                    type="button"
                    size="sm"
                    variant="destructive"
                    disabled={busy}
                    onClick={() => setConfirmUnfollow(true)}
                  >
                    <Trash2 className="h-4 w-4" />
                    {t("lists.action.unfollow")}
                  </Button>
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

            <section>
              <h3 className={SECTION_HEADING}>{t("lists.detail.coverage")}</h3>
              <ListCoverageBar counts={subscription.counts} showLegend />
            </section>

            <section>
              <h3 className={SECTION_HEADING}>{t("lists.detail.settings")}</h3>
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
                  className="mt-2 inline-block text-[12.5px] text-[var(--scry-accent-text)] hover:underline"
                >
                  {t("lists.detail.openAtProvider")}
                </a>
              ) : null}
            </section>

            <section>
              <div className="mb-2 flex items-center justify-between gap-2">
                <h3 className={SECTION_HEADING}>{t("lists.preview.nextSyncHeading")}</h3>
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
              </div>
              {preview ? <ListPreviewSummary preview={preview} idPrefix="list-detail" /> : null}
            </section>

            <section>
              <h3 className={SECTION_HEADING}>
                {t("lists.detail.titles", { count: memberships?.totalCount ?? 0 })}
              </h3>
              {memberships && memberships.items.length > 0 ? (
                <>
                  <Table id="list-detail-memberships" density="dense">
                    <TableHeader>
                      <TableRow>
                        <TableHead className="w-10">#</TableHead>
                        <TableHead>{t("lists.detail.title")}</TableHead>
                        <TableHead>{t("lists.detail.state")}</TableHead>
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
                        >
                          <TableCell className="text-[12px] text-[var(--scry-muted)]">{membership.rank ?? "—"}</TableCell>
                          <TableCell>
                            <span className="block text-[13px] text-[var(--scry-ink2)]">
                              {membership.displayTitle ?? membership.itemKey}
                              {membership.year ? (
                                <span className="text-[var(--scry-muted)]"> ({membership.year})</span>
                              ) : null}
                            </span>
                            {membership.leftAt ? (
                              <span className="block text-[11.5px] text-[var(--scry-muted)]">
                                {t("lists.detail.leftAt", { time: formatDate(membership.leftAt) })}
                              </span>
                            ) : null}
                          </TableCell>
                          <TableCell>
                            <Badge tone={listMembershipStateTone(membership.state)}>
                              {t(listMembershipStateLabelKey(membership.state))}
                            </Badge>
                            {membership.stateReason ? (
                              <span className="mt-0.5 block text-[11.5px] text-[var(--scry-muted)]">
                                {membership.stateReason}
                              </span>
                            ) : null}
                          </TableCell>
                        </TableRow>
                      ))}
                    </TableBody>
                  </Table>
                  {memberships.totalCount > membershipPageSize ? (
                    <div className="mt-2 flex items-center justify-between text-[12px] text-[var(--scry-muted)]">
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
            </section>

            <section>
              <h3 className={SECTION_HEADING}>{t("lists.detail.history")}</h3>
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
                        <span className="basis-full text-[var(--scry-danger-text)]">{run.errorMessage}</span>
                      ) : null}
                    </li>
                  ))}
                </ul>
              ) : (
                <p className="text-[12.5px] text-[var(--scry-muted)]">{t("lists.detail.noRuns")}</p>
              )}
            </section>
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
