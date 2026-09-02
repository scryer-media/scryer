# Feature Specification: Indexer Search (aggregate raw search + assign-on-grab)

**Status**: Draft — 2026-09-02, awaiting the operator's sign-off on the decisions in plan.md
**Design handoff**: `~/.claude/plans/indexer-search/handoff/` (README.md + `IndexersView.dc.html`
prototype + screenshots). When this spec and the handoff disagree on visuals, the prototype wins;
when they disagree on behaviour, plan.md's decisions win.
**Oracle**: Prowlarr's Search page (flat aggregate table, category dropdown, per-indexer grab).
Scryer must match its coverage and exceed it on health visibility and grab semantics.

## Problem

Operators cannot see what their indexers actually return. Today every search is bound to a
catalog title, so an operator whose title-matching is failing for one release has no way to find
that release, inspect it, and hand it to the right title (or straight to a download client).

## User stories

- **US1 — Search my indexers.** As an operator I type a query, pick a search kind, optionally
  narrow indexers and categories, and see one merged table of what every enabled indexer
  returned, with each indexer's health (ok / slow / failed + error word) inline.
- **US2 — Refine.** I filter the merged set by protocol, resolution, source, audio/HDR, flags
  and size without re-querying; counts never shift as I toggle facets.
- **US3 — Grab and assign.** From a row or a multi-selection I open one dialog, pick the
  library title the release belongs to (or season + coverage for episodic titles), see the
  download client and import path that follow from that choice, and grab. Scryer then imports
  and renames it under that title exactly as an interactive-search grab would.
- **US4 — Grab unlinked.** When no title fits, I grab straight to a download client with an
  explicit warning; the download appears in Activity as needing manual import.
- **US5 — Retry.** When some indexers failed, I retry only those without losing the rest.

## Functional requirements

Search
- FR-001 Search kinds: Movie, Series, Anime, Raw. (Design lists music/book; Scryer has no such
  facets — dropped, see plan D2.)
- FR-002 Indexer scope defaults to all enabled indexers with interactive search enabled;
  operator may restrict to a subset. Disabled/backoff indexers are shown as skipped, not hidden.
- FR-003 Categories default from the kind's routing defaults; operator may override.
- FR-004 Advanced per-search limits: min/max size (GiB), min seeders, max age (days),
  per-indexer result limit (default 100, cap 250). Not persisted.
- FR-005 The search runs as a server-side job: accepted immediately, polled, cancellable,
  results stream in per indexer as they complete (same lifecycle as interactive search).
- FR-006 Per-indexer outcome: state ok/slow/failed/skipped, result count, elapsed ms, and a
  short error word on failure (`timeout`, `auth`, `http 503`, …).
- FR-007 Totals: matched (raw survivors of advanced limits), passing (after facets + rejections),
  indexers queried/responded, elapsed, and job age.
- FR-008 Releases carry: raw title (never rewritten), protocol, indexer (id, name, priority),
  size, published time, category label, file summary, release group, seeders/leechers (torrent),
  grabs (usenet), flags, rejections, info URL. Download URLs never leave the server.
- FR-009 Facets and their counts are computed server-side over the full merged set.
- FR-010 Rejections are context-free only (plan D6): a release rejected by the kind's default
  quality profile or by Release rules is shown red-edged with the rule named, stays visible and
  stays grabbable. No score, rank, or "best match" is ever shown.
- FR-011 Retry re-runs only the failed indexers inside the same job and merges the results.
- FR-012 Completed jobs stay addressable for 30 minutes so a grab dialog can be opened late;
  a grab against an expired job returns a typed, actionable error.

Grab
- FR-020 Every grab entry point opens the grab dialog; nothing is queued without a target
  decision (title or explicit "unlinked").
- FR-021 Title picker candidates are ranked server-side: name match, then open gap, then rest.
  Each candidate carries kind, gap label (`Wanted · cutoff unmet`, `S06 · 4 missing`,
  `Upgrade wanted`, `Complete`), profile name, root path, default download client, and (for
  episodic titles) seasons with missing counts.
- FR-022 Episodic targets require season + coverage (episode / season pack / full series);
  coverage is pre-selected from the parsed release name and the operator may correct it.
- FR-023 Download client defaults from the target's routing and is overridable per grab;
  import path is derived from the target and read-only.
- FR-024 A linked grab uses the existing operator-queued submission path: same submission
  row, same tracked download, same import guards, same history events as an interactive-search
  grab. Rejected releases may be grabbed only after the operator ticks the override.
- FR-025 Multi-grab assigns one target to every selected release; per-release outcomes
  (queued / conflict / error) are reported individually. Mixed targets are not supported.
- FR-026 An unlinked grab submits to the chosen client, records a Scryer submission with no
  title (orphan scope), emits a grab history event, and surfaces in Activity as needing manual
  import. Requires the system-settings permission.
- FR-027 "Count as an upgrade" is implemented per plan D7 and labelled to match exactly what it
  does.

Page
- FR-030 Lives at `/integrations/indexers/search` as a pane of the Indexers page.
- FR-031 Only the pane scrolls; the document never does; the results table scrolls inside its
  card at every width (handoff §2 "Scroll ownership").
- FR-032 All colours via `--scry-*` tokens; light mode and accent changes need no edits.
- FR-033 Every user-visible string localised in all 10 locales.
- FR-034 Stable `id`/`data-ui` selectors on every control the e2e flow needs.

## Out of scope

- Adding a new catalog title from the grab dialog (grab unlinked, match in Import).
- Cross-indexer deduplication, mirror counts, grouped view, release scoring.
- Server-persisted saved searches (plan D10: per-browser bookmark only).
- A true "mark cutoff satisfied" acquisition flag (plan D7).

## Acceptance

The handoff's §13 checklist, verbatim, plus:
- [ ] A linked grab from this page and an interactive-search grab of the same release produce
      identical submission rows (minus ids/timestamps) and identical history events.
- [ ] An unlinked grab appears in Activity within one poll cycle as a manual-import candidate
      carrying the release name and indexer.
- [ ] Retry after a forced indexer failure adds that indexer's results without duplicating
      others.
- [ ] e2e flow (plan WP6) passes on the release gate.
