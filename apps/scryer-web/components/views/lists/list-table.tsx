import * as React from "react";
import { ArrowDown, ArrowUp, ArrowUpDown, RefreshCw } from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { Checkbox } from "@/components/ui/checkbox";
import { IconButton } from "@/components/ui/icon-button";
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
import type { ListProviderManifest, ListSubscription } from "@/lib/types/lists";
import { formatUiDateTime } from "@/lib/utils/date-format";
import {
  type ListSort,
  type ListSortKey,
  listKindLabelKey,
  listModeLabelKey,
  listSyncStateLabelKey,
  listSyncStateTone,
  shownListSyncState,
  sortListSubscriptions,
} from "@/lib/utils/lists";

import { ListCoverageBar } from "./list-coverage-bar";
import { ProviderTile } from "./provider-tile";

type ListTableProps = {
  subscriptions: ListSubscription[];
  providers: ListProviderManifest[];
  canManageLists: boolean;
  busyIds: ReadonlySet<string>;
  onOpen: (subscription: ListSubscription) => void;
  onSetEnabled: (subscription: ListSubscription, enabled: boolean) => void;
  onSyncNow: (subscription: ListSubscription) => void;
};

export function ListTable({
  subscriptions,
  providers,
  canManageLists,
  busyIds,
  onOpen,
  onSetEnabled,
  onSyncNow,
}: ListTableProps) {
  const t = useTranslate();
  const dateTimeFormat = useUiDateTimeFormat();
  const providerByType = new Map(providers.map((provider) => [provider.providerType, provider]));
  // Until a column is picked the lists keep the order they were loaded in.
  const [sort, setSort] = React.useState<ListSort | null>(null);
  const sorted = React.useMemo(() => sortListSubscriptions(subscriptions, sort), [subscriptions, sort]);
  const sortableHead = (key: ListSortKey, label: string) => {
    const active = sort?.key === key;
    const Icon = !active ? ArrowUpDown : sort.descending ? ArrowDown : ArrowUp;
    return (
      <TableHead aria-sort={active ? (sort.descending ? "descending" : "ascending") : "none"}>
        <button
          id={`lists-table-sort-${key}`}
          type="button"
          className="inline-flex items-center gap-1.5 rounded-sm text-left font-medium transition-colors hover:text-[var(--scry-ink)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--scry-focus)]"
          onClick={() => setSort({ key, descending: active ? !sort.descending : false })}
        >
          <span>{label}</span>
          <Icon aria-hidden="true" className={active ? "h-3.5 w-3.5" : "h-3.5 w-3.5 text-[var(--scry-faint2)]"} />
        </button>
      </TableHead>
    );
  };

  return (
    <Table id="lists-table" wrapperClassName="rounded-[12px] border border-[var(--scry-border3)]">
      <TableHeader>
        <TableRow>
          {sortableHead("name", t("lists.table.list"))}
          <TableHead className="hidden md:table-cell">{t("lists.table.coverage")}</TableHead>
          <TableHead className="hidden sm:table-cell">{t("lists.table.mode")}</TableHead>
          {canManageLists ? <TableHead className="w-[1%] text-center">{t("label.enabled")}</TableHead> : null}
          {sortableHead("sync", t("lists.table.lastSync"))}
          {canManageLists ? <TableHead className="w-[1%] text-right">{t("label.actions")}</TableHead> : null}
        </TableRow>
      </TableHeader>
      <TableBody>
        {sorted.map((subscription) => {
          const provider = providerByType.get(subscription.source.provider) ?? null;
          const busy = busyIds.has(subscription.id);
          const state = shownListSyncState(subscription);
          return (
            <TableRow
              key={subscription.id}
              id={`list-row-${subscription.id}`}
              data-ui="list-subscription-row"
              className="cursor-pointer hover:bg-[var(--scry-hover)]"
              onClick={() => onOpen(subscription)}
            >
              <TableCell>
                <div className="flex min-w-0 items-center gap-3">
                  <ProviderTile provider={provider} size="sm" />
                  <div className="min-w-0">
                    <button
                      type="button"
                      className="block max-w-full truncate text-left text-[14px] font-semibold text-[var(--scry-ink)] hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--scry-focus)]"
                      onClick={(event) => {
                        event.stopPropagation();
                        onOpen(subscription);
                      }}
                    >
                      {subscription.name}
                    </button>
                    <div className="mt-0.5 flex flex-wrap items-center gap-1.5 text-[12px] text-[var(--scry-muted)]">
                      {/* The logo names the provider on screen; this names it to a screen reader. */}
                      <span className="sr-only">{provider?.name ?? subscription.source.provider}</span>
                      {subscription.kinds.map((kind) => (
                        <Badge key={kind} tone="outline" className="px-1.5 py-0 text-[10.5px]">
                          {t(listKindLabelKey(kind))}
                        </Badge>
                      ))}
                    </div>
                  </div>
                </div>
              </TableCell>
              <TableCell className="hidden w-36 md:table-cell">
                <ListCoverageBar counts={subscription.counts} legend="hover" />
              </TableCell>
              <TableCell className="hidden sm:table-cell">
                <span className="text-[13px] text-[var(--scry-ink2)]">{t(listModeLabelKey(subscription.mode))}</span>
              </TableCell>
              {canManageLists ? (
                <TableCell className="text-center" onClick={(event) => event.stopPropagation()}>
                  <Checkbox
                    id={`list-enabled-${subscription.id}`}
                    size="large"
                    checked={subscription.enabled}
                    disabled={busy}
                    aria-label={`${t("label.enabled")}: ${subscription.name}`}
                    onCheckedChange={(checked) => onSetEnabled(subscription, checked === true)}
                  />
                </TableCell>
              ) : null}
              <TableCell>
                <div className="flex flex-col items-start gap-1">
                  <Badge tone={listSyncStateTone(state)}>{t(listSyncStateLabelKey(state, subscription.sync.lastAt))}</Badge>
                  <span className="text-[11.5px] text-[var(--scry-muted)]">
                    {subscription.sync.lastAt
                      ? formatUiDateTime(subscription.sync.lastAt, dateTimeFormat)
                      : t("lists.table.neverSynced")}
                  </span>
                  {state === "FAIL" && subscription.sync.errorMessage ? (
                    <span className="max-w-[260px] text-[11.5px] text-[var(--scry-danger-text)]">
                      {subscription.sync.errorMessage}
                    </span>
                  ) : null}
                </div>
              </TableCell>
              {canManageLists ? (
                <TableCell className="text-right" onClick={(event) => event.stopPropagation()}>
                  <div className="flex items-center justify-end gap-2">
                    <IconButton
                      id={`list-sync-${subscription.id}`}
                      label={t("lists.action.syncNow")}
                      tone="accent"
                      disabled={busy || !subscription.enabled}
                      onClick={() => onSyncNow(subscription)}
                    >
                      <RefreshCw className="h-4 w-4" />
                    </IconButton>
                  </div>
                </TableCell>
              ) : null}
            </TableRow>
          );
        })}
      </TableBody>
    </Table>
  );
}
