# Scryer observability examples

Ready-to-import monitoring for a Scryer instance:

| File | What it is |
|---|---|
| `scryer-grafana-dashboard.json` | Grafana dashboard: overview stats, serving, acquisition, indexers, downloads, imports, jobs and health, plus collapsed rows for outbound rate limiting and catalog activity. |
| `scryer-alerts.yml` | Eleven Prometheus alerting rules covering the failures that silently reduce what Scryer does: backed-off indexers, unreachable download clients, stale queue snapshots, overdue jobs, low disk, import rejection spikes. |
| `prometheus.yml` | A minimal scrape configuration with the API-key authorization Scryer requires. |

The full metric reference, label semantics and upgrade notes live in
[`scryer-docs/deployment/prometheus-metrics.md`](https://github.com/scryer-media/scryer-docs/blob/main/deployment/prometheus-metrics.md).

## 1. Turn metrics on and create a scrape key

1. Set `SCRYER_METRICS=1` in Scryer's environment and restart it. Metrics are off
   by default; without the variable `/metrics` does not exist.
2. In Scryer open **Settings → Security → API keys** as a user who has the
   **Manage System Settings** permission, and create a key labelled for
   Prometheus. Only keys owned by such a user can read `/metrics`: a browser
   session, a key belonging to an ordinary user, or an anonymous request are all
   refused, even when login is disabled.
3. Write the key to a file only Prometheus can read, for example
   `/etc/prometheus/secrets/scryer_api_key`, and reference it with
   `credentials_file` as `prometheus.yml` does.

Check it from the Prometheus host:

```bash
curl -fsS -H "Authorization: Bearer $(cat /etc/prometheus/secrets/scryer_api_key)" http://scryer:8080/metrics | head
```

A `401` means the key was not recognised. A `403` means the key works but its
owner lacks the permission.

## 2. Load the alert rules

Copy `scryer-alerts.yml` next to your Prometheus configuration and list it under
`rule_files` (the sample `prometheus.yml` already does). Validate before
reloading:

```bash
promtool check rules scryer-alerts.yml
```

The thresholds are starting points. The two worth tuning first are
`ScryerRootFolderLowDiskSpace` (free-space fraction) and `ScryerJobOverdue`
(how long a scheduled job may go without succeeding).

## 3. Import the dashboard

In Grafana: **Dashboards → New → Import → Upload dashboard JSON file**, choose
`scryer-grafana-dashboard.json`, pick your Prometheus data source when prompted,
and import. Grafana 10.4 or newer.

The dashboard has three variables at the top:

- **Instance** — the `instance` label of the scrape target. Select one instance
  for the *Health checks* timeline; every other panel aggregates correctly
  across several.
- **Indexer** and **Download client** — narrow the indexer and download rows to
  the entities you are investigating.

Rows, top to bottom:

1. **Overview** — version, uptime, clients down, indexers in backoff, health
   errors, queue depth, grabs and imports in the last 24 hours. If everything
   here is green, Scryer is working.
2. **Serving** — request rate by status class, latency percentiles, slowest
   routes, GraphQL operations and errors, in-flight requests and live WebSocket
   connections.
3. **Acquisition** — grabs by trigger, every submission outcome, why candidates
   were rejected, search volume, and the RSS funnel (fetched → matched →
   grabbed).
4. **Indexers** — query rate and status per indexer, a table of indexers
   currently in backoff with when they return, latency, skips by reason, error
   classes, and upstream 429s.
5. **Downloads** — queue depth by state, a reachability timeline per download
   client, snapshot age, read errors and latency per client, failed downloads.
6. **Imports and library** — imports, upgrades and rejections, rejection
   reasons, bytes moved, files deleted by reason, library scans, import-lane
   waits.
7. **Jobs, tasks and health** — a health-check timeline, a scheduled-jobs table
   (time since last success, next run, failures in 24h), root-folder free space,
   task durations, task errors and panics, worker errors.
8. **Outbound HTTP and rate limits** (collapsed) — destinations in cooldown,
   waits caused by rate limiting, transport retries.
9. **Catalog, metadata and events** (collapsed) — domain event volume, metadata
   hydration, the hydration backlog and wanted projection, subtitles.

Two annotation streams are wired in: a restart marker driven by
`scryer_process_start_time_seconds`, and a marker whenever an indexer enters
backoff.

## Notes

- Every `*_seconds` family is a real histogram with explicit buckets, so the
  percentile panels are exact and survive infrequent observations.
- Label sets idle for 24 hours are evicted. A series that disappears is usually
  an indexer or client you removed, not an outage; the alert rules avoid
  `absent()` for that reason.
- `scryer_download_client_refresh_duration_seconds` changed meaning in 0.20
  (per-client read, with a `client` label). The whole poll cycle is now
  `scryer_download_queue_poll_cycle_duration_seconds`.
