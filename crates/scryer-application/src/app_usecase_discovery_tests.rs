use super::*;

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
