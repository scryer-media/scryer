import * as React from "react";
import { cn } from "@/lib/utils";

type TableOverflow = "auto" | "clip" | "visible";
type TableLayout = "auto" | "fixed";
type TableDensity = "default" | "dense";

type TableProps = React.TableHTMLAttributes<HTMLTableElement> & {
  wrapperClassName?: string;
  overflow?: TableOverflow;
  layout?: TableLayout;
  density?: TableDensity;
};

function Table({
  className,
  wrapperClassName,
  overflow = "auto",
  layout = "auto",
  density = "default",
  ...props
}: TableProps) {
  const overflowClass =
    overflow === "clip"
      ? "overflow-x-hidden"
      : overflow === "visible"
        ? "overflow-visible"
        : "overflow-x-auto";

  return (
    <div
      data-slot="table-wrapper"
      className={cn("relative w-full", overflowClass, wrapperClassName)}
    >
      <table
        data-slot="table"
        data-density={density}
        data-layout={layout}
        className={cn(
          "w-full caption-bottom",
          density === "dense" ? "text-[13px]" : "text-sm",
          layout === "fixed" && "table-fixed",
          className,
        )}
        {...props}
      />
    </div>
  );
}

function TableHeader({
  className,
  ...props
}: React.HTMLAttributes<HTMLTableSectionElement>) {
  return (
    <thead
      data-slot="table-header"
      className={cn("[&_tr]:border-b", className)}
      {...props}
    />
  );
}

function TableBody({
  className,
  ...props
}: React.HTMLAttributes<HTMLTableSectionElement>) {
  return (
    <tbody
      data-slot="table-body"
      className={cn("[&_tr:last-child]:border-0", className)}
      {...props}
    />
  );
}

function TableRow({ className, ...props }: React.ComponentProps<"tr">) {
  return (
    <tr
      data-slot="table-row"
      className={cn("border-b border-border transition-colors", className)}
      {...props}
    />
  );
}

function TableHead({
  className,
  ...props
}: React.ThHTMLAttributes<HTMLTableHeaderCellElement>) {
  return (
    <th
      data-slot="table-head"
      className={cn(
        "h-11 px-3 text-left align-middle font-medium text-foreground bg-muted border-b border-border",
        className,
      )}
      {...props}
    />
  );
}

function TableCell({
  className,
  ...props
}: React.TdHTMLAttributes<HTMLTableCellElement>) {
  return (
    <td
      data-slot="table-cell"
      className={cn("px-3 py-2 align-middle", className)}
      {...props}
    />
  );
}

function TableCheckboxHead({
  className,
  ...props
}: React.ThHTMLAttributes<HTMLTableHeaderCellElement>) {
  return (
    <TableHead
      data-slot="table-checkbox-head"
      className={cn("w-12 px-2 text-center", className)}
      {...props}
    />
  );
}

function TableCheckboxCell({
  className,
  ...props
}: React.TdHTMLAttributes<HTMLTableCellElement>) {
  return (
    <TableCell
      data-slot="table-checkbox-cell"
      className={cn("w-12 px-2 text-center", className)}
      {...props}
    />
  );
}

function TableActionsHead({
  className,
  ...props
}: React.ThHTMLAttributes<HTMLTableHeaderCellElement>) {
  return (
    <TableHead
      data-slot="table-actions-head"
      className={cn("w-32 px-3 text-center", className)}
      {...props}
    />
  );
}

function TableActionsCell({
  className,
  ...props
}: React.TdHTMLAttributes<HTMLTableCellElement>) {
  return (
    <TableCell
      data-slot="table-actions-cell"
      className={cn("w-32 px-3 text-center", className)}
      {...props}
    />
  );
}

function TableCodeCell({
  className,
  ...props
}: React.TdHTMLAttributes<HTMLTableCellElement>) {
  return (
    <TableCell
      data-slot="table-code-cell"
      className={cn(
        "whitespace-nowrap font-[var(--font-code)] tabular-nums",
        className,
      )}
      {...props}
    />
  );
}

export {
  Table,
  TableActionsCell,
  TableActionsHead,
  TableBody,
  TableCell,
  TableCheckboxCell,
  TableCheckboxHead,
  TableCodeCell,
  TableHead,
  TableHeader,
  TableRow,
};
