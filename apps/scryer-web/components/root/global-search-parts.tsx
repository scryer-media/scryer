import * as React from "react";
import type { LucideIcon } from "lucide-react";
import {
  ArrowRight,
  Eye,
  Info,
  Loader2,
  Search,
  SearchX,
} from "lucide-react";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import { TitlePosterSlot } from "@/components/title-poster-slot";
import { TitleCard } from "@/components/title-card";
import { cn } from "@/lib/utils";
import type { Facet } from "@/lib/types/titles";

type SearchSurface = "desktop" | "mobile";

type SearchResultDataAttribute =
  | "data-global-search-result"
  | "data-mobile-global-search-result";

type MetadataActionKind = "add" | "inCatalog" | "request" | "unavailable";
type SearchResultKind = "catalog" | "metadata";

type SearchResultExternalId = {
  source?: string | null;
  value?: string | number | null;
};

type SearchResultIdentityInput = {
  actionKind?: MetadataActionKind;
  externalIds?: SearchResultExternalId[] | null;
  facet: Facet;
  imdbId?: string | null;
  kind: SearchResultKind;
  smgId?: string | number | null;
  titleId?: string | null;
  titleName: string;
  tvdbId?: string | number | null;
  year?: string | number | null;
};

function searchResultAttribute(attribute: SearchResultDataAttribute) {
  return { [attribute]: "true" } as Partial<
    Record<SearchResultDataAttribute, "true">
  >;
}

function dataAttributeValue(value: string | number | null | undefined) {
  const text = String(value ?? "").trim();
  return text.length > 0 ? text : null;
}

function externalIdValue(
  externalIds: SearchResultExternalId[] | null | undefined,
  source: string,
) {
  return (
    externalIds
      ?.find(
        (externalId) =>
          externalId.source?.trim().toLowerCase() === source.toLowerCase(),
      )
      ?.value?.toString()
      .trim() || null
  );
}

function searchResultIdentityAttributes({
  actionKind,
  externalIds,
  facet,
  imdbId,
  kind,
  titleId,
  titleName,
  smgId,
  tvdbId,
  year,
}: SearchResultIdentityInput) {
  const normalizedImdbId =
    dataAttributeValue(imdbId) ?? externalIdValue(externalIds, "imdb");
  const normalizedTvdbId =
    dataAttributeValue(tvdbId) ?? externalIdValue(externalIds, "tvdb");
  const normalizedSmgId =
    dataAttributeValue(smgId) ?? externalIdValue(externalIds, "smg");
  return {
    "data-global-search-result-kind": kind,
    // Test-hook surface: facet is SCREAMING_SNAKE in app state, but DOM ids
    // and data attributes stay lowercase (same convention as selectorId).
    "data-global-search-result-facet": facet.toLowerCase(),
    "data-global-search-result-title": titleName,
    ...(actionKind
      ? { "data-global-search-result-action": actionKind }
      : {}),
    ...(dataAttributeValue(titleId)
      ? { "data-global-search-result-title-id": dataAttributeValue(titleId)! }
      : {}),
    ...(normalizedImdbId
      ? { "data-global-search-result-imdb-id": normalizedImdbId }
      : {}),
    ...(normalizedTvdbId
      ? { "data-global-search-result-tvdb-id": normalizedTvdbId }
      : {}),
    ...(normalizedSmgId
      ? { "data-global-search-result-smg-id": normalizedSmgId }
      : {}),
    ...(dataAttributeValue(year)
      ? { "data-global-search-result-year": dataAttributeValue(year)! }
      : {}),
  } satisfies Partial<Record<`data-${string}`, string>>;
}

export function SearchSectionLoading({
  compact = false,
  label,
}: {
  compact?: boolean;
  label: string;
}) {
  return (
    <div
      className={cn(
        "flex items-center justify-center gap-3 rounded-[12px] border border-dashed border-[var(--scry-border2)] bg-[linear-gradient(180deg,var(--scry-surfC),var(--scry-surfF))] px-4 py-3 text-sm text-[var(--scry-muted3)]",
        compact ? "min-h-20" : "min-h-24",
      )}
    >
      <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-[9px] bg-[var(--scry-chip)] text-[var(--scry-accent-ring)]">
        <Loader2 className="h-4 w-4 animate-spin" />
      </span>
      <span className="font-medium">{label}</span>
    </div>
  );
}

type SearchRouteCommandButtonProps = {
  ariaLabel: string;
  description: string;
  displayLabel: string;
  Icon: LucideIcon;
  onClick: () => void;
  onKeyDown: React.KeyboardEventHandler<HTMLButtonElement>;
  resultAttribute: SearchResultDataAttribute;
  showDescription: boolean;
  surface: SearchSurface;
};

export function SearchRouteCommandButton({
  ariaLabel,
  description,
  displayLabel,
  Icon,
  onClick,
  onKeyDown,
  resultAttribute,
  showDescription,
  surface,
}: SearchRouteCommandButtonProps) {
  const isDesktop = surface === "desktop";

  return (
    <button
      type="button"
      {...searchResultAttribute(resultAttribute)}
      className={cn(
        "group relative flex min-w-0 items-center gap-3 overflow-hidden rounded-[12px] border border-[var(--scry-border)] bg-[var(--scry-surfA)] p-3 text-left transition focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary/25",
        isDesktop
          ? "hover:border-[var(--scry-bhover)] hover:bg-[var(--scry-hover)]"
          : "w-full active:bg-[var(--scry-hover)]",
      )}
      onClick={onClick}
      onKeyDown={onKeyDown}
      aria-label={ariaLabel}
      title={ariaLabel}
    >
      <span
        aria-hidden="true"
        className={cn(
          "absolute inset-y-3 left-0 w-0.5 rounded-r-full bg-[var(--scry-accent-ring)] opacity-0 transition group-focus-visible:opacity-100",
          isDesktop ? "group-hover:opacity-100" : "group-active:opacity-100",
        )}
      />
      <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-[9px] border border-[rgba(var(--scry-accent-rgb),0.22)] bg-[rgba(var(--scry-accent-rgb),0.14)] text-[var(--scry-accent-text)]">
        <Icon className="h-4 w-4" />
      </span>
      <span className="min-w-0 flex-1">
        <span className="block truncate text-sm font-semibold text-[var(--scry-ink2)]">
          {displayLabel}
        </span>
        {showDescription ? (
          <span className="mt-0.5 block truncate text-xs text-[var(--scry-muted3)]">
            {description}
          </span>
        ) : null}
      </span>
      <span
        className={cn(
          "flex h-8 w-8 shrink-0 items-center justify-center rounded-[8px] text-[var(--scry-faint2)] transition",
          isDesktop
            ? "group-hover:bg-[var(--scry-chip)] group-hover:text-[var(--scry-ink2)]"
            : "group-active:bg-[var(--scry-chip)] group-active:text-[var(--scry-ink2)]",
        )}
      >
        <ArrowRight className="h-4 w-4" />
      </span>
    </button>
  );
}

type SearchCatalogResultButtonProps = {
  ariaLabel: string;
  createdAt?: string | null;
  emptyLabel: string;
  externalIds?: SearchResultExternalId[] | null;
  facet: Facet;
  facetLabel: string;
  id?: string;
  metadataFetchedAt?: string | null;
  monitoredLabel: string;
  onClick: () => void;
  onKeyDown: React.KeyboardEventHandler<HTMLButtonElement>;
  posterAlt: string;
  posterUrl?: string | null;
  resultAttribute: SearchResultDataAttribute;
  secondaryParts: Array<string | null | undefined>;
  surface: SearchSurface;
  titleId: string;
  titleName: string;
  viewLabel: string;
  year?: string | number | null;
};

export function SearchCatalogResultButton({
  ariaLabel,
  createdAt,
  emptyLabel,
  externalIds,
  facet,
  facetLabel,
  id,
  metadataFetchedAt,
  monitoredLabel,
  onClick,
  onKeyDown,
  posterAlt,
  posterUrl,
  resultAttribute,
  secondaryParts,
  surface,
  titleId,
  titleName,
  viewLabel,
  year,
}: SearchCatalogResultButtonProps) {
  const isDesktop = surface === "desktop";
  const visibleSecondaryParts = secondaryParts.filter(
    (part): part is string => Boolean(part),
  );
  const identityAttributes = searchResultIdentityAttributes({
    externalIds,
    facet,
    kind: "catalog",
    titleId,
    titleName,
    year,
  });

  return (
    <button
      id={id}
      type="button"
      onClick={onClick}
      {...searchResultAttribute(resultAttribute)}
      {...identityAttributes}
      onKeyDown={onKeyDown}
      className={cn(
        "group flex w-full items-center gap-[13px] rounded-[12px] border border-[var(--scry-border)] bg-[var(--scry-surfA)] p-2.5 text-left transition focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary/25",
        isDesktop
          ? "hover:border-[var(--scry-bhover)] hover:bg-[var(--scry-hover)]"
          : "flex-wrap shadow-[0_8px_20px_rgba(0,0,0,0.20)] active:bg-[var(--scry-hover)] sm:flex-nowrap",
      )}
      aria-label={ariaLabel}
      title={ariaLabel}
    >
      <div className="relative h-16 w-11 flex-none overflow-hidden rounded-[7px] border border-[var(--scry-border2)] bg-muted">
        <TitlePosterSlot
          src={posterUrl}
          metadataFetchedAt={metadataFetchedAt}
          createdAt={createdAt}
          alt={posterAlt}
          className="h-full w-full object-cover"
          placeholderClassName="flex h-full w-full items-center justify-center text-[10px] text-muted-foreground"
          emptyLabel={emptyLabel}
          loading="lazy"
        />
        <div className="pointer-events-none absolute inset-x-0 bottom-0 h-1/2 bg-gradient-to-t from-black/70 to-transparent" />
      </div>
      <div className="min-w-0 flex-1">
        <p className="truncate text-sm font-semibold text-[var(--scry-ink2)]">
          {titleName}
        </p>
        <p
          className={cn(
            "mt-0.5 truncate text-[var(--scry-muted)]",
            isDesktop ? "text-[11.5px]" : "text-xs",
          )}
        >
          {monitoredLabel}
          {visibleSecondaryParts.length > 0 ? (
            <>
              {" \u00b7 "}
              {visibleSecondaryParts.join(" \u00b7 ")}
            </>
          ) : null}
        </p>
        <div className="mt-2 flex min-w-0 flex-wrap items-center gap-2">
          <span
            className={cn(
              "truncate rounded-md bg-[rgba(var(--scry-accent-rgb),0.16)] px-2 py-0.5 text-[10px] font-bold uppercase tracking-wide text-[var(--scry-accent-text)]",
              isDesktop ? "max-w-[9rem]" : "max-w-[8rem]",
            )}
          >
            {facetLabel}
          </span>
        </div>
      </div>
      <span
        className={cn(
          "h-[34px] shrink-0 items-center gap-1.5 rounded-[9px] border border-[var(--scry-bhover2)] bg-[var(--scry-soft3)] text-[12.5px] font-semibold text-[var(--scry-body)]",
          isDesktop
            ? "hidden px-3.5 transition group-hover:border-primary/40 group-hover:text-[var(--scry-ink2)] sm:inline-flex"
            : "inline-flex w-full justify-center px-3 sm:w-auto sm:justify-start",
        )}
      >
        <Eye className="h-3.5 w-3.5" />
        {viewLabel}
      </span>
    </button>
  );
}

type SearchMetadataPosterButtonProps = {
  actionKind: MetadataActionKind;
  actionTitle: string;
  disabled: boolean;
  facet: Facet;
  id: string;
  imdbId?: string | null;
  name: string;
  onClick: () => void;
  onKeyDown: React.KeyboardEventHandler<HTMLButtonElement>;
  posterUrl?: string | null;
  resultAttribute: SearchResultDataAttribute;
  smgId?: string | number | null;
  tvdbId?: string | number | null;
  year?: string | number | null;
  yearLabel: string | number | null;
};

export function SearchMetadataPosterButton({
  actionKind,
  actionTitle,
  disabled,
  facet,
  id,
  imdbId,
  name,
  onClick,
  onKeyDown,
  posterUrl,
  resultAttribute,
  smgId,
  tvdbId,
  year,
  yearLabel,
}: SearchMetadataPosterButtonProps) {
  const identityAttributes = searchResultIdentityAttributes({
    actionKind,
    facet,
    imdbId,
    kind: "metadata",
    smgId,
    titleName: name,
    tvdbId,
    year,
  });
  return (
    <div
      className={cn("w-[124px] flex-none", disabled && "opacity-80")}
      data-global-search-result-card="metadata"
      {...identityAttributes}
    >
      <TitleCard
        title={name}
        year={yearLabel}
        facet={facet}
        posterUrl={posterUrl}
        addable={!disabled && actionKind === "add"}
        requestable={!disabled && actionKind === "request"}
        compact
        onAdd={onClick}
        onRequest={onClick}
        interactiveProps={{
          id,
          onKeyDown,
          title: actionTitle,
          "aria-label": actionTitle,
          ...searchResultAttribute(resultAttribute),
          ...identityAttributes,
        }}
      />
    </div>
  );
}

type SearchFooterTipProps = {
  canViewCatalog: boolean;
  footerTip: string;
  searchTipsLabel: string;
  surface: SearchSurface;
  tipIndexers: string;
  tipTabs: string;
  tipTitles: string;
};

export function SearchFooterTip({
  canViewCatalog,
  footerTip,
  searchTipsLabel,
  surface,
  tipIndexers,
  tipTabs,
  tipTitles,
}: SearchFooterTipProps) {
  const isMobile = surface === "mobile";

  return (
    <div
      className={cn(
        "flex flex-wrap items-center justify-center gap-2 pt-1 text-xs text-[var(--scry-muted3)]",
        isMobile && "text-center",
      )}
    >
      <Info
        className={cn(
          "h-3.5 w-3.5 text-[var(--scry-faint2)]",
          isMobile && "shrink-0",
        )}
      />
      <span>{footerTip}</span>
      <Popover>
        <PopoverTrigger asChild>
          <button
            type="button"
            className="font-medium text-[var(--scry-accent-ring)] transition hover:text-[var(--scry-accent-text)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary/35"
          >
            {searchTipsLabel}
          </button>
        </PopoverTrigger>
        <PopoverContent
          align="center"
          sideOffset={8}
          className={cn(
            "z-[70] w-72 border-[var(--scry-border2)] bg-[linear-gradient(180deg,var(--scry-soft),var(--scry-bg))] p-3 text-xs shadow-[0_18px_48px_rgba(0,0,0,0.32)]",
            isMobile && "max-w-[calc(100vw-2rem)]",
          )}
        >
          <div className="space-y-2 text-[var(--scry-muted3)]">
            <p>{tipTitles}</p>
            <p>{tipTabs}</p>
            {canViewCatalog ? <p>{tipIndexers}</p> : null}
          </div>
        </PopoverContent>
      </Popover>
    </div>
  );
}

type SearchEmptyStateProps = {
  className?: string;
  description: string;
  icon: "search" | "searchX";
  title: string;
};

export function SearchEmptyState({
  className,
  description,
  icon,
  title,
}: SearchEmptyStateProps) {
  const Icon = icon === "searchX" ? SearchX : Search;

  return (
    <div
      className={cn(
        "flex flex-col items-center justify-center py-12 text-center",
        className,
      )}
    >
      <div className="mb-4 flex h-14 w-14 items-center justify-center rounded-[15px] border border-[var(--scry-border2)] bg-[var(--scry-chip)] text-[var(--scry-faint2)]">
        <Icon className="h-6 w-6" />
      </div>
      <p className="max-w-full break-all text-[17px] font-bold text-[var(--scry-ink2)]">
        {title}
      </p>
      <p className="mt-1 max-w-sm text-sm leading-6 text-[var(--scry-muted3)]">
        {description}
      </p>
    </div>
  );
}
