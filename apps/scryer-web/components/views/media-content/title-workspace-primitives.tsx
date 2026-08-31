import * as React from "react";
import { Check, Loader2, X } from "lucide-react";
import { cn } from "@/lib/utils";

import { TitlePosterSlot } from "@/components/title-poster-slot";
import { ActionTooltip } from "@/components/ui/tooltip";
import { selectPosterVariantUrl } from "@/lib/utils/poster-images";

export function TitleWorkspaceHero({
  backgroundUrl,
  closeLabel,
  onClose,
  headerActions,
  children,
}: {
  backgroundUrl?: string | null;
  closeLabel: string;
  onClose: () => void;
  headerActions?: React.ReactNode;
  children: React.ReactNode;
}) {
  const [failedBackgroundUrl, setFailedBackgroundUrl] = React.useState<string | null>(null);
  React.useEffect(() => {
    setFailedBackgroundUrl(null);
  }, [backgroundUrl]);
  const displayedBackgroundUrl =
    failedBackgroundUrl === backgroundUrl ? null : backgroundUrl;
  return (
    <section className="relative mb-3 overflow-hidden bg-[var(--scry-bg)]">
      {displayedBackgroundUrl ? (
        <img
          src={displayedBackgroundUrl}
          alt=""
          aria-hidden="true"
          className="absolute inset-0 h-full w-full object-cover opacity-45 saturate-90"
          loading="lazy"
          decoding="async"
          onError={() => setFailedBackgroundUrl(displayedBackgroundUrl)}
        />
      ) : null}
      <div className="absolute inset-0 bg-[linear-gradient(105deg,rgba(8,12,22,0.96)_30%,rgba(8,12,22,0.55)_70%,rgba(8,12,22,0.2))]" />
      <div className="absolute inset-x-0 bottom-0 h-1/2 bg-gradient-to-b from-transparent to-[var(--scry-bg)]" />
      <div className="absolute right-2.5 top-2.5 z-10 flex items-center gap-1.5">
        {headerActions}
        <ActionTooltip content={closeLabel}>
          <button
            type="button"
            aria-label={closeLabel}
            className="flex size-8 items-center justify-center rounded-[9px] border !border-[rgba(var(--scry-accent-rgb),0.55)] bg-slate-950/60 text-[#dde4f5] backdrop-blur-md transition hover:bg-slate-950/75 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--scry-focus)]"
            onClick={onClose}
          >
            <X className="h-4 w-4" />
          </button>
        </ActionTooltip>
      </div>
      <div className="relative flex flex-col gap-4 p-[18px] sm:flex-row sm:pr-16">
        {children}
      </div>
    </section>
  );
}

export function TitleWorkspacePosterFrame({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <div className="relative mx-auto h-[300px] w-[200px] shrink-0 overflow-hidden rounded-[9px] border border-[#2a3556] bg-[var(--scry-inset)] shadow-[0_8px_22px_rgba(0,0,0,0.5)] sm:mx-0">
      {children}
    </div>
  );
}

export function TitleWorkspaceActionGrid({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <div className="mb-3 grid grid-cols-12 overflow-hidden rounded-[12px] border border-[var(--scry-border)] bg-[var(--scry-border)] [gap:1px] [&>*:nth-child(-n+4)]:col-span-3 [&>*:nth-child(n+5)]:col-span-4 sm:grid-cols-7 sm:[&>*:nth-child(-n+4)]:col-span-1 sm:[&>*:nth-child(n+5)]:col-span-1">
      {children}
    </div>
  );
}

export function TitleWorkspaceActionButton({
  id,
  icon: Icon,
  label,
  loading = false,
  destructive = false,
  active = false,
  pressed,
  disabled = false,
  expanded,
  controlsId,
  onClick,
}: {
  id?: string;
  icon: React.ComponentType<{ className?: string }>;
  label: string;
  loading?: boolean;
  destructive?: boolean;
  active?: boolean;
  pressed?: boolean;
  disabled?: boolean;
  expanded?: boolean;
  controlsId?: string;
  onClick: () => void;
}) {
  const actionDisabled = disabled || loading;

  return (
    <button
      id={id}
      type="button"
      aria-label={label}
      aria-pressed={pressed}
      aria-expanded={expanded}
      aria-controls={controlsId}
      className={cn(
        "flex min-h-[96px] min-w-0 flex-col items-center justify-center gap-2 bg-[var(--scry-card2)] px-2 py-3 text-[var(--scry-muted2)] transition hover:bg-[var(--scry-hover)] hover:text-[var(--scry-ink2)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-[var(--scry-focus)] disabled:cursor-not-allowed disabled:opacity-55",
        active &&
          "bg-[rgba(var(--scry-accent-rgb),0.13)] text-[var(--scry-accent-text)] shadow-[inset_0_-2px_0_var(--scry-accent-ring)]",
        destructive && "text-destructive hover:text-destructive",
      )}
      disabled={actionDisabled}
      onClick={onClick}
    >
      {loading ? (
        <Loader2 className="size-6 animate-spin" />
      ) : (
        <Icon className="size-6" />
      )}
      <span className="truncate text-center text-[11px] font-bold uppercase tracking-[0.04em]">
        {label}
      </span>
    </button>
  );
}

export function TitleWorkspaceSectionCard({
  children,
  className,
}: {
  children: React.ReactNode;
  className?: string;
}) {
  return (
    <section
      className={cn(
        "rounded-[12px] border border-[var(--scry-border)] bg-[var(--scry-card2)] p-4",
        className,
      )}
    >
      {children}
    </section>
  );
}

export function TitleWorkspaceHeaderLink({
  icon: Icon,
  label,
  onClick,
}: {
  icon: React.ComponentType<{ className?: string }>;
  label: string;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      className="inline-flex h-7 items-center gap-1 rounded-[7px] px-1.5 text-[12px] font-semibold text-[var(--scry-accent-ring)] transition hover:bg-[rgba(var(--scry-accent-rgb),0.12)] hover:text-[var(--scry-accent-text)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--scry-focus)]"
      onClick={onClick}
    >
      <span>{label}</span>
      <Icon className="h-[13px] w-[13px]" />
    </button>
  );
}

export function TitleWorkspaceSectionHeader({
  icon: Icon,
  title,
  action,
}: {
  icon: React.ComponentType<{ className?: string }>;
  title: React.ReactNode;
  action?: React.ReactNode;
}) {
  return (
    <div className="mb-3.5 flex min-w-0 items-center justify-between gap-3">
      <div className="flex min-w-0 items-center gap-2.5">
        <Icon className="h-4 w-4 shrink-0 text-[var(--scry-accent-text)]" />
        <h3 className="truncate text-[14px] font-semibold text-[var(--scry-ink2)]">
          {title}
        </h3>
      </div>
      {action ? <div className="shrink-0">{action}</div> : null}
    </div>
  );
}

type TitleBulkPosterStackItem = {
  id: string;
  name: string;
  posterUrl?: string | null;
  metadataFetchedAt?: string | null;
  createdAt?: string | null;
};

const TITLE_BULK_POSTER_STACK_SLOTS = [
  {
    transform: "translateX(0) rotate(0deg)",
    zIndex: 3,
    opacity: 1,
    boxShadow:
      "0 16px 34px rgba(0,0,0,0.5), 0 0 0 1.5px var(--scry-accent-ring)",
  },
  {
    transform: "translateX(-51px) translateY(16px) rotate(-7deg) scale(0.94)",
    zIndex: 2,
    opacity: 0.82,
    boxShadow: "0 12px 26px rgba(0,0,0,0.42)",
  },
  {
    transform: "translateX(51px) translateY(23px) rotate(7deg) scale(0.88)",
    zIndex: 1,
    opacity: 0.6,
    boxShadow: "0 10px 22px rgba(0,0,0,0.36)",
  },
] as const;

/**
 * Fanned, art-only poster stack for the bulk multi-select panel. Renders up to
 * three of the selected titles' posters; the top card sits upright with an
 * accent ring + check badge and the two behind fan out with rotation. Purely
 * decorative — the "{n} selected" count beside it is the accessible readout.
 */
export function TitleBulkPosterStack({
  titles,
}: {
  titles: ReadonlyArray<TitleBulkPosterStackItem>;
}) {
  const cards = titles.slice(0, TITLE_BULK_POSTER_STACK_SLOTS.length);
  if (cards.length === 0) {
    return null;
  }

  return (
    <div
      aria-hidden="true"
      className="relative flex h-[276px] w-full items-center justify-center"
    >
      {cards.map((title, index) => {
        const slot = TITLE_BULK_POSTER_STACK_SLOTS[index];
        return (
          <div
            key={title.id}
            className="absolute h-[249px] w-[186px] overflow-hidden rounded-[20px] border border-white/20"
            style={{
              transform: slot.transform,
              zIndex: slot.zIndex,
              opacity: slot.opacity,
              boxShadow: slot.boxShadow,
            }}
          >
            <TitlePosterSlot
              src={selectPosterVariantUrl(title.posterUrl, "w250")}
              metadataFetchedAt={title.metadataFetchedAt}
              createdAt={title.createdAt}
              emptyLabel=""
              alt={title.name}
              className="h-full w-full object-cover"
              placeholderClassName="flex h-full w-full items-center justify-center bg-[#0c1322]"
            />
            <div
              className="pointer-events-none absolute inset-0"
              style={{
                background:
                  "linear-gradient(180deg,rgba(255,255,255,0.16) 0%,transparent 36%,rgba(0,0,0,0.32) 72%,rgba(0,0,0,0.62) 100%)",
              }}
            />
            {index === 0 ? (
              <span className="absolute right-3 top-3 flex h-[30px] w-[30px] items-center justify-center rounded-full bg-[var(--scry-accent)] shadow-[0_2px_8px_rgba(0,0,0,0.45)]">
                <Check className="h-[18px] w-[18px] text-[#04121f]" strokeWidth={3.4} />
              </span>
            ) : null}
          </div>
        );
      })}
    </div>
  );
}
