// Discovery dashboard rail ordering.
//
// Rails are organised by *reason*, never by medium: the dashboard's media-type
// chips (All / Movies / Series / Anime) own medium, so a rail identity like
// "Movies For You" is a category error. Scryer is also an unbiased entry point,
// so no rail is allowed to carry a provider name - renames live in
// SECTION_DISPLAY_NAME_KEYS + locale files, not here.
//
// This lives outside the view component so the shelf order, the merge rules, and
// the legacy-payload tolerance stay unit-testable.

// Only the fields the ordering actually inspects, so tests can use plain
// fixtures and the view can pass its full DiscoveryHomeSection.
export type OrderableDiscoveryItem = {
  targetKind: string;
  targetKey: string;
};

export type OrderableDiscoverySection = {
  sectionId: string;
  sectionType: string;
  totalCount: number;
  items: readonly OrderableDiscoveryItem[];
};

export function discoverySectionType(section: { sectionType: string }) {
  return section.sectionType.trim().toUpperCase();
}

// Mirrors the view's own item identity so merge dedupe and hero visibility agree.
function discoveryItemKey(item: OrderableDiscoveryItem) {
  return `${item.targetKind}:${item.targetKey}`;
}

// Public-promotion rails: SMG's v2 feed surfaces these two "new release window"
// sections. They arrive at feed-bottom with raw SMG titles; we give them curated
// (locale-owned) names + icons and pin them between the personalized shelf and
// the world shelf.
export const NEW_ON_STREAMING_SECTION_TYPE = "NEW_ON_STREAMING";
export const NEW_ON_PHYSICAL_SECTION_TYPE = "NEW_ON_PHYSICAL";
export const PUBLIC_PROMOTION_SECTION_TYPES = [
  NEW_ON_STREAMING_SECTION_TYPE,
  NEW_ON_PHYSICAL_SECTION_TYPE,
] as const;
const PUBLIC_PROMOTION_SECTION_TYPE_SET = new Set<string>(
  PUBLIC_PROMOTION_SECTION_TYPES,
);

export function sectionIsPublicPromotion(section: { sectionType: string }) {
  return PUBLIC_PROMOTION_SECTION_TYPE_SET.has(discoverySectionType(section));
}

// The rail we prefer to seed the hero side-rail from, by explicit type rather
// than by sniffing raw gateway titles.
export const HERO_RAIL_PREFERRED_SECTION_TYPE = "POPULAR_RIGHT_NOW";

// Rails that are redundant *as rails* but NOT redundant in content.
//
// Scryer asks the gateway for the whole default feed (empty section_types), and
// the gateway dedupes across sections first-section-wins in request order. The
// rails it delivers are therefore already DISJOINT: POPULAR_RIGHT_NOW holds what
// TRENDING_NOW did not claim, and the three anime rails hold what
// POPULAR_WITH_ANIME_FANS did not claim. Hiding a source rail would delete those
// titles outright rather than remove duplicates - and would strand the
// server-chosen hero, which is usually a TRENDING_NOW member. So we fold the
// source rail's items into its target rail instead of dropping either.
const MERGED_SECTION_TARGETS = new Map<string, string>([
  ["TRENDING_NOW", "POPULAR_RIGHT_NOW"],
  ["POPULAR_WITH_ANIME_FANS", "ANIME_THIS_WEEK"],
]);

// The slot a section occupies in the world shelf. A merge source that found no
// target still renders - in its target's slot - so content never vanishes and the
// reading order stays stable whichever half the gateway happened to send.
function worldShelfSlotType(section: { sectionType: string }) {
  const sectionType = discoverySectionType(section);
  return MERGED_SECTION_TARGETS.get(sectionType) ?? sectionType;
}

// Personalized shelf, most specific reason first. Theme/tag rails ("Because You
// Like Isekai") outrank genre rails ("Because You Like Animation") - the same
// specificity ladder the Rust composer uses for dedupe priority, so the reading
// order matches the order in which titles were claimed.
const PERSONALIZED_SHELF_SECTION_ORDER = [
  "BECAUSE_YOU_LIKE_TAG",
  "BECAUSE_YOU_LIKE_GENRE",
  // Weekly tops. Kept ahead of the top-rated rail so the shelf reads
  // reason -> recency -> quality.
  "TOP_MOVIES_THIS_WEEK",
  "TOP_SERIES_THIS_WEEK",
  "TOP_ANIME_THIS_WEEK",
  // Scryer's own composer emits TOP_RATED; TOP_RATED_FOR_YOU is the gateway's
  // name for the same idea. TOP_RATED reaches the personalized array only when at
  // least one of its items is personalized - otherwise it arrives on the public
  // surface, which is why it also holds a world-shelf slot below.
  "TOP_RATED_FOR_YOU",
  "TOP_RATED",
];

// FOR_YOU is a generic "we have no better reason" rail, so it renders only when
// there is no library context at all to build reason-based rails from.
const GENERIC_FOR_YOU_FALLBACK_SECTION_TYPES = new Set(["FOR_YOU"]);

// World shelf (gateway public feed) in a fixed reading order, regardless of the
// order the gateway happened to emit. Anything added later that is not listed
// here still renders, just after these, in feed order.
const WORLD_SHELF_SECTION_ORDER = [
  "POPULAR_RIGHT_NOW",
  "POPULAR_MOVIES",
  "POPULAR_SERIES",
  "UPCOMING_MOVIES",
  "UPCOMING_SERIES",
  "TOP_RATED",
  "EVERGREEN_POPULAR",
  "ANIME_THIS_WEEK",
  "ANIME_SEASON_STANDOUTS",
  "MOST_ANTICIPATED_ANIME",
  "UPCOMING_NEXT_SEASON",
];

// Editor's Picks closes the page, always - after even unrecognised feed rails.
const CURATED_SECTION_TYPE = "CURATED";

// Retired rail identities that older persisted payloads and snapshots can still
// contain. They stay *recognised* (rather than forgotten) so a stale snapshot
// never renders a ghost rail: medium is chip-owned, not a rail identity, and
// BECAUSE_YOU_HAVE duplicated the title page's More Like This shelf. These are
// the only sections whose items are allowed to disappear from the page.
const SUPPRESSED_LEGACY_SECTION_TYPES = new Set([
  "BECAUSE_YOU_HAVE",
  "MOVIES_FOR_YOU",
  "SERIES_FOR_YOU",
  "ANIME_FOR_YOU",
]);

function sectionIsRenderable(section: OrderableDiscoverySection) {
  return (
    section.items.length > 0 &&
    !SUPPRESSED_LEGACY_SECTION_TYPES.has(discoverySectionType(section))
  );
}

function mergeRedundantPublicSections<
  TSection extends OrderableDiscoverySection,
>(sections: readonly TSection[]): TSection[] {
  const targetsByType = new Map<string, TSection>();
  for (const section of sections) {
    const sectionType = discoverySectionType(section);
    if (!targetsByType.has(sectionType)) {
      targetsByType.set(sectionType, section);
    }
  }

  const mergedItemsByTarget = new Map<TSection, OrderableDiscoveryItem[]>();
  const mergedAway = new Set<TSection>();
  for (const section of sections) {
    const targetType = MERGED_SECTION_TARGETS.get(discoverySectionType(section));
    if (!targetType) {
      continue;
    }
    const target = targetsByType.get(targetType);
    if (!target) {
      // Target absent from this payload: the source keeps rendering, and
      // worldShelfSlotType puts it where the target would have been.
      continue;
    }
    mergedAway.add(section);
    mergedItemsByTarget.set(target, [
      ...(mergedItemsByTarget.get(target) ?? []),
      ...section.items,
    ]);
  }

  return sections
    .filter((section) => !mergedAway.has(section))
    .map((section) => {
      const mergedItems = mergedItemsByTarget.get(section);
      if (!mergedItems) {
        return section;
      }
      // Target items lead: the gateway's first-section-wins dedupe means the
      // target claimed those titles under the stronger signal. The delivered
      // rails are disjoint, so this dedupe is belt-and-braces.
      const seen = new Set<string>();
      const items = [...section.items, ...mergedItems].filter((item) => {
        const key = discoveryItemKey(item);
        if (seen.has(key)) {
          return false;
        }
        seen.add(key);
        return true;
      });
      return { ...section, items, totalCount: items.length } as TSection;
    });
}

function orderDiscoveryHomeSectionsInternal<
  TSection extends OrderableDiscoverySection,
>(input: {
  publicSections: readonly TSection[];
  personalizedSections: readonly TSection[];
  completeCollection: TSection | null;
}): { sections: TSection[]; heroVisibilitySections: TSection[] } {
  const publicSections = mergeRedundantPublicSections(
    input.publicSections.filter(sectionIsRenderable),
  );
  const personalizedSections =
    input.personalizedSections.filter(sectionIsRenderable);
  const completeCollection =
    input.completeCollection && sectionIsRenderable(input.completeCollection)
      ? input.completeCollection
      : null;

  const usedPersonalizedSections = new Set<TSection>();
  const takePersonalizedSections = (sectionType: string) =>
    personalizedSections.filter((section) => {
      if (discoverySectionType(section) !== sectionType) {
        return false;
      }
      usedPersonalizedSections.add(section);
      return true;
    });

  const personalizedShelf = PERSONALIZED_SHELF_SECTION_ORDER.flatMap(
    takePersonalizedSections,
  );
  const unknownPersonalizedSections = personalizedSections.filter(
    (section) =>
      !usedPersonalizedSections.has(section) &&
      !GENERIC_FOR_YOU_FALLBACK_SECTION_TYPES.has(discoverySectionType(section)),
  );
  const hasLibrarySections =
    personalizedShelf.length > 0 ||
    completeCollection !== null ||
    unknownPersonalizedSections.length > 0;
  const fallbackSections = hasLibrarySections
    ? []
    : personalizedSections.filter((section) =>
        GENERIC_FOR_YOU_FALLBACK_SECTION_TYPES.has(discoverySectionType(section)),
      );

  const publicPromotionSections = PUBLIC_PROMOTION_SECTION_TYPES.flatMap(
    (sectionType) =>
      publicSections.filter(
        (section) => discoverySectionType(section) === sectionType,
      ),
  );
  const worldShelfSections = WORLD_SHELF_SECTION_ORDER.flatMap((slotType) =>
    publicSections.filter((section) => worldShelfSlotType(section) === slotType),
  );
  const curatedSections = publicSections.filter(
    (section) => discoverySectionType(section) === CURATED_SECTION_TYPE,
  );
  const placedPublicSections = new Set<TSection>([
    ...publicPromotionSections,
    ...worldShelfSections,
    ...curatedSections,
  ]);
  const remainingPublicSections = publicSections.filter(
    (section) => !placedPublicSections.has(section),
  );

  const ordered = [
    ...personalizedShelf,
    ...(completeCollection ? [completeCollection] : []),
    ...unknownPersonalizedSections,
    ...fallbackSections,
    ...publicPromotionSections,
    ...worldShelfSections,
    ...remainingPublicSections,
    ...curatedSections,
  ];
  const floorExemptIds = new Set<string>(
    [
      ...personalizedShelf,
      ...(completeCollection ? [completeCollection] : []),
      ...unknownPersonalizedSections,
      ...fallbackSections,
    ].map((section) => section.sectionId),
  );
  return applyCrossRailPresentation(ordered, floorExemptIds);
}

// orderDiscoveryHomeSections is the primary reading-order entry point; the
// Detailed variant additionally exposes the post-dedupe PRE-floor list, which
// is what hero visibility must be checked against — a floored thin rail must
// never blank a hero whose item rendered nowhere else.
export function orderDiscoveryHomeSections<
  TSection extends OrderableDiscoverySection,
>(input: {
  publicSections: readonly TSection[];
  personalizedSections: readonly TSection[];
  completeCollection: TSection | null;
}): TSection[] {
  return orderDiscoveryHomeSectionsDetailed(input).sections;
}

export function orderDiscoveryHomeSectionsDetailed<
  TSection extends OrderableDiscoverySection,
>(input: {
  publicSections: readonly TSection[];
  personalizedSections: readonly TSection[];
  completeCollection: TSection | null;
}): { sections: TSection[]; heroVisibilitySections: TSection[] } {
  return orderDiscoveryHomeSectionsInternal(input);
}

// MIN_PUBLIC_RAIL_ITEMS: a public rail that keeps fewer unique items than this
// after cross-rail dedupe hides instead of rendering — a one-card rail reads as
// breakage (owner incident 2026-08-05). Personalized rails and the collection
// completer are exempt: a thin personal rail is still meaningful.
export const MIN_PUBLIC_RAIL_ITEMS = 3;

// applyCrossRailPresentation is the final stage of the reading order: Scryer —
// not the gateway — owns which rail shows a title (owner directive 2026-08-05;
// the gateway now ships public-feed sections FULL, so the same title may arrive
// in several sections). First occurrence in reading order wins; later
// duplicates drop; public rails falling under MIN_PUBLIC_RAIL_ITEMS hide.
function applyCrossRailPresentation<TSection extends OrderableDiscoverySection>(
  ordered: readonly TSection[],
  floorExemptIds: ReadonlySet<string>,
): { sections: TSection[]; heroVisibilitySections: TSection[] } {
  const seen = new Set<string>();
  const deduped: TSection[] = [];
  for (const section of ordered) {
    const items = section.items.filter((item) => {
      const key = discoveryItemKey(item);
      if (seen.has(key)) {
        return false;
      }
      seen.add(key);
      return true;
    });
    if (items.length === 0) {
      continue;
    }
    deduped.push(
      items.length === section.items.length ? section : { ...section, items },
    );
  }
  const sections = deduped.filter(
    (section) =>
      floorExemptIds.has(section.sectionId) ||
      section.items.length >= MIN_PUBLIC_RAIL_ITEMS,
  );
  return { sections, heroVisibilitySections: deduped };
}
