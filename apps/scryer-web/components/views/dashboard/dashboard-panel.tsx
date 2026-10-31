import type { LucideIcon } from "lucide-react";
import { ChevronRight } from "lucide-react";
import { Link } from "react-router";

import { Badge } from "@/components/ui/badge";
import { Card } from "@/components/ui/card";
import { cn } from "@/lib/utils";

/**
 * The shared shell every dashboard panel sits in: `Card` with a dense header
 * carrying an icon, a title, optional count/health pills and an optional link
 * out to the full page.
 *
 * Purely a composition of `Card`, `Badge` and react-router's `Link` — it exists
 * so the panels share one header rhythm and any panel added later inherits it,
 * not because the app was missing a primitive.
 *
 * The shell fills its dashboard grid cell; the body takes the height left
 * under the header, and `bodyClassName` decides how it scrolls.
 */
export function DashboardPanel({
  icon: Icon,
  title,
  count,
  pills,
  linkTo,
  linkLabel,
  bodyClassName,
  className,
  children,
}: {
  icon: LucideIcon;
  title: string;
  /** Full total for the badge, not the number of visible rows. */
  count?: number | null;
  /** Extra header chips, e.g. "4/5 healthy" · "1 erroring". */
  pills?: React.ReactNode;
  linkTo?: string;
  linkLabel?: string;
  bodyClassName?: string;
  className?: string;
  children: React.ReactNode;
}) {
  return (
    <Card
      data-slot="dashboard-panel"
      className={cn(
        "flex h-full min-h-0 min-w-0 flex-col gap-0 p-0",
        className,
      )}
    >
      <div className="flex min-w-0 shrink-0 flex-wrap items-center gap-x-2 gap-y-1 border-b border-border px-3 py-2">
        <Icon
          className="h-4 w-4 shrink-0 text-[var(--scry-muted2)]"
          aria-hidden="true"
        />
        <h2 className="min-w-0 truncate text-[13px] font-semibold text-[var(--scry-ink2)]">
          {title}
        </h2>
        {typeof count === "number" ? (
          <Badge className="tabular-nums">{count}</Badge>
        ) : null}
        {pills}
        {linkTo && linkLabel ? (
          <Link
            to={linkTo}
            className="ml-auto inline-flex shrink-0 items-center gap-0.5 whitespace-nowrap text-[11px] font-medium text-[var(--scry-muted)] transition-colors hover:text-[var(--scry-ink2)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--scry-accent-ring)] focus-visible:ring-offset-0"
          >
            {linkLabel}
            <ChevronRight className="h-3 w-3" aria-hidden="true" />
          </Link>
        ) : null}
      </div>
      <div className={cn("min-h-0 min-w-0 flex-1", bodyClassName)}>
        {children}
      </div>
    </Card>
  );
}

/**
 * Placeholder line for a panel with nothing to show. Kept next to the shell so
 * every empty panel reads the same and none of them collapse to zero height.
 */
export function DashboardPanelEmpty({ message }: { message: string }) {
  return (
    <p className="px-3 py-6 text-center text-[12px] text-[var(--scry-muted2)]">
      {message}
    </p>
  );
}
