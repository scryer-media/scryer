# Scryer 0.21.12 release notes

These notes cover what's changed since **0.21.11**.

## Highlights

- **Downloads with several or nested archives import as one complete result.** Scryer now finds archive sets in download subdirectories, extracts every set rather than stopping after the first, and continues into archives contained inside other archives. Split 7z and ZIP sets are recognized from their first volume, configured archive passwords are tried when extraction asks for one, and an otherwise useful download is no longer rejected just because one archive set fails or the only loose video is a sample.

- **RSS polling catches up without repeatedly reporting the same gap.** Indexer plugins now receive Scryer's stored RSS marker and can page backward toward it. Scryer records a gap only when the available pages genuinely fail to reach that marker, which makes gap reporting more accurate when an indexer needs more than one page to cover the time since the previous poll.

- **Background searching revisits items when they become actionable.** Search coverage recorded before an episode airs no longer prevents another search after release, and refreshed episode metadata now picks up newly announced or changed air dates. When a library scan confirms that a monitored movie or episode file has disappeared, Scryer clears its stale coverage and wakes acquisition so the missing item can be found again. Background sweeps can also use all eligible indexers concurrently while each destination still keeps its own pacing and cooldown.

- **List syncing is safer and more predictable.** A sync requested while another run is active is retained, disabled or unfollowed lists stop acting promptly, and each successful add or request is recorded as it happens. Filters and exclusions on one list no longer prevent another list from applying its leave action. Personal-list requests are held for review when policy requires approval, duplicate public follows are rejected atomically, and failed verdict actions keep the original request identity for a clean retry.

- **Administrator bootstrap and sign-in behavior are configurable.** Operators can configure the initial administrator and enforce a disabled-administrator policy without losing bootstrap settings. The settings UI explains why form login cannot be disabled while the default administrator remains active, and alternate sign-in methods are presented beneath the active form.

- **Traditional Chinese is available throughout the web app.** New Hong Kong and Taiwan locale dictionaries add `zh-HK` and `zh-TW` interface choices.

## Included fixes

- **Interactive search:** a whole-season search lists only releases that actually cover multiple episodes. Season parts and multi-season packs remain distinct from a full-season pack, and starting a season search opens the season being searched.
- **Activity:** failed imports offer a retry action directly from activity history. Background queue refreshes no longer insert and remove a loading line that made the table jump on every poll.
- **Grabbing (PostgreSQL):** accepting a grab and observing the same download at the same time no longer deadlock or fail with a binding-key conflict. Both paths now lock and write the binding before the download row, accept an identical existing binding, and report a genuinely different binding as a named conflict. Grab-submission logs also identify the exact stage being awaited when a queue operation stalls.
- **Requests:** an automatically approved request now starts its wanted search even when the requester has request permission but not manage-titles permission. A request that leaves monitoring at its default no longer narrows a series to future episodes only.
- **Background searching:** disabled indexers and indexers with automatic search turned off no longer keep scopes permanently pending. Indexers behind the same Prowlarr or Hydra host keep independent scheduler cooldowns, and a scope deferred behind an active library scan is logged with its consecutive deferral count.
- **Indexer settings:** standalone indexers expose both the minimum interval between queries and the per-minute query budget in the web editor. Operators can also control enabled state, automatic and interactive search, interval, budget and burst locally for Prowlarr-managed children without those choices being overwritten by the next parent sync.
- **Lists:** titles known only by TMDB id can be added or requested from list rows. Select-valued provider settings survive an edit, exclusions match the correct external-id kind, fetched item counts and text are bounded across pages, and run summaries report only work performed by that run.
- **Lists and permissions:** a hidden **Manage Lists** grant is preserved while unrelated app permissions are edited. List source URLs and parameters are redacted for viewers who cannot manage lists, including URL user information and credential-like query parameters.
- **Monitoring upgrades:** legacy migration monitoring snapshots are applied at most once and then discarded, preventing stale state from replaying on later starts.
- **Recycle bin and restore:** recycled, restored and relocated files use the verified copier. A custom recycle-bin path that reaches a library root through a symbolic link is refused, and restoring over an existing destination is handled through the same verified path.
- **Logs:** retention applies only to archives created by Scryer's own rotation, leaving unrelated files alone.
- **Notifications:** recycle-bin purges emit notifications, rename events identify the surviving path, post-processing severity follows the script result, and upgrade-origin metadata is retained without incorrectly attaching media servers.
- **Indexer pacing:** cancelling a reserved pacing slot returns its interval spacing instead of unnecessarily delaying later requests.
- **Quality profiles:** action buttons on each quality-tier row have consistent spacing.
- **Title deletion:** deleting a single series is queued through the existing title-job path.

## API and plugin changes

These affect scripts, integrations and custom plugins. The Scryer web app and bundled plugins are already updated.

- **Indexer plugin SDK 3.13.0:** search requests can include an RSS catch-up marker. Indexer plugins may use it to page toward the last item Scryer stored while remaining compatible with ordinary searches that have no marker.
- **Notifications:** external ids now carry their entity kind, allowing notification consumers to distinguish ids belonging to different media entities.
- **Lists:** list-policy and synchronization responses retain request identity and report per-run actions more precisely. Consumers should use the external-id kind when comparing exclusions or library matches.

## Upgrading

Scryer applies three database migrations automatically on startup for legacy monitoring snapshots, list-exclusion external-id kinds and pre-air episode search coverage, on both SQLite and PostgreSQL.

The legacy monitoring snapshot migration intentionally discards obsolete snapshots after upgrade so they cannot be replayed. Current title, collection and episode monitoring state remains authoritative.

Search coverage recorded on or before an episode's air date is reopened during the upgrade so released episodes can enter the background search walk.

Backups must be restored with the same Scryer version that created them. Create a fresh backup after upgrading for use with this version.
