use super::*;
use crate::DownloadSourceKind;

// ── extract_http_status_from_message ─────────────────────────────────────────

#[test]
fn extracts_404_from_error_message() {
    assert_eq!(
        extract_http_status_from_message("request failed with status 404"),
        Some(404)
    );
}

#[test]
fn extracts_503_from_error_message() {
    assert_eq!(
        extract_http_status_from_message("HTTP status 503 Service Unavailable"),
        Some(503)
    );
}

#[test]
fn extracts_status_case_insensitive() {
    assert_eq!(
        extract_http_status_from_message("received STATUS 429 too many requests"),
        Some(429)
    );
}

#[test]
fn returns_none_when_no_status_keyword() {
    assert_eq!(extract_http_status_from_message("connection refused"), None);
}

#[test]
fn returns_none_for_empty_message() {
    assert_eq!(extract_http_status_from_message(""), None);
}

#[test]
fn returns_none_when_status_keyword_has_no_digits() {
    assert_eq!(extract_http_status_from_message("status ok"), None);
}

// ── is_4xx_or_5xx_status ─────────────────────────────────────────────────────

#[test]
fn is_4xx_true_for_client_errors() {
    for status in [400u16, 401, 403, 404, 422, 429] {
        assert!(is_4xx_or_5xx_status(status), "{status} should be 4xx/5xx");
    }
}

#[test]
fn is_5xx_true_for_server_errors() {
    for status in [500u16, 502, 503, 504] {
        assert!(is_4xx_or_5xx_status(status), "{status} should be 4xx/5xx");
    }
}

#[test]
fn is_2xx_false() {
    assert!(!is_4xx_or_5xx_status(200));
    assert!(!is_4xx_or_5xx_status(201));
    assert!(!is_4xx_or_5xx_status(204));
}

#[test]
fn is_3xx_false() {
    assert!(!is_4xx_or_5xx_status(301));
    assert!(!is_4xx_or_5xx_status(302));
}

// ── is_indexer_http_error ─────────────────────────────────────────────────────

#[test]
fn repository_error_with_status_404_is_http_error() {
    let err = AppError::Repository("indexer returned status 404 not found".to_string());
    assert!(is_indexer_http_error(&err));
}

#[test]
fn repository_error_with_status_503_is_http_error() {
    let err = AppError::Repository("upstream status 503".to_string());
    assert!(is_indexer_http_error(&err));
}

#[test]
fn repository_error_with_200_is_not_http_error() {
    let err = AppError::Repository("status 200 ok".to_string());
    assert!(!is_indexer_http_error(&err));
}

#[test]
fn non_repository_error_is_not_http_error() {
    let err = AppError::Validation("bad input".to_string());
    assert!(!is_indexer_http_error(&err));
}

#[test]
fn connection_refused_is_not_http_error() {
    let err = AppError::Repository("connection refused".to_string());
    assert!(!is_indexer_http_error(&err));
}

fn make_search_result(
    source: &str,
    title: &str,
    download_url: &str,
    source_kind: DownloadSourceKind,
) -> IndexerSearchResult {
    IndexerSearchResult {
        indexer_id: None,
        source: source.to_string(),
        title: title.to_string(),
        link: None,
        download_url: Some(download_url.to_string()),
        source_kind: Some(source_kind),
        size_bytes: None,
        published_at: None,
        thumbs_up: None,
        thumbs_down: None,
        indexer_languages: None,
        indexer_subtitles: None,
        indexer_grabs: None,
        password_hint: None,
        parsed_release_metadata: Some(parse_release_metadata(title)),
        quality_profile_decision: None,
        extra: HashMap::new(),
        response_attributes: Default::default(),
        guid: None,
        info_url: None,
        provenance: None,
        candidate_token: None,
        queue_scope: None,
        coverage_scope: None,
        auto_eligible: None,
        auto_decision_code: None,
        auto_decision_summary: None,
    }
}

#[test]
fn cross_indexer_release_dedup_prefers_higher_priority_source() {
    let results = vec![
        make_search_result(
            "Lower Priority",
            "Signal.Run.S01E12.720p.WEB-DL.x264-NTb",
            "https://example.test/low",
            DownloadSourceKind::NzbUrl,
        ),
        make_search_result(
            "Higher Priority",
            "Signal.Run.S01E12.720p.WEB-DL.x264-NTb",
            "https://example.test/high",
            DownloadSourceKind::NzbUrl,
        ),
    ];

    let deduped = dedupe_cross_indexer_release_results(
        results,
        &HashMap::from([
            ("Lower Priority".to_string(), 50),
            ("Higher Priority".to_string(), 10),
        ]),
        "nzb",
    );

    assert_eq!(deduped.len(), 1);
    assert_eq!(deduped[0].source, "Higher Priority");
    assert_eq!(
        deduped[0].download_url.as_deref(),
        Some("https://example.test/high")
    );
}

#[test]
fn cross_indexer_release_dedup_prefers_unlimited_grabs() {
    let mut limited = make_search_result(
        "Limited Source",
        "Synthetic.Run.S01E12.720p.WEB-DL.x264-GRP",
        "https://example.test/limited",
        DownloadSourceKind::NzbUrl,
    );
    limited
        .extra
        .insert("grab_current".into(), serde_json::json!(1));
    limited
        .extra
        .insert("grab_max".into(), serde_json::json!(100));
    let unlimited = make_search_result(
        "Unlimited Source",
        "Synthetic.Run.S01E12.720p.WEB-DL.x264-GRP",
        "https://example.test/unlimited",
        DownloadSourceKind::NzbUrl,
    );

    let deduped = dedupe_cross_indexer_release_results(
        vec![limited, unlimited],
        &HashMap::from([
            ("Limited Source".to_string(), 1),
            ("Unlimited Source".to_string(), 50),
        ]),
        "nzb",
    );

    assert_eq!(deduped.len(), 1);
    assert_eq!(deduped[0].source, "Unlimited Source");
}

#[test]
fn cross_indexer_release_dedup_prefers_lower_grab_usage() {
    let mut higher_usage = make_search_result(
        "Higher Usage",
        "Synthetic.Run.S01E12.720p.WEB-DL.x264-GRP",
        "https://example.test/higher",
        DownloadSourceKind::NzbUrl,
    );
    higher_usage
        .extra
        .insert("grab_current".into(), serde_json::json!(80));
    higher_usage
        .extra
        .insert("grab_max".into(), serde_json::json!(100));
    let mut lower_usage = make_search_result(
        "Lower Usage",
        "Synthetic.Run.S01E12.720p.WEB-DL.x264-GRP",
        "https://example.test/lower",
        DownloadSourceKind::NzbUrl,
    );
    lower_usage
        .extra
        .insert("grab_current".into(), serde_json::json!(2));
    lower_usage
        .extra
        .insert("grab_max".into(), serde_json::json!(10));

    let deduped = dedupe_cross_indexer_release_results(
        vec![higher_usage, lower_usage],
        &HashMap::new(),
        "nzb",
    );

    assert_eq!(deduped.len(), 1);
    assert_eq!(deduped[0].source, "Lower Usage");
}

#[test]
fn only_admissible_automatic_decisions_are_reusable() {
    for decision in [
        "eligible",
        "pending_delay",
        "minimum_age",
        "download_client_unavailable",
    ] {
        let mut candidate = make_search_result(
            "Synthetic Source",
            "Synthetic.Run.S01E12.720p.WEB-DL.x264-GRP",
            "https://example.test/admissible",
            DownloadSourceKind::NzbUrl,
        );
        candidate.auto_decision_code = Some(decision.into());
        assert!(candidate_is_reusable(&candidate), "{decision}");
    }

    for decision in ["quality_rejected", "title_mismatch", "blocked"] {
        let mut candidate = make_search_result(
            "Synthetic Source",
            "Synthetic.Run.S01E12.720p.WEB-DL.x264-GRP",
            "https://example.test/rejected",
            DownloadSourceKind::NzbUrl,
        );
        candidate.auto_decision_code = Some(decision.into());
        assert!(!candidate_is_reusable(&candidate), "{decision}");
    }
}

#[test]
fn cross_indexer_release_dedup_prefers_profile_protocol_before_indexer_priority() {
    let results = vec![
        make_search_result(
            "Preferred Torrent",
            "Signal.Run.S01E12.720p.WEB-DL.x264-NTb",
            "https://example.test/torrent",
            DownloadSourceKind::TorrentFile,
        ),
        make_search_result(
            "Higher Priority Usenet",
            "Signal.Run.S01E12.720p.WEB-DL.x264-NTb",
            "https://example.test/usenet",
            DownloadSourceKind::NzbUrl,
        ),
    ];

    let deduped = dedupe_cross_indexer_release_results(
        results,
        &HashMap::from([
            ("Preferred Torrent".to_string(), 50),
            ("Higher Priority Usenet".to_string(), 10),
        ]),
        "torrent",
    );

    assert_eq!(deduped.len(), 1);
    assert_eq!(deduped[0].source, "Preferred Torrent");
}

#[test]
fn release_blocklist_matches_the_release_name_case_and_whitespace_insensitively() {
    // Grab paths keep the indexer's casing and failure paths lowercase, so the
    // stored key and the candidate are both normalized before comparison.
    let blocklist = TitleReleaseBlocklistSignatures {
        release_names: HashSet::from([(
            "idx-1".to_string(),
            "signal.run.s01e12.1080p.web-dl.x265-ntb".to_string(),
        )]),
        info_hashes: HashSet::new(),
    };

    assert!(is_release_blocklisted(
        Some("idx-1"),
        "Signal.Run.S01E12.1080p.WEB-DL.x265-NTb",
        None,
        &blocklist,
    ));
    assert!(is_release_blocklisted(
        Some("idx-1"),
        "  signal.run.s01e12.1080p.web-dl.x265-ntb\n",
        None,
        &blocklist,
    ));
    assert!(!is_release_blocklisted(
        Some("idx-1"),
        "Signal.Run.S01E13.1080p.WEB-DL.x265-NTb",
        None,
        &blocklist,
    ));
    assert!(!is_release_blocklisted(Some("idx-1"), "", None, &blocklist));
    assert!(!is_release_blocklisted(
        Some("idx-1"),
        "   ",
        None,
        &blocklist
    ));
    assert!(!is_release_blocklisted(
        Some("idx-1"),
        "Signal.Run.S01E12.1080p.WEB-DL.x265-NTb",
        None,
        &TitleReleaseBlocklistSignatures::default(),
    ));
}

#[test]
fn a_block_is_scoped_to_its_indexer_unless_it_names_none() {
    let name = "signal.run.s01e12.1080p.web-dl.x265-ntb";
    let scoped = TitleReleaseBlocklistSignatures {
        release_names: HashSet::from([("idx-1".to_string(), name.to_string())]),
        info_hashes: HashSet::new(),
    };
    assert!(is_release_blocklisted(Some("idx-1"), name, None, &scoped));
    // The same release from a different indexer is still grabbable.
    assert!(!is_release_blocklisted(Some("idx-2"), name, None, &scoped));

    // An empty stored indexer is the every-indexer wildcard.
    let wildcard = TitleReleaseBlocklistSignatures {
        release_names: HashSet::from([(String::new(), name.to_string())]),
        info_hashes: HashSet::new(),
    };
    assert!(is_release_blocklisted(Some("idx-2"), name, None, &wildcard));
    assert!(is_release_blocklisted(None, name, None, &wildcard));
}

#[test]
fn an_infohash_block_ignores_both_the_indexer_and_the_name() {
    let info_hash = "0123456789abcdef0123456789abcdef01234567";
    let blocklist = TitleReleaseBlocklistSignatures {
        release_names: HashSet::new(),
        info_hashes: HashSet::from([info_hash.to_string()]),
    };
    assert!(is_release_blocklisted(
        Some("idx-2"),
        "Signal.Run.S01E12.REPACK.1080p.WEB-DL.x265-NTb",
        Some(&info_hash.to_ascii_uppercase()),
        &blocklist,
    ));
    assert!(!is_release_blocklisted(
        Some("idx-2"),
        "Signal.Run.S01E12.REPACK.1080p.WEB-DL.x265-NTb",
        Some("fedcba9876543210fedcba9876543210fedcba98"),
        &blocklist,
    ));
}

#[test]
fn structured_dispatch_queries_collapse_equivalent_episode_variants() {
    let queries = vec![
        "Synthetic Atlas 035".to_string(),
        "Synthetic Atlas S02E11".to_string(),
        "Synthetic Atlas S02".to_string(),
        "Synthetic Atlas".to_string(),
    ];

    let deduped = dedupe_structured_dispatch_queries(queries, Some(2), Some(11), Some(35));

    assert_eq!(deduped, vec!["Synthetic Atlas 035".to_string()]);
}

#[test]
fn structured_dispatch_queries_keep_distinct_base_titles() {
    let queries = vec![
        "Synthetic Atlas S02E11".to_string(),
        "Alternate Atlas S02E11".to_string(),
    ];

    let deduped = dedupe_structured_dispatch_queries(queries.clone(), Some(2), Some(11), None);

    assert_eq!(deduped, queries);
}

#[test]
fn structured_dispatch_query_dedupe_does_not_run_for_plain_title_searches() {
    let queries = vec![
        "Synthetic Atlas 035".to_string(),
        "Synthetic Atlas S02E11".to_string(),
        "Synthetic Atlas".to_string(),
    ];

    let deduped = dedupe_structured_dispatch_queries(queries.clone(), None, None, None);

    assert_eq!(deduped, queries);
}

#[test]
fn structured_dispatch_queries_keep_legitimate_numbered_titles_when_absolute_episode_differs() {
    let queries = vec![
        "Synthetic Atlas 2049".to_string(),
        "Synthetic Atlas S02E11".to_string(),
    ];

    let deduped = dedupe_structured_dispatch_queries(queries.clone(), Some(2), Some(11), Some(35));

    assert_eq!(deduped, queries);
}

fn synthetic_indexer_config(
    id: &str,
    provider_type: &str,
    is_enabled: bool,
    enable_interactive_search: bool,
    enable_auto_search: bool,
    managed_parent_config_id: Option<&str>,
) -> scryer_domain::IndexerConfig {
    scryer_domain::IndexerConfig {
        id: id.to_string(),
        name: format!("Synthetic {id}"),
        provider_type: provider_type.to_string(),
        base_url: "https://example.invalid".to_string(),
        api_key_encrypted: None,
        rate_limit_seconds: None,
        rate_limit_burst: None,
        disabled_until: None,
        is_enabled,
        enable_interactive_search,
        enable_auto_search,
        proxy_config_id: None,
        download_client_id: None,
        seeding_profile_id: None,
        managed_parent_config_id: managed_parent_config_id.map(str::to_string),
        managed_child_key: None,
        managed_metadata_json: None,
        caps_snapshot_json: None,
        last_health_status: None,
        last_error_message: None,
        last_error_at: None,
        config_json: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    }
}

#[test]
fn structured_query_collapse_runs_for_nab_only_search_sets() {
    let configs = vec![
        synthetic_indexer_config("direct", "nzbgeek", true, true, true, None),
        synthetic_indexer_config("proxy", "newznab", true, true, true, Some("parent")),
        synthetic_indexer_config("parent", "prowlarr", true, false, false, None),
    ];

    assert!(should_collapse_structured_nab_queries(
        &configs,
        None,
        SearchMode::Interactive,
        chrono::Utc::now(),
    ));
}

#[test]
fn structured_query_collapse_skips_when_no_configs_are_eligible() {
    assert!(!should_collapse_structured_nab_queries(
        &[],
        None,
        SearchMode::Interactive,
        chrono::Utc::now(),
    ));
}

#[test]
fn structured_query_collapse_skips_when_auto_mode_is_disabled_in_managed_metadata() {
    let mut proxy = synthetic_indexer_config("proxy", "newznab", true, true, true, Some("parent"));
    proxy.managed_metadata_json = Some(
        serde_json::json!({
            "enable_automatic_search": false
        })
        .to_string(),
    );

    assert!(!should_collapse_structured_nab_queries(
        &[proxy],
        None,
        SearchMode::Auto,
        chrono::Utc::now(),
    ));
}

#[test]
fn structured_query_collapse_skips_when_non_nab_indexers_are_eligible() {
    let configs = vec![
        synthetic_indexer_config("direct", "nzbgeek", true, true, true, None),
        synthetic_indexer_config("other", "id_only_anime_indexer", true, true, true, None),
    ];

    assert!(!should_collapse_structured_nab_queries(
        &configs,
        None,
        SearchMode::Interactive,
        chrono::Utc::now(),
    ));
}

// ── D11: the merge comparator orders tier-first ─────────────────────────────

/// One scored result, as the search lane would have produced it.
fn scored_search_result(
    title: &str,
    tier_index: Option<usize>,
    preference_score: i32,
    allowed: bool,
) -> IndexerSearchResult {
    let mut result = make_search_result(
        "indexer",
        title,
        &format!("https://example.invalid/{title}.nzb"),
        DownloadSourceKind::NzbUrl,
    );
    result.quality_profile_decision = Some(crate::QualityProfileDecision {
        release_score: preference_score,
        scoring_log: Vec::new(),
        allowed,
        block_codes: if allowed {
            Vec::new()
        } else {
            vec!["source_in_profile_blocklist".to_string()]
        },
        preference_score,
        tier_index,
    });
    result
}

/// **D11.** The interactive search's incremental merge re-sorts a partial
/// snapshot with `compare_release_search_results`, and the payload then
/// truncates to the requested limit — so the comparator decides which releases
/// the operator sees at all.
///
/// It ordered allowed → score. With the quality tier no longer inside the score
/// (it is a comparison step now), a 720p release scoring +300 listed above a
/// 2160p one scoring +100, and on a truncated page the 2160p release simply
/// vanished.
#[test]
fn the_merge_comparator_orders_tier_before_score() {
    let mut merged = [
        scored_search_result("Portmere.2024.720p.WEB-DL-GRP", Some(2), 300, true),
        scored_search_result("Portmere.2024.1080p.WEB-DL-GRP", Some(1), -50, true),
        scored_search_result("Portmere.2024.2160p.WEB-DL-GRP", Some(0), 100, true),
        // Blocked sorts last whatever its tier and score.
        scored_search_result(
            "Portmere.2024.2160p.BluRay.REMUX-GRP",
            Some(0),
            5_000,
            false,
        ),
        // A quality the profile does not list sorts below every listed tier.
        scored_search_result("Portmere.2024.480p.DVDRip-GRP", None, 900, true),
    ];

    merged.sort_by(compare_release_search_results);

    let order = merged
        .iter()
        .map(|result| result.title.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        order,
        vec![
            "Portmere.2024.2160p.WEB-DL-GRP",
            "Portmere.2024.1080p.WEB-DL-GRP",
            "Portmere.2024.720p.WEB-DL-GRP",
            "Portmere.2024.480p.DVDRip-GRP",
            "Portmere.2024.2160p.BluRay.REMUX-GRP",
        ]
    );
}

/// Within one tier, a PROPER outranks the plain release, and only then does the
/// score decide — the same order `SearchRank` uses.
#[test]
fn the_merge_comparator_orders_revision_before_score() {
    let mut merged = [
        scored_search_result("Portmere.2024.1080p.WEB-DL-GRP", Some(1), 900, true),
        scored_search_result("Portmere.2024.PROPER.1080p.WEB-DL-GRP", Some(1), 100, true),
    ];
    merged.sort_by(compare_release_search_results);

    assert_eq!(
        merged[0].title.as_str(),
        "Portmere.2024.PROPER.1080p.WEB-DL-GRP"
    );
}

/// **D21.** Seeders rank after indexer priority, and "no information" (usenet,
/// or a torrent indexer that omits the field) ties with them rather than sorting
/// below a torrent with one seeder.
#[test]
fn seeders_rank_more_is_better_and_unknown_is_not_zero() {
    use crate::acquisition::scoring::listing_negated_seeders;

    let mut healthy = make_search_result(
        "indexer",
        "Portmere.2024.1080p.WEB-DL-GRP",
        "magnet:?xt=urn:btih:aaaa",
        DownloadSourceKind::MagnetUri,
    );
    healthy
        .extra
        .insert("seeders".to_string(), serde_json::json!(42));

    let mut dead = healthy.clone();
    dead.extra
        .insert("seeders".to_string(), serde_json::json!(0));

    let usenet = make_search_result(
        "indexer",
        "Portmere.2024.1080p.WEB-DL-GRP",
        "https://example.invalid/a.nzb",
        DownloadSourceKind::NzbUrl,
    );

    assert_eq!(listing_negated_seeders(&healthy), -42);
    assert!(listing_negated_seeders(&healthy) < listing_negated_seeders(&dead));
    assert_eq!(
        listing_negated_seeders(&usenet),
        listing_negated_seeders(&dead),
        "no seeder information must not sort below a torrent with none"
    );
}
