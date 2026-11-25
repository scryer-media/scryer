import { Button } from "@/components/ui/button";
import { useTranslate } from "@/lib/context/translate-context";
import type { RuleTablePage } from "@/lib/utils/rule-table";

/**
 * The previous and next buttons under a rule table, shown only when the
 * rules span more than one page. Ids derive from `idPrefix`:
 * `-pager`, `-page-range`, `-previous-page` and `-next-page`.
 */
export function RuleTablePager({
  idPrefix,
  page,
  onPageChange,
}: {
  idPrefix: string;
  page: RuleTablePage<unknown>;
  onPageChange: (page: number) => void;
}) {
  const t = useTranslate();
  if (page.pageCount <= 1) return null;
  return (
    <div
      id={`${idPrefix}-pager`}
      className="flex items-center justify-between gap-3 border-t border-border px-3 py-2"
    >
      <p
        id={`${idPrefix}-page-range`}
        className="text-[12.5px] text-[var(--scry-muted3)]"
      >
        {t("pendingImports.pageRange", {
          start: page.start,
          end: page.end,
          total: page.total,
        })}
      </p>
      <div className="flex items-center gap-2">
        <Button
          id={`${idPrefix}-previous-page`}
          type="button"
          variant="secondary"
          size="sm"
          disabled={page.page === 0}
          onClick={() => onPageChange(page.page - 1)}
        >
          {t("pendingImports.prev")}
        </Button>
        <Button
          id={`${idPrefix}-next-page`}
          type="button"
          variant="secondary"
          size="sm"
          disabled={page.page >= page.pageCount - 1}
          onClick={() => onPageChange(page.page + 1)}
        >
          {t("pendingImports.next")}
        </Button>
      </div>
    </div>
  );
}
