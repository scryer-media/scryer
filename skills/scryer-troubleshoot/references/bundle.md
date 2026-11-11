# Diagnostics bundle contents

`GET $SCRYER_URL/api/diagnostics/bundle` returns a zstd-compressed tar
(`scryer-diagnostics-<version>-<YYYYMMDDTHHMMSSZ>.tar.zst`, format version 1).
It needs the **Manage system settings** permission. Scryer builds it in
memory and writes nothing to disk. The compressed size cap is 8 MiB.

Refusals are JSON `{"error": "<sentence>", "reason": "<code>"}`:

| HTTP | reason | meaning |
|---|---|---|
| 401 | `HTTP_DIAGNOSTICS_AUTHENTICATION_REQUIRED` | key, session or authless access not accepted; a step-up or enrollment-only session also lands here |
| 403 | `HTTP_DIAGNOSTICS_PERMISSION_DENIED` | the user lacks Manage system settings |
| 500 | `HTTP_DIAGNOSTICS_BUNDLE_FAILED` | Scryer failed to assemble the bundle |

The same button is in the web UI under **System > Service Logs**.

## Members

| member | always there? | read it how |
|---|---|---|
| `manifest.json` | yes | read whole (small) |
| `summary.json` | yes | jq extracts only |
| `environment.json` | yes | jq extracts only |
| `logs/buffer.log` | unless sanitizing failed or known values were incomplete | `log-search.sh` |
| `logs/file.log` | only when file logging is configured (`SCRYER_LOG_FILE`) | `log-search.sh` |
| `metrics.prom` | only when metrics are enabled (`SCRYER_METRICS`); first to go at the size cap | grep named families |

Never read a log or `metrics.prom` whole.

### manifest.json

`format_version`, `generated_at`, `scryer_version`,
`compressed_size_cap_bytes`, `files[{name, bytes}]`,
`omissions[{item, reason}]`, `truncations[{item, reason, dropped_lines}]`,
`sanitization`.

`sanitization`: `best_effort` (always true), `passes` (in order:
structured_field_names, secret_patterns, urls, emails,
paths_keep_file_name, known_values, host_port, bare_hosts, ip_addresses),
`path_policy` (`keep_file_name` or `redact_all`), `known_values` (a count of
configured values replaced), `short_values_not_replaced`,
`vocabulary_values_not_replaced`.

Omission reasons you may see:

- `file_logging_not_configured`, `metrics_disabled`, `size_cap`
- `sanitizer_failed`
- `known_values_incomplete:<sources>`: Scryer could not read every configured
  value it needs to scrub, so it drops `logs/buffer.log`, `logs/file.log` and
  `metrics.prom` together. Note this in any bug report; the operator can
  paste filtered log lines from the UI instead.
- `read_failed:<not_found|validation|temporarily_unavailable|error>` on a
  `summary.*` section
- `no_instance_wide_count` (`summary.wanted_items`; always omitted)
- `not_portable_on_this_platform`, `not_portable_on_windows`,
  `not_embedded_in_build` (`build.commit`), `not_exposed_by_build`
  (`wasmtime_version`), `systemd_exec_pid_not_set`, `not_sqlite`,
  `probe_task_failed`
- file-log sources: `not_found`, `permission_denied`, `invalid_data`,
  `io_error`, `read_task_failed`

Truncation reasons: `read_cap` (file log read stops at 24 MiB across the
active file and the two newest rotated `.gz` archives) and `size_cap`
(oldest file-log lines go first, then buffer lines).

### Logs

`logs/buffer.log` is the in-memory ring of the last 1000 lines.
With the default `SCRYER_LOG_FORMAT=json` each line is
`{"timestamp", "level", "target", "fields": {"message", ...}, "context"}`;
`level` is uppercase (`INFO`, `WARN`, `ERROR`). The text format uses local
time. Live lines are also available through the `serviceLogs(limit)` query
(1 to 2000 lines), but that output is not sanitized; prefer the bundle.

### summary.json sections

Counts and tallies only; names appear as pseudonyms such as
`indexer-1(newznab)` or `quality-profile-2`.

| section | keys |
|---|---|
| `quality_profiles` | total, global_scoring_persona, profiles[{pseudonym, global_default, criteria}] |
| `indexers` | total, by_provider_type{type:{enabled,disabled}}, interactive_search_enabled, auto_search_enabled, with_proxy, health_status, last_error_codes |
| `download_clients` | total, by_client_type, status, with_remote_path_mappings |
| `libraries` | total, by_facet, root_folders, root_folders_by_facet |
| `rule_sets` | user{total,enabled,by_phase}, managed{total,enabled,disabled_reason_codes}, packs[...] |
| `delay_profiles` | total, enabled |
| `seeding_profiles` | total |
| `notification_channels`, `subtitle_providers` | total, by_type, or {available:false} |
| `media_servers` | total, by_provider, with_path_mappings |
| `proxies` | total, by_provider_type |
| `lists` | total, enabled, by_scope |
| `plugins` | [{plugin_id, version, plugin_type, provider_type, builtin, enabled, sdk_version, support_tier}] |
| `health_checks` | [{kind, severity}] |
| `job_runs` | sampled (last 500), by_job{key:{statuses, latest_status, latest_started_at, latest_trigger}} |
| `titles` | total, monitored, movie, series, anime |
| `download_queue` | items, by_state, by_client_type |
| `release_decisions` | total_rows, sampled (up to 500), by_code |
| `indexer_errors` | sampled (up to 500), by_classification, by_operation |
| `users` | total, by_account_kind |

### environment.json sections

| section | keys |
|---|---|
| `build` | version, profile, build_lane, target_os, target_arch, target_family |
| `installation` | kind, upgrade_owner, self_upgrade_eligible, self_upgrade_reason, tray_supervised, package, self_upgrade_disabled_by_environment |
| `container` | docker_env_file, podman_env_file, container_variable, kubernetes, cgroup_hints |
| `launcher` | systemd_unit, launchd_job, windows_tray_supervised |
| `os` | family, path_style, cpu_count, name, version, kernel, total_memory_bytes |
| `process` | started_at, uptime_seconds, peak_resident_bytes |
| `datastore_files` | database_bytes, wal_bytes, shared_memory_bytes |
| `network` | bind_port, bind_address_kind, tls_enabled, base_path_configured, public_url_from_environment, cors_origins_configured |
| `auth` | mode, form_login_enabled, authless_local_access, passkey_enabled, webauthn_configured, environment_override_active |
| `metrics` | enabled, renderable |
| `logging` | file_logging_enabled, log_filter |
| `time_and_locale` | utc_offset_seconds, locale |
| `scryer_variables` | every `SCRYER_*` name; values shown only for SCRYER_LOG_FORMAT, SCRYER_METRICS, SCRYER_AUTH_ENABLED, SCRYER_PACKAGE, else `<set>` |
| `datastore` | engine, current_migration_key |
| `setup_complete` | bool |
| `smg_compatibility_notice` | present, status, minimum_version, your_version, has_upgrade_deadline |
| `settings.general` | api_explorer_enabled, experimental_features_enabled, keep_history_forever, history_retention_days, image_cache_max_size_mb, plugin_http_ca_bundle_configured, ... |
| `settings.acquisition` | enabled, poll_interval_seconds, walk_interval_seconds, walk_intensity, long_tail_backfill_max_scopes_per_cycle, long_tail_reconverge_days; internal, evidence only: never suggest changing them, route an abnormal value to `scryer-report-bug` |
| `settings.recycle_bin` | enabled, custom_path_configured, retention_days, effective_paths, has_validation_error |
| `network_configuration` | public_url_configured, trusted_proxies_configured, trusted_proxy_count |
| `filesystems` | roots[{root, role, exists, filesystem_type, total_bytes, available_bytes, device_group}], same_filesystem_pairs |

`filesystems.roots[].role` is `library_root` or `download_root`; a pair in
`same_filesystem_pairs` means hardlinks between those roots are possible.

## Which sections matter per symptom

| symptom | first reads |
|---|---|
| nothing downloads | `settings.acquisition`, `summary.indexers`, `summary.download_clients`, `summary.release_decisions.by_code`, `summary.indexer_errors`, `summary.job_runs.by_job` (`rss_sync`, `acquisition_search`, `pending_release_processing`) |
| downloads do not import | `summary.download_queue`, `summary.download_clients.with_remote_path_mappings`, `filesystems`, `container`, logs matching `import` |
| wrong release chosen | `summary.quality_profiles`, `summary.rule_sets`, `summary.delay_profiles`, `summary.release_decisions.by_code` |
| search finds nothing | `summary.indexers`, `summary.indexer_errors`, metric `scryer_indexer_errors_total` |
| UI or login | `auth`, `network`, `network_configuration`, logs matching `auth|session|login|cors` |
| performance | `os`, `process`, `datastore_files`, `datastore`, `summary.titles`, metrics `scryer_http_request_duration_seconds`, `scryer_datastore_writer_gate_wait_seconds` |
| upgrade | `build`, `installation`, `smg_compatibility_notice`, `summary.job_runs.by_job.application_upgrade` |

## Extract recipes

```sh
B=<bundle-dir>
jq '{v:.build.version, kind:.installation, os:.os|{family,name,version}}' "$B/environment.json"
jq '.settings.acquisition' "$B/environment.json"
jq '.indexers, .download_clients' "$B/summary.json"
jq '.release_decisions.by_code | to_entries | sort_by(-.value) | .[0:10]' "$B/summary.json"
jq '.job_runs.by_job | with_entries(.value |= {latest_status, latest_started_at})' "$B/summary.json"
jq '.filesystems' "$B/environment.json"
grep -E '^(scryer_indexer_errors_total|scryer_download_client_up|scryer_health_check_status)' "$B/metrics.prom" | head -n 40
```

Metric families present in builds with metrics on:
`scryer_indexer_errors_total`, `scryer_indexer_backoff_active`,
`scryer_indexer_queries_total`, `scryer_download_client_up`,
`scryer_health_check_status` (labels `source`, `status`),
`scryer_import_rejections_total`, `scryer_job_runs_total`,
`scryer_grabs_total`, `scryer_datastore_writer_gate_wait_seconds`,
`scryer_http_request_duration_seconds`,
`scryer_background_worker_errors_total`, `scryer_build_info`.
