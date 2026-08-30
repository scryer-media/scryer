# Scryer 0.19.0 release notes

> Draft — these notes describe the net change from `scryer-v0.18.21` to
> `scryer-v0.19.0`. They intentionally describe the completed release rather
> than the order individual release-branch changes landed.

## Highlights

0.19.0 is a substantial reliability and operations release. It adds supported
in-app updates for eligible Windows installations, makes large import backlogs
move through the system without one slow copy holding everything else up, and
removes several sources of lost download/import evidence. Acquisition now works
through large wanted backlogs concurrently, prioritizes title and season packs,
and gives indexers much more realistic time and concurrency budgets.

The release also adds operator-facing indexer error history, structured JSON
logging with an in-app inspector, richer series-movie ratings and cast, live
import activity, media-server playback links, user API keys and OAuth client
registrations, plus a broad set of quality, manual-import, database, Windows,
plugin, and GraphQL hardening work.

## What changed

### Supported in-app application updates

- **Eligible Windows portable and direct-MSI installations can now install an
  Scryer update from the application.** The update experience reports the
  installed and available versions, whether the installation is eligible, why
  it is not eligible when it is not, and job progress while the update is being
  prepared and applied.
- Updates are driven by a signed release manifest and verify the advertised
  artifact, digest, archive layout, and staged files before replacement. The
  update flow coordinates maintenance and restart work so normal application
  activity is not silently overwritten mid-operation.
- Windows portable updates use the portable artifact layout; direct MSI updates
  use an elevation helper for the installer hand-off. This also improves the
  portable Windows package and long-path handling.
- Docker, Homebrew, winget, supervised Windows, disabled, and unsupported
  layouts remain operator-managed. The UI makes that ownership explicit rather
  than attempting an unsafe self-update.

### Faster, safer imports on busy or slow storage

- **Imports are no longer funneled through a single global or title-wide
  worker.** The bounded import workers can use their available capacity even
  when two releases belong to the same title. Final placement remains protected
  by a normalized destination-path permit, so concurrency does not allow two
  writers to claim the same file.
- File placement is now drive-aware. Each download client has a fast-placement
  lane for hardlinks, symlinks, moves, and other short operations, while long
  fallback copies use up to two slots per physical destination volume. A slow
  copy therefore cannot consume the lane needed to hardlink another completed
  download or block imports going to a different drive.
- Manual and automatic imports share the same bounded execution, client,
  destination, and volume coordination. Pending rows stay pending until actual
  execution begins, and in-flight work is de-duplicated across poll cycles.
- Long file operations run in a monitored worker process. The stall watchdog is
  progress-aware and deliberately generous for large copies. When a genuinely
  stalled worker can be confirmed stopped, Scryer preserves the result for
  manual reconciliation instead of silently retrying a possibly partial
  transfer. If termination cannot be confirmed, the affected execution lane
  remains held rather than racing a second writer against it.
- Destination collisions are now reconciled under the same path permit through
  physical placement, catalog ownership, artifact recording, and source
  cleanup. Byte-identical interrupted imports can be recovered; different
  content or foreign ownership returns a typed conflict and preserves both the
  source and destination instead of leaking a platform rename error.
- Import sources are classified as directories, regular files, archives,
  missing paths, or unsupported objects before enumeration. A direct video is
  never sent through the directory walker, eliminating the Windows “directory
  name is invalid” failure and its Linux “not a directory” counterpart.
- Import verification now uses the completed-download identity first, while
  retaining the tracked queue identity as a compatibility alias. Artifact
  evidence is normalized and persisted before destructive source cleanup. This
  fixes the class of failure where a successful move later looked like an empty
  source and was incorrectly marked *Import Blocked*.
- Manual import now qualifies video content more carefully, rejects empty and
  clearly invalid or inaccessible video candidates, preserves containment
  checks, and exposes useful media facts where they are available. Known video
  extensions remain a manual fallback when native probing cannot parse an
  unusual but potentially valid container. Archive imports are blocked with an
  actionable result when no archive extractor is available.
- Archive-backed manual imports receive durable workspaces and stronger retry
  behavior. Manual selections retain their durable download identity, avoiding
  lost association when queue and completed-history identifiers differ.
- Activity now includes live import streams and clearer import/history detail,
  making long placement, verification, and reconciliation work visible while it
  is happening.
- **Verified season, multi-season, and complete-series packs now retain every
  catalog-resolved member.** Monitoring controls what Scryer searches for; it
  does not discard an already-downloaded, safely identified episode merely
  because that episode is currently unmonitored. Members that cannot be
  identified safely remain held for Manual Import.

### Acquisition now converges a large wanted backlog in parallel

- **Wanted convergence now runs up to eight target work items concurrently.**
  This removes the former single-target head-of-line bottleneck without creating
  unbounded background tasks.
- Pack-first behavior is explicit: Scryer tries one title-wide or multi-season
  pack first, then eligible season packs, and only then releases the remaining
  individual episode work. A viable, grabbed, delayed, or ambiguous pack
  suppresses the scopes it covers for that cycle; a definitive miss lets the
  dependent searches proceed.
- Unrelated titles continue progressing while a slow pack search is in flight.
  Saved-candidate recovery, route failover, client submission, coverage rules,
  hot/cold selection, and the configured per-cycle scope limit retain their
  existing policy.
- **A completed search is now treated as evidence, not merely an empty result.**
  Indexer coverage is recorded only when the provider can attest to complete
  search semantics. Partial results can still supply releases, but they do not
  falsely mark a wanted scope converged and prevent a later retry.
- Search strategies can now stream candidates and resume provider pagination
  instead of requiring every page to finish before evaluation. Built-in
  Newznab support can select the appropriate RSS or search request form and use
  provider-profile behavior while retaining the generic Newznab path.
- A converged scope now keeps a ranked standby corpus when no release is
  grabbed. Temporary client outages, delay windows, and removable blocklist
  entries no longer discard the only candidates or leave the scope marked
  complete with nothing available to reconsider. Episode and season-pack
  searches also retain their own evidence rather than substituting one scope's
  results for the other.
- Search coverage is invalidated when the relevant indexer endpoint, secret,
  proxy, managed routing, capabilities, or declared search semantics change.
  Missing scopes are also rechecked on a slow 30-day cadence by default; set
  the long-tail re-converge setting to `0` only when an operator deliberately
  wants to opt out of that safety backstop.
- Background indexer capacity now scales with the configured indexer inventory
  instead of being constrained by one small fixed global request count. The
  downstream request gate remains bounded, so the extra orchestration
  concurrency does not turn into unlimited network pressure.
- Indexer requests now have a 120-second operation budget, with admission waits
  accounted for separately. Slow but healthy indexers therefore have time to
  answer without allowing a queue wait to consume their entire request window.
- **Operator-started searches always query providers live.** They no longer
  reuse a background candidate corpus or reject a viable pack solely because it
  contains an unmonitored episode; background convergence remains governed by
  its normal monitoring and reuse rules.
- Operators that need a different RSS cadence can now set
  `SCRYER_RSS_TARGET_INTERVAL_SECS`; the existing default behavior is unchanged.

### More transparent and isolated indexer behavior

- **Indexer error history is now stored and visible in Settings.** Operators can
  inspect recent HTTP failures, captured response diagnostics where available,
  and transport failures that do not have an HTTP response. The UI no longer
  presents a red indexer status with an empty diagnostic history for this class
  of error.
- Rate-limit cooldowns now use an indexer-specific rate-limit domain rather
  than coupling unrelated configured indexers that happen to share a host. A
  Prowlarr-managed child uses its configured child identity as well.
- Interactive search reports an indexer skipped for cooldown, including the
  remaining duration, rather than silently omitting it from the result.
- Per-indexer hard-failure backoff and real host-level request pacing remain in
  place. The changes narrow rate-limit coupling; they do not weaken provider
  protection.
- A failed capability refresh now makes the indexer unhealthy and retires its
  stale convergence coverage until it can be verified again. Newznab connection
  tests validate the returned capabilities document and specifically explain
  the common NZBGeek website-versus-API-host configuration mistake.

### Better download identity, queue recovery, and manual-import continuity

- Download tracking now uses a canonical identity across accepted submissions,
  queue observations, completed history, import artifacts, and manual-import
  work. This is especially important for clients that expose one identifier in
  the queue and another once completion is recorded.
- Every production acquisition lane now passes a complete, caller-created
  submission through one title-bound coordinator. Manual, RSS, convergence,
  pending, and forced grabs share the same admission, routing, persistence, and
  uncertainty rules, preventing two concurrent release choices from mutating a
  downloader for the same title before either can observe the other.
- Accepted grabs claim their resolved download-client binding immediately, so
  recovery and client routing do not lose the route that accepted the release.
- The original grab-time torrent infohash is persisted with the submission, and
  blocklist identity is keyed to the release rather than a mutable download
  URL. Retry and duplicate decisions therefore survive provider URL changes.
- Completion, history, artifact, and retry paths now carry compatibility aliases
  where older data used a previous identity shape. SQLite and PostgreSQL receive
  matching migration coverage.
- The completed-download pipeline has stronger state reconciliation and more
  precise activity-vs-import handling, reducing duplicate work and making
  recoverable outcomes visible rather than disappearing from the queue.
- Download history remains available when a client evicts its completed item,
  and queue recovery restores the configured client binding from the durable
  submission. A settled download no longer continues to conflict with the
  acquisition scope it has already satisfied.

### Quality profiles: Balanced favors sensible releases

- **Balanced no longer rewards progressively larger releases.** Its size curve
  now peaks around the expected file size and penalizes large, very large,
  massive, and excessive releases while retaining a hard veto for clearly
  impossible excessive sizes.
- For a two-hour 2160p movie, Balanced uses a 32 Mbps baseline: roughly 28.5
  GiB for an H.265 Blu-ray and roughly 38 GiB when the codec is unknown. Runtime
  scaling remains proportional for longer films.
- A non-preferred remux receives a substantial penalty and does not inflate the
  expected-size budget. This applies to 1080p as well as UHD. Selecting
  **Prefer Remux** explicitly restores a remux bonus, a small penalty for
  non-remux alternatives, and the existing extra remux size tolerance.
- Large remuxes remain eligible when they are the only viable result; Balanced
  ranks a sensible alternative much higher rather than turning this preference
  into a hard rejection. Audiophile, Efficient, and Compatible retain their
  existing remux and physical-size behavior.
- A built-in **Anime** profile now defaults to 1080p, 720p, and 576p, with a
  1080p archival quality and no implied remux preference. 576p is available in
  quality-tier settings and has an explicit size expectation. Existing
  system-owned defaults move the Anime facet to this profile automatically;
  customized profile catalogs and explicit user choices are left untouched.
- **Pack size scoring is coverage-aware.** A credible per-episode size reported
  for a season or complete pack can avoid an inappropriate small-file penalty,
  but that inferred reading can never add a size bonus. The same basis is used
  when choosing, importing, and later comparing an incumbent.
- Small releases are now ranked down by the size curve rather than rejected
  solely for being small. At import, an advertised size is retained only when
  it is reasonably close to the landed bytes; a material mismatch is scored on
  the bytes actually received.

### Playback and library experience

- **Title and episode surfaces can now offer “Watch in” links** for exact items
  found by a configured and linked Jellyfin, Plex, or Emby connection. Links are
  only offered for a verified provider item—Scryer does not perform a loose
  provider-side search when opening playback.
- Media-server scanning now retains the provider item identity needed for those
  links and improves scan, refresh, and catalog reconciliation behavior.
- Expanded series-movie rows now match the title overview more closely, with
  hydrated rating pills, compact external links, and original and dubbed cast
  rails. Ratings and credits are stored in the existing normalized metadata
  tables during hydration and read locally; opening the panel never triggers a
  live metadata-provider request.
- Rename templates can contain optional groups, allowing punctuation and
  surrounding text to appear only when every required token in that group has
  a value.
- Title history contains richer acquisition detail. Download and import activity
  views have been tightened around live import state, episode work, pending
  imports, and the actions appropriate to each state. Activity displays the raw
  release name submitted by the provider rather than a parser-normalized title.
- Torrents whose media import has completed but which remain retained for
  seeding now have an explicit **Imported · Seeding** activity state and
  filter, rather than looking indistinguishable from fully completed work.
- Fix Match uses the GraphQL facet data directly, avoiding stale client-side
  assumptions. The updated manual-import UI includes the completed download
  identity needed to resume work correctly.

### Security, accounts, and integrations

- Users can create, name, inspect, expire, and revoke their own API keys when
  the administrator allows it. Keys are represented as managed identities with
  scoped ownership rather than a single shared application secret.
- Internal, non-protocol identity digests now use domain-separated BLAKE3.
  Legacy v1 password hashes are retired; any account that still has one is
  marked as requiring a password change instead of remaining on the obsolete
  verifier.
- Administrators can manage registered OAuth clients and redirect URIs. The
  authorization and token flow now has durable client, grant, and refresh-token
  handling.
- Authless administrator setups can manage API keys without being blocked by an
  absent interactive session. Settings and profile credential flows also have
  clearer validation and state handling.
- External media-server account and connection work was tightened across Plex,
  Jellyfin, and Emby, including the identity and policy data needed for playback
  links and account-aware access.

### Platform, plugin, and developer improvements

- The plugin host is updated to Wasmtime 48 while retaining the compatibility
  gates needed for older supported plugin artifacts.
- Built-in indexers now run as WASI Preview 2 components through a typed WIT
  interface. The host retains its legacy module path for supported existing
  plugin artifacts while preferring verified component-form built-ins.
- The plugin SDK adds bounded batched HTTP requests and typed indexer search
  outcomes for partial, deferred/rate-limited, and invalid responses. Newer
  indexers can therefore fan out safely while telling the host whether a result
  set is complete; legacy plugins remain conservative and cannot attest an
  incomplete empty response as coverage.
- Component-based challenge solvers now share one deadline across solving and
  any clearance replay, preventing a retry from silently extending the
  operator-visible request budget.
- `cargo xtask serve` now verifies the catalog-pinned built-in plugin assets
  before startup and materializes missing or stale bytes locally. Development
  and CI therefore exercise the published artifacts rather than checked-in
  binaries.
- Normal runtime logs now default to one JSON object per line across stdout,
  rotating files, and the in-app stream. Request, actor, workflow, and resource
  context is attached centrally without requiring individual log calls to
  repeat it. The live viewer renders readable rows, supports full-object search
  and filtering, and opens syntax-highlighted pretty JSON for inspection and
  copying. Set `SCRYER_LOG_FORMAT=text` to retain the legacy text format.
- Persisted domain-event payloads and release-decision explanations use compact,
  dictionary-backed storage while remaining transparent to application reads,
  backups, and the scoring explanation UI.
- Windows packaging and CI received substantial hardening, including the
  portable archive format, MSI validation, long-path awareness, and more
  realistic Rust build time budgets. The former build-heartbeat watchdog has
  been removed.
- The web client, GraphQL schema, and generated query contracts were updated
  together for active-import streams, application upgrades, indexer errors,
  API keys, OAuth clients, playback links, durable manual-import data, and the
  revised queue/activity surfaces.

## Upgrade notes

- **No manual database conversion is required.** Startup applies the SQLite or
  PostgreSQL migrations for indexer-error history, OAuth client registrations,
  manual-import archive workspaces, normalized series-movie ratings and credits,
  API keys, canonical download identities, grab-time infohashes, media-server
  playback links, BLAKE3 identity backfills, compact event storage, and durable
  manual-import selection identity.
- The canonical-default migration preserves existing configured library roots;
  established library paths are not rewritten during upgrade.
- The BLAKE3 authorization-fingerprint change invalidates existing interactive
  sessions, so users will need to sign in again after the upgrade. Accounts
  still carrying the retired v1 password format require a password change.
- Operational log output changes to NDJSON by default. Existing text-oriented
  collectors can set `SCRYER_LOG_FORMAT=text`; no existing rotating log files
  need to be converted.
- Existing downloads and imports do not need to be removed or re-added. The
  canonical-identity compatibility aliases and artifact verification changes are
  specifically intended to reconcile pre-existing queue/history differences and
  previously moved sources safely.
- If a long import was interrupted, inspect the import record before manually
  retrying it. A *manual reconciliation required* result is intentional: it
  means Scryer could not safely prove that the prior worker stopped.
- In-app updates are available only for eligible installation types. Continue to
  update Docker, Homebrew, winget, supervised, and unsupported installations
  through their existing operator-owned method.
- Balanced users should review any custom minimum-score thresholds after
  upgrading. Scores for large releases—especially remuxes without **Prefer
  Remux**—will be lower by design. The behavior is ranking-first: a large
  release remains available if no better candidate exists.
- Plugin authors and generated GraphQL clients should regenerate against the
  included schema. The release adds application-update, API-key, OAuth,
  indexer-error, media-playback, active-import, and durable import/download
  identity surfaces.

## GraphQL and integration compatibility

- The GraphQL schema has expanded substantially. New optional fields, enum
  values, inputs, queries, mutations, and subscriptions support the features
  above; regenerate strongly typed clients and review exhaustive enum handling.
- Notable additions include application-upgrade status and start operations,
  user API-key operations, OAuth client registration and authorization data,
  indexer error-history data, active-import streams, exact media-server playback
  links, series-movie rating and locally loaded credit data, and
  download/manual-import identity fields.
- Queue clients that exhaustively match GraphQL enums must also handle
  `IMPORTED_SEEDING` and the `SEEDING` activity filter.
- Download-client and indexer plugin hosts receive the runtime, timeout, and
  compatibility improvements in this release. Existing supported plugins remain
  protected by capability/version gates where a newer host command is required.

## Reliability fixes included in this release

- Corrected manual-import GraphQL contract drift and retained known-extension
  parse failures as a manual-review path instead of treating every parser issue
  as a bad video file.
- Corrected absolute and alternate episode-number mapping for packs, including
  catalog-grounded anime numbering, and made explicit release evidence outrank
  context-inferred numbering. Spelled-out season numbers now bound the neutral
  title correctly.
- Subtitles-only releases are rejected before submission, while downloader
  warnings that do not prove a torrent failed no longer auto-fail the grab.
- Interactive search now reports incomplete provider results, Newznab RSS
  requests select the provider's advertised function, and corrupt legacy
  Newznab configuration is skipped instead of preventing startup.
- Queued manual imports remain visible until a worker actually starts them, and
  grab/import notifications link directly to the affected activity.
- Preserved PostgreSQL baseline seed parity with SQLite migrations.
- Improved startup, local development, and schema synchronization behavior,
  including a cleaner `xtask serve` bootstrap path.
- Fixed several table, sort, routing, Watch In, activity, history, profile, and
  credential UI edge cases discovered during the release stabilization work.
