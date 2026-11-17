import { ArrowDown, ArrowUp, ArrowUpDown } from "lucide-react";
import type * as React from "react";

import { InfoHelp } from "@/components/common/info-help";
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
  /// Explains the column, beside the sort button rather than inside it.
  help?: React.ReactNode;
};

/** A column heading that sorts its table: the first press sorts ascending, the next reverses it. */
export function SortableHead({
  id,
  label,
  active,
  descending,
  onSort,
  className,
  help,
}: SortableHeadProps) {
  const Icon = !active ? ArrowUpDown : descending ? ArrowDown : ArrowUp;
  const button = (
    <button
        id={id}
        type="button"
        className="inline-flex items-center gap-1.5 rounded-sm text-left font-medium transition-colors hover:text-[var(--scry-ink)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--scry-focus)]"
        onClick={onSort}
      >
        <span>{label}</span>
        <Icon
          aria-hidden="true"
          className={cn("h-3.5 w-3.5", !active && "text-[var(--scry-faint2)]")}
        />
      </button>
  );
  return (
    <TableHead
      className={className}
      aria-sort={active ? (descending ? "descending" : "ascending") : "none"}
    >
      {help ? (
        <span className="inline-flex items-center gap-1">
          {button}
          <InfoHelp text={help} ariaLabel={label} />
        </span>
      ) : (
        button
      )}
    </TableHead>
  );
}
