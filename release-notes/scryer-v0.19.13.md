# Scryer 0.19.13 release notes

## Highlights

- Indexer request accounting now reflects the requests Scryer actually sends. Searches, RSS syncs, retries, extra result pages, capability refreshes, Prowlarr child calls, challenge-solver traffic and download-artifact fetches all count against the correct indexer, and the total survives restarts on both SQLite and PostgreSQL. The System page and Prometheus metrics now share the same send-level accounting instead of reporting optimistic quota headroom.
- Download grabs now belong to Scryer from the moment an indexer resolves them. NZB payloads are streamed into bounded temporary storage, checked for valid XML and the expected category, compressed, and retained until the download client has accepted them. This closes races around expiring URLs and temporary files, preserves the original indexer and source history, and prevents malformed, oversized or mismatched payloads from reaching the client.
- Daily-quota failures returned inside a successful Newznab HTTP response are recognized as rate limits. Indexers that report an exhausted request or download allotment now enter the escalating system backoff instead of being retried about once a minute, while ordinary HTTP 429 responses continue to follow their own `Retry-After` timing.
- Anime search and import now carry community numbering through the complete manual and automatic paths. Interactive searches ask for community season forms as well as the official numbering; differently romanized cour titles can anchor safely when there is a single clear match; exact cour titles can admit and translate bounded episode lists and complete cour packs; and translated releases are evaluated against their official episode coordinates. Ambiguous or incomplete pack mappings remain held for review rather than being widened or guessed.
- The release parser recognizes fused episode labels such as `Ep03`, `S01EP04` and `Episode07` for series and anime, without treating episode-shaped words in movie titles as numbering. It also keeps leading bracketed release-group tags out of title matching, preserves bounded pack scope during recovery, and recognizes disc source metadata immediately after a bare season token.
- Episode search results now identify releases that cover more than the wanted episode. The pack badge uses the backend's resolved queue scope, so the size and coverage shown in the row agree with what Scryer will actually queue.
- Per-indexer routing is more predictable. RSS categories remain attached to the indexer they were configured for, library overrides inherit and exclude categories correctly, and Prowlarr generic-query searches retain their routed categories. The category picker also stays open, keeps its scroll position, and saves once per checkbox change.
- Indexer connection failures now show the actionable message returned by the indexer or transport instead of a generic internal-server error. Bad API keys, unreachable hosts, TLS or DNS failures, rate limits and provider-specific errors can be diagnosed directly from the settings page.
- Discovery gateway feed requests now use instance authentication and stay within the gateway's documented query-shape limits.

## Included fixes

- Manual Import uses the selected title, aliases, catalog episodes and grabbed pack scope when parsing each member. The dialog is wider and long filenames wrap so the full name remains visible.
- A plugin installation whose progress subscription ends before its final snapshot no longer leaves the plugin permanently showing "Installing". Scryer clears the stale operation and reloads the catalog to show the actual result.
- Concurrent image-cache writes no longer exceed the configured cache budget, PostgreSQL reports cache usage correctly, and otherwise complete JPEG or PNG images with trailing bytes are accepted.
- Movie files are attached before their pending-import records are consumed, preventing a completed import from losing its title attachment if the final persistence step fails.
- The web client recovers missed invalidations after an event-stream reconnect that cannot resume from a known event, so views refresh instead of remaining stale.
- Parser coverage now includes a reviewed real-world release corpus with exact regression gates. Recovery no longer turns unresolved season attachments or empty context into confident metadata, and acquisition rejects incomplete or out-of-scope parser coverage rather than silently shrinking it.
- Prowlarr capability fan-out is tested by observed concurrency rather than wall-clock timing, removing a load-sensitive test failure without weakening the concurrency guarantee.

## Upgrading

A startup migration establishes a clean epoch for the corrected indexer request counter. It resets only Scryer's locally observed requests-today values, removes synthetic connection-test rows, and preserves the quota values reported by each provider. Browser-solver proxy assignments are removed from Prowlarr and its managed child indexers because artifact resolution is now owned by Scryer and Prowlarr solver routing is not supported. No manual database action is required.
