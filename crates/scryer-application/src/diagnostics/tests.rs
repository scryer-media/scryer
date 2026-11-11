//! Sanitizer unit tests and the end-to-end canary test. Every fixture value
//! is synthetic.

use std::io::Read;

use chrono::{TimeZone, Utc};
use serde_json::{Value, json};

use super::sanitize::{FieldClass, KnownValues, Sanitizer, classify_field};
use super::*;

fn patterns() -> Sanitizer {
    Sanitizer::patterns_only()
}

// ---- scrubbers -------------------------------------------------------------

#[test]
fn urls_drop_host_userinfo_path_and_query_but_keep_scheme_and_port() {
    let s = patterns();
    let out = s.text("fetching https://user:hunter2@feed.example.net:8443/api?t=search&q=x now");
    assert_eq!(out, "fetching https://<host>:8443/<…> now");
    assert_eq!(
        s.text("see http://localhost:7979/api"),
        "see http://localhost:7979/<…>"
    );
    assert_eq!(
        s.text("bound to http://127.0.0.1:7979"),
        "bound to http://127.0.0.1:7979"
    );
}

#[test]
fn known_hosts_keep_their_pseudonym_inside_urls() {
    let mut known = KnownValues::new();
    let pseudonym = known.entity("indexer", Some("newznab"), "idx-1");
    known.name(&pseudonym, "Synthetic Feed");
    known.url(&pseudonym, "https://feed.synthetic.invalid/api");
    let s = known.into_sanitizer();
    assert_eq!(
        s.text("Synthetic Feed failed: https://feed.synthetic.invalid/api?apikey=abc"),
        "<indexer-1(newznab)> failed: https://<indexer-1(newznab)>/<…>apikey=<redacted>"
    );
    assert_eq!(
        s.text("dns lookup feed.synthetic.invalid timed out"),
        "dns lookup <indexer-1(newznab)> timed out"
    );
}

#[test]
fn credential_parameters_and_headers_are_redacted() {
    let s = patterns();
    assert_eq!(
        s.text("apikey=abcdef123456&t=caps"),
        "apikey=<redacted>&t=caps"
    );
    assert_eq!(s.text("api_key: \"abcdef\""), "api_key: \"<redacted>\"");
    assert_eq!(
        s.text("password=swordfish next"),
        "password=<redacted> next"
    );
    assert_eq!(
        s.text("Authorization: Basic dXNlcjpwYXNz"),
        "Authorization: <redacted>"
    );
    assert_eq!(s.text("cookie: sid=abc; theme=dark"), "cookie: <redacted>");
    assert_eq!(
        s.text("X-Api-Key: 0123456789abcdef"),
        "X-Api-Key: <redacted>"
    );
    assert_eq!(s.text("sent Bearer abcdefghijkl"), "sent Bearer <redacted>");
    assert_eq!(
        s.text("key ska_AbCdEf0123456789 used"),
        "key <scryer-api-key> used"
    );
    assert_eq!(
        s.text("jwt eyJhbGciOi.eyJzdWIiOiIx.c2lnbmF0dXJl ok"),
        "jwt <jwt> ok"
    );
    assert_eq!(
        s.text("added magnet:?xt=urn:btih:abc&dn=x"),
        "added magnet:<redacted>"
    );
}

#[test]
fn ip_addresses_are_replaced_but_lookalikes_survive() {
    let s = patterns();
    assert_eq!(s.text("peer 203.0.113.9 connected"), "peer <ip> connected");
    assert_eq!(
        s.text("peer 203.0.113.9:6881 connected"),
        "peer <ip>:6881 connected"
    );
    assert_eq!(s.text("peer 2001:db8::1 connected"), "peer <ip> connected");
    assert_eq!(
        s.text("loopback 127.0.0.1 and ::1 stay"),
        "loopback 127.0.0.1 and ::1 stay"
    );
    // Near misses.
    assert_eq!(s.text("version 0.22.3 build"), "version 0.22.3 build");
    assert_eq!(s.text("plugin v1.2.3.4 loaded"), "plugin v1.2.3.4 loaded");
    assert_eq!(s.text("dotted 1.2.3.4.5 id"), "dotted 1.2.3.4.5 id");
    assert_eq!(s.text("ratio 1.50 reached 2:1"), "ratio 1.50 reached 2:1");
    assert_eq!(
        s.text("at 2026-11-10T12:30:45.123Z took 00:01:02"),
        "at 2026-11-10T12:30:45.123Z took 00:01:02"
    );
    assert_eq!(s.text("hash deadbeefcafe"), "hash deadbeefcafe");
}

#[test]
fn emails_and_bare_hosts_are_replaced() {
    let s = patterns();
    assert_eq!(
        s.text("notify someone.else@example.org now"),
        "notify <email> now"
    );
    assert_eq!(s.text("resolving tracker.example.com"), "resolving <host>");
    assert_eq!(
        s.text("upstream relay.example.net:563"),
        "upstream <host>:563"
    );
    // A file name is not a host.
    assert_eq!(s.text("wrote main.rs:120"), "wrote main.rs:120");
    assert_eq!(
        s.text("found Some.Show.S01E01.1080p.WEB.mkv"),
        "found Some.Show.S01E01.1080p.WEB.mkv"
    );
}

#[test]
fn paths_lose_their_directories_but_keep_file_names() {
    let s = patterns();
    assert_eq!(
        s.text("importing /srv/media/incoming/Synthetic.Release.2020.1080p.mkv done"),
        "importing <path>/Synthetic.Release.2020.1080p.mkv done"
    );
    assert_eq!(
        s.text("scanning /srv/media/movies"),
        "scanning <path>/<dir>"
    );
    assert_eq!(
        s.text(r"moved C:\Users\someone\Downloads\Synthetic.Release.mkv"),
        "moved <path>/Synthetic.Release.mkv"
    );
    assert_eq!(
        s.text(r"share \\nas-box\media\Synthetic.Release.mkv"),
        "share <path>/Synthetic.Release.mkv"
    );
    assert_eq!(
        s.text("path=/srv/My Media/Synthetic.Release.mkv size=10"),
        "path=<path>/Synthetic.Release.mkv size=10"
    );
    assert_eq!(
        s.text("/remote/downloads => /local/downloads"),
        "<path>/<dir> => <path>/<dir>"
    );
    // Scryer routes are request paths, not files.
    assert_eq!(s.text("POST /graphql 200"), "POST /graphql 200");
    assert_eq!(
        s.text("GET /api/diagnostics/bundle"),
        "GET /api/diagnostics/bundle"
    );
    // Ratios and fractions are not paths.
    assert_eq!(s.text("progress 3/4 done"), "progress 3/4 done");
}

#[test]
fn known_roots_keep_their_pseudonym() {
    let mut known = KnownValues::new();
    let root = known.entity("root", None, "root-a");
    known.path(&root, "/srv/synthetic-media/movies");
    let s = known.into_sanitizer();
    assert_eq!(
        s.text("imported /srv/synthetic-media/movies/Feature (2020)/Feature.2020.mkv"),
        "imported <root-1>/<…>/Feature.2020.mkv"
    );
    assert_eq!(
        s.text("root /srv/synthetic-media/movies is full"),
        "root <root-1> is full"
    );
}

#[test]
fn known_names_match_whole_words_case_insensitively_longest_first() {
    let mut known = KnownValues::new();
    let a = known.entity("download-client", Some("sabnzbd"), "a");
    known.name(&a, "Home Sab");
    let b = known.entity("download-client", Some("sabnzbd"), "b");
    known.name(&b, "Home Sab Two");
    known.name("tag-1", "kid");
    let s = known.into_sanitizer();
    assert_eq!(
        s.text("home sab two and HOME SAB polled"),
        "<download-client-2(sabnzbd)> and <download-client-1(sabnzbd)> polled"
    );
    // `kid` inside another word is left alone.
    assert_eq!(s.text("kidnapped kid"), "kidnapped <tag-1>");
}

#[test]
fn protected_vocabulary_is_not_taken_over_by_a_name() {
    let mut known = KnownValues::new();
    known.protect("newznab");
    let pseudonym = known.entity("indexer", Some("newznab"), "x");
    known.name(&pseudonym, "Newznab");
    let s = known.into_sanitizer();
    assert_eq!(s.text("newznab caps ok"), "newznab caps ok");
    assert_eq!(s.skipped_protected_values(), 1);
}

#[test]
fn short_values_are_counted_not_replaced() {
    let mut known = KnownValues::new();
    known.name("user-1", "jo");
    let s = known.into_sanitizer();
    assert_eq!(s.text("jo logged in"), "jo logged in");
    assert_eq!(s.skipped_short_values(), 1);
}

#[test]
fn field_names_decide_their_values() {
    assert_eq!(classify_field("api_key"), FieldClass::Secret);
    assert_eq!(classify_field("X-Api-Key"), FieldClass::Secret);
    assert_eq!(classify_field("base_url"), FieldClass::Url);
    assert_eq!(classify_field("client_ip"), FieldClass::Host);
    assert_eq!(classify_field("display_name"), FieldClass::User);
    assert_eq!(classify_field("indexer_name"), FieldClass::EntityName);
    assert_eq!(classify_field("file_name"), FieldClass::Plain);
    assert_eq!(classify_field("release_name"), FieldClass::Plain);
    assert_eq!(classify_field("root_path"), FieldClass::Path);
    assert_eq!(classify_field("decision_code"), FieldClass::Plain);
}

#[test]
fn structured_log_lines_are_sanitized_by_field_and_value() {
    let s = patterns();
    let line = json!({
        "timestamp": "2026-11-10T12:00:00Z",
        "level": "INFO",
        "target": "scryer::indexer",
        "fields": {
            "message": "query to https://feed.example.net/api failed for 198.51.100.4",
            "indexer_name": "Unregistered Feed",
            "api_key": "abcdef",
            "release_name": "Synthetic.Release.2020.mkv",
            "attempt": 3,
        },
        "context": {
            "request": { "client_ip": "198.51.100.7", "operation_name": "Titles" },
            "actor": { "display_name": "someone", "id": "u-1" },
        },
    })
    .to_string();
    let out: Value = serde_json::from_str(&s.log_line(&line)).expect("json");
    assert_eq!(out["timestamp"], "2026-11-10T12:00:00Z");
    assert_eq!(out["target"], "scryer::indexer");
    assert_eq!(
        out["fields"]["message"],
        "query to https://<host>/<…> failed for <ip>"
    );
    assert_eq!(out["fields"]["indexer_name"], "<name>");
    assert_eq!(out["fields"]["api_key"], "<redacted>");
    assert_eq!(out["fields"]["release_name"], "Synthetic.Release.2020.mkv");
    assert_eq!(out["fields"]["attempt"], 3);
    assert_eq!(out["context"]["request"]["client_ip"], "<ip>");
    assert_eq!(out["context"]["request"]["operation_name"], "Titles");
    assert_eq!(out["context"]["actor"]["display_name"], "<user>");
}

#[test]
fn text_log_lines_are_sanitized_by_key_and_value() {
    let s = patterns();
    let line = "2026-11-10T12:00:00.000Z  WARN request{client_ip=198.51.100.7}: scryer: grab failed indexer_name=\"Unregistered Feed\" token=abc123 ratio=1.5";
    assert_eq!(
        s.log_line(line),
        "2026-11-10T12:00:00.000Z  WARN request{client_ip=<ip>}: scryer: grab failed indexer_name=\"<name>\" token=<redacted> ratio=1.5"
    );
    assert_eq!(s.log_line("\u{1b}[32mINFO\u{1b}[0m ok"), "INFO ok");
}

#[test]
fn metrics_labels_are_sanitized_and_re_escaped() {
    let mut known = KnownValues::new();
    let pseudonym = known.entity("indexer", Some("torznab"), "x");
    known.name(&pseudonym, "Synthetic Tracker");
    let s = known.into_sanitizer();
    let exposition = "# TYPE scryer_requests_total counter\n\
scryer_requests_total{indexer=\"Synthetic Tracker\",host=\"tracker.example.org\",le=\"0.5\"} 12\n\
scryer_up 1\n";
    assert_eq!(
        s.metrics(exposition),
        "# TYPE scryer_requests_total counter\n\
scryer_requests_total{indexer=\"<indexer-1(torznab)>\",host=\"<host>\",le=\"0.5\"} 12\n\
scryer_up 1\n"
    );
}

#[test]
fn config_json_harvests_secrets_hosts_users_and_mapping_paths() {
    let mut known = KnownValues::new();
    let pseudonym = known.entity("download-client", Some("qbittorrent"), "c");
    known.config_json(
        &pseudonym,
        &json!({
            "host": "seedbox.synthetic.invalid",
            "port": 8080,
            "username": "seeduser",
            "password": "seedpass",
            "remote_path_mappings": "/remote/seeds => /local/seeds",
        })
        .to_string(),
    );
    let s = known.into_sanitizer();
    assert_eq!(
        s.text("seeduser on seedbox.synthetic.invalid used seedpass"),
        "<download-client-1(qbittorrent)> on <download-client-1(qbittorrent)> used <redacted>"
    );
    assert_eq!(
        s.text("moving /local/seeds/Synthetic.Release.mkv"),
        "moving <download-client-1(qbittorrent)-path>/Synthetic.Release.mkv"
    );
}

#[test]
fn bundle_file_name_is_versioned_and_timestamped() {
    let at = Utc.with_ymd_and_hms(2026, 11, 10, 19, 5, 0).unwrap();
    assert_eq!(
        diagnostics_bundle_file_name("0.22.3", at),
        "scryer-diagnostics-0.22.3-20261110T190500Z.tar.zst"
    );
}

// ---- end to end ------------------------------------------------------------

/// Every sensitive class gets a canary that must not survive.
const CANARIES: &[&str] = &[
    "CanaryIndexerAlpha",
    "canary-indexer-host",
    "canaryindexerapikey0123",
    "CanaryClientBeta",
    "canary-client-host",
    "canaryclientpass987",
    "canaryclientuser",
    "canary-remote-dir",
    "canary-local-dir",
    "CanaryLibraryGamma",
    "canary-mount-alpha",
    "canary-user-delta",
    "canary-env-value-epsilon",
    "canary-hostname-zeta",
    "CanaryMountEta",
    "canary-logdir-theta",
    "canary-unknown-host",
    "203.0.113.77",
    "2001:db8::77",
    "canary.person",
    "canarybearertoken123",
    "ska_canaryscryerkey42",
    "eyJcanaryhdr",
    "canarycookie77",
    "CanaryWinUser",
    "canary-unc-server",
    "CanaryMetricIndexer",
];

fn indexer(
    id: &str,
    name: &str,
    provider: &str,
    base_url: &str,
    api_key: &str,
) -> scryer_domain::IndexerConfig {
    let now = Utc::now();
    scryer_domain::IndexerConfig {
        id: id.to_string(),
        name: name.to_string(),
        provider_type: provider.to_string(),
        base_url: base_url.to_string(),
        api_key_encrypted: Some(api_key.to_string()),
        rate_limit_seconds: None,
        rate_limit_burst: None,
        max_queries_per_minute: None,
        disabled_until: None,
        is_enabled: true,
        enable_interactive_search: true,
        enable_auto_search: true,
        proxy_config_id: None,
        download_client_id: None,
        seeding_profile_id: None,
        managed_parent_config_id: None,
        managed_child_key: None,
        managed_metadata_json: None,
        caps_snapshot_json: None,
        last_health_status: None,
        last_error_message: None,
        last_error_code: Some("INDEXER_SYNTHETIC_FAILURE".to_string()),
        last_error_at: None,
        config_json: None,
        created_at: now,
        updated_at: now,
    }
}

fn unpack(bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
    let tar = zstd::decode_all(bytes).expect("zstd");
    let mut archive = tar::Archive::new(tar.as_slice());
    archive
        .entries()
        .expect("entries")
        .map(|entry| {
            let mut entry = entry.expect("entry");
            let name = entry.path().expect("path").to_string_lossy().into_owned();
            let mut content = Vec::new();
            entry.read_to_end(&mut content).expect("read");
            (name, content)
        })
        .collect()
}

#[tokio::test]
async fn bundle_contains_no_canary_and_keeps_types_counts_and_correlation() {
    let (app, admin) = crate::lib_tests::bootstrap();
    let now = Utc::now();

    app.services
        .integrations
        .indexer_configs
        .create(indexer(
            "idx-canary",
            "CanaryIndexerAlpha",
            "newznab",
            "https://canary-indexer-host.synthetic.invalid/api",
            "canaryindexerapikey0123",
        ))
        .await
        .expect("indexer");
    app.services
        .integrations
        .indexer_configs
        .create(indexer(
            "idx-other",
            "CanaryMetricIndexer",
            "torznab",
            "http://198.51.100.20:9117/torznab",
            "othersyntheticapikey",
        ))
        .await
        .expect("indexer");
    app.services
        .integrations
        .download_client_configs
        .create(scryer_domain::DownloadClientConfig {
            id: "dc-canary".to_string(),
            name: "CanaryClientBeta".to_string(),
            client_type: "sabnzbd".to_string(),
            config_json: json!({
                "host": "canary-client-host.synthetic.invalid",
                "port": 8080,
                "username": "canaryclientuser",
                "password": "canaryclientpass987",
                "remote_path_mappings": "/canary-remote-dir/complete => /canary-local-dir/complete",
            })
            .to_string(),
            client_priority: 0,
            is_enabled: true,
            status: scryer_domain::DownloadClientStatus::Healthy,
            last_error: None,
            last_seen_at: None,
            proxy_config_id: None,
            created_at: now,
            updated_at: now,
        })
        .await
        .expect("download client");
    app.services
        .catalog
        .libraries
        .create(
            scryer_domain::Library {
                id: "lib-canary".to_string(),
                facet: scryer_domain::MediaFacet::Movie,
                name: "CanaryLibraryGamma".to_string(),
                slug: "canarylibrarygamma".to_string(),
                is_default: false,
                roots: Vec::new(),
                created_at: now,
                updated_at: now,
            },
            vec![crate::LibraryRootDraft {
                id: None,
                path: "/srv/canary-mount-alpha/movies".to_string(),
                is_default: true,
            }],
        )
        .await
        .expect("library");
    app.services
        .identity
        .users
        .create(scryer_domain::User::new_admin("canary-user-delta"))
        .await
        .expect("user");

    let log_dir = tempfile::tempdir().expect("tempdir");
    let log_dir_path = log_dir.path().join("canary-logdir-theta");
    std::fs::create_dir(&log_dir_path).expect("log dir");
    let log_file = log_dir_path.join("scryer.log");
    std::fs::write(
        &log_file,
        "2026-11-10T11:00:00Z  INFO scryer: older file line from CanaryClientBeta at /canary-local-dir/complete/Synthetic.Feature.2020.1080p.mkv\n",
    )
    .expect("log file");

    let buffered = vec![
        json!({
            "timestamp": "2026-11-10T12:00:00Z",
            "level": "WARN",
            "target": "scryer::indexer",
            "fields": {
                "message": "search on CanaryIndexerAlpha failed: GET https://canary-indexer-host.synthetic.invalid/api?apikey=canaryindexerapikey0123 from 203.0.113.77",
                "indexer": "CanaryIndexerAlpha",
                "ratio": "1.50",
                "version": "0.22.3",
            },
            "context": {
                "request": { "client_ip": "2001:db8::77" },
                "actor": { "display_name": "api (Diag) obo canary-user-delta" },
            },
        })
        .to_string(),
        "2026-11-10T12:00:01Z  INFO scryer: host canary-hostname-zeta started; data dir /Volumes/CanaryMountEta/data; env SCRYER_SYNTHETIC=canary-env-value-epsilon".to_string(),
        "2026-11-10T12:00:02Z  INFO scryer: notify canary.person@example.org; Authorization: Bearer canarybearertoken123".to_string(),
        "2026-11-10T12:00:03Z  INFO scryer: key ska_canaryscryerkey42 jwt eyJcanaryhdr.eyJcanarybody.canarysignature Cookie: sid=canarycookie77".to_string(),
        r"2026-11-10T12:00:04Z  INFO scryer: windows import C:\Users\CanaryWinUser\Downloads\Synthetic.Feature.2020.1080p.mkv share \\canary-unc-server\media\Other.File.mkv".to_string(),
        "2026-11-10T12:00:05Z  INFO scryer: upstream canary-unknown-host.example.com:443 reset".to_string(),
        "2026-11-10T12:00:06Z  INFO scryer: rooted /srv/canary-mount-alpha/movies/Feature/Synthetic.Feature.2020.1080p.mkv for CanaryLibraryGamma".to_string(),
    ];

    let metrics = "# TYPE scryer_indexer_requests_total counter\n\
scryer_indexer_requests_total{indexer=\"CanaryMetricIndexer\",result=\"ok\"} 4\n\
scryer_indexer_requests_total{indexer=\"CanaryIndexerAlpha\",result=\"error\"} 2\n\
scryer_download_client_up{client=\"CanaryClientBeta\",host=\"canary-client-host.synthetic.invalid:8080\"} 1\n";

    let mut environment = serde_json::Map::new();
    environment.insert("installation_kind".to_string(), json!("PORTABLE"));
    environment.insert(
        "scryer_env".to_string(),
        json!({ "SCRYER_SYNTHETIC": "<set>", "SCRYER_LOG_FORMAT": "json" }),
    );
    let host = DiagnosticsHostInputs {
        buffered_log_lines: buffered,
        log_file: Some(log_file.clone()),
        metrics: DiagnosticsMetrics::Rendered(metrics.to_string()),
        environment,
        environment_omissions: vec![("process_memory".to_string(), "not_portable".to_string())],
        sensitive_values: vec![
            "canary-hostname-zeta".to_string(),
            "canary-env-value-epsilon".to_string(),
        ],
        sensitive_paths: vec![
            (
                "data-dir".to_string(),
                "/Volumes/CanaryMountEta/data".to_string(),
            ),
            (
                "log-dir".to_string(),
                log_dir_path.to_string_lossy().into_owned(),
            ),
        ],
        version: "0.22.3".to_string(),
        generated_at: Utc.with_ymd_and_hms(2026, 11, 10, 19, 0, 0).unwrap(),
    };

    let bundle = app
        .build_diagnostics_bundle(&admin, host)
        .await
        .expect("bundle");
    assert_eq!(
        bundle.file_name,
        "scryer-diagnostics-0.22.3-20261110T190000Z.tar.zst"
    );
    assert!(bundle.bytes.len() <= DIAGNOSTICS_BUNDLE_MAX_COMPRESSED_BYTES);

    let members = unpack(&bundle.bytes);
    let names = members
        .iter()
        .map(|(name, _)| name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        vec![
            "manifest.json",
            "summary.json",
            "environment.json",
            "logs/buffer.log",
            "logs/file.log",
            "metrics.prom",
        ]
    );

    let everything = members
        .iter()
        .flat_map(|(_, bytes)| bytes.iter().copied())
        .collect::<Vec<u8>>();
    let everything = String::from_utf8_lossy(&everything).to_lowercase();
    for canary in CANARIES {
        assert!(
            !everything.contains(&canary.to_lowercase()),
            "canary {canary} leaked into the bundle"
        );
    }
    // The temp directory the log file sits in is a host path too.
    assert!(!everything.contains(&log_dir.path().to_string_lossy().to_lowercase()));

    let member = |name: &str| {
        String::from_utf8(
            members
                .iter()
                .find(|(member, _)| member == name)
                .expect("member")
                .1
                .clone(),
        )
        .expect("utf8")
    };

    // Types, counts and codes survive.
    let summary: Value = serde_json::from_str(&member("summary.json")).expect("summary");
    assert_eq!(summary["indexers"]["total"], 2);
    assert_eq!(
        summary["indexers"]["by_provider_type"]["newznab"]["enabled"],
        1
    );
    assert_eq!(
        summary["indexers"]["by_provider_type"]["torznab"]["enabled"],
        1
    );
    assert_eq!(
        summary["indexers"]["last_error_codes"]["INDEXER_SYNTHETIC_FAILURE"],
        2
    );
    assert_eq!(
        summary["download_clients"]["by_client_type"]["sabnzbd"]["enabled"],
        1
    );
    assert_eq!(summary["download_clients"]["with_remote_path_mappings"], 1);
    assert!(summary["libraries"]["by_facet"]["movie"].as_u64() >= Some(1));
    assert!(summary["libraries"]["root_folders"].as_u64() >= Some(1));
    assert!(summary["quality_profiles"]["profiles"][0]["criteria"].is_object());
    assert_eq!(
        summary["quality_profiles"]["profiles"][0]["pseudonym"],
        "quality-profile-1"
    );

    // One entity, one pseudonym, across logs and metrics.
    let buffer = member("logs/buffer.log");
    assert!(buffer.contains("<indexer-1(newznab)>"), "{buffer}");
    assert!(
        buffer.contains("https://<indexer-1(newznab)>/<…>"),
        "{buffer}"
    );
    assert!(buffer.contains("<ip>"), "{buffer}");
    assert!(buffer.contains("(movie)>"), "{buffer}");
    assert!(
        buffer.contains(">/<…>/Synthetic.Feature.2020.1080p.mkv for <library-"),
        "{buffer}"
    );
    assert!(
        buffer.contains("<path>/Synthetic.Feature.2020.1080p.mkv"),
        "{buffer}"
    );
    assert!(buffer.contains("\"ratio\":\"1.50\""), "{buffer}");
    assert!(buffer.contains("\"version\":\"0.22.3\""), "{buffer}");
    let file = member("logs/file.log");
    assert!(file.contains("<download-client-1(sabnzbd)>"), "{file}");
    assert!(file.contains("Synthetic.Feature.2020.1080p.mkv"), "{file}");
    let metrics = member("metrics.prom");
    assert!(
        metrics.contains("indexer=\"<indexer-2(torznab)>\""),
        "{metrics}"
    );
    assert!(
        metrics.contains("indexer=\"<indexer-1(newznab)>\""),
        "{metrics}"
    );
    assert!(
        metrics.contains("client=\"<download-client-1(sabnzbd)>\""),
        "{metrics}"
    );

    let manifest: Value = serde_json::from_str(&member("manifest.json")).expect("manifest");
    assert_eq!(
        manifest["format_version"],
        DIAGNOSTICS_BUNDLE_FORMAT_VERSION
    );
    assert_eq!(manifest["sanitization"]["path_policy"], "keep_file_name");
    assert!(
        manifest["omissions"]
            .as_array()
            .expect("omissions")
            .iter()
            .any(|omission| omission["item"] == "environment.process_memory")
    );
    assert_eq!(manifest["files"].as_array().expect("files").len(), 5);
}

#[tokio::test]
async fn disabled_metrics_and_missing_file_logging_are_recorded_as_omissions() {
    let (app, admin) = crate::lib_tests::bootstrap();
    let host = DiagnosticsHostInputs {
        buffered_log_lines: vec!["2026-11-10T12:00:00Z INFO scryer: started".to_string()],
        log_file: None,
        metrics: DiagnosticsMetrics::Disabled,
        environment: serde_json::Map::new(),
        environment_omissions: Vec::new(),
        sensitive_values: Vec::new(),
        sensitive_paths: Vec::new(),
        version: "0.22.3".to_string(),
        generated_at: Utc.with_ymd_and_hms(2026, 11, 10, 19, 0, 0).unwrap(),
    };
    let bundle = app
        .build_diagnostics_bundle(&admin, host)
        .await
        .expect("bundle");
    let members = unpack(&bundle.bytes);
    assert!(members.iter().all(|(name, _)| name != "metrics.prom"));
    assert!(members.iter().all(|(name, _)| name != "logs/file.log"));
    let manifest: Value = serde_json::from_slice(&members[0].1).expect("manifest");
    let omissions = manifest["omissions"].as_array().expect("omissions");
    assert!(
        omissions
            .iter()
            .any(|o| o["item"] == "metrics.prom" && o["reason"] == "metrics_disabled")
    );
    assert!(
        omissions
            .iter()
            .any(|o| o["item"] == "logs/file.log" && o["reason"] == "file_logging_not_configured")
    );
}

#[tokio::test]
async fn bundle_requires_the_system_settings_permission() {
    let (app, _) = crate::lib_tests::bootstrap();
    let mut viewer = scryer_domain::User::new_admin("synthetic-viewer");
    viewer.authorization = scryer_domain::UserAuthorization {
        loaded: true,
        ..Default::default()
    };
    let host = DiagnosticsHostInputs {
        buffered_log_lines: Vec::new(),
        log_file: None,
        metrics: DiagnosticsMetrics::Disabled,
        environment: serde_json::Map::new(),
        environment_omissions: Vec::new(),
        sensitive_values: Vec::new(),
        sensitive_paths: Vec::new(),
        version: "0.22.3".to_string(),
        generated_at: Utc::now(),
    };
    let error = app
        .build_diagnostics_bundle(&viewer, host)
        .await
        .expect_err("refused");
    assert!(matches!(
        error,
        crate::AppError::Unauthorized(_) | crate::AppError::UnauthorizedReason { .. }
    ));
}

#[test]
fn oversized_logs_are_trimmed_oldest_first_and_recorded() {
    // Random-looking lines defeat compression so the cap is reached.
    let mut state = 0x2545_F491_4F6C_DD1D_u64;
    let lines = (0..120_000)
        .map(|index| {
            let mut text = format!("{index:08} ");
            for _ in 0..12 {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                text.push_str(&format!("{state:016x}"));
            }
            text
        })
        .collect::<Vec<_>>();
    let inputs = archive::ArchiveInputs {
        version: "0.22.3".to_string(),
        generated_at: Utc.with_ymd_and_hms(2026, 11, 10, 19, 0, 0).unwrap(),
        summary: json!({}),
        environment: serde_json::Map::new(),
        buffered_log_lines: vec!["newest buffered line".to_string()],
        file_logs: logs::LogFileRead {
            lines,
            sources: vec!["active".to_string()],
            configured: true,
            ..Default::default()
        },
        metrics: DiagnosticsMetrics::Disabled,
        omissions: Vec::new(),
        known_values_incomplete: Vec::new(),
    };
    let bytes = archive::build(&patterns(), inputs).expect("bundle");
    assert!(bytes.len() <= DIAGNOSTICS_BUNDLE_MAX_COMPRESSED_BYTES);
    let members = unpack(&bytes);
    let manifest: Value = serde_json::from_slice(&members[0].1).expect("manifest");
    let truncation = &manifest["truncations"][0];
    assert_eq!(truncation["item"], "logs/file.log");
    assert_eq!(truncation["reason"], "size_cap");
    let file = String::from_utf8(
        members
            .iter()
            .find(|(name, _)| name == "logs/file.log")
            .expect("file log")
            .1
            .clone(),
    )
    .expect("utf8");
    // The newest line is kept, the oldest is gone.
    assert!(file.contains("00119999 "));
    assert!(!file.starts_with("00000000 "));
    let buffer = &members
        .iter()
        .find(|(name, _)| name == "logs/buffer.log")
        .expect("buffer")
        .1;
    assert_eq!(buffer, b"newest buffered line\n");
}

#[test]
fn incomplete_known_values_leave_logs_and_metrics_out() {
    let inputs = archive::ArchiveInputs {
        version: "0.22.3".to_string(),
        generated_at: Utc::now(),
        summary: json!({}),
        environment: serde_json::Map::new(),
        buffered_log_lines: vec!["Unharvested Name logged".to_string()],
        file_logs: logs::LogFileRead::absent(),
        metrics: DiagnosticsMetrics::Rendered("up 1\n".to_string()),
        omissions: Vec::new(),
        known_values_incomplete: vec!["indexers".to_string()],
    };
    let bytes = archive::build(&patterns(), inputs).expect("bundle");
    let members = unpack(&bytes);
    let names = members
        .iter()
        .map(|(name, _)| name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        vec!["manifest.json", "summary.json", "environment.json"]
    );
    let manifest: Value = serde_json::from_slice(&members[0].1).expect("manifest");
    assert!(
        manifest["omissions"]
            .as_array()
            .expect("omissions")
            .iter()
            .any(|o| o["item"] == "logs/buffer.log"
                && o["reason"] == "known_values_incomplete:indexers")
    );
}
