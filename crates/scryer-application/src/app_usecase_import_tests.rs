use super::*;

// ── has_scryer_origin ─────────────────────────────────────────────────────

#[test]
fn has_scryer_origin_with_title_id() {
    let params = vec![
        ("*scryer_title_id".to_string(), "abc-123".to_string()),
        ("category".to_string(), "movie".to_string()),
    ];
    assert!(has_scryer_origin(&params));
}

#[test]
fn has_scryer_origin_without_title_id() {
    let params = vec![("category".to_string(), "movie".to_string())];
    assert!(!has_scryer_origin(&params));
}

#[test]
fn has_scryer_origin_empty_params() {
    let params: Vec<(String, String)> = vec![];
    assert!(!has_scryer_origin(&params));
}

// ── extract_parameter ─────────────────────────────────────────────────────

#[test]
fn extract_parameter_found() {
    let params = vec![
        ("*scryer_title_id".to_string(), "abc-123".to_string()),
        ("category".to_string(), "movie".to_string()),
    ];
    assert_eq!(
        extract_parameter(&params, "*scryer_title_id"),
        Some("abc-123".to_string())
    );
}

#[test]
fn extract_parameter_not_found() {
    let params = vec![("category".to_string(), "movie".to_string())];
    assert_eq!(extract_parameter(&params, "*scryer_title_id"), None);
}

#[test]
fn extract_parameter_empty_params() {
    let params: Vec<(String, String)> = vec![];
    assert_eq!(extract_parameter(&params, "anything"), None);
}

#[test]
fn extract_parameter_first_match() {
    let params = vec![
        ("key".to_string(), "first".to_string()),
        ("key".to_string(), "second".to_string()),
    ];
    assert_eq!(extract_parameter(&params, "key"), Some("first".to_string()));
}

// ── normalize_imdb_id ─────────────────────────────────────────────────────

#[test]
fn normalize_imdb_id_with_prefix() {
    assert_eq!(
        normalize_imdb_id("tt1234567"),
        Some("tt1234567".to_string())
    );
}

#[test]
fn normalize_imdb_id_digits_only() {
    assert_eq!(
        normalize_imdb_id("1234567"),
        Some("tt1234567".to_string())
    );
}

#[test]
fn normalize_imdb_id_with_extra_chars() {
    assert_eq!(
        normalize_imdb_id("tt0123456abc"),
        Some("tt0123456".to_string())
    );
}

#[test]
fn normalize_imdb_id_empty() {
    assert_eq!(normalize_imdb_id(""), None);
}

#[test]
fn normalize_imdb_id_no_digits() {
    assert_eq!(normalize_imdb_id("abcdef"), None);
}
