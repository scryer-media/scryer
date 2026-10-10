import * as React from "react";
import { Check, Eye, Plus } from "lucide-react";

import { LoadingMark } from "@/components/common/loading-mark";
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
import type { ListPreview, ListPreviewItem } from "@/lib/types/lists";
import type { Facet } from "@/lib/types/titles";
import { listIntervalParts, listKindLabelKey } from "@/lib/utils/lists";

import { ListPreviewPoster } from "./list-preview-summary";
import { SortableHead } from "./sortable-head";

/** How many of a list's titles its row previews. */
const PREVIEW_TITLES = 5;

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
  /** Reads the list as it stands, with no filters; left out when it cannot be read without more input. */
  onPreview?: () => Promise<ListPreview | null>;
};

type CatalogSortKey = "name" | "type";
type CatalogSort = { key: CatalogSortKey; descending: boolean };
/** A row's preview: still loading, or the titles it found. */
type RowPreview = { titles: ListPreviewItem[] | null };

/**
 * The lists a provider offers, one per row, each with its follow action and a
 * preview of its first titles. Until a column is picked the rows keep the
 * order the provider gives them.
 */
export function CatalogListTable({ id, rows }: { id: string; rows: readonly CatalogListRow[] }) {
  const t = useTranslate();
  const [sort, setSort] = React.useState<CatalogSort | null>(null);
  const [previews, setPreviews] = React.useState<Readonly<Record<string, RowPreview>>>({});
  const showSchedule = rows.some((row) => row.intervalSeconds != null);
  const showPreview = rows.some((row) => row.onPreview);
  const columns = 3 + (showSchedule ? 1 : 0);

  const sorted = React.useMemo(() => {
    if (!sort) return rows;
    const factor = sort.descending ? -1 : 1;
    const byName = (a: CatalogListRow, b: CatalogListRow) =>
      a.name.localeCompare(b.name, undefined, { sensitivity: "base", numeric: true });
    const type = (row: CatalogListRow) => row.kinds.map((kind) => t(listKindLabelKey(kind))).join(" ");
    return [...rows].sort((a, b) => {
      if (sort.key === "name") return byName(a, b) * factor;
      return type(a).localeCompare(type(b)) * factor || byName(a, b);
    });
  }, [rows, sort, t]);

  const sortableHead = (key: CatalogSortKey, label: string, className?: string) => {
    const active = sort?.key === key;
    return (
      <SortableHead
        id={`${id}-sort-${key}`}
        label={label}
        className={className}
        active={active}
        descending={active && sort.descending}
        onSort={() => setSort({ key, descending: active ? !sort.descending : false })}
      />
    );
  };

  const togglePreview = (row: CatalogListRow) => {
    if (!row.onPreview) return;
    if (previews[row.key]) {
      setPreviews(({ [row.key]: _closed, ...open }) => open);
      return;
    }
    setPreviews((open) => ({ ...open, [row.key]: { titles: null } }));
    void row.onPreview().then((preview) => {
      setPreviews((open) => {
        // Closed while it loaded, or the read failed and has already been reported.
        if (!open[row.key]) return open;
        if (!preview) {
          const { [row.key]: _failed, ...rest } = open;
          return rest;
        }
        return { ...open, [row.key]: { titles: preview.wouldAdd.slice(0, PREVIEW_TITLES) } };
      });
    });
  };

  return (
    <Table
      id={id}
      data-ui="list-catalog-table"
      density="dense"
      wrapperClassName="rounded-[12px] border border-[var(--scry-border3)]"
    >
      <TableHeader>
        <TableRow>
          {sortableHead("name", t("lists.table.list"))}
          {sortableHead("type", t("label.type"), "hidden w-[1%] whitespace-nowrap sm:table-cell")}
          {showSchedule ? (
            <TableHead className="hidden w-[1%] whitespace-nowrap md:table-cell">{t("lists.detail.interval")}</TableHead>
          ) : null}
          <TableHead className="w-[1%] text-right">
            <span className="sr-only">{t("label.actions")}</span>
          </TableHead>
        </TableRow>
      </TableHeader>
      <TableBody>
        {sorted.map((row) => {
          const interval = row.intervalSeconds != null ? listIntervalParts(row.intervalSeconds) : null;
          const preview = previews[row.key];
          const previewId = `${row.followId}-preview`;
          return (
            <React.Fragment key={row.key}>
              <TableRow
                data-ui="list-catalog-row"
                className={preview ? "border-b-0 hover:bg-[var(--scry-hover)]" : "hover:bg-[var(--scry-hover)]"}
              >
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
                  <div className="flex items-center justify-end gap-1.5">
                    {showPreview ? (
                      <Button
                        id={`${row.followId}-preview-toggle`}
                        type="button"
                        size="xs"
                        variant={preview ? "secondary" : "outline"}
                        className={row.onPreview ? undefined : "invisible"}
                        disabled={!row.onPreview}
                        aria-expanded={Boolean(preview)}
                        aria-controls={preview ? previewId : undefined}
                        onClick={() => togglePreview(row)}
                      >
                        <Eye className="h-3.5 w-3.5" />
                        {t("lists.preview.run")}
                      </Button>
                    ) : null}
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
                  </div>
                </TableCell>
              </TableRow>
              {preview ? (
                <TableRow id={previewId} data-ui="list-catalog-preview">
                  <TableCell colSpan={columns} className="pb-3 pt-0">
                    {preview.titles === null ? (
                      <p role="status" className="flex items-center gap-2 text-[12.5px] text-[var(--scry-muted)]">
                        <LoadingMark className="h-4 w-4" />
                        {t("label.loading")}
                      </p>
                    ) : preview.titles.length === 0 ? (
                      <p className="text-[12.5px] text-[var(--scry-muted)]">{t("lists.preview.nothingToAdd")}</p>
                    ) : (
                      <ul className="grid max-w-[560px] grid-cols-5 gap-2">
                        {preview.titles.map((title) => (
                          <li key={title.itemKey} className="min-w-0">
                            <ListPreviewPoster item={title} />
                          </li>
                        ))}
                      </ul>
                    )}
                  </TableCell>
                </TableRow>
              ) : null}
            </React.Fragment>
          );
        })}
      </TableBody>
    </Table>
  );
}
