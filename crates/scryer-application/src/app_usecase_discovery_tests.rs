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
        guid: None,
        info_url: None,
    }
}

#[test]
fn cross_indexer_release_dedup_prefers_higher_priority_source() {
    let results = vec![
        make_search_result(
            "Lower Priority",
            "Firefly.S01E12.720p.WEB-DL.x264-NTb",
            "https://example.test/low",
            DownloadSourceKind::NzbUrl,
        ),
        make_search_result(
            "Higher Priority",
            "Firefly.S01E12.720p.WEB-DL.x264-NTb",
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
