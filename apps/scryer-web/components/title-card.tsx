import * as React from "react";
import { Clock, Eye, EyeOff, Plus, Send, X } from "lucide-react";
import type { LucideIcon } from "lucide-react";
import type { Facet } from "@/lib/types/titles";
import { TitlePosterSlot } from "@/components/title-poster-slot";
import { ActionTooltip } from "@/components/ui/tooltip";
import { useTranslate } from "@/lib/context/translate-context";
import { facetById } from "@/lib/facets/registry";
import { cn } from "@/lib/utils";

/**
 * Optional top-right corner badge (e.g. a discovery relation pill or a
 * release-recency marker). Kept generic so non-discovery callers can reuse it.
 */
export type TitleCardCornerBadge = {
  /** Already-localized short label. */
  label: string;
  icon?: LucideIcon | null;
  /** Visual emphasis. `accent` matches the app accent; `neutral` is a dark chip. */
  tone?: "accent" | "neutral";
  /** Optional native title/tooltip text. */
  title?: string;
};

const FACET_BADGE_CLASS: Record<Facet, string> = {
  MOVIE:
    "bg-[linear-gradient(135deg,rgba(var(--scry-facet-movie-rgb),0.96),rgba(var(--scry-facet-movie-rgb),0.72))] text-white",
  SERIES:
    "bg-[linear-gradient(135deg,rgba(var(--scry-facet-series-rgb),0.96),rgba(var(--scry-facet-series-rgb),0.72))] text-white",
  ANIME:
    "bg-[linear-gradient(135deg,rgba(var(--scry-facet-anime-rgb),0.96),rgba(var(--scry-facet-anime-rgb),0.72))] text-white",
};

const FACET_LABEL_KEY: Record<Facet, string> = {
  MOVIE: "search.facetMovie",
  SERIES: "search.facetSeries",
  ANIME: "search.facetAnime",
};

function facetBadgeIcon(facet: Facet | null | undefined): LucideIcon | null {
  return facet ? (facetById(facet)?.icon ?? null) : null;
}

const ACCENT_ACTION_STYLE: React.CSSProperties = {
  backgroundColor: "rgb(var(--scry-accent-rgb))",
};

const REQUESTED_ACTION_STYLE: React.CSSProperties = {
  backgroundImage: "linear-gradient(135deg, #e6b347, #c2851a)",
  boxShadow: "0 12px 28px rgba(206, 150, 40, 0.4)",
};

export type TitleCardProps = {
  /** Display title, e.g. "Oppenheimer". */
  title: string;
  /** Release year (or any short subtitle), shown under the title. */
  year?: number | string | null;
  /** Facet drives the top-left badge label + color. */
  facet?: Facet | null;
  /** Override the badge text (already localized). Defaults to the facet label. */
  facetLabel?: string | null;
  posterUrl?: string | null;
  /** Hydration hints so a still-fetching poster shows a brief spinner, not empty art. */
  metadataFetchedAt?: string | null;
  createdAt?: string | null;
  /** Placeholder text when there's no poster art. Defaults to the localized "No art". */
  emptyLabel?: string;
  /** Operator with library access can add it → centered "+" action on hover. */
  addable?: boolean;
  /** Member without direct access can request it → centered paper-airplane on hover. */
  requestable?: boolean;
  /** Already requested / pending → amber clock; takes precedence over add/request. */
  requested?: boolean;
  /** Library monitored state. When non-null, shows an eye / eye-off indicator. */
  monitored?: boolean | null;
  /** Denser sizing (action button, title, badge) for small grid/rail contexts. */
  compact?: boolean;
  /** Hide title/year until the card is hovered or keyboard-focused. */
  revealTextOnHover?: boolean;
  /** Informational adult-content marker, shown as an 18+ poster badge. */
  isAdult?: boolean;
  /** Optional top-right corner badge (discovery relation / recency marker). */
  cornerBadge?: TitleCardCornerBadge | null;
  /**
   * When provided, a small dismiss (×) control shows on hover/focus in the
   * top-right, letting the user hide the card locally. Used by discovery's
   * privacy-safe "not interested". Rendered above the corner badge when both
   * are present.
   */
  onDismiss?: () => void;
  /** Accessible label for the dismiss control (already localized). */
  dismissLabel?: string;
  /** Click the card body (opens overview/detail). When omitted, the body is inert. */
  onOpen?: () => void;
  onAdd?: () => void;
  onRequest?: () => void;
  selected?: boolean;
  className?: string;
  /**
   * Attributes spread onto the primary full-card click target (the single-action
   * / open button) — e.g. an id, onKeyDown, or data-* attribute for list
   * keyboard navigation. Only applies when the whole card is the click target.
   */
  interactiveProps?: React.ButtonHTMLAttributes<HTMLButtonElement> &
    Partial<Record<`data-${string}`, string>>;
};

/**
 * The single, consistent interactive poster used everywhere a movie, series, or
 * anime is surfaced — facet overviews and every discovery surface.
 *
 * At rest it's a sharp poster with the facet badge and the title at the base.
 * On hover/focus (or when selected) a frosted-glass veil settles over the
 * poster and the action surfaces in the center: "+" to add (operators), a paper
 * airplane to request (members), both side by side when a person can do either,
 * an amber clock once requested, and nothing when it's browse-only.
 *
 * When exactly one action is possible the WHOLE card is the click target for it
 * (the center icon is just a hint) so there's no small target to aim for. The
 * two-button layout only appears in the rare add+request case.
 *
 * Kept deliberately light (no per-card timers) so it stays cheap in dense,
 * unvirtualized contexts like discovery rails.
 */
function TitleCardImpl({
  title,
  year,
  facet,
  facetLabel,
  posterUrl,
  metadataFetchedAt,
  createdAt,
  emptyLabel,
  addable = false,
  requestable = false,
  requested = false,
  monitored,
  compact = false,
  revealTextOnHover = false,
  isAdult = false,
  cornerBadge,
  onDismiss,
  dismissLabel,
  onOpen,
  onAdd,
  onRequest,
  selected = false,
  className,
  interactiveProps,
}: TitleCardProps) {
  const t = useTranslate();
  const badgeLabel = facetLabel ?? (facet ? t(FACET_LABEL_KEY[facet]) : null);
  const badgeColorClass = facet
    ? FACET_BADGE_CLASS[facet]
    : "bg-[rgba(4,6,12,0.82)] text-[var(--scry-muted2)]";
  const BadgeIcon = facetBadgeIcon(facet);
  const hasYear = year != null && `${year}`.trim() !== "";
  const hasPosterArt = Boolean(posterUrl);
  const revealBaseTextOnHover = revealTextOnHover && hasPosterArt;

  // Exactly one action available → the whole card triggers it.
  const actionCount = requested ? 0 : Number(addable) + Number(requestable);
  const wholeCardActs = actionCount === 1;
  const wholeCardHandler = wholeCardActs ? (addable ? onAdd : onRequest) : onOpen;
  const wholeCardLabel = wholeCardActs
    ? `${addable ? t("discovery.add") : t("discovery.request")}: ${title}`
    : title;
  const hasCenter = requested || addable || requestable;

  // Frost the poster on hover/focus (or when selected). Transition only the
  // transform — the blur/brightness snap, so sweeping the cursor across many
  // cards never animates an expensive `filter: blur`.
  const posterClass = cn(
    "h-full w-full object-cover transition-transform duration-200",
    "group-hover:scale-105 group-hover:blur-md group-hover:brightness-[0.6]",
    "group-focus-within:scale-105 group-focus-within:blur-md group-focus-within:brightness-[0.6]",
    selected && "scale-105 blur-md brightness-[0.6]",
  );
  // Reveal the centered action on the same hover/focus/selected cue.
  const revealClass = cn(
    "opacity-0 transition-opacity duration-200 group-hover:opacity-100 group-focus-within:opacity-100",
    selected && "opacity-100",
  );
  const actionBaseClass = cn(
    "flex items-center justify-center text-white shadow-lg transition",
    compact ? "h-11 w-11 rounded-[13px]" : "h-14 w-14 rounded-[16px]",
  );
  // Visual-only affordance (single action / requested) — the whole card clicks.
  const actionVisualClass = cn(actionBaseClass, "group-hover:brightness-110");
  // Individually interactive buttons (the rare add+request case).
  const actionButtonClass = cn(
    actionBaseClass,
    "pointer-events-none hover:brightness-110 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-white/80 group-hover:pointer-events-auto group-focus-within:pointer-events-auto",
    selected && "pointer-events-auto",
  );
  const plusIconClass = compact ? "h-5 w-5" : "h-6 w-6";
  const sendIconClass = compact ? "h-4 w-4" : "h-5 w-5";

  return (
    <div
      className={cn(
        "group relative aspect-[2/3] w-full overflow-hidden rounded-[16px] border border-[var(--scry-border2)] bg-[var(--scry-card2)]",
        className,
      )}
    >
      {/* Poster (sharp by default; frosts on hover) */}
      <div className="absolute inset-0">
        <TitlePosterSlot
          src={posterUrl}
          metadataFetchedAt={metadataFetchedAt}
          createdAt={createdAt}
          alt={title}
          className={posterClass}
          placeholderClassName="h-full w-full"
          emptyLabel={emptyLabel ?? t("label.noArt")}
          fallbackTitle={title}
          fallbackSubtitle={hasYear ? year : null}
          fallbackTone={facet ?? "neutral"}
          fallbackShowText={false}
          loading="lazy"
          decoding="async"
        />
        {/* Base scrim so the title stays legible over a sharp poster */}
        <div
          aria-hidden="true"
          className={cn(
            "absolute inset-0 bg-gradient-to-t from-black/75 via-black/5 to-transparent",
            revealBaseTextOnHover &&
              "opacity-0 transition-opacity duration-200 group-hover:opacity-100 group-focus-within:opacity-100",
          )}
        />
      </div>

      {/* Card body click target — performs the action when there's exactly one,
          otherwise opens the detail. Paints under the two-button layout. */}
      {wholeCardHandler ? (
        <button
          type="button"
          onClick={wholeCardHandler}
          aria-label={wholeCardLabel}
          {...interactiveProps}
          className={cn(
            "absolute inset-0 z-0 cursor-pointer rounded-[16px] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--scry-accent-ring)]",
            interactiveProps?.className,
          )}
        />
      ) : null}

      {/* Top-left: facet badge + optional monitored indicator (always visible) */}
      <div className="pointer-events-none absolute left-2.5 top-2.5 z-10 flex items-center gap-1.5">
        {badgeLabel ? (
          <span
            className={cn(
              "inline-flex items-center gap-1.5 rounded-[8px] font-black uppercase tracking-[0.035em] shadow-[inset_0_1px_0_rgba(255,255,255,0.28),0_8px_18px_rgba(0,0,0,0.22)]",
              compact ? "px-2 py-1 text-[10px]" : "px-2.5 py-1 text-[11px]",
              badgeColorClass,
            )}
          >
            {BadgeIcon ? (
              <BadgeIcon
                className={compact ? "h-3 w-3" : "h-3.5 w-3.5"}
                aria-hidden="true"
              />
            ) : null}
            {badgeLabel}
          </span>
        ) : null}
        {monitored != null ? (
          <span className="flex h-[25px] w-[25px] items-center justify-center rounded-[7px] border border-white/10 bg-[rgba(4,6,12,0.82)]">
            {monitored ? (
              <Eye className="h-3.5 w-3.5 text-[var(--scry-success-text-soft)]" />
            ) : (
              <EyeOff className="h-3.5 w-3.5 text-[var(--scry-faint2)]" />
            )}
          </span>
        ) : null}
      </div>

      {/* Top-right: informational adult marker, optional corner badge, and dismiss control
          (revealed on hover/focus). Kept clear of the top-left facet badge. */}
      {isAdult || cornerBadge || onDismiss ? (
        <div className="absolute right-2.5 top-2.5 z-30 flex items-center gap-1.5">
          {cornerBadge ? (
            <span
              title={cornerBadge.title}
              className={cn(
                "pointer-events-none inline-flex max-w-[110px] items-center gap-1 truncate rounded-[8px] font-semibold uppercase tracking-[0.03em] shadow-[0_6px_14px_rgba(0,0,0,0.28)] backdrop-blur",
                compact ? "px-1.5 py-0.5 text-[9.5px]" : "px-2 py-0.5 text-[10px]",
                cornerBadge.tone === "neutral"
                  ? "border border-white/15 bg-[rgba(4,6,12,0.78)] text-[var(--scry-text2)]"
                  : "border border-[rgba(var(--scry-accent-rgb),0.42)] bg-[rgba(var(--scry-accent-rgb),0.24)] text-[#dfe3ff]",
              )}
            >
              {cornerBadge.icon ? (
                <cornerBadge.icon
                  className={compact ? "h-2.5 w-2.5" : "h-3 w-3"}
                  aria-hidden="true"
                />
              ) : null}
              <span className="truncate">{cornerBadge.label}</span>
            </span>
          ) : null}
          {onDismiss ? (
            <ActionTooltip content={dismissLabel ?? t("discovery.notInterested")}>
              <button
                type="button"
                aria-label={dismissLabel ?? t("discovery.notInterested")}
                onClick={(event) => {
                  event.stopPropagation();
                  event.preventDefault();
                  onDismiss();
                }}
                className={cn(
                  "pointer-events-auto flex items-center justify-center rounded-full border border-white/15 bg-[rgba(4,6,12,0.82)] text-white/80 opacity-0 transition hover:border-[var(--scry-accent)] hover:bg-[var(--scry-accent)] hover:text-white focus-visible:opacity-100 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-white/80 group-hover:opacity-100 group-focus-within:opacity-100",
                  compact ? "h-6 w-6" : "h-7 w-7",
                  selected && "opacity-100",
                )}
              >
                <X className={compact ? "h-3 w-3" : "h-3.5 w-3.5"} />
              </button>
            </ActionTooltip>
          ) : null}
          {isAdult ? (
            <span
              aria-label="18 and over"
              className={cn(
                "pointer-events-none inline-flex items-center rounded-[8px] bg-[linear-gradient(135deg,rgba(var(--scry-danger-rgb),0.96),rgba(var(--scry-danger-rgb),0.72))] font-black tracking-[0.035em] text-white shadow-[inset_0_1px_0_rgba(255,255,255,0.28),0_8px_18px_rgba(0,0,0,0.22)]",
                compact ? "px-2 py-1 text-[10px]" : "px-2.5 py-1 text-[11px]",
              )}
            >
              18+
            </span>
          ) : null}
        </div>
      ) : null}

      {/* Centered action affordance — revealed on hover/focus/selection */}
      {hasCenter ? (
        <div
          className={cn(
            "pointer-events-none absolute inset-0 z-20 flex items-center justify-center gap-2.5",
            revealClass,
          )}
        >
          {requested ? (
            <ActionTooltip content={t("discovery.requested")}>
              <span
                className={cn(actionVisualClass, "cursor-default")}
                style={REQUESTED_ACTION_STYLE}
                role="img"
                aria-label={t("discovery.requested")}
              >
                <Clock className={plusIconClass} />
              </span>
            </ActionTooltip>
          ) : wholeCardActs ? (
            <span
              className={actionVisualClass}
              style={ACCENT_ACTION_STYLE}
              aria-hidden="true"
            >
              {addable ? (
                <Plus className={plusIconClass} />
              ) : (
                <Send className={sendIconClass} />
              )}
            </span>
          ) : (
            <>
              <ActionTooltip content={t("discovery.add")}>
                <button
                  type="button"
                  onClick={onAdd}
                  className={actionButtonClass}
                  style={ACCENT_ACTION_STYLE}
                  aria-label={`${t("discovery.add")}: ${title}`}
                >
                  <Plus className={plusIconClass} />
                </button>
              </ActionTooltip>
              <ActionTooltip content={t("discovery.request")}>
                <button
                  type="button"
                  onClick={onRequest}
                  className={actionButtonClass}
                  style={ACCENT_ACTION_STYLE}
                  aria-label={`${t("discovery.request")}: ${title}`}
                >
                  <Send className={sendIconClass} />
                </button>
              </ActionTooltip>
            </>
          )}
        </div>
      ) : null}

      {/* Title + year at the base (always visible) */}
      <div
        className={cn(
          "pointer-events-none absolute inset-x-0 bottom-0 z-10 text-center",
          compact ? "px-2.5 pb-2.5" : "px-3 pb-3.5",
          revealBaseTextOnHover &&
            "translate-y-2 opacity-0 transition duration-200 group-hover:translate-y-0 group-hover:opacity-100 group-focus-within:translate-y-0 group-focus-within:opacity-100",
        )}
      >
        <p
          className={cn(
            "line-clamp-2 font-bold leading-tight text-white",
            compact ? "text-[13px]" : "text-[17px]",
          )}
          style={{
            fontFamily:
              "var(--font-space-grotesk), var(--font-inter), ui-sans-serif, system-ui, -apple-system, sans-serif",
            textShadow: "0 1px 3px rgba(0,0,0,0.65)",
          }}
        >
          {title}
        </p>
        {hasYear ? (
          <p
            className={cn(
              "mt-0.5 font-medium text-white/70",
              compact ? "text-[11px]" : "text-[12.5px]",
            )}
          >
            {year}
          </p>
        ) : null}
      </div>
    </div>
  );
}

export const TitleCard = React.memo(TitleCardImpl);
