import { Check, Plus } from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { useTranslate } from "@/lib/context/translate-context";
import type { Facet } from "@/lib/types/titles";
import { listIntervalParts, listKindLabelKey } from "@/lib/utils/lists";

/** One list a provider offers to follow. */
export type CatalogListRow = {
  key: string;
  name: string;
  description?: string | null;
  kinds: readonly Facet[];
  /** How often the list syncs once followed; left out when the offer does not say. */
  intervalSeconds?: number | null;
  /** A list already followed is shown as such and cannot be followed again. */
  followed: boolean;
  followId: string;
  onFollow: () => void;
};

/** The lists a provider offers, one per row, each with its follow action. */
export function CatalogListTable({ rows }: { rows: readonly CatalogListRow[] }) {
  const t = useTranslate();
  const showSchedule = rows.some((row) => row.intervalSeconds != null);

  return (
    <Table
      data-ui="list-catalog-table"
      density="dense"
      wrapperClassName="rounded-[12px] border border-[var(--scry-border3)]"
    >
      <TableHeader>
        <TableRow>
          <TableHead>{t("lists.table.list")}</TableHead>
          <TableHead className="hidden w-[1%] whitespace-nowrap sm:table-cell">{t("label.type")}</TableHead>
          {showSchedule ? (
            <TableHead className="hidden w-[1%] whitespace-nowrap md:table-cell">{t("lists.detail.interval")}</TableHead>
          ) : null}
          <TableHead className="w-[1%] text-right">
            <span className="sr-only">{t("label.actions")}</span>
          </TableHead>
        </TableRow>
      </TableHeader>
      <TableBody>
        {rows.map((row) => {
          const interval = row.intervalSeconds != null ? listIntervalParts(row.intervalSeconds) : null;
          return (
            <TableRow key={row.key} data-ui="list-catalog-row" className="hover:bg-[var(--scry-hover)]">
              <TableCell>
                <p className="text-[13.5px] font-semibold text-[var(--scry-ink)]">{row.name}</p>
                {row.description ? (
                  <p className="text-[12px] text-[var(--scry-muted)]">{row.description}</p>
                ) : null}
              </TableCell>
              <TableCell className="hidden sm:table-cell">
                <div className="flex gap-1">
                  {row.kinds.map((kind) => (
                    <Badge key={kind} tone="neutral" className="px-1.5 py-0 text-[10.5px]">
                      {t(listKindLabelKey(kind))}
                    </Badge>
                  ))}
                </div>
              </TableCell>
              {showSchedule ? (
                <TableCell className="hidden whitespace-nowrap text-[12px] text-[var(--scry-muted)] md:table-cell">
                  {interval ? t("lists.catalog.every", { interval: t(interval.key, { count: interval.count }) }) : null}
                </TableCell>
              ) : null}
              <TableCell className="text-right">
                <Button
                  id={row.followId}
                  type="button"
                  size="xs"
                  variant={row.followed ? "outline" : "primary"}
                  disabled={row.followed}
                  onClick={row.onFollow}
                >
                  {row.followed ? <Check className="h-3.5 w-3.5" /> : <Plus className="h-3.5 w-3.5" />}
                  {t(row.followed ? "lists.catalog.followed" : "lists.catalog.follow")}
                </Button>
              </TableCell>
            </TableRow>
          );
        })}
      </TableBody>
    </Table>
  );
}
