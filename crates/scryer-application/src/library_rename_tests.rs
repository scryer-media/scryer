use super::*;
use std::collections::BTreeMap;

fn tokens(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

// ── render_rename_template ────────────────────────────────────────────────

#[test]
fn render_simple_tokens() {
    let t = tokens(&[("title", "Inception"), ("year", "2010"), ("ext", "mkv")]);
    let result = render_rename_template("{title} ({year}).{ext}", &t);
    assert_eq!(result, "Inception (2010).mkv");
}

#[test]
fn render_full_movie_template() {
    let t = tokens(&[
        ("title", "Inception"),
        ("year", "2010"),
        ("quality", "1080p"),
        ("ext", "mkv"),
    ]);
    let result = render_rename_template("{title} ({year}) - {quality}.{ext}", &t);
    assert_eq!(result, "Inception (2010) - 1080p.mkv");
}

#[test]
fn render_with_zero_padding() {
    let t = tokens(&[
        ("title", "Show"),
        ("season", "2"),
        ("episode", "5"),
        ("ext", "mkv"),
    ]);
    let result = render_rename_template("{title} - S{season:2}E{episode:2}.{ext}", &t);
    assert_eq!(result, "Show - S02E05.mkv");
}

#[test]
fn render_padding_non_numeric_passthrough() {
    let t = tokens(&[("title", "Show"), ("quality", "1080p")]);
    // "1080p" is not purely digits, so padding is skipped
    let result = render_rename_template("{quality:5}", &t);
    assert_eq!(result, "1080p");
}

#[test]
fn render_missing_token_empty() {
    let t = tokens(&[("title", "Movie")]);
    let result = render_rename_template("{title} ({year}).{ext}", &t);
    assert_eq!(result, "Movie ()"); // missing tokens become empty; trailing dot trimmed
}

#[test]
fn render_no_tokens_passthrough() {
    let t = BTreeMap::new();
    let result = render_rename_template("plain text no tokens", &t);
    assert_eq!(result, "plain text no tokens");
}

#[test]
fn render_unclosed_brace_passthrough() {
    let t = tokens(&[("title", "Movie")]);
    let result = render_rename_template("{title} - {unclosed", &t);
    assert_eq!(result, "Movie - {unclosed");
}

#[test]
fn render_token_case_insensitive() {
    let t = tokens(&[("title", "Movie")]);
    let result = render_rename_template("{Title}", &t);
    assert_eq!(result, "Movie");
}

// ── sanitize_filesystem_component ─────────────────────────────────────────

#[test]
fn sanitize_replaces_illegal_chars() {
    let result = sanitize_filesystem_component("movie: title <v2> | test");
    assert!(!result.contains(':'));
    assert!(!result.contains('<'));
    assert!(!result.contains('>'));
    assert!(!result.contains('|'));
}

#[test]
fn sanitize_replaces_slashes() {
    let result = sanitize_filesystem_component("movie/title\\test");
    assert!(!result.contains('/'));
    assert!(!result.contains('\\'));
}

#[test]
fn sanitize_replaces_question_and_asterisk() {
    let result = sanitize_filesystem_component("What? No*Way");
    assert!(!result.contains('?'));
    assert!(!result.contains('*'));
}

#[test]
fn sanitize_preserves_valid_chars() {
    let result = sanitize_filesystem_component("Movie Title (2024) - 1080p.mkv");
    assert_eq!(result, "Movie Title (2024) - 1080p.mkv");
}

// ── collapse_separators ───────────────────────────────────────────────────

#[test]
fn collapse_double_spaces() {
    let result = collapse_separators("movie  title   name");
    assert_eq!(result, "movie title name");
}

#[test]
fn collapse_double_dots() {
    let result = collapse_separators("movie..title...name");
    assert_eq!(result, "movie.title.name");
}

#[test]
fn collapse_double_dashes() {
    let result = collapse_separators("movie--title---name");
    assert_eq!(result, "movie-title-name");
}

#[test]
fn collapse_trims_leading_trailing_separators() {
    let result = collapse_separators("..movie.title..");
    assert_eq!(result, "movie.title");
}

#[test]
fn collapse_mixed_whitespace() {
    let result = collapse_separators("movie \t title");
    // tabs become spaces
    assert!(!result.contains('\t'));
}

// ── resolve_template_token ────────────────────────────────────────────────

#[test]
fn resolve_token_simple() {
    let t = tokens(&[("title", "Movie")]);
    assert_eq!(resolve_template_token(&t, "title"), "Movie");
}

#[test]
fn resolve_token_with_padding() {
    let t = tokens(&[("episode", "3")]);
    assert_eq!(resolve_template_token(&t, "episode:2"), "03");
}

#[test]
fn resolve_token_padding_wider() {
    let t = tokens(&[("episode", "5")]);
    assert_eq!(resolve_template_token(&t, "episode:3"), "005");
}

#[test]
fn resolve_token_already_wide_enough() {
    let t = tokens(&[("episode", "123")]);
    assert_eq!(resolve_template_token(&t, "episode:2"), "123");
}

#[test]
fn resolve_token_missing_returns_empty() {
    let t = BTreeMap::new();
    assert_eq!(resolve_template_token(&t, "missing"), "");
}

// ── build_rename_plan_fingerprint ─────────────────────────────────────────

#[test]
fn fingerprint_deterministic() {
    let items = vec![RenamePlanItem {
        collection_id: None,
        current_path: "/media/movie.mkv".to_string(),
        proposed_path: Some("/media/Movie (2024).mkv".to_string()),
        normalized_filename: Some("Movie (2024).mkv".to_string()),
        collision: false,
        reason_code: "rename".to_string(),
        write_action: RenameWriteAction::Move,
        source_size_bytes: Some(1024),
        source_mtime_unix_ms: Some(1000),
    }];
    let fp1 = build_rename_plan_fingerprint(
        &items,
        "{title} ({year}).{ext}",
        &RenameCollisionPolicy::Skip,
        &RenameMissingMetadataPolicy::FallbackTitle,
    );
    let fp2 = build_rename_plan_fingerprint(
        &items,
        "{title} ({year}).{ext}",
        &RenameCollisionPolicy::Skip,
        &RenameMissingMetadataPolicy::FallbackTitle,
    );
    assert_eq!(fp1, fp2);
    assert!(!fp1.is_empty());
}

#[test]
fn fingerprint_changes_with_different_template() {
    let items = vec![];
    let fp1 = build_rename_plan_fingerprint(
        &items,
        "template_a",
        &RenameCollisionPolicy::Skip,
        &RenameMissingMetadataPolicy::FallbackTitle,
    );
    let fp2 = build_rename_plan_fingerprint(
        &items,
        "template_b",
        &RenameCollisionPolicy::Skip,
        &RenameMissingMetadataPolicy::FallbackTitle,
    );
    assert_ne!(fp1, fp2);
}

#[test]
fn fingerprint_changes_with_different_policy() {
    let items = vec![];
    let fp1 = build_rename_plan_fingerprint(
        &items,
        "template",
        &RenameCollisionPolicy::Skip,
        &RenameMissingMetadataPolicy::FallbackTitle,
    );
    let fp2 = build_rename_plan_fingerprint(
        &items,
        "template",
        &RenameCollisionPolicy::Error,
        &RenameMissingMetadataPolicy::FallbackTitle,
    );
    assert_ne!(fp1, fp2);
}

// ── RenameWriteAction / RenameApplyStatus as_str ──────────────────────────

#[test]
fn write_action_as_str() {
    assert_eq!(RenameWriteAction::Noop.as_str(), "noop");
    assert_eq!(RenameWriteAction::Move.as_str(), "move");
    assert_eq!(RenameWriteAction::Replace.as_str(), "replace");
    assert_eq!(RenameWriteAction::Skip.as_str(), "skip");
    assert_eq!(RenameWriteAction::Error.as_str(), "error");
}

#[test]
fn apply_status_as_str() {
    assert_eq!(RenameApplyStatus::Applied.as_str(), "applied");
    assert_eq!(RenameApplyStatus::Skipped.as_str(), "skipped");
    assert_eq!(RenameApplyStatus::Failed.as_str(), "failed");
}

#[test]
fn collision_policy_as_str() {
    assert_eq!(RenameCollisionPolicy::Skip.as_str(), "skip");
    assert_eq!(RenameCollisionPolicy::Error.as_str(), "error");
    assert_eq!(
        RenameCollisionPolicy::ReplaceIfBetter.as_str(),
        "replace_if_better"
    );
}

#[test]
fn missing_metadata_policy_as_str() {
    assert_eq!(RenameMissingMetadataPolicy::Skip.as_str(), "skip");
    assert_eq!(
        RenameMissingMetadataPolicy::FallbackTitle.as_str(),
        "fallback_title"
    );
}
