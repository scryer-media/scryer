import { ArrowDown, ArrowUp, ArrowUpDown } from "lucide-react";

import { TableHead } from "@/components/ui/table";
import { cn } from "@/lib/utils";

type SortableHeadProps = {
  id: string;
  label: string;
  /** Whether the table is sorted by this column, and which way. */
  active: boolean;
  descending: boolean;
  onSort: () => void;
  className?: string;
};

/** A column heading that sorts its table: the first press sorts ascending, the next reverses it. */
export function SortableHead({ id, label, active, descending, onSort, className }: SortableHeadProps) {
  const Icon = !active ? ArrowUpDown : descending ? ArrowDown : ArrowUp;
  return (
    <TableHead className={className} aria-sort={active ? (descending ? "descending" : "ascending") : "none"}>
      <button
        id={id}
        type="button"
        className="inline-flex items-center gap-1.5 rounded-sm text-left font-medium transition-colors hover:text-[var(--scry-ink)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--scry-focus)]"
        onClick={onSort}
      >
        <span>{label}</span>
        <Icon aria-hidden="true" className={cn("h-3.5 w-3.5", !active && "text-[var(--scry-faint2)]")} />
      </button>
    </TableHead>
  );
}
