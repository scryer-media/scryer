use super::*;
use std::path::Path;

// ── is_video_file ─────────────────────────────────────────────────────────

#[test]
fn video_file_mkv() {
    assert!(is_video_file(Path::new("/media/movie.mkv")));
}

#[test]
fn video_file_mp4() {
    assert!(is_video_file(Path::new("/downloads/episode.mp4")));
}

#[test]
fn video_file_avi() {
    assert!(is_video_file(Path::new("old_movie.avi")));
}

#[test]
fn video_file_m2ts() {
    assert!(is_video_file(Path::new("/bluray/BDMV/STREAM/00001.m2ts")));
}

#[test]
fn video_file_webm() {
    assert!(is_video_file(Path::new("clip.webm")));
}

#[test]
fn video_file_case_insensitive() {
    assert!(is_video_file(Path::new("movie.MKV")));
    assert!(is_video_file(Path::new("movie.Mp4")));
}

#[test]
fn not_video_file_subtitle() {
    assert!(!is_video_file(Path::new("movie.srt")));
}

#[test]
fn not_video_file_nfo() {
    assert!(!is_video_file(Path::new("movie.nfo")));
}

#[test]
fn not_video_file_image() {
    assert!(!is_video_file(Path::new("poster.jpg")));
}

#[test]
fn not_video_file_no_extension() {
    assert!(!is_video_file(Path::new("README")));
}

#[test]
fn not_video_file_directory() {
    assert!(!is_video_file(Path::new("/media/movies/")));
}

#[test]
fn not_video_file_nzb() {
    assert!(!is_video_file(Path::new("download.nzb")));
}

// ── match_fuzzy ───────────────────────────────────────────────────────────

#[test]
fn fuzzy_exact_match() {
    assert!(match_fuzzy("Cowboy Bebop", "cowboy bebop"));
}

#[test]
fn fuzzy_partial_match_beginning() {
    assert!(match_fuzzy("Cowboy Bebop", "cow"));
}

#[test]
fn fuzzy_partial_match_middle() {
    assert!(match_fuzzy("Cowboy Bebop", "boy be"));
}

#[test]
fn fuzzy_partial_match_end() {
    assert!(match_fuzzy("Cowboy Bebop", "bebop"));
}

#[test]
fn fuzzy_case_insensitive() {
    assert!(match_fuzzy("Cowboy Bebop", "COWBOY"));
    assert!(match_fuzzy("cowboy bebop", "BEBOP"));
}

#[test]
fn fuzzy_no_match() {
    assert!(!match_fuzzy("Cowboy Bebop", "naruto"));
}

#[test]
fn fuzzy_empty_query_matches_everything() {
    assert!(match_fuzzy("Cowboy Bebop", ""));
    assert!(match_fuzzy("", ""));
}

#[test]
fn fuzzy_empty_candidate_no_match() {
    assert!(!match_fuzzy("", "cowboy"));
}

#[test]
fn fuzzy_whitespace_query() {
    assert!(match_fuzzy("Cowboy Bebop", "  "));
}

// ── normalize_tags ────────────────────────────────────────────────────────

#[test]
fn tags_lowercased() {
    let result = normalize_tags(&["Anime".into(), "ACTION".into()]);
    assert_eq!(result, vec!["action", "anime"]);
}

#[test]
fn tags_deduplication() {
    let result = normalize_tags(&["anime".into(), "Anime".into(), "ANIME".into()]);
    assert_eq!(result, vec!["anime"]);
}

#[test]
fn tags_sorted() {
    let result = normalize_tags(&["zebra".into(), "alpha".into(), "middle".into()]);
    assert_eq!(result, vec!["alpha", "middle", "zebra"]);
}

#[test]
fn tags_whitespace_trimmed() {
    let result = normalize_tags(&[" anime ".into(), "  tv  ".into()]);
    assert_eq!(result, vec!["anime", "tv"]);
}

#[test]
fn tags_empty_strings_ignored() {
    let result = normalize_tags(&["anime".into(), "".into(), "  ".into()]);
    assert_eq!(result, vec!["anime"]);
}

#[test]
fn tags_scryer_prefix_preserves_case() {
    let result = normalize_tags(&["scryer:season-folder:disabled".into(), "anime".into()]);
    assert!(result.contains(&"scryer:season-folder:disabled".to_string()));
    assert!(result.contains(&"anime".to_string()));
}

#[test]
fn tags_empty_input() {
    let result = normalize_tags(&[]);
    assert!(result.is_empty());
}

// ── User entitlements ─────────────────────────────────────────────────────

#[test]
fn admin_has_all_entitlements() {
    let admin = User::new_admin("root");
    assert!(admin.has_all_entitlements());
    assert!(admin.has_entitlement(&Entitlement::ViewCatalog));
    assert!(admin.has_entitlement(&Entitlement::MonitorTitle));
    assert!(admin.has_entitlement(&Entitlement::ManageTitle));
    assert!(admin.has_entitlement(&Entitlement::TriggerActions));
    assert!(admin.has_entitlement(&Entitlement::ManageConfig));
    assert!(admin.has_entitlement(&Entitlement::ViewHistory));
}

#[test]
fn user_with_limited_entitlements() {
    let user = User {
        id: Id::new().0,
        username: "viewer".to_string(),
        password_hash: None,
        entitlements: vec![Entitlement::ViewCatalog, Entitlement::ViewHistory],
    };
    assert!(user.has_entitlement(&Entitlement::ViewCatalog));
    assert!(user.has_entitlement(&Entitlement::ViewHistory));
    assert!(!user.has_entitlement(&Entitlement::ManageConfig));
    assert!(!user.has_entitlement(&Entitlement::TriggerActions));
    assert!(!user.has_all_entitlements());
}

#[test]
fn user_with_no_entitlements() {
    let user = User {
        id: Id::new().0,
        username: "empty".to_string(),
        password_hash: None,
        entitlements: vec![],
    };
    assert!(!user.has_entitlement(&Entitlement::ViewCatalog));
    assert!(!user.has_all_entitlements());
}

#[test]
fn user_with_password_hash_has_all_entitlements() {
    let user = User::with_password_hash("admin", "hashed_pw");
    assert!(user.has_all_entitlements());
    assert_eq!(user.password_hash.as_deref(), Some("hashed_pw"));
}

// ── ImportStatus / ImportDecision as_str ───────────────────────────────────

#[test]
fn import_status_as_str() {
    assert_eq!(ImportStatus::Pending.as_str(), "pending");
    assert_eq!(ImportStatus::Running.as_str(), "running");
    assert_eq!(ImportStatus::Processing.as_str(), "processing");
    assert_eq!(ImportStatus::Completed.as_str(), "completed");
    assert_eq!(ImportStatus::Failed.as_str(), "failed");
    assert_eq!(ImportStatus::Skipped.as_str(), "skipped");
}

#[test]
fn import_decision_as_str() {
    assert_eq!(ImportDecision::Imported.as_str(), "imported");
    assert_eq!(ImportDecision::Rejected.as_str(), "rejected");
    assert_eq!(ImportDecision::Skipped.as_str(), "skipped");
    assert_eq!(ImportDecision::Conflict.as_str(), "conflict");
    assert_eq!(ImportDecision::Unmatched.as_str(), "unmatched");
    assert_eq!(ImportDecision::Failed.as_str(), "failed");
}

#[test]
fn import_strategy_as_str() {
    assert_eq!(ImportStrategy::HardLink.as_str(), "hardlink");
    assert_eq!(ImportStrategy::Copy.as_str(), "copy");
}

// ── NewTitle ──────────────────────────────────────────────────────────────

#[test]
fn new_title_with_defaults() {
    let title = NewTitle::with_defaults("Test Movie", MediaFacet::Movie);
    assert_eq!(title.name, "Test Movie");
    assert_eq!(title.facet, MediaFacet::Movie);
    assert!(title.monitored);
    assert!(title.tags.is_empty());
    assert!(title.external_ids.is_empty());
}

// ── parse_query ───────────────────────────────────────────────────────────

#[test]
fn parse_query_trims_and_lowercases() {
    assert_eq!(parse_query("  Cowboy Bebop  "), "cowboy bebop");
    assert_eq!(parse_query("UPPERCASE"), "uppercase");
}

#[test]
fn config_field_type_accepts_secret_alias() {
    assert_eq!(
        ConfigFieldType::parse("secret"),
        Some(ConfigFieldType::Password)
    );
}
