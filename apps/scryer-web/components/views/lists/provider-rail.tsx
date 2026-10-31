import type * as React from "react";

import { cn } from "@/lib/utils";

/** The column of providers beside a lists pane; it scrolls sideways where there is no room for a column. */
export function ProviderRail({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <nav
      aria-label={label}
      className="flex gap-1.5 overflow-x-auto md:flex-col md:overflow-visible"
    >
      {children}
    </nav>
  );
}

type ProviderRailEntryProps = {
  id: string;
  active: boolean;
  onSelect: () => void;
  /** The provider's tile, or an icon the same size. */
  icon: React.ReactNode;
  name: string;
  /** A second line under the name, such as the linked account. */
  detail?: React.ReactNode;
};

/** One provider in the rail; the open one is marked. */
export function ProviderRailEntry({
  id,
  active,
  onSelect,
  icon,
  name,
  detail,
}: ProviderRailEntryProps) {
  return (
    <button
      id={id}
      type="button"
      aria-pressed={active}
      onClick={onSelect}
      className={cn(
        "flex flex-none items-center gap-2.5 rounded-[10px] border px-2.5 py-2 text-left transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--scry-focus)]",
        active
          ? "border-[var(--scry-baccent)] bg-[rgba(var(--scry-accent-rgb),0.12)]"
          : "border-transparent hover:bg-[var(--scry-hover)]",
      )}
    >
      {icon}
      <span className="min-w-0">
        <span className="block truncate text-[13px] font-semibold text-[var(--scry-ink2)]">
          {name}
        </span>
        {detail ? (
          <span className="block truncate text-[11.5px] text-[var(--scry-muted)]">
            {detail}
          </span>
        ) : null}
      </span>
    </button>
  );
}
