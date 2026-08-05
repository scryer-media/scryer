import * as React from "react";
import type { CSSProperties } from "react";
import type { LucideIcon } from "lucide-react";
import {
  Building2,
  CalendarDays,
  Check,
  ChevronRight,
  Disc3,
  Eye,
  Film,
  Heart,
  Loader2,
  MonitorPlay,
  PanelRightClose,
  PanelRightOpen,
  Plus,
  Send,
  SlidersHorizontal,
  Sparkles,
  Star,
  Tag,
  X,
} from "lucide-react";
import { useTranslate } from "@/lib/context/translate-context";
import { HorizontalScrollFade } from "@/components/common/horizontal-scroll-fade";
import { Button } from "@/components/ui/button";
import { MultiSelectDropdown } from "@/components/ui/multi-select-dropdown";
import { TitleCard } from "@/components/title-card";
import { TitleRatingsStrip } from "@/components/views/title-ratings-strip";
import { facetById } from "@/lib/facets/registry";
import {
  discoveryItemDisplayTitle,
  usefulDiscoveryTitle,
} from "@/lib/utils/discovery-display";
import { discoveryItemFacet } from "@/lib/utils/discovery-actions";
import {
  HERO_RAIL_PREFERRED_SECTION_TYPE,
  NEW_ON_PHYSICAL_SECTION_TYPE,
  NEW_ON_STREAMING_SECTION_TYPE,
  discoverySectionType,
  orderDiscoveryHomeSectionsDetailed,
  sectionIsPublicPromotion,
} from "@/lib/utils/discovery-sections";
import {
  selectBackdropVariantUrl,
  selectPosterVariantUrl,
} from "@/lib/utils/poster-images";
import { cn } from "@/lib/utils";
import type {
  DiscoveryHomePayload,
  DiscoveryHomeCard,
  DiscoveryHomeFilterOptions,
  DiscoveryHomeFilters,
  DiscoveryHomeHero,
  DiscoveryHomeSection,
  DiscoveryHomeStatus,
  Facet,
} from "@/lib/types";

type DiscoveryViewProps = {
  home: DiscoveryHomePayload | null;
  loading: boolean;
  error: string | null;
  manageableFacets: Facet[];
  requestableFacets: Facet[];
  filterOptions: DiscoveryHomeFilterOptions;
  onFiltersChange: (filters: DiscoveryHomeFilters) => void;
  onRefresh: () => void;
  onAction: (item: DiscoveryHomeCard) => void;
};

type DiscoveryContentType = Facet;

const DISCOVERY_CONTENT_TYPES: DiscoveryContentType[] = [
  "MOVIE",
  "SERIES",
  "ANIME",
];
const DEFAULT_DISCOVERY_CONTENT_TYPES: DiscoveryContentType[] = [
  "MOVIE",
  "SERIES",
  "ANIME",
];
const DISCOVERY_FACET_PILL_CLASS: Record<DiscoveryContentType, string> = {
  MOVIE:
    "bg-[linear-gradient(135deg,rgba(var(--scry-facet-movie-rgb),0.96),rgba(var(--scry-facet-movie-rgb),0.72))] text-white",
  SERIES:
    "bg-[linear-gradient(135deg,rgba(var(--scry-facet-series-rgb),0.96),rgba(var(--scry-facet-series-rgb),0.72))] text-white",
  ANIME:
    "bg-[linear-gradient(135deg,rgba(var(--scry-facet-anime-rgb),0.96),rgba(var(--scry-facet-anime-rgb),0.72))] text-white",
};
const DEFAULT_MINIMUM_YEAR = 1900;
const DEFAULT_MINIMUM_RATING = 0;

function emptyDiscoveryHomeFilters(): DiscoveryHomeFilters {
  return {
    contentTypes: [],
    genreTagKeys: [],
    themeTagKeys: [],
    studioSlugs: [],
  };
}

function discoveryHomeFiltersSignature(filters: DiscoveryHomeFilters) {
  return JSON.stringify(filters);
}

function discoveryFacetIcon(
  facet: DiscoveryContentType | null | undefined,
): LucideIcon | null {
  return facet ? (facetById(facet)?.icon ?? null) : null;
}

const FILTER_RANGE_CLASS_NAME =
  "h-1.5 w-full appearance-none rounded-full bg-transparent accent-[var(--scry-accent)] [&::-moz-range-progress]:h-1.5 [&::-moz-range-progress]:rounded-full [&::-moz-range-progress]:bg-transparent [&::-moz-range-thumb]:h-[15px] [&::-moz-range-thumb]:w-[15px] [&::-moz-range-thumb]:rounded-full [&::-moz-range-thumb]:border-0 [&::-moz-range-thumb]:bg-white [&::-moz-range-thumb]:shadow-[0_1px_5px_rgba(0,0,0,0.5)] [&::-moz-range-track]:h-1.5 [&::-moz-range-track]:rounded-full [&::-moz-range-track]:bg-transparent [&::-webkit-slider-runnable-track]:h-1.5 [&::-webkit-slider-runnable-track]:rounded-full [&::-webkit-slider-runnable-track]:bg-transparent [&::-webkit-slider-thumb]:mt-[-4.5px] [&::-webkit-slider-thumb]:h-[15px] [&::-webkit-slider-thumb]:w-[15px] [&::-webkit-slider-thumb]:appearance-none [&::-webkit-slider-thumb]:rounded-full [&::-webkit-slider-thumb]:bg-white [&::-webkit-slider-thumb]:shadow-[0_1px_5px_rgba(0,0,0,0.5)]";
const FILTER_RANGE_THUMB_POINTER_CLASS_NAME =
  "pointer-events-none [&::-moz-range-thumb]:pointer-events-auto [&::-webkit-slider-thumb]:pointer-events-auto";

function defaultMaximumDiscoveryYear() {
  return new Date().getFullYear() + 3;
}

function itemStableKey(item: DiscoveryHomeCard) {
  return `${item.targetKind}:${item.targetKey}`;
}

function itemIdentityKey(item: DiscoveryHomeCard) {
  return `${item.targetKind}:${item.targetKey}`;
}

function itemTypeLabel(item: DiscoveryHomeCard) {
  const raw = item.contentType || item.targetKind;
  return raw.replace(/[_-]+/g, " ").trim().toUpperCase();
}

function itemCalendarBadgeLabel(item: DiscoveryHomeCard) {
  return item.year ? String(item.year) : null;
}

function hashHue(value: string) {
  let hash = 0;
  for (let index = 0; index < value.length; index += 1) {
    hash = (hash * 31 + value.charCodeAt(index)) % 360;
  }
  return hash;
}

function posterFallbackStyle(item: DiscoveryHomeCard): CSSProperties {
  const hue = hashHue(item.targetKey || item.displayTitle);
  return {
    background: `radial-gradient(135% 100% at 50% 6%, hsl(${hue} 46% 33%) 0%, hsl(${(hue + 328) % 360} 52% 18%) 44%, #06080f 100%)`,
  };
}

function heroBackdropUrl(item: DiscoveryHomeHero | null): string | null {
  return selectBackdropVariantUrl(item?.backgroundUrl ?? null, "w1280") ?? null;
}

function itemMatchScore(item: DiscoveryHomeHero) {
  return item.matchedSubjectCount && item.matchedSubjectCount > 0
    ? String(item.matchedSubjectCount)
    : null;
}

// Animation is a medium, anime is a tradition, and the two must not comingle
// inside one reason-based rail. SMG's feed does not enforce that, so the anime
// rail is narrowed to anime and the general rails drop anime. Section identity is
// deliberately left intact - the display name is owned by
// SECTION_DISPLAY_NAME_KEYS, so there is no need to rewrite the sectionType here.
const ANIME_ONLY_PUBLIC_SECTION_TYPES = new Set([
  "ANIME_THIS_WEEK",
  "POPULAR_WITH_ANIME_FANS",
]);
// TRENDING_NOW and POPULAR_RIGHT_NOW merge into one "Right Now" rail, so both
// halves must anime-exclude identically - otherwise the merged rail leaks anime
// that the anime band already owns.
const ANIME_EXCLUDING_PUBLIC_SECTION_TYPES = new Set([
  "TRENDING_NOW",
  "POPULAR_RIGHT_NOW",
  "POPULAR_SERIES",
]);

function normalizedPublicHomeSections(home: DiscoveryHomePayload | null) {
  const publicSections = home?.publicSections ?? [];
  return publicSections.flatMap((section) => {
    const sectionType = discoverySectionType(section);
    const keepsOnlyAnime = ANIME_ONLY_PUBLIC_SECTION_TYPES.has(sectionType);
    if (
      !keepsOnlyAnime &&
      !ANIME_EXCLUDING_PUBLIC_SECTION_TYPES.has(sectionType)
    ) {
      return [section];
    }
    const items = section.items.filter((item) =>
      keepsOnlyAnime
        ? discoveryItemFacet(item) === "ANIME"
        : discoveryItemFacet(item) !== "ANIME",
    );
    return items.length > 0
      ? [{ ...section, totalCount: items.length, items }]
      : [];
  });
}

function normalizedSectionText(section: DiscoveryHomeSection) {
  return `${section.sectionId} ${section.sectionType} ${section.title} ${section.surface}`.toLowerCase();
}

// sectionType -> i18n key. Preferred over SMG's raw section.title so product can
// re-word a rail purely in locale files. Unmapped types fall back to the raw title.
// This is also how provider names are kept off the dashboard: EVERGREEN_POPULAR
// arrives from SMG titled "Netflix Most Watched", and Scryer is an unbiased entry
// point, so the rail is renamed rather than pitched.
const SECTION_DISPLAY_NAME_KEYS: Record<string, string> = {
  [NEW_ON_STREAMING_SECTION_TYPE]: "discovery.section.newOnStreaming",
  [NEW_ON_PHYSICAL_SECTION_TYPE]: "discovery.section.newOnPhysical",
  EVERGREEN_POPULAR: "discovery.section.allTimeFavorites",
  // Merged into a single rail; the surviving half carries the same name whichever
  // one the gateway sent.
  POPULAR_RIGHT_NOW: "discovery.section.rightNow",
  TRENDING_NOW: "discovery.section.rightNow",
  FOR_YOU: "discovery.section.fromYourTaste",
  COMPLETE_THE_COLLECTION: "discovery.section.almostComplete",
  // Sourced from MAL/AniList, so the generic SMG title lies about the content.
  UPCOMING_NEXT_SEASON: "discovery.section.nextAnimeSeason",
  // ANIME_THIS_WEEK is the *weekly* rail (animecorner weekly / MAL top airing)
  // and absorbs the anime-fan-poll rail; ANIME_SEASON_STANDOUTS is the seasonal
  // one. Naming them the other way round mislabels both.
  ANIME_THIS_WEEK: "discovery.section.popularInAnime",
  POPULAR_WITH_ANIME_FANS: "discovery.section.popularInAnime",
  ANIME_SEASON_STANDOUTS: "discovery.section.thisSeasonInAnime",
  UPCOMING_MOVIES: "discovery.section.comingSoonMovies",
  UPCOMING_SERIES: "discovery.section.comingSoonSeries",
  TOP_RATED_FOR_YOU: "discovery.section.topRatedForYou",
  // Emitted by Scryer's own composer with a hardcoded English title.
  TOP_RATED: "discovery.section.topRated",
};

const SECTION_ICONS: Record<string, LucideIcon> = {
  [NEW_ON_STREAMING_SECTION_TYPE]: MonitorPlay,
  [NEW_ON_PHYSICAL_SECTION_TYPE]: Disc3,
};

function sectionDisplayTitle(
  section: DiscoveryHomeSection,
  t: ReturnType<typeof useTranslate>,
) {
  const key = SECTION_DISPLAY_NAME_KEYS[discoverySectionType(section)];
  if (key) {
    const label = t(key);
    if (label && label !== key) {
      return label;
    }
  }
  // A rail with no mapping and no gateway title still needs a truthful heading.
  return section.title || t("discovery.section.recommended");
}

function sectionIcon(section: DiscoveryHomeSection): LucideIcon | null {
  return SECTION_ICONS[discoverySectionType(section)] ?? null;
}

function sectionIsCompleteCollection(section: DiscoveryHomeSection) {
  return (
    discoverySectionType(section) === "COMPLETE_THE_COLLECTION" ||
    section.sectionId === "complete_the_collection"
  );
}

function orderedHomeSections(home: DiscoveryHomePayload | null) {
  if (!home) {
    return { sections: [], heroVisibilitySections: [] };
  }
  // Detailed variant: hero visibility is validated against the post-dedupe
  // PRE-floor list, so a rail hidden by the thin-rail floor can never blank a
  // hero whose item rendered nowhere else.
  return orderDiscoveryHomeSectionsDetailed<DiscoveryHomeSection>({
    publicSections: normalizedPublicHomeSections(home),
    personalizedSections: home.personalizedSections,
    completeCollection: home.completeCollection,
  });
}

function sectionIsUpcoming(section: DiscoveryHomeSection) {
  const haystack = normalizedSectionText(section);
  return haystack.includes("upcoming") || haystack.includes("future");
}

function uniqueDiscoveryItems(items: DiscoveryHomeCard[]) {
  const seen = new Set<string>();
  return items.filter((item) => {
    const key = itemIdentityKey(item);
    if (seen.has(key)) {
      return false;
    }
    seen.add(key);
    return true;
  });
}

function sectionWithoutItem(
  section: DiscoveryHomeSection | null,
  item: DiscoveryHomeCard | null,
) {
  if (!section || !item) {
    return section;
  }
  const itemKey = itemIdentityKey(item);
  const items = section.items.filter(
    (candidate) => itemIdentityKey(candidate) !== itemKey,
  );
  return items.length > 0 ? { ...section, items } : null;
}

function heroItemIsVisibleInSections(
  item: DiscoveryHomeHero,
  sections: DiscoveryHomeSection[],
) {
  const itemKey = itemIdentityKey(item);
  return sections.some((section) =>
    section.items.some((candidate) => itemIdentityKey(candidate) === itemKey),
  );
}

function normalizedDiscoveryContentType(
  value: string | null | undefined,
): DiscoveryContentType | null {
  switch (value?.trim().toLowerCase()) {
    case "anime":
      return "ANIME";
    case "series":
      return "SERIES";
    case "movie":
      return "MOVIE";
    default:
      return null;
  }
}

function itemContentType(item: DiscoveryHomeCard): DiscoveryContentType | null {
  const contentType = item.contentType?.trim();
  return contentType
    ? normalizedDiscoveryContentType(contentType)
    : normalizedDiscoveryContentType(item.targetKind);
}

function discoveryItemDisplayGenreLabels(item: DiscoveryHomeHero): string[] {
  return [...new Set(item.genreTags.map((tag) => tag.name.trim()).filter(Boolean))];
}

function discoveryItemHasUsefulTitle(item: DiscoveryHomeCard) {
  return Boolean(
    usefulDiscoveryTitle(item.displayTitle) ||
      usefulDiscoveryTitle(item.sortTitle) ||
      usefulDiscoveryTitle(item.originalTitle),
  );
}

function findHeroRailSection(sections: DiscoveryHomeSection[]) {
  // Never fold a public-promotion rail into the hero column — those stay as full
  // rails in their own tier.
  const eligible = sections.filter(
    (section) => !sectionIsPublicPromotion(section),
  );
  // Preference is by section *type*, not by sniffing the gateway's raw title: a
  // text match on "trend" would silently hijack the hero for any future rail that
  // happens to be titled "Trending in ...".
  return (
    eligible.find(
      (section) =>
        discoverySectionType(section) === HERO_RAIL_PREFERRED_SECTION_TYPE,
    ) ??
    eligible[0] ??
    null
  );
}

function DiscoveryRailCard({
  item,
  size = "md",
  variant = "default",
  fillHeight = false,
  canManageTitle,
  canRequestMedia,
  onAction,
  onDismiss,
  dismissLabel,
}: {
  item: DiscoveryHomeCard;
  size?: "sm" | "md";
  variant?: "default" | "upcoming";
  fillHeight?: boolean;
  canManageTitle: boolean;
  canRequestMedia: boolean;
  onAction: (item: DiscoveryHomeCard) => void;
  onDismiss?: (item: DiscoveryHomeCard) => void;
  dismissLabel?: string;
}) {
  const compactSize = size === "sm";
  const upcoming = variant === "upcoming" && !compactSize;
  const owned = item.ownedInInput;
  const addable = !owned && canManageTitle;
  const requestable = !owned && !canManageTitle && canRequestMedia;
  const facet = itemContentType(item);
  const subtitle = upcoming ? itemCalendarBadgeLabel(item) : item.year;
  const handleAction = React.useCallback(
    () => onAction(item),
    [onAction, item],
  );
  const handleDismiss = React.useMemo(
    () => (onDismiss ? () => onDismiss(item) : undefined),
    [onDismiss, item],
  );
  return (
    <div
      className={cn(
        "flex-none",
        fillHeight
          ? "w-[152px] lg:aspect-[2/3] lg:h-full lg:w-auto"
          : compactSize
            ? "w-[120px]"
            : "w-[152px]",
      )}
    >
      <TitleCard
        title={discoveryItemDisplayTitle(item)}
        year={subtitle}
        facet={facet}
        facetLabel={itemTypeLabel(item)}
        posterUrl={selectPosterVariantUrl(item.posterUrl, "w250")}
        addable={addable}
        requestable={requestable}
        compact={!fillHeight}
        onDismiss={handleDismiss}
        dismissLabel={dismissLabel}
        onAdd={addable ? handleAction : undefined}
        onRequest={requestable ? handleAction : undefined}
      />
    </div>
  );
}

function DiscoverySectionRail({
  section,
  manageableFacets,
  requestableFacets,
  onAction,
  onDismissItem,
  compact = false,
  fillHeight = false,
  variant = "default",
}: {
  section: DiscoveryHomeSection;
  manageableFacets: ReadonlySet<Facet>;
  requestableFacets: ReadonlySet<Facet>;
  onAction: (item: DiscoveryHomeCard) => void;
  onDismissItem?: (item: DiscoveryHomeCard) => void;
  compact?: boolean;
  fillHeight?: boolean;
  variant?: "default" | "upcoming";
}) {
  const t = useTranslate();
  const items = React.useMemo(
    () => uniqueDiscoveryItems(section.items),
    [section.items],
  );
  const HeaderIcon = sectionIcon(section);
  const heading = sectionDisplayTitle(section, t);
  const dismissLabel = t("discovery.notInterested");

  return (
    <section
      className={cn(
        "mb-7",
        fillHeight && "lg:flex lg:h-full lg:min-h-0 lg:flex-col",
      )}
    >
      <div className="mb-3.5 flex items-center justify-between gap-3">
        <h3 className="m-0 inline-flex items-center gap-2 font-[var(--font-space-grotesk)] text-lg font-semibold text-[var(--scry-ink2)]">
          {HeaderIcon ? (
            <HeaderIcon
              className="h-4 w-4 text-[var(--scry-accent-text)]"
              aria-hidden="true"
            />
          ) : null}
          {heading}
        </h3>
        <span className="inline-flex items-center gap-1 text-[12.5px] font-medium text-[var(--scry-muted)]">
          {t("discovery.viewAll")}
          <ChevronRight className="h-3.5 w-3.5" />
        </span>
      </div>
      <HorizontalScrollFade
        className={cn(
          "flex gap-3.5 overflow-x-auto pb-1.5 [scrollbar-width:none] [&::-webkit-scrollbar]:hidden",
          fillHeight && "lg:h-full",
        )}
        containerClassName={cn(fillHeight && "lg:min-h-0 lg:flex-1")}
        fadeClassName="to-[var(--scry-bg)]"
      >
        {items.map((item) => {
          const facet = discoveryItemFacet(item);
          return (
            <DiscoveryRailCard
              key={itemStableKey(item)}
              item={item}
              size={compact ? "sm" : "md"}
              variant={variant}
              fillHeight={fillHeight}
              canManageTitle={facet !== null && manageableFacets.has(facet)}
              canRequestMedia={facet !== null && requestableFacets.has(facet)}
              onAction={onAction}
              onDismiss={onDismissItem}
              dismissLabel={dismissLabel}
            />
          );
        })}
      </HorizontalScrollFade>
    </section>
  );
}

function DiscoveryHero({
  item,
  canManageTitle,
  canRequestMedia,
  onAction,
}: {
  item: DiscoveryHomeHero;
  canManageTitle: boolean;
  canRequestMedia: boolean;
  onAction: (item: DiscoveryHomeCard) => void;
}) {
  const t = useTranslate();
  const titleLabel = discoveryItemDisplayTitle(item);
  const match = itemMatchScore(item);
  const genres = discoveryItemDisplayGenreLabels(item).slice(0, 3);
  const facet = itemContentType(item);
  const FacetIcon = discoveryFacetIcon(facet);
  const detailItems = [item.year ? String(item.year) : null].filter(
    (detail): detail is string => Boolean(detail),
  );
  const backdropUrl = heroBackdropUrl(item);
  const heroActionAvailable =
    !item.ownedInInput && (canManageTitle || canRequestMedia);
  const HeroActionIcon = canManageTitle ? Plus : Send;
  const heroActionLabel = canManageTitle
    ? t("discovery.add")
    : t("discovery.request");
  return (
    <section className="group relative min-h-[340px] overflow-hidden rounded-[18px] border border-[var(--scry-border2)] bg-slate-950 lg:h-full">
      {backdropUrl ? (
        <img
          src={backdropUrl}
          alt=""
          aria-hidden="true"
          data-discovery-hero-backdrop="true"
          className="absolute inset-0 h-full w-full object-cover transition-transform duration-200 group-hover:scale-105 group-hover:blur-md group-hover:brightness-[0.6] group-focus-within:scale-105 group-focus-within:blur-md group-focus-within:brightness-[0.6]"
        />
      ) : (
        <div
          className="absolute inset-0"
          style={posterFallbackStyle(item)}
          data-discovery-hero-backdrop-fallback="true"
        />
      )}
      <div className="absolute inset-0 bg-gradient-to-r from-slate-950/80 via-slate-950/45 to-slate-950/0" />
      <div className="absolute inset-0 bg-gradient-to-t from-slate-950/55 via-slate-950/15 to-transparent" />
      <div
        aria-hidden="true"
        className="pointer-events-none absolute inset-0 z-30 bg-[radial-gradient(circle_at_center,rgba(var(--scry-accent-rgb),0.3),transparent_48%)] opacity-0 transition-opacity duration-200 group-hover:opacity-100 group-hover:backdrop-blur-sm group-focus-within:opacity-100 group-focus-within:backdrop-blur-sm"
      />
      {heroActionAvailable ? (
        <>
          <button
            type="button"
            onClick={() => onAction(item)}
            aria-label={`${heroActionLabel}: ${titleLabel}`}
            className="absolute inset-0 z-20 cursor-pointer rounded-[18px] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-white/80"
          />
          <span
            aria-hidden="true"
            className="pointer-events-none absolute left-1/2 top-1/2 z-40 flex h-16 w-16 -translate-x-1/2 -translate-y-1/2 items-center justify-center rounded-[16px] bg-[var(--scry-accent)] text-primary-foreground opacity-0 shadow-[0_0_0_1px_rgba(var(--scry-accent-rgb),0.45),0_0_36px_rgba(var(--scry-accent-rgb),0.5),0_18px_36px_rgba(0,0,0,0.42)] transition duration-200 group-hover:opacity-100 group-focus-within:opacity-100"
          >
            <HeroActionIcon className="h-7 w-7" aria-hidden="true" />
          </span>
        </>
      ) : null}
      <div className="relative flex min-h-[340px] flex-col p-6 sm:p-8 lg:h-full">
        <div className="max-w-[min(78%,760px)] max-sm:max-w-full">
          <div className="mb-3.5 flex flex-wrap gap-2">
            <span className="rounded-[7px] border border-[rgba(var(--scry-accent-rgb),0.4)] bg-[rgba(var(--scry-accent-rgb),0.22)] px-2.5 py-1 text-[11px] font-bold uppercase tracking-[0.04em] text-[#c3c9ff]">
              {t("discovery.featured")}
            </span>
            <span
              className={cn(
                "inline-flex items-center gap-1.5 rounded-[8px] px-2.5 py-1 text-[11px] font-black uppercase tracking-[0.035em] shadow-[inset_0_1px_0_rgba(255,255,255,0.28),0_8px_18px_rgba(0,0,0,0.22)]",
                facet
                  ? DISCOVERY_FACET_PILL_CLASS[facet]
                  : "bg-white/15 text-[#cfd7ee]",
              )}
            >
              {FacetIcon ? (
                <FacetIcon className="h-3.5 w-3.5" aria-hidden="true" />
              ) : null}
              {itemTypeLabel(item)}
            </span>
          </div>
          <h2 className="m-0 mb-3 font-[var(--font-space-grotesk)] text-[clamp(2rem,3vw,46px)] font-bold leading-[0.96] text-white drop-shadow">
            {titleLabel}
          </h2>
          <div className="mb-3.5 flex flex-wrap items-center gap-3 text-[13px] text-[var(--scry-text2)]">
            {detailItems.map((detail, index) => (
              <React.Fragment key={`${detail}-${index}`}>
                {index > 0 ? (
                  <span className="h-1 w-1 rounded-full bg-[var(--scry-faint2)]" />
                ) : null}
                <span className="font-semibold capitalize">{detail}</span>
              </React.Fragment>
            ))}
            {match ? (
              <span className="inline-flex items-center gap-1 rounded-[7px] bg-[var(--scry-success-bg)] px-2 py-0.5 font-bold text-[var(--scry-success-text-soft)]">
                <Heart className="h-3.5 w-3.5" />
                {match}
              </span>
            ) : null}
          </div>
          <TitleRatingsStrip
            ratings={{
              rating: item.rating,
              ratingSources: item.ratingSources ?? [],
              externalRatings: item.externalRatings ?? [],
            }}
            variant="hero"
          />
          {item.overview ? (
            <p className="m-0 line-clamp-2 max-w-[620px] text-[13.5px] leading-6 text-[#b7c0dd] sm:line-clamp-3 lg:max-2xl:hidden 2xl:line-clamp-4">
              {item.overview}
            </p>
          ) : null}
          {genres.length > 0 ? (
            <div className="mt-3 flex flex-wrap gap-2 lg:max-2xl:hidden">
              {genres.map((genre) => (
                <span
                  key={genre}
                  className="rounded-[8px] border border-white/10 bg-white/10 px-3 py-1.5 text-xs text-[#cfd7ee]"
                >
                  {genre}
                </span>
              ))}
            </div>
          ) : null}
        </div>
      </div>
    </section>
  );
}

function DiscoveryFilterMultiSelect({
  options,
  selectedValues,
  placeholder,
  ariaLabel,
  onSelectedValuesChange,
}: {
  options: Array<{ key: string; name: string }>;
  selectedValues: string[];
  placeholder: string;
  ariaLabel: string;
  onSelectedValuesChange: (values: string[]) => void;
}) {
  const triggerLabel =
    selectedValues.length > 0
      ? selectedValues.length === 1
        ? (options.find((option) => option.key === selectedValues[0])?.name ??
          selectedValues[0])
        : `${selectedValues.length} selected`
      : placeholder;

  return (
    <MultiSelectDropdown
      options={options.map((option) => ({
        value: option.key,
        label: option.name,
      }))}
      selectedValues={selectedValues}
      onSelectedValuesChange={onSelectedValuesChange}
      triggerLabel={triggerLabel}
      placeholder={placeholder}
      ariaLabel={ariaLabel}
      size="compact"
      chrome="toolbar"
    />
  );
}

function DiscoveryFilterChips({
  values,
  labels,
  onRemove,
}: {
  values: string[];
  labels?: ReadonlyMap<string, string>;
  onRemove: (value: string) => void;
}) {
  if (values.length === 0) {
    return null;
  }

  return (
    <div className="mt-3 flex flex-wrap gap-2">
      {values.map((value) => (
        <button
          key={value}
          type="button"
          onClick={() => onRemove(value)}
          className="inline-flex max-w-full items-center gap-2 rounded-[8px] border border-[rgba(var(--scry-accent-rgb),0.34)] bg-[rgba(var(--scry-accent-rgb),0.15)] px-3 py-1.5 text-xs font-semibold text-[var(--scry-accent-text)] transition hover:border-[rgba(var(--scry-accent-rgb),0.48)] hover:bg-[rgba(var(--scry-accent-rgb),0.22)]"
        >
          <span className="truncate">
            {labels?.get(value) ?? discoveryFilterOptionLabel(value)}
          </span>
          <X className="h-3.5 w-3.5 opacity-75" aria-hidden="true" />
        </button>
      ))}
    </div>
  );
}

function discoveryFilterOptionLabel(value: string) {
  return value
    .replace(/[_-]+/g, " ")
    .replace(/\b\w/g, (character) => character.toUpperCase());
}

function DiscoveryFilterLabel({
  children,
  icon,
}: {
  children: React.ReactNode;
  icon: React.ReactNode;
}) {
  return (
    <div className="mb-2.5 flex items-center gap-1.5 text-xs font-bold uppercase tracking-[0.05em] text-[var(--scry-muted2)]">
      {icon}
      {children}
    </div>
  );
}

function DiscoveryFilters({
  variant = "desktop",
  filterOptions,
  availableContentTypes,
  selectedContentTypes,
  selectedGenres,
  selectedThemes,
  selectedStudioSlugs,
  minimumYear,
  maximumYear,
  minimumRating,
  hiddenItemCount,
  onToggleContentType,
  onGenresChange,
  onThemesChange,
  onToggleStudioSlug,
  onMinimumYearChange,
  onMaximumYearChange,
  onMinimumRatingChange,
  onClear,
  onShowHidden,
  onCollapse,
  onRequestClose,
}: {
  variant?: "desktop" | "mobile";
  filterOptions: DiscoveryHomeFilterOptions;
  availableContentTypes: DiscoveryContentType[];
  selectedContentTypes: DiscoveryContentType[];
  selectedGenres: string[];
  selectedThemes: string[];
  selectedStudioSlugs: string[];
  minimumYear: number;
  maximumYear: number;
  minimumRating: number;
  hiddenItemCount: number;
  onToggleContentType: (contentType: DiscoveryContentType) => void;
  onGenresChange: (genres: string[]) => void;
  onThemesChange: (themes: string[]) => void;
  onToggleStudioSlug: (studioSlug: string) => void;
  onMinimumYearChange: (year: number) => void;
  onMaximumYearChange: (year: number) => void;
  onMinimumRatingChange: (rating: number) => void;
  onClear: () => void;
  onShowHidden: () => void;
  onCollapse?: () => void;
  onRequestClose?: () => void;
}) {
  const t = useTranslate();
  const contentTypes: Array<{
    key: DiscoveryContentType;
    label: string;
  }> = availableContentTypes.map((key) => ({
    key,
    label:
      key === "MOVIE"
        ? t("discovery.type.movies")
        : key === "SERIES"
          ? t("discovery.type.series")
          : t("discovery.type.anime"),
  }));
  const { genres, themes, studioSlugs: studioSlugOptions } = filterOptions;
  const genreLabels = new Map(genres.map((option) => [option.key, option.name]));
  const themeLabels = new Map(themes.map((option) => [option.key, option.name]));
  const minimumYearBound = DEFAULT_MINIMUM_YEAR;
  const maximumYearBound = defaultMaximumDiscoveryYear();
  const yearSpan = Math.max(1, maximumYearBound - minimumYearBound);
  const minimumYearPercent =
    ((minimumYear - minimumYearBound) / yearSpan) * 100;
  const maximumYearPercent =
    ((maximumYear - minimumYearBound) / yearSpan) * 100;
  const ratingPercent = (Math.min(Math.max(minimumRating, 0), 10) / 10) * 100;

  return (
    <aside
      className={cn(
        "relative flex min-h-0 flex-col overflow-y-auto bg-[var(--scry-surf)] px-[18px] py-4",
        variant === "desktop"
          ? "w-[284px] flex-none border-l border-[var(--scry-border3)] max-xl:hidden"
          : "h-full w-full border-l border-[var(--scry-border3)] shadow-[-18px_0_38px_rgba(0,0,0,0.36)]",
      )}
    >
      <div className="mb-4 flex items-center justify-between gap-3">
        <div className="flex items-center gap-3">
          <div className="flex size-9 shrink-0 items-center justify-center rounded-[10px] border border-[var(--scry-baccent)] bg-[linear-gradient(135deg,rgba(var(--scry-accent-rgb),0.32),rgba(155,91,255,0.2))] text-[var(--scry-accent-text)]">
            <SlidersHorizontal className="h-[18px] w-[18px]" />
          </div>
          <span className="text-[16px] font-semibold text-[var(--scry-ink2)]">
          {t("discovery.filters")}
          </span>
        </div>
        <div className="flex items-center gap-2">
          <button
            type="button"
            className="text-xs font-medium text-[var(--scry-accent-ring)]"
            onClick={onClear}
          >
            {t("discovery.clearAll")}
          </button>
          {onCollapse ? (
            <button
              type="button"
              aria-label={t("discovery.closeFilters")}
              title={t("discovery.closeFilters")}
              onClick={onCollapse}
              className="flex size-7 shrink-0 items-center justify-center rounded-[7px] border border-[var(--scry-baccent)] bg-[var(--scry-inset)] text-[var(--scry-accent-text)] transition hover:bg-[var(--scry-hover)]"
            >
              <PanelRightClose className="h-3.5 w-3.5" />
            </button>
          ) : null}
          {onRequestClose ? (
            <button
              type="button"
              aria-label={t("discovery.closeFilters")}
              onClick={onRequestClose}
              className="inline-flex h-8 w-8 items-center justify-center rounded-[8px] border border-[var(--scry-border2)] bg-[var(--scry-bg)] text-[var(--scry-muted)]"
            >
              <X className="h-4 w-4" />
            </button>
          ) : null}
        </div>
      </div>
      <DiscoveryFilterLabel icon={<Disc3 className="h-3.5 w-3.5 shrink-0" aria-hidden="true" />}>
        {t("discovery.contentType")}
      </DiscoveryFilterLabel>
      <div className="mb-5 grid gap-2">
        {contentTypes.map((entry) => (
          <button
            key={entry.key}
            type="button"
            aria-pressed={selectedContentTypes.includes(entry.key)}
            onClick={() => onToggleContentType(entry.key)}
            className="flex items-center justify-between rounded-[9px] border border-[var(--scry-border2)] bg-[var(--scry-bg)] px-3 py-2 text-[13px] text-[var(--scry-text2)]"
          >
            <span className="inline-flex items-center gap-2">
              <span
                className={cn(
                  "flex h-[18px] w-[18px] items-center justify-center rounded-[5px] border",
                  selectedContentTypes.includes(entry.key)
                    ? "border-[var(--scry-accent)] bg-[var(--scry-accent)]"
                    : "border-[var(--scry-border2)] bg-transparent",
                )}
              >
                {selectedContentTypes.includes(entry.key) ? (
                  <Check className="h-3 w-3 text-white" />
                ) : null}
              </span>
              {entry.label}
            </span>
          </button>
        ))}
      </div>
      <DiscoveryFilterLabel icon={<Film className="h-3.5 w-3.5 shrink-0" aria-hidden="true" />}>
        {t("discovery.genres")}
      </DiscoveryFilterLabel>
      <div className="mb-5">
        <DiscoveryFilterMultiSelect
          options={genres}
          selectedValues={selectedGenres}
          placeholder={t("discovery.selectGenres")}
          ariaLabel={t("discovery.genres")}
          onSelectedValuesChange={onGenresChange}
        />
        <DiscoveryFilterChips
          values={selectedGenres}
          labels={genreLabels}
          onRemove={(genre) =>
            onGenresChange(
              selectedGenres.filter((selectedGenre) => selectedGenre !== genre),
            )
          }
        />
      </div>
      <DiscoveryFilterLabel icon={<Tag className="h-3.5 w-3.5 shrink-0" aria-hidden="true" />}>
        {t("discovery.tags")}
      </DiscoveryFilterLabel>
      <div className="mb-5">
        <DiscoveryFilterMultiSelect
          options={themes}
          selectedValues={selectedThemes}
          placeholder={t("discovery.selectTags")}
          ariaLabel={t("discovery.tags")}
          onSelectedValuesChange={onThemesChange}
        />
        <DiscoveryFilterChips
          values={selectedThemes}
          labels={themeLabels}
          onRemove={(tag) =>
            onThemesChange(
              selectedThemes.filter((selectedTheme) => selectedTheme !== tag),
            )
          }
        />
      </div>
      {studioSlugOptions.length > 0 ? (
        <>
          <DiscoveryFilterLabel icon={<Building2 className="h-3.5 w-3.5 shrink-0" aria-hidden="true" />}>
            {t("discovery.studio")}
          </DiscoveryFilterLabel>
          <div className="mb-5 flex flex-wrap gap-2">
            {studioSlugOptions.map((studioSlug) => {
              const active = selectedStudioSlugs.includes(studioSlug);
              return (
                <button
                  key={studioSlug}
                  type="button"
                  aria-pressed={active}
                  onClick={() => onToggleStudioSlug(studioSlug)}
                  className={cn(
                    "inline-flex max-w-full items-center rounded-[8px] border px-3 py-1.5 text-xs font-semibold transition",
                    active
                      ? "border-[rgba(var(--scry-accent-rgb),0.48)] bg-[rgba(var(--scry-accent-rgb),0.22)] text-[var(--scry-accent-text)]"
                      : "border-[var(--scry-border2)] bg-[var(--scry-bg)] text-[var(--scry-text2)] hover:border-[rgba(var(--scry-accent-rgb),0.34)]",
                  )}
                >
                  <span className="truncate">
                    {discoveryFilterOptionLabel(studioSlug)}
                  </span>
                </button>
              );
            })}
          </div>
        </>
      ) : null}
      <div className="mb-2.5 flex items-center justify-between">
        <DiscoveryFilterLabel icon={<CalendarDays className="h-3.5 w-3.5 shrink-0" aria-hidden="true" />}>
          {t("discovery.releaseYear")}
        </DiscoveryFilterLabel>
        <span className="text-[11.5px] text-[var(--scry-faint)]">
          {minimumYear} - {maximumYear}
        </span>
      </div>
      <div className="relative mb-6 h-5">
        <div className="absolute left-0 right-0 top-1/2 h-1.5 -translate-y-1/2 rounded-full bg-[var(--scry-border2)]" />
        <div
          className="absolute top-1/2 h-1.5 -translate-y-1/2 rounded-full bg-gradient-to-r from-[var(--scry-accent)] to-[var(--scry-accent-ring)]"
          style={{
            left: `${minimumYearPercent}%`,
            right: `${100 - maximumYearPercent}%`,
          }}
        />
        <input
          type="range"
          min={minimumYearBound}
          max={maximumYearBound}
          value={minimumYear}
          aria-label={t("discovery.releaseYear")}
          onChange={(event) =>
            onMinimumYearChange(Math.min(Number(event.target.value), maximumYear))
          }
          className={cn(
            "absolute left-0 right-0 top-1/2 -translate-y-1/2 bg-transparent",
            FILTER_RANGE_CLASS_NAME,
            FILTER_RANGE_THUMB_POINTER_CLASS_NAME,
          )}
        />
        <input
          type="range"
          min={minimumYearBound}
          max={maximumYearBound}
          value={maximumYear}
          aria-label={t("discovery.releaseYear")}
          onChange={(event) =>
            onMaximumYearChange(Math.max(Number(event.target.value), minimumYear))
          }
          className={cn(
            "absolute left-0 right-0 top-1/2 -translate-y-1/2 bg-transparent",
            FILTER_RANGE_CLASS_NAME,
            FILTER_RANGE_THUMB_POINTER_CLASS_NAME,
          )}
        />
      </div>
      <div className="mb-2.5 flex items-center justify-between">
        <DiscoveryFilterLabel icon={<Star className="h-3.5 w-3.5 shrink-0" aria-hidden="true" />}>
          {t("discovery.minimumRating")}
        </DiscoveryFilterLabel>
        <span className="text-[11.5px] font-bold text-[var(--scry-accent-ring)]">
          {minimumRating.toFixed(1)}+
        </span>
      </div>
      <div className="relative h-5">
        <div className="absolute left-0 right-0 top-1/2 h-1.5 -translate-y-1/2 rounded-full bg-[var(--scry-border2)]" />
        <div
          className="absolute left-0 top-1/2 h-1.5 -translate-y-1/2 rounded-full bg-gradient-to-r from-[var(--scry-accent)] to-[var(--scry-accent-ring)]"
          style={{ width: `${ratingPercent}%` }}
        />
        <input
          type="range"
          min={0}
          max={10}
          step={0.5}
          value={minimumRating}
          onChange={(event) => onMinimumRatingChange(Number(event.target.value))}
          className={cn(
            "absolute left-0 right-0 top-1/2 -translate-y-1/2",
            FILTER_RANGE_CLASS_NAME,
          )}
        />
      </div>
      {hiddenItemCount > 0 ? (
        <div className="mt-6 border-t border-[var(--scry-border3)] pt-4">
          <div className="mb-2.5 flex items-center justify-between gap-2">
            <span className="text-xs font-bold uppercase tracking-[0.05em] text-[var(--scry-muted2)]">
              {t("discovery.hiddenTitles")}
            </span>
            <span className="text-[11px] text-[var(--scry-faint)]">
              {hiddenItemCount}
            </span>
          </div>
          <button
            type="button"
            onClick={onShowHidden}
            className="inline-flex w-full items-center justify-center gap-2 rounded-[9px] border border-[var(--scry-border2)] bg-[var(--scry-bg)] px-3 py-2 text-[12.5px] font-semibold text-[var(--scry-text2)] transition hover:border-[rgba(var(--scry-accent-rgb),0.34)]"
          >
            <Eye className="h-3.5 w-3.5 text-[var(--scry-accent-text)]" />
            {t("discovery.showHidden")}
          </button>
        </div>
      ) : null}
    </aside>
  );
}

// --- Local-only "not interested" (SI2: no server state, no telemetry) ---

const HIDDEN_ITEMS_STORAGE_KEY = "scryer.discovery.hiddenItems.v1";

function readHiddenItemKeys(): string[] {
  if (typeof window === "undefined") {
    return [];
  }
  try {
    const raw = window.localStorage.getItem(HIDDEN_ITEMS_STORAGE_KEY);
    if (!raw) {
      return [];
    }
    const parsed = JSON.parse(raw) as unknown;
    return Array.isArray(parsed)
      ? parsed.filter((value): value is string => typeof value === "string")
      : [];
  } catch {
    return [];
  }
}

function writeHiddenItemKeys(keys: string[]) {
  if (typeof window === "undefined") {
    return;
  }
  try {
    window.localStorage.setItem(
      HIDDEN_ITEMS_STORAGE_KEY,
      JSON.stringify(keys),
    );
  } catch {
    // Storage may be unavailable (private mode / quota) — hiding stays
    // in-memory for the session, which is acceptable for a local preference.
  }
}

function useHiddenDiscoveryItems() {
  const [hiddenKeys, setHiddenKeys] = React.useState<Set<string>>(
    () => new Set(),
  );
  // Hydrate from storage on mount only (avoids SSR mismatch).
  React.useEffect(() => {
    setHiddenKeys(new Set(readHiddenItemKeys()));
  }, []);
  const hideItem = React.useCallback((item: DiscoveryHomeCard) => {
    setHiddenKeys((current) => {
      const key = itemStableKey(item);
      if (current.has(key)) {
        return current;
      }
      const next = new Set(current);
      next.add(key);
      writeHiddenItemKeys([...next]);
      return next;
    });
  }, []);
  const resetHidden = React.useCallback(() => {
    setHiddenKeys((current) => {
      if (current.size === 0) {
        return current;
      }
      writeHiddenItemKeys([]);
      return new Set();
    });
  }, []);
  return { hiddenKeys, hideItem, resetHidden };
}

function sectionsWithoutHiddenItems(
  sections: DiscoveryHomeSection[],
  hiddenKeys: Set<string>,
) {
  if (hiddenKeys.size === 0) {
    return sections;
  }
  return sections
    .map((section) => ({
      ...section,
      items: section.items.filter(
        (item) => !hiddenKeys.has(itemStableKey(item)),
      ),
    }))
    .filter((section) => section.items.length > 0);
}

function sectionsForDiscoveryFacets(
  sections: DiscoveryHomeSection[],
  allowedFacets: ReadonlySet<Facet>,
) {
  return sections
    .map((section) => {
      const items = section.items.filter((item) => {
        const facet = discoveryItemFacet(item);
        return facet !== null && allowedFacets.has(facet);
      });
      const removedCount = section.items.length - items.length;
      return {
        ...section,
        totalCount: Math.max(items.length, section.totalCount - removedCount),
        items,
      };
    })
    .filter((section) => section.items.length > 0);
}

// --- Freshness indicator (SW5) ---

// Locale-aware "3 hours ago"-style phrasing without per-locale strings for the
// relative part. Falls back to null when the timestamp is missing/unparseable.
function DiscoveryPendingUpdateChip({
  status,
}: {
  status: DiscoveryHomeStatus | null | undefined;
}) {
  const t = useTranslate();
  const pendingChanges = status?.pendingContextChangeCount ?? 0;
  if (pendingChanges === 0) {
    return null;
  }
  return (
    <span
      className="inline-flex items-center gap-1.5 rounded-[8px] border border-[var(--scry-warning-border)] bg-[var(--scry-warning-bg)] px-2.5 py-1 text-[11.5px] font-medium text-[var(--scry-warning-text)]"
      title={t("discovery.updatePendingHint")}
    >
      {t("discovery.updatePending")}
    </span>
  );
}

export function DiscoveryView({
  home,
  loading,
  error,
  manageableFacets,
  requestableFacets,
  filterOptions,
  onFiltersChange,
  onRefresh,
  onAction,
}: DiscoveryViewProps) {
  const t = useTranslate();
  const manageableFacetSet = React.useMemo(
    () => new Set(manageableFacets),
    [manageableFacets],
  );
  const requestableFacetSet = React.useMemo(
    () => new Set(requestableFacets),
    [requestableFacets],
  );
  const discoverableFacets = React.useMemo(
    () =>
      DISCOVERY_CONTENT_TYPES.filter(
        (facet) =>
          manageableFacetSet.has(facet) || requestableFacetSet.has(facet),
      ),
    [manageableFacetSet, requestableFacetSet],
  );
  const discoverableFacetSet = React.useMemo(
    () => new Set(discoverableFacets),
    [discoverableFacets],
  );
  const [selectedContentTypes, setSelectedContentTypes] = React.useState<
    DiscoveryContentType[]
  >(DEFAULT_DISCOVERY_CONTENT_TYPES);
  React.useEffect(() => {
    setSelectedContentTypes((current) => {
      const visibleSelection = current.filter((contentType) =>
        discoverableFacetSet.has(contentType),
      );
      const next =
        visibleSelection.length > 0
          ? visibleSelection
          : [...discoverableFacets];
      return current.length === next.length &&
        current.every((contentType, index) => contentType === next[index])
        ? current
        : next;
    });
  }, [discoverableFacetSet, discoverableFacets]);
  const [selectedGenres, setSelectedGenres] = React.useState<string[]>([]);
  const [selectedThemes, setSelectedThemes] = React.useState<string[]>([]);
  const [minimumYear, setMinimumYear] =
    React.useState(DEFAULT_MINIMUM_YEAR);
  const [maximumYear, setMaximumYear] = React.useState(
    defaultMaximumDiscoveryYear,
  );
  const [minimumRating, setMinimumRating] = React.useState(
    DEFAULT_MINIMUM_RATING,
  );
  const [selectedStudioSlugs, setSelectedStudioSlugs] = React.useState<
    string[]
  >([]);
  const [filtersOpen, setFiltersOpen] = React.useState(false);
  const [filterRailCollapsed, setFilterRailCollapsed] = React.useState(false);
  const { hiddenKeys, hideItem, resetHidden } = useHiddenDiscoveryItems();
  const orderedResult = React.useMemo(
    () => orderedHomeSections(home),
    [home],
  );
  const orderedSections = orderedResult.sections;
  const capabilitySections = React.useMemo(
    () => sectionsForDiscoveryFacets(orderedSections, discoverableFacetSet),
    [discoverableFacetSet, orderedSections],
  );
  // Local-only "not interested" remains a presentation preference.
  const rawSections = React.useMemo(
    () => sectionsWithoutHiddenItems(capabilitySections, hiddenKeys),
    [capabilitySections, hiddenKeys],
  );
  const hiddenItemCount = React.useMemo(() => {
    if (hiddenKeys.size === 0) {
      return 0;
    }
    const visibleFeedKeys = new Set(
      capabilitySections
        .flatMap((section) => section.items)
        .map((item) => itemStableKey(item)),
    );
    let count = 0;
    for (const key of hiddenKeys) {
      if (visibleFeedKeys.has(key)) {
        count += 1;
      }
    }
    return count;
  }, [capabilitySections, hiddenKeys]);
  const yearBounds = React.useMemo(
    () => ({
      minimum: DEFAULT_MINIMUM_YEAR,
      maximum: defaultMaximumDiscoveryYear(),
    }),
    [],
  );
  const effectiveMaximumYear = Math.max(
    Math.min(Math.max(maximumYear, yearBounds.minimum), yearBounds.maximum),
    yearBounds.minimum,
  );
  const effectiveMinimumYear = Math.min(
    Math.max(minimumYear, yearBounds.minimum),
    effectiveMaximumYear,
  );
  const effectiveSelectedContentTypes = React.useMemo(
    () =>
      selectedContentTypes.filter((contentType) =>
        discoverableFacetSet.has(contentType),
      ),
    [discoverableFacetSet, selectedContentTypes],
  );
  const serverFilters = React.useMemo<DiscoveryHomeFilters>(() => {
    const allDiscoverableTypesSelected =
      effectiveSelectedContentTypes.length === discoverableFacets.length &&
      discoverableFacets.every((contentType) =>
        effectiveSelectedContentTypes.includes(contentType),
      );
    return {
      contentTypes: allDiscoverableTypesSelected
        ? []
        : effectiveSelectedContentTypes,
      genreTagKeys: selectedGenres,
      themeTagKeys: selectedThemes,
      studioSlugs: selectedStudioSlugs,
      minimumYear:
        effectiveMinimumYear > yearBounds.minimum
          ? effectiveMinimumYear
          : undefined,
      maximumYear:
        effectiveMaximumYear < yearBounds.maximum
          ? effectiveMaximumYear
          : undefined,
      minimumRating:
        minimumRating > DEFAULT_MINIMUM_RATING ? minimumRating : undefined,
    };
  }, [
    discoverableFacets,
    effectiveMaximumYear,
    effectiveMinimumYear,
    effectiveSelectedContentTypes,
    minimumRating,
    selectedGenres,
    selectedStudioSlugs,
    selectedThemes,
    yearBounds.maximum,
    yearBounds.minimum,
  ]);
  const serverFiltersRef = React.useRef(serverFilters);
  const serverFiltersSignature = discoveryHomeFiltersSignature(serverFilters);
  const serverFiltersSignatureRef = React.useRef(serverFiltersSignature);
  React.useEffect(() => {
    serverFiltersRef.current = serverFilters;
    serverFiltersSignatureRef.current = serverFiltersSignature;
  }, [serverFilters, serverFiltersSignature]);
  const lastPublishedFiltersSignatureRef = React.useRef(
    serverFiltersSignature,
  );
  React.useEffect(() => {
    const currentSignature = serverFiltersSignatureRef.current;
    if (
      lastPublishedFiltersSignatureRef.current === currentSignature
    ) {
      return;
    }
    lastPublishedFiltersSignatureRef.current = currentSignature;
    onFiltersChange(serverFiltersRef.current);
  }, [
    effectiveSelectedContentTypes,
    onFiltersChange,
    selectedGenres,
    selectedStudioSlugs,
    selectedThemes,
  ]);
  React.useEffect(() => {
    const expectedSignature = serverFiltersSignatureRef.current;
    if (
      lastPublishedFiltersSignatureRef.current === expectedSignature
    ) {
      return;
    }
    const timeout = window.setTimeout(
      () => {
        const currentFilters = serverFiltersRef.current;
        const currentSignature = discoveryHomeFiltersSignature(currentFilters);
        if (lastPublishedFiltersSignatureRef.current === currentSignature) {
          return;
        }
        lastPublishedFiltersSignatureRef.current = currentSignature;
        onFiltersChange(currentFilters);
      },
      250,
    );
    return () => window.clearTimeout(timeout);
  }, [
    effectiveMaximumYear,
    effectiveMinimumYear,
    minimumRating,
    onFiltersChange,
  ]);
  const sections = React.useMemo(
    () => rawSections,
    [rawSections],
  );
  const heroSections = React.useMemo(
    () =>
      // Pre-floor list: a thin rail hidden from the page must not blank the
      // hero when its item rendered in no other rail.
      sectionsWithoutHiddenItems(
        sectionsForDiscoveryFacets(
          orderedResult.heroVisibilitySections,
          discoverableFacetSet,
        ),
        hiddenKeys,
      ).filter((section) => !sectionIsCompleteCollection(section)),
    [discoverableFacetSet, hiddenKeys, orderedResult.heroVisibilitySections],
  );
  const heroItem = React.useMemo(
    () => {
      const configuredHeroFacet = home?.heroItem
        ? discoveryItemFacet(home.heroItem)
        : null;
      return home?.heroItem &&
        configuredHeroFacet !== null &&
        discoverableFacetSet.has(configuredHeroFacet) &&
        discoveryItemHasUsefulTitle(home.heroItem) &&
        heroItemIsVisibleInSections(home.heroItem, heroSections)
        ? home.heroItem
        : null;
    },
    [discoverableFacetSet, heroSections, home?.heroItem],
  );
  const heroFacet = heroItem ? discoveryItemFacet(heroItem) : null;
  const heroRailSection = React.useMemo(
    () => findHeroRailSection(heroSections),
    [heroSections],
  );
  const heroRailSectionWithoutHero = React.useMemo(
    () => sectionWithoutItem(heroRailSection, heroItem),
    [heroItem, heroRailSection],
  );
  const railSections = React.useMemo(
    () =>
      sections
        .filter((section) => section.sectionId !== heroRailSection?.sectionId)
        .map((section) => sectionWithoutItem(section, heroItem))
        .filter((section): section is DiscoveryHomeSection => Boolean(section)),
    [heroItem, heroRailSection, sections],
  );
  const primaryRailSections = React.useMemo(
    () => railSections.filter((section) => !sectionIsUpcoming(section)),
    [railSections],
  );
  const upcomingRailSections = React.useMemo(
    () => railSections.filter(sectionIsUpcoming),
    [railSections],
  );
  const hasRenderableContent =
    heroItem !== null ||
    primaryRailSections.length > 0 ||
    upcomingRailSections.length > 0;
  const hasPendingDiscoveryChanges =
    (home?.status?.pendingContextChangeCount ?? 0) > 0;
  const toggleContentType = React.useCallback(
    (contentType: DiscoveryContentType) => {
      setSelectedContentTypes((current) =>
        current.includes(contentType)
          ? current.filter((item) => item !== contentType)
          : [...current, contentType],
      );
    },
    [],
  );
  const toggleStudioSlug = React.useCallback((studioSlug: string) => {
    setSelectedStudioSlugs((current) =>
      current.includes(studioSlug)
        ? current.filter((value) => value !== studioSlug)
        : [...current, studioSlug],
    );
  }, []);
  const clearFilters = React.useCallback(() => {
    const clearedFilters = emptyDiscoveryHomeFilters();
    const clearedFiltersSignature = discoveryHomeFiltersSignature(clearedFilters);
    if (
      lastPublishedFiltersSignatureRef.current !== clearedFiltersSignature
    ) {
      lastPublishedFiltersSignatureRef.current = clearedFiltersSignature;
      onFiltersChange(clearedFilters);
    }
    setSelectedContentTypes(discoverableFacets);
    setSelectedGenres([]);
    setSelectedThemes([]);
    setSelectedStudioSlugs([]);
    setMinimumYear(Math.max(yearBounds.minimum, DEFAULT_MINIMUM_YEAR));
    setMaximumYear(yearBounds.maximum);
    setMinimumRating(DEFAULT_MINIMUM_RATING);
  }, [
    discoverableFacets,
    onFiltersChange,
    yearBounds.maximum,
    yearBounds.minimum,
  ]);
  React.useEffect(() => {
    if (!filtersOpen || typeof document === "undefined") {
      return undefined;
    }
    const previousOverflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        setFiltersOpen(false);
      }
    };
    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.body.style.overflow = previousOverflow;
      document.removeEventListener("keydown", onKeyDown);
    };
  }, [filtersOpen]);
  const filterProps = {
    filterOptions,
    availableContentTypes: discoverableFacets,
    selectedContentTypes: effectiveSelectedContentTypes,
    selectedGenres,
    selectedThemes,
    selectedStudioSlugs,
    minimumYear: effectiveMinimumYear,
    maximumYear: effectiveMaximumYear,
    minimumRating,
    hiddenItemCount,
    onToggleContentType: toggleContentType,
    onGenresChange: setSelectedGenres,
    onThemesChange: setSelectedThemes,
    onToggleStudioSlug: toggleStudioSlug,
    onMinimumYearChange: setMinimumYear,
    onMaximumYearChange: setMaximumYear,
    onMinimumRatingChange: setMinimumRating,
    onClear: clearFilters,
    onShowHidden: resetHidden,
  };

  if (loading && !home) {
    return (
      <div className="flex min-h-[360px] items-center justify-center">
        <Loader2 className="h-6 w-6 animate-spin text-[var(--scry-accent)]" />
      </div>
    );
  }

  return (
    <div
      id="discovery-view"
      data-ui="discovery-view"
      className="flex min-h-0 flex-1"
    >
      <main className="min-w-0 flex-1 overflow-y-auto px-7 py-6 pb-16 max-sm:px-4">
        <div
          className={cn(
            "mb-5 items-center justify-between gap-3",
            // Always present below xl (holds the mobile filters button); on xl
            // only when the discovery data is waiting on an update.
            "flex max-xl:flex",
            hasPendingDiscoveryChanges || filterRailCollapsed
              ? "xl:flex"
              : "xl:hidden",
          )}
        >
          <DiscoveryPendingUpdateChip status={home?.status} />
          <button
            type="button"
            aria-label={t("discovery.openFilters")}
            onClick={() => setFiltersOpen(true)}
            className="inline-flex h-9 shrink-0 items-center gap-2 rounded-[9px] border border-[var(--scry-border2)] bg-[var(--scry-bg)] px-3 text-[12.5px] font-semibold text-[var(--scry-ink2)] max-xl:inline-flex xl:hidden"
          >
            <SlidersHorizontal className="h-4 w-4 text-[var(--scry-accent-text)]" />
            {t("discovery.filters")}
          </button>
          {filterRailCollapsed ? (
            <button
              type="button"
              aria-label={t("discovery.openFilters")}
              title={t("discovery.openFilters")}
              onClick={() => setFilterRailCollapsed(false)}
              className="ml-auto hidden h-[2.8125rem] w-[2.8125rem] shrink-0 items-center justify-center rounded-[11px] border !border-[rgba(var(--scry-accent-rgb),0.55)] bg-[var(--scry-inset)] text-[var(--scry-accent-text)] shadow-none transition hover:bg-[var(--scry-hover)] xl:inline-flex"
            >
              <PanelRightOpen className="h-[1.125rem] w-[1.125rem]" />
            </button>
          ) : null}
        </div>

        {error ? (
          <div className="mb-5 flex items-center justify-between gap-4 rounded-[12px] border border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)] px-4 py-3 text-sm text-[var(--scry-danger-text)]">
            <span>{error}</span>
            <Button type="button" size="sm" variant="outline" onClick={onRefresh}>
              {t("label.retry")}
            </Button>
          </div>
        ) : null}

        {heroItem ? (
          <div className="mb-7 grid grid-cols-1 items-stretch gap-5 lg:grid-cols-[minmax(0,clamp(30rem,42vw,50rem))_minmax(0,1fr)] lg:min-h-[clamp(440px,46vh,520px)]">
            <DiscoveryHero
              item={heroItem}
              canManageTitle={
                heroFacet !== null && manageableFacetSet.has(heroFacet)
              }
              canRequestMedia={
                heroFacet !== null && requestableFacetSet.has(heroFacet)
              }
              onAction={onAction}
            />
            {heroRailSectionWithoutHero ? (
              <DiscoverySectionRail
                section={heroRailSectionWithoutHero}
                fillHeight
                manageableFacets={manageableFacetSet}
                requestableFacets={requestableFacetSet}
                onAction={onAction}
                onDismissItem={hideItem}
              />
            ) : null}
          </div>
        ) : null}

        {primaryRailSections.length > 0 ? (
          primaryRailSections.map((section) => (
            <DiscoverySectionRail
              key={section.sectionId}
              section={section}
              manageableFacets={manageableFacetSet}
              requestableFacets={requestableFacetSet}
              onAction={onAction}
              onDismissItem={hideItem}
            />
          ))
        ) : !loading && !hasRenderableContent ? (
          <div className="rounded-[14px] border border-[var(--scry-border)] bg-[var(--scry-surf)] px-5 py-8 text-center">
            <Sparkles className="mx-auto mb-3 h-6 w-6 text-[var(--scry-accent-text)]" />
            <h2 className="mb-2 font-[var(--font-space-grotesk)] text-lg font-semibold text-[var(--scry-ink2)]">
              {t("discovery.emptyTitle")}
            </h2>
            <p className="mx-auto max-w-md text-sm leading-6 text-[var(--scry-muted3)]">
              {t("discovery.emptyDescription")}
            </p>
          </div>
        ) : null}

        {upcomingRailSections.map((section) => (
          <DiscoverySectionRail
            key={section.sectionId}
            section={section}
            variant="upcoming"
            manageableFacets={manageableFacetSet}
            requestableFacets={requestableFacetSet}
            onAction={onAction}
            onDismissItem={hideItem}
          />
        ))}
      </main>
      {filtersOpen ? (
        <div className="fixed inset-0 z-50 xl:hidden">
          <button
            type="button"
            aria-label={t("discovery.closeFilters")}
            className="absolute inset-0 bg-slate-950/65 backdrop-blur-sm"
            onClick={() => setFiltersOpen(false)}
          />
          <div className="absolute bottom-0 right-0 top-0 w-[min(360px,100%)]">
            <DiscoveryFilters
              {...filterProps}
              variant="mobile"
              onRequestClose={() => setFiltersOpen(false)}
            />
          </div>
        </div>
      ) : null}
      {!filterRailCollapsed ? (
        <DiscoveryFilters
          {...filterProps}
          onCollapse={() => setFilterRailCollapsed(true)}
        />
      ) : null}
    </div>
  );
}
