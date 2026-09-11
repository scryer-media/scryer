# scryer-vNEXT — "Because You Like {Genre}" rail quality

Notes for the rail-quality track (plan `scryer-docs/plans/154-because-you-like-rail-quality-plan.md`).
Fold this section into the next release's notes file when the version is cut.

Personalized rails now refuse to recommend a medium you do not watch, refuse to
name a rail after a genre one community tag invented, and refuse to fill
themselves with titles they have no reason to believe in.

## Highlights
- **Medium is an axis, not a genre.** Every title is classified as live action,
  Western animation, or anime, and a medium your library owns *none* of is
  excluded from every personalized rail — theme rails, genre rails, For You, and
  the Anime and Animation rails themselves. Above zero, a rail may hold at most
  twice the medium's share of your library (floor: two slots), and refills from
  the next candidates rather than truncating. The Animation rail now means
  animation, not "anything that is not anime".
- **Rails are ordered by relevance.** Rail order leads with the gateway's
  blended recommendation score, then the number of your titles that pointed at
  it, then credible rating evidence, then the title's standing, and only then
  falls back to the catalog order. A title's name is the very last tiebreak, so
  a rail can no longer decay into alphabetical order.
- **A genre needs corroboration to name a rail.** A canonical genre counts only
  when it is confident (≥ 0.9) and either two independent sources agree or a
  provider's own genre list carries it. One AniList community tag no longer
  makes a show "Crime" — in your library profile or on the rail.
- **Rails carry evidence or they do not ship.** A genre rail's items need a real
  edge to your library, a credible rating, or standing at or above the pool's
  median. Theme rails and For You accept a credible rating without an edge, but
  never the evidence-less band. A rail is emitted with however many items
  qualify and dropped entirely below eight — it is never padded. A label needs
  at least max(3, 10% of your library) titles carrying it before it can name a
  rail, and the rail ladder itself widens with the library: under 10 owned
  titles you get For You only, 10–49 adds one theme and one genre rail, and 50
  or more opens the full ladder.
- **A growing library re-snapshots itself.** A fresh install that imports for
  hours after its first snapshot no longer waits a day for rails that match the
  library it now has: growth past the label-support floor, or by a quarter,
  schedules a new snapshot.

## Rollback
Four system settings turn the new behavior off without a downgrade
(unset = enabled):
- `discovery.medium_affinity_gate` (default `true`) — the zero rule and the
  proportional medium cap.
- `discovery.genre_rail_corroboration` (default `true`) — the corroborated-genre
  requirement, in the matcher and in the library profile.
- `discovery.genre_rail_credibility_gate` (default `true`) — the item evidence
  floor and the eight-item rail minimum.
- `discovery.rail_order` (`relevance` default, `legacy`) — rail ordering.

## Minimum gateway (SMG) contract
This release **requires** an SMG build that implements the following. The fields
are requested unconditionally and there is no fallback path: an older gateway
fails the discovery sync loudly rather than serving degraded rails.

1. `DiscoveryContextSnapshotTitle` and `DiscoveryContextChangesTitle` both expose
   - `recommendation_score: Float!` — SMG's blended relevance for the target,
     `0` when the target was not reranked.
   - `base_rank: Float!` — the target's popularity base rank, `0` when unknown.

   `rank_score` keeps its existing meaning (the strongest single edge score) and
   is neither relevance nor popularity.
2. `DiscoveryContextSnapshotSubmitInput` and `DiscoveryContextChangesInput` both
   accept an optional
   `mediumMix: DiscoveryContextMediumMixInput`, where
   `input DiscoveryContextMediumMixInput { liveAction: Int! = 0, animation: Int! = 0, anime: Int! = 0 }`
   carries the count of owned titles per medium. SMG hard-excludes a medium
   whose count is zero and applies a prior above zero. Scryer always sends it,
   and folds it into the context fingerprint, so a library that gains its first
   anime invalidates the cached run.

## Upgrade notes
- Migration `0238` adds `recommendation_score` and `base_rank` to
  `discovery_items` plus a relevance-ordered index. It is additive and applies
  to existing databases in place; no rebuild is required, and the columns fill
  in on the next discovery sync.
