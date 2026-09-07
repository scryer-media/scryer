# Indexer correctness: release-NEXT integration

Implementation target: `release-0.19.13`, based on `dd37b3c2d`.
Compatibility reference: `release-NEXT` at `81e8de958`.
This note describes the required later adaptation; it does not claim that NEXT
has been changed or validated by the 0.19.13 tests.

## Artifact ownership

Indexer artifact resolution belongs before download-client submission. The
resolver must also be callable without a selected download client or a catalog
title. Use host-owned artifact HTTP with the assigned solver, request accounting,
and authenticated download URL supplied by search. Artifact resolution must not
invoke a plugin action or depend on a new plugin contract.

When integrating into NEXT:

- Route `queue_unlinked_release` through the artifact resolver before its
  direct submission call. Preserve the operator's pinned download client,
  category selection, authorization, and unlinked-download tracking.
- Replace the browser-download call to `DownloadClient::fetch_release_artifact`
  with the indexer resolver. Preserve actor-owned search-result lookup, full
  session requirements, filename/content-type handling, multi-file packaging,
  payload limits, and rejection of magnets as downloadable files. Browser
  downloads must not create a download-client submission or canonical job ID.
- Keep staged NZB ownership alive through submission and client failover.
  Borrow a staged reference for submission rather than fetching the URL again.
- Retain download-client API proxying. Moving indexer artifact HTTP out of the
  router must not remove proxying for submit/status/queue/history operations.

## Proxy applicability

0.19.13 has solver-only `IndexerProxyConfig` assignments. NEXT replaces these
with `ProxyConfig`, `proxy_config_id`, and the derived `ProxyKind` families.

The Prowlarr exclusion applies to `ChallengeSolver` (Byparr/Trawl), not to
`Transport` or `Tunnel`. Permit parent-selected HTTP/SOCKS/SSH/WireGuard egress
and preserve the existing parent-to-child assignment inheritance. Cleanup must
select solver provider types, never clear every Prowlarr proxy assignment.

Use NEXT's existing proxy factories, encrypted credentials, DNS policy, tunnel
lifecycle, and health reporting. Missing or disabled assigned transports must
fail without falling back to direct egress. Apply the assignment to parent
management requests as well as child searches and grabs: the reference NEXT
management constructor does not receive a resolved proxy configuration.

## Accounting and runtime compatibility

Count Scryer HTTP dispatch attempts after local admission and before sending.
A challenge-solver POST on an indexer's behalf counts as one request to that
indexer. Do not count proxy handshakes or Prowlarr's internal work as indexer
requests. Child-endpoint requests, parent management requests, and
external artifact redirects remain distinguishable. Saved connection tests use
the saved accounting identity; unsaved tests do not create persistent rows.

NEXT's reference tracker predates the dispatch-accounting and UTC-day fixes.
Do not replace corrected persistence with its older quota-observation counter.
Preserve the startup accounting marker across integration so existing corrected
counts are not reset again. Keep provider-reported quota state independent.

Port shared accounting and structured quota classification to NEXT's component
host. Do not restore the legacy runtime removed there. Preserve per-operation
capture ownership, host artifact accounting, explicit retry deadlines, and
shutdown dispatch closure followed by an awaited persistence drain. Adapt the
existing metric collectors without adding a second increment for each request.

## Required integration verification

- Real component concurrent searches and host artifact requests: exact dispatch totals,
  independent error capture, and Newznab 500/501 signals reaching health.
- Prowlarr parent/child search, caps, management, and artifact calls through
  inherited transports; no solver calls; no direct fallback on tunnel failure.
- Unlinked grabs and browser downloads through the resolver, including
  authenticated download URLs, solver sessions, redirects, and magnets.
- One artifact fetch across client failover, with both indexer and
  download-client proxy assignments configured and independently observed.
- SQLite/PostgreSQL startup-marker preservation, daily counts, retrying flushes,
  and shutdown with an admitted dispatch racing gate closure.

The branches already have different migrations using the same version numbers.
Do not resolve those unrelated histories by overwriting migration contents or
silently accepting checksum changes as part of this indexer adaptation.
