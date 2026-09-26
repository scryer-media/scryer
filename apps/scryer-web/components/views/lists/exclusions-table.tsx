import * as React from "react";
import { Ban, Trash2 } from "lucide-react";

import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { LoadingMark } from "@/components/common/loading-mark";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input, integerInputProps, sanitizeDigits } from "@/components/ui/input";
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
import { useUiDateTimeFormat } from "@/lib/context/ui-settings-context";
import type { ListExclusion, ListSubscription } from "@/lib/types/lists";
import type { Facet } from "@/lib/types/titles";
import { formatUiDateTime } from "@/lib/utils/date-format";
import {
  LIST_KINDS,
  listKindLabelKey,
  parseExternalIdList,
  type AddListExclusionInput,
} from "@/lib/utils/lists";

const ALL_LISTS = "__all__";

type ExclusionsTableProps = {
  exclusions: ListExclusion[];
  subscriptions: ListSubscription[];
  loading: boolean;
  busyIds: ReadonlySet<string>;
  onAdd: (input: AddListExclusionInput) => Promise<boolean>;
  onRemove: (exclusion: ListExclusion) => Promise<boolean>;
};

/** Titles no list may add, for every list or for one public list. */
export function ExclusionsTable({
  exclusions,
  subscriptions,
  loading,
  busyIds,
  onAdd,
  onRemove,
}: ExclusionsTableProps) {
  const t = useTranslate();
  const dateTimeFormat = useUiDateTimeFormat();
  const [title, setTitle] = React.useState("");
  const [year, setYear] = React.useState("");
  const [kind, setKind] = React.useState<Facet>("MOVIE");
  const [ids, setIds] = React.useState("");
  const [scope, setScope] = React.useState(ALL_LISTS);
  const [adding, setAdding] = React.useState(false);
  const [pendingRemoval, setPendingRemoval] = React.useState<ListExclusion | null>(null);
  const parsedIds = parseExternalIdList(ids);
  const canAdd = !adding && title.trim().length > 0 && parsedIds.length > 0;

  const submit = async (event: React.FormEvent) => {
    event.preventDefault();
    if (!canAdd) return;
    setAdding(true);
    try {
      const ok = await onAdd({
        kind,
        externalIds: parsedIds,
        displayTitle: title.trim(),
        year: year ? Number(year) : null,
        scope: scope === ALL_LISTS ? "ALL_LISTS" : "LIST",
        subscriptionId: scope === ALL_LISTS ? null : scope,
      });
      if (ok) {
        setTitle("");
        setYear("");
        setIds("");
      }
    } finally {
      setAdding(false);
    }
  };

  return (
    <div className="space-y-4">
      <form
        id="list-exclusion-add"
        onSubmit={(event) => void submit(event)}
        className="space-y-3 rounded-[12px] border border-[var(--scry-border3)] bg-[var(--scry-surf)] p-4"
      >
        <h2 className="text-[13px] font-semibold text-[var(--scry-ink2)]">{t("lists.exclusions.addHeading")}</h2>
        <div className="grid gap-3 sm:grid-cols-[minmax(0,2fr)_100px_140px]">
          <label className="space-y-1.5" htmlFor="list-exclusion-title">
            <span className="block text-sm font-medium text-[var(--scry-ink2)]">{t("lists.exclusions.titleLabel")}</span>
            <Input id="list-exclusion-title" value={title} onChange={(event) => setTitle(event.target.value)} />
          </label>
          <label className="space-y-1.5" htmlFor="list-exclusion-year">
            <span className="block text-sm font-medium text-[var(--scry-ink2)]">{t("lists.exclusions.yearLabel")}</span>
            <Input
              id="list-exclusion-year"
              {...integerInputProps}
              maxLength={4}
              value={year}
              onChange={(event) => setYear(sanitizeDigits(event.target.value))}
            />
          </label>
          <SingleSelectField
            id="list-exclusion-kind"
            label={t("lists.exclusions.kindLabel")}
            value={kind}
            options={LIST_KINDS.map((value) => ({ value, label: t(listKindLabelKey(value)) }))}
            onValueChange={(value) => setKind(value as Facet)}
          />
        </div>
        <div className="grid gap-3 sm:grid-cols-2">
          <label className="space-y-1.5" htmlFor="list-exclusion-ids">
            <span className="block text-sm font-medium text-[var(--scry-ink2)]">{t("lists.exclusions.idsLabel")}</span>
            <Input
              id="list-exclusion-ids"
              placeholder="tmdb:603, imdb:tt0133093"
              value={ids}
              onChange={(event) => setIds(event.target.value)}
            />
            <span className="block text-xs text-[var(--scry-muted)]">{t("lists.exclusions.idsHelp")}</span>
          </label>
          <SingleSelectField
            id="list-exclusion-scope"
            label={t("lists.exclusions.scopeLabel")}
            value={scope}
            options={[
              { value: ALL_LISTS, label: t("lists.exclusions.scopeAll") },
              ...subscriptions.map((subscription) => ({
                value: subscription.id,
                label: t("lists.exclusions.scopeList", { name: subscription.name }),
              })),
            ]}
            onValueChange={setScope}
          />
        </div>
        <div className="flex justify-end">
          <Button id="list-exclusion-add-submit" type="submit" disabled={!canAdd}>
            {adding ? <LoadingMark className="h-4 w-4" /> : <Ban className="h-4 w-4" />}
            {t("lists.exclusions.add")}
          </Button>
        </div>
      </form>

      {loading && exclusions.length === 0 ? (
        <div className="flex items-center gap-2 text-sm text-[var(--scry-muted)]">
          <LoadingMark className="h-4 w-4" />
          {t("label.loading")}
        </div>
      ) : exclusions.length === 0 ? (
        <p id="list-exclusions-empty" className="text-[13px] text-[var(--scry-muted)]">
          {t("lists.exclusions.empty")}
        </p>
      ) : (
        <Table id="list-exclusions-table" wrapperClassName="rounded-[12px] border border-[var(--scry-border3)]">
          <TableHeader>
            <TableRow>
              <TableHead>{t("lists.exclusions.titleLabel")}</TableHead>
              <TableHead className="hidden sm:table-cell">{t("lists.exclusions.idsLabel")}</TableHead>
              <TableHead>{t("lists.exclusions.scopeLabel")}</TableHead>
              <TableHead className="hidden md:table-cell">{t("lists.exclusions.created")}</TableHead>
              <TableHead className="w-[1%] text-right">{t("label.actions")}</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {exclusions.map((exclusion) => (
              <TableRow
                key={exclusion.id}
                id={`list-exclusion-${exclusion.id}`}
                data-exclusion-id={exclusion.id}
              >
                <TableCell>
                  <span className="block text-[13.5px] font-medium text-[var(--scry-ink2)]">
                    {exclusion.displayTitle}
                    {exclusion.year ? <span className="text-[var(--scry-muted)]"> ({exclusion.year})</span> : null}
                  </span>
                  <span className="text-[11.5px] text-[var(--scry-muted)]">{t(listKindLabelKey(exclusion.kind))}</span>
                </TableCell>
                <TableCell className="hidden sm:table-cell">
                  <span className="font-[var(--font-code)] text-[12px] text-[var(--scry-muted)]">
                    {exclusion.externalIds.map((id) => `${id.source}:${id.value}`).join(", ")}
                  </span>
                </TableCell>
                <TableCell>
                  {exclusion.scope === "ALL_LISTS" ? (
                    <Badge tone="neutral">{t("lists.exclusions.scopeAll")}</Badge>
                  ) : (
                    <Badge tone="info">
                      {t("lists.exclusions.scopeList", {
                        name: exclusion.subscriptionName ?? exclusion.subscriptionId ?? "",
                      })}
                    </Badge>
                  )}
                </TableCell>
                <TableCell className="hidden text-[12px] text-[var(--scry-muted)] md:table-cell">
                  {formatUiDateTime(exclusion.createdAt, dateTimeFormat)}
                </TableCell>
                <TableCell className="text-right">
                  {/* Not `list-exclusion-…`: that id family belongs to the rows alone. */}
                  <Button
                    id={`list-exclusions-remove-${exclusion.id}`}
                    type="button"
                    variant="ghost"
                    size="icon-sm"
                    aria-label={t("lists.exclusions.remove", { name: exclusion.displayTitle })}
                    disabled={busyIds.has(exclusion.id)}
                    onClick={() => setPendingRemoval(exclusion)}
                  >
                    <Trash2 className="h-4 w-4" />
                  </Button>
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      )}

      <ConfirmDialog
        open={pendingRemoval !== null}
        title={t("lists.exclusions.removeTitle", { name: pendingRemoval?.displayTitle ?? "" })}
        description={t("lists.exclusions.removeDescription")}
        confirmLabel={t("label.remove")}
        cancelLabel={t("label.cancel")}
        contentId="list-exclusion-remove-dialog"
        confirmButtonId="list-exclusion-remove-confirm"
        isBusy={pendingRemoval ? busyIds.has(pendingRemoval.id) : false}
        onCancel={() => setPendingRemoval(null)}
        onConfirm={async () => {
          if (!pendingRemoval) return;
          const ok = await onRemove(pendingRemoval);
          if (ok) setPendingRemoval(null);
        }}
      />
    </div>
  );
}
