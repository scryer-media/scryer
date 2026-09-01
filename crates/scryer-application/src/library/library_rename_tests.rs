use super::*;
use chrono::Utc;
use scryer_domain::{Collection, CollectionType, ExternalId, MediaFacet, Title};
use std::collections::BTreeMap;
use std::path::PathBuf;

fn tokens(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

// ── render_rename_template ────────────────────────────────────────────────

#[test]
fn render_simple_tokens() {
    let t = tokens(&[("title", "Neon Cipher"), ("year", "2010"), ("ext", "mkv")]);
    let result = render_rename_template("{title} ({year}).{ext}", &t);
    assert_eq!(result, "Neon Cipher (2010).mkv");
}

#[test]
fn render_full_movie_template() {
    let t = tokens(&[
        ("title", "Neon Cipher"),
        ("year", "2010"),
        ("quality", "1080p"),
        ("ext", "mkv"),
    ]);
    let result = render_rename_template("{title} ({year}) - {quality}.{ext}", &t);
    assert_eq!(result, "Neon Cipher (2010) - 1080p.mkv");
}

#[test]
fn render_rename_template_literal_braces_with_double_brace_escape() {
    let t = tokens(&[("ext", "mkv")]);
    let result = render_rename_template("{{edition-Directors Cut}}.{ext}", &t);
    assert_eq!(result, "{edition-Directors Cut}.mkv");
}

#[test]
fn render_rename_template_literal_braces_around_resolved_token() {
    let t = tokens(&[("edition", "IMAX"), ("ext", "mkv")]);
    let result = render_rename_template("{{edition-{edition}}}.{ext}", &t);
    assert_eq!(result, "{edition-IMAX}.mkv");
}

#[test]
fn render_rename_template_replaces_token_spaces_with_underscore() {
    let t = tokens(&[("title", "12 Tides a Shore"), ("ext", "mkv")]);
    let result = render_rename_template("{title|space:_}.{ext}", &t);
    assert_eq!(result, "12_Tides_a_Shore.mkv");
}

#[test]
fn render_rename_template_replaces_token_spaces_with_dot() {
    let t = tokens(&[("title", "12 Tides a Shore"), ("ext", "mkv")]);
    let result = render_rename_template("{title|space:.}.{ext}", &t);
    assert_eq!(result, "12.Tides.a.Shore.mkv");
}

#[test]
fn render_rename_template_replaces_token_spaces_with_dash() {
    let t = tokens(&[("title", "12 Tides a Shore"), ("ext", "mkv")]);
    let result = render_rename_template("{title|space:-}.{ext}", &t);
    assert_eq!(result, "12-Tides-a-Shore.mkv");
}

#[test]
fn render_rename_template_removes_token_spaces() {
    let t = tokens(&[("title", "12 Tides a Shore"), ("ext", "mkv")]);
    let result = render_rename_template("{title|space:}.{ext}", &t);
    assert_eq!(result, "12TidesaShore.mkv");
}

#[test]
fn render_rename_template_truncates_token_chars() {
    let t = tokens(&[("title", "Harbor Kestrels"), ("ext", "mkv")]);
    let result = render_rename_template("{title|truncate:6}.{ext}", &t);
    assert_eq!(result, "Harbor.mkv");
}

#[test]
fn render_rename_template_applies_truncate_before_later_filters() {
    let t = tokens(&[("title", "12 Tides a Shore"), ("ext", "mkv")]);
    let result = render_rename_template("{title|truncate:8|space:}.{ext}", &t);
    assert_eq!(result, "12Tides.mkv");
}

#[test]
fn validate_rename_template_accepts_supported_filters_and_literals() {
    validate_rename_template("{title|truncate:64|space:_}.{ext}")
        .expect("filtered rename template is allowed");
    validate_rename_template("{{title|truncate:0}}.{ext}")
        .expect("escaped literal braces are not token filters");
}

#[test]
fn validate_rename_template_for_facet_rejects_unavailable_tokens() {
    validate_rename_template_for_facet(
        "{title} - S{season_order:2}E{episode:2} ({absolute_episode}) - {quality}.{ext}",
        &MediaFacet::Anime,
    )
    .expect("anime episode tokens are allowed");

    let error = validate_rename_template_for_facet(
        "{title} - S{season_order:2}E{episode:2} - {quality}.{ext}",
        &MediaFacet::Movie,
    )
    .expect_err("movie templates cannot use episode tokens");
    assert!(
        error
            .to_string()
            .contains("unsupported rename template token")
    );
}

#[test]
fn validate_rename_template_rejects_invalid_filters_and_tokens() {
    let error = validate_rename_template("{title|truncate:0}.{ext}")
        .expect_err("truncate limit must be positive");
    assert!(
        error
            .to_string()
            .contains("unsupported rename template token")
    );

    let error =
        validate_rename_template("{unknown}.{ext}").expect_err("unknown tokens are rejected");
    assert!(
        error
            .to_string()
            .contains("unsupported rename template token")
    );
}

#[test]
fn render_rename_template_sanitizes_space_filter_in_literal_braces() {
    let t = tokens(&[("ext", "mkv")]);
    let result = render_rename_template("{{title|space:_}}.{ext}", &t);
    assert_eq!(result, "{title space _}.mkv");
}

#[test]
fn render_rename_template_sanitizes_escaped_literal_illegal_chars() {
    let t = tokens(&[("ext", "mkv")]);
    let result = render_rename_template("{{title|space:_}} {{bad:literal}}.{ext}", &t);

    assert!(!result.contains('|'));
    assert!(!result.contains(':'));
    assert_eq!(result, "{title space _} {bad literal}.mkv");
}

#[test]
fn render_rename_template_replaces_token_spaces_inside_literal_braces() {
    let t = tokens(&[("edition", "Directors Cut"), ("ext", "mkv")]);
    let result = render_rename_template("{{edition-{edition|space:_}}}.{ext}", &t);
    assert_eq!(result, "{edition-Directors_Cut}.mkv");
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
fn render_rename_template_optional_group_includes_present_guard() {
    let t = tokens(&[
        ("title", "Quiet Meridian"),
        ("season_order", "0"),
        ("episode", "4"),
        ("absolute_episode", ""),
        ("episode_title", "The Sudden Signal Rehearsal"),
        ("quality", "2160p"),
        ("ext", "mkv"),
    ]);
    let template = "{title} - S{season_order:2}E{episode:2}{?absolute_episode: ({absolute_episode})}{?episode_title: - {episode_title|truncate:64}} - {quality}.{ext}";

    assert_eq!(
        render_rename_template(template, &t),
        "Quiet Meridian - S00E04 - The Sudden Signal Rehearsal - 2160p.mkv"
    );

    let numbered = tokens(&[
        ("title", "Quiet Meridian"),
        ("season_order", "1"),
        ("episode", "1"),
        ("absolute_episode", "1"),
        (
            "episode_title",
            "A title that is deliberately longer than sixty-four characters to verify truncation",
        ),
        ("quality", "1080p"),
        ("ext", "mkv"),
    ]);
    assert_eq!(
        render_rename_template(template, &numbered),
        "Quiet Meridian - S01E01 (1) - A title that is deliberately longer than sixty-four characters t - 1080p.mkv"
    );
}

#[test]
fn render_rename_template_optional_group_supports_literal_braces_and_filters() {
    let t = tokens(&[("edition", "Directors Cut")]);
    let result = render_rename_template("{?edition:{{cut-{edition|space:_}}}}", &t);

    assert_eq!(result, "{cut-Directors_Cut}");
}

#[test]
fn validate_rename_template_optional_groups() {
    validate_rename_template_for_facet(
        "{title}{?absolute_episode: ({absolute_episode})}{?episode_title: - {episode_title|truncate:64}}.{ext}",
        &MediaFacet::Anime,
    )
    .expect("anime optional groups should allow episode tokens and filters");
    validate_rename_template("{?edition:{{literal|else:edition}}}")
        .expect("escaped literal text is not an optional fallback branch");

    let unavailable_guard = validate_rename_template_for_facet(
        "{title}{?absolute_episode: ({absolute_episode})}.{ext}",
        &MediaFacet::Movie,
    )
    .expect_err("movie templates cannot use anime optional guards");
    assert!(
        unavailable_guard
            .to_string()
            .contains("unsupported rename template token")
    );

    let nested = validate_rename_template("{?title: {?edition: ({edition})}}");
    assert!(
        nested
            .expect_err("nested optional groups are unsupported")
            .to_string()
            .contains("does not support nested optional groups")
    );

    let fallback = validate_rename_template("{?title: {title}|else: fallback}");
    assert!(
        fallback
            .expect_err("optional fallback branches are unsupported")
            .to_string()
            .contains("does not support optional-group fallback branches")
    );
}

#[test]
fn render_title_folder_template_trims_empty_year_group() {
    let t = tokens(&[("title", "Movie")]);
    let result = render_title_folder_template("{title} ({year})", &t);
    assert_eq!(result, "Movie");
}

#[test]
fn title_folder_template_optional_group_omits_missing_values() {
    validate_title_folder_template("{title}{?year: ({year})}")
        .expect("title folder optional groups should be supported");
    validate_season_folder_template("{?season:Season {season}}")
        .expect("the required season token can be an optional group guard");

    let without_year = tokens(&[("title", "Movie")]);
    assert_eq!(
        render_title_folder_template("{title}{?year: ({year})}", &without_year),
        "Movie"
    );

    let with_year = tokens(&[("title", "Movie"), ("year", "2004")]);
    assert_eq!(
        render_title_folder_template("{title}{?year: ({year})}", &with_year),
        "Movie (2004)"
    );
}

#[test]
fn render_title_folder_template_literal_braces_around_resolved_token() {
    let t = tokens(&[("edition", "IMAX")]);
    let result = render_title_folder_template("{{edition-{edition}}}", &t);
    assert_eq!(result, "{edition-IMAX}");
}

#[test]
fn render_title_folder_template_literal_braces_around_external_id() {
    let t = tokens(&[("tmdb_id", "123")]);
    let result = render_title_folder_template("{{tmdb-{tmdb_id}}}", &t);
    assert_eq!(result, "{tmdb-123}");
}

#[test]
fn render_title_folder_template_trims_empty_literal_brace_group() {
    let t = tokens(&[]);
    let result = render_title_folder_template("{{{edition}}}", &t);
    assert_eq!(result, "");
}

#[test]
fn render_title_folder_template_replaces_token_spaces() {
    let t = tokens(&[("title", "12 Tides a Shore"), ("year", "2013")]);
    let result = render_title_folder_template("{title|space:_} ({year})", &t);
    assert_eq!(result, "12_Tides_a_Shore (2013)");
}

#[test]
fn render_title_folder_template_truncates_before_replacing_spaces() {
    let t = tokens(&[("title", "12 Tides a Shore"), ("year", "2013")]);
    let result = render_title_folder_template("{title|truncate:8|space:_} ({year})", &t);
    assert_eq!(result, "12_Tides (2013)");
}

#[test]
fn render_rename_template_disarms_reserved_device_filename() {
    let t = tokens(&[("title", "CON"), ("ext", "mkv")]);
    let result = render_rename_template("{title}.{ext}", &t);
    assert_eq!(result, "CON_.mkv");
}

#[test]
fn render_title_folder_template_disarms_reserved_device_prefix() {
    let t = tokens(&[("title", "Com1 Sat"), ("year", "2024")]);
    let result = render_title_folder_template("{title} ({year})", &t);
    assert_eq!(result, "Com1_Sat (2024)");
}

#[test]
fn configured_title_folder_path_disarms_reserved_device_title() {
    let title = test_movie_title("CON");
    let path = configured_title_folder_path("/library", &title, "{title}", None);
    assert_eq!(path, std::path::Path::new("/library/CON_"));
}

#[test]
fn configured_title_folder_path_sanitizes_empty_template_fallback_title() {
    let title = test_movie_title("../Escape");
    let path = configured_title_folder_path("/library", &title, "", None);
    assert_eq!(path, std::path::Path::new("/library/Escape"));
}

#[test]
fn configured_title_folder_path_prefers_title_year_over_parsed_release_year() {
    let mut title = test_movie_title("Movie");
    title.year = Some(2024);
    let path =
        configured_title_folder_path("/library/movies", &title, "{title} ({year})", Some(2025));
    assert_eq!(path, std::path::Path::new("/library/movies/Movie (2024)"));
}

#[test]
fn title_folder_tokens_include_external_ids_and_prefer_normalized_imdb() {
    let mut title = test_movie_title("Movie");
    title.imdb_id = Some("https://www.imdb.com/title/tt0468569/".to_string());
    title.external_ids = vec![
        ExternalId {
            source: "imdb".to_string(),
            value: "tt0000001".to_string(),
        },
        ExternalId {
            source: "TMDB".to_string(),
            value: " 155 ".to_string(),
        },
        ExternalId {
            source: "tvdb".to_string(),
            value: " 123456 ".to_string(),
        },
        ExternalId {
            source: "anidb".to_string(),
            value: " 69 ".to_string(),
        },
        ExternalId {
            source: "mal".to_string(),
            value: " 21 ".to_string(),
        },
        ExternalId {
            source: "anilist".to_string(),
            value: " 21 ".to_string(),
        },
    ];

    let tokens = build_title_folder_tokens(&title, None);

    assert_eq!(tokens.get("imdb_id").map(String::as_str), Some("tt0468569"));
    assert_eq!(tokens.get("tmdb_id").map(String::as_str), Some("155"));
    assert_eq!(tokens.get("tvdb_id").map(String::as_str), Some("123456"));
    assert_eq!(tokens.get("anidb_id").map(String::as_str), Some("69"));
    assert_eq!(tokens.get("mal_id").map(String::as_str), Some("21"));
    assert_eq!(tokens.get("anilist_id").map(String::as_str), Some("21"));
}

#[test]
fn title_folder_template_accepts_external_id_tokens_and_trims_missing_groups() {
    validate_title_folder_template("{title} [{tmdb_id}]").expect("external ID token is allowed");
    validate_title_folder_template("{title} [{tmdb_id:8}]")
        .expect("external ID token padding is allowed");
    validate_title_folder_template("{title|space:_} [{tmdb_id|truncate:4}]")
        .expect("folder token filters are allowed");

    let mut title = test_movie_title("Movie");
    title.external_ids = vec![ExternalId {
        source: "tmdb".to_string(),
        value: "155".to_string(),
    }];
    let tokens = build_title_folder_tokens(&title, None);
    let rendered = render_title_folder_template("{title} [{tmdb_id:8}]", &tokens);

    assert_eq!(rendered, "Movie [00000155]");

    let title = test_movie_title("Movie");
    let tokens = build_title_folder_tokens(&title, None);
    let rendered = render_title_folder_template("{title} [{tmdb_id}]", &tokens);
    assert_eq!(rendered, "Movie");
}

#[test]
fn title_folder_template_rejects_invalid_truncate_filter() {
    let error = validate_title_folder_template("{title|truncate:0}")
        .expect_err("truncate limit must be positive");
    assert!(
        error
            .to_string()
            .contains("unsupported folder template token")
    );

    let error = validate_title_folder_template("{title|truncate:abc}")
        .expect_err("truncate limit must be numeric");
    assert!(
        error
            .to_string()
            .contains("unsupported folder template token")
    );
}

#[test]
fn season_folder_template_requires_season_and_rejects_episode_tokens() {
    validate_season_folder_template("Season {season}")
        .expect("the regular season template should accept the season token");
    validate_season_folder_template("Season {season:0}")
        .expect("zero-width padding should be accepted");
    validate_season_folder_template("{title} S{season:2}")
        .expect("season templates should accept title tokens and numeric padding");
    validate_season_folder_template("{{S{season}}}")
        .expect("season templates should accept escaped literal braces");

    assert!(validate_season_folder_template("Season").is_err());
    assert!(validate_season_folder_template("Season {episode}").is_err());
}

#[test]
fn season_folder_template_rejects_invalid_padding() {
    for template in [
        "Season {season:}",
        "Season {season:abc}",
        "Season {season:2x}",
        "Season {season:241}",
        "Season {season:999999999999999999999999999999999999999}",
    ] {
        assert!(
            validate_season_folder_template(template).is_err(),
            "invalid padding should be rejected: {template}"
        );
    }
}

#[test]
fn folder_templates_reject_illegal_literal_characters() {
    for illegal in ['<', '>', ':', '"', '/', '\\', '|', '?', '*', '\n'] {
        let template = format!("Season{illegal} {{season}}");
        assert!(
            validate_season_folder_template(&template).is_err(),
            "illegal literal should be rejected: {illegal:?}"
        );
    }
}

#[test]
fn specials_folder_template_accepts_literal_and_folder_safe_tokens() {
    validate_specials_folder_template("Specials").expect("literal specials folders are valid");
    validate_specials_folder_template("{title} Specials")
        .expect("specials folders should accept title tokens");

    assert!(validate_specials_folder_template("Specials {quality}").is_err());
}

#[test]
fn render_episode_folder_name_selects_regular_and_specials_templates() {
    let title = test_movie_title("Neon Cipher");

    assert_eq!(
        render_episode_folder_name(&title, 3, "Season {season}", "Specials"),
        "Season 3"
    );
    assert_eq!(
        render_episode_folder_name(&title, 12, "{title|space:.}.S{season:3}", "Specials"),
        "Neon.Cipher.S012"
    );
    assert_eq!(
        render_episode_folder_name(&title, 3, "{{S{season}}}", "Specials"),
        "{S3}"
    );
    assert_eq!(
        render_episode_folder_name(&title, 0, "Season {season}", "{title} Specials"),
        "Neon Cipher Specials"
    );
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
fn sanitize_replaces_ascii_control_chars() {
    let result = sanitize_filesystem_component("Bad\u{0000}Name\u{001f}: Cut");
    assert!(!result.contains('\u{0000}'));
    assert!(!result.contains('\u{001f}'));
    assert!(!result.contains(':'));
    assert_eq!(result, "Bad Name Cut");
}

#[test]
fn sanitize_preserves_valid_chars() {
    let result = sanitize_filesystem_component("Movie Title (2024) - 1080p.mkv");
    assert_eq!(result, "Movie Title (2024) - 1080p.mkv");
}

#[test]
fn sanitize_disarms_exact_windows_reserved_device_names() {
    for (raw, expected) in [
        ("CON", "CON_"),
        ("prn", "prn_"),
        ("AUX", "AUX_"),
        ("NUL", "NUL_"),
        ("COM1", "COM1_"),
        ("LPT9", "LPT9_"),
    ] {
        assert_eq!(sanitize_filesystem_component(raw), expected);
    }
}

#[test]
fn sanitize_disarms_reserved_device_extension_forms() {
    for (raw, expected) in [("CON.mkv", "CON_.mkv"), ("aux.srt", "aux_.srt")] {
        assert_eq!(sanitize_filesystem_component(raw), expected);
    }
}

#[test]
fn sanitize_disarms_reserved_device_leading_tokens() {
    for (raw, expected) in [
        ("Con Game", "Con_Game"),
        ("Com1 Sat", "Com1_Sat"),
        ("LPT9 - Finale.mkv", "LPT9_Finale.mkv"),
    ] {
        assert_eq!(sanitize_filesystem_component(raw), expected);
    }
}

#[test]
fn sanitize_leaves_non_reserved_device_prefixes_unchanged() {
    for value in ["Conan", "Comedy", "COM10", "LPT10", "Auxiliary"] {
        assert_eq!(sanitize_filesystem_component(value), value);
    }
}

#[test]
fn truncate_generated_filename_preserves_extension_and_utf8_byte_budget() {
    let long_stem = "長".repeat(120);
    let filename = format!("{long_stem}.mkv");
    let result = truncate_generated_filename_component(&filename);

    assert!(result.ends_with(".mkv"));
    assert!(
        result.len() <= GENERATED_COMPONENT_MAX_BYTES - GENERATED_COMPONENT_SUFFIX_RESERVE_BYTES,
        "truncated filename exceeded budget: {} bytes",
        result.len()
    );
    assert!(std::str::from_utf8(result.as_bytes()).is_ok());
}

#[test]
fn finalize_generated_filename_sanitizes_fallback_title_before_join() {
    let result = finalize_generated_filename_component("../Unsafe\\Title:Name.mkv");

    assert!(!result.contains('/'));
    assert!(!result.contains('\\'));
    assert!(!result.contains(':'));
    assert_eq!(result, "Unsafe Title Name.mkv");
}

#[cfg(windows)]
#[test]
fn configured_title_folder_path_truncates_generated_folder_component() {
    let title = test_movie_title(&"Long ".repeat(100));
    let path = configured_title_folder_path("/library", &title, "{title}", None);
    let folder = path.file_name().unwrap().to_string_lossy();

    assert!(
        folder.len() <= GENERATED_COMPONENT_MAX_BYTES - GENERATED_COMPONENT_SUFFIX_RESERVE_BYTES,
        "truncated folder exceeded budget: {} bytes",
        folder.len()
    );
}

#[cfg(not(windows))]
#[test]
fn configured_title_folder_path_caps_long_component() {
    let title_name = format!("{}Long", "Long ".repeat(99));
    let title = test_movie_title(&title_name);
    let path = configured_title_folder_path("/library", &title, "{title}", None);
    let expected = truncate_generated_folder_component(&title_name);

    assert_eq!(
        path.file_name().and_then(|folder| folder.to_str()),
        Some(expected.as_str())
    );
}

#[cfg(unix)]
#[test]
fn infer_title_folder_path_after_rename_decodes_stored_paths() {
    use std::os::unix::ffi::OsStringExt;

    let existing_root = PathBuf::from(std::ffi::OsString::from_vec(
        b"/library/old-\xFF-root".to_vec(),
    ));
    let current_path = existing_root.join("Season 01").join("Episode.mkv");
    let final_path = PathBuf::from("/library/new-root/Season 01/Episode.mkv");
    let mut title = test_movie_title("Encoded Root");
    title.folder_path = Some(path_to_stored_string(&existing_root));

    let inferred = infer_title_folder_path_after_rename(
        &title,
        true,
        &path_to_stored_string(&current_path),
        &path_to_stored_string(&final_path),
    )
    .expect("infer folder path");

    assert_eq!(
        stored_path_to_path_buf(&inferred),
        PathBuf::from("/library/new-root")
    );
}

#[test]
fn episode_rename_parent_uses_configured_season_folder_template() {
    let mut title = test_movie_title("Test Show");
    title.facet = MediaFacet::Series;
    title.folder_path = Some("/library/old/Test Show".to_string());
    let current_file = std::path::Path::new("/library/old/Test Show/Season 01/Episode.mkv");

    let regular = episode_parent_path_for_renamed_file(
        &title,
        true,
        current_file,
        "/library/series",
        "{title}",
        Some(3),
        "{title|space:.}.S{season:2}",
        "Extras",
    );
    assert_eq!(
        regular,
        PathBuf::from("/library/series/Test Show/Test.Show.S03")
    );

    let specials = episode_parent_path_for_renamed_file(
        &title,
        true,
        current_file,
        "/library/series",
        "{title}",
        Some(0),
        "Season {season}",
        "Extras",
    );
    assert_eq!(specials, PathBuf::from("/library/series/Test Show/Extras"));

    title.tags = vec!["scryer:season-folder:disabled".to_string()];
    let flat = episode_parent_path_for_renamed_file(
        &title,
        false,
        current_file,
        "/library/series",
        "{title}",
        Some(3),
        "Season {season}",
        "Extras",
    );
    assert_eq!(flat, PathBuf::from("/library/series/Test Show"));
}

#[cfg(not(windows))]
#[test]
fn rename_planning_path_key_preserves_case_on_non_windows() {
    assert_ne!(
        rename_planning_path_key("/media/Movie.mkv"),
        rename_planning_path_key("/media/movie.mkv")
    );
}

#[cfg(windows)]
#[test]
fn rename_planning_path_key_folds_case_and_separators_on_windows() {
    assert_eq!(
        rename_planning_path_key(r"C:\Media\Movie.mkv"),
        rename_planning_path_key("C:/media/movie.mkv")
    );
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
        media_file_id: None,
        series_movie_link_ids: Vec::new(),
        current_path: "/data/movie.mkv".to_string(),
        proposed_path: Some("/data/Movie (2024).mkv".to_string()),
        normalized_filename: Some("Movie (2024).mkv".to_string()),
        collision: false,
        reason_code: "rename".to_string(),
        write_action: RenameWriteAction::Move,
        source_size_bytes: Some(1024),
        source_mtime_unix_ms: Some(1000),
    }];
    let fp1 = build_rename_plan_fingerprint(
        &items,
        "{title}.{ext}",
        &RenameCollisionPolicy::Skip,
        &RenameMissingMetadataPolicy::FallbackTitle,
    );
    let fp2 = build_rename_plan_fingerprint(
        &items,
        "{title}.{ext}",
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

fn test_movie_title(name: &str) -> Title {
    Title {
        id: "title-1".to_string(),
        name: name.to_string(),
        facet: MediaFacet::Movie,
        library_id: scryer_domain::default_library_id_for_facet(&MediaFacet::Movie),
        root_folder_id: scryer_domain::root_folder_id_for_path("/data/test"),
        monitored: true,
        tags: vec![],
        canonical_tags: vec![],
        external_ids: vec![],
        created_by: None,
        created_at: Utc::now(),
        year: Some(2024),
        overview: None,
        poster_url: None,
        poster_source_url: None,
        background_url: None,
        background_source_url: None,
        sort_title: None,
        catalog_sort_key: String::new(),
        slug: None,
        imdb_id: None,
        runtime_minutes: None,
        popularity: None,
        content_status: None,
        language: None,
        first_aired: None,
        network: None,
        studio: None,
        country: None,
        aliases: vec![],
        tagged_aliases: vec![],
        metadata_language: None,
        metadata_fetched_at: None,
        min_availability: None,
        digital_release_date: None,
        folder_path: None,
    }
}

fn test_movie_collection(path: &str) -> Collection {
    Collection {
        id: "collection-1".to_string(),
        title_id: "title-1".to_string(),
        collection_type: CollectionType::Movie,
        collection_index: "1".to_string(),
        label: Some("720p".to_string()),
        ordered_path: Some(path.to_string()),
        narrative_order: None,
        first_episode_number: None,
        last_episode_number: None,
        monitored: true,
        created_at: Utc::now(),
    }
}

fn test_media_file(path: &str) -> TitleMediaFile {
    TitleMediaFile {
        id: "media-1".to_string(),
        title_id: "title-1".to_string(),
        episode_id: None,
        series_movie_link_ids: Vec::new(),
        role: crate::MediaFileRole::Primary,
        file_path: path.to_string(),
        size_bytes: 1_000,
        announced_size_bytes: None,
        source_signature_scheme: None,
        source_signature_value: None,
        content_hashes: None,
        quality_label: Some("720p".to_string()),
        scan_status: "scanned".to_string(),
        created_at: "2026-04-11T00:00:00Z".to_string(),
        video_codec: Some(crate::release_parser::VideoCodec::H265),
        video_width: Some(3840),
        video_height: Some(2160),
        video_bitrate_kbps: Some(15_000),
        video_bit_depth: Some(10),
        video_hdr_format: Some("HDR10".to_string()),
        dovi_profile: None,
        dovi_bl_compat_id: None,
        video_frame_rate: Some("23.976".to_string()),
        video_profile: Some("Main 10".to_string()),
        audio_codec: Some("dts".to_string()),
        audio_profile: Some("DTS-HD MA + DTS:X IMAX".to_string()),
        audio_channels: Some(8),
        audio_bitrate_kbps: Some(4_000),
        audio_languages: vec!["eng".to_string()],
        audio_streams: vec![crate::AudioStreamDetail {
            codec: Some("dts".to_string()),
            profile: Some("DTS-HD MA + DTS:X IMAX".to_string()),
            channels: Some(8),
            language: Some("eng".to_string()),
            name: None,
            bitrate_kbps: Some(4_000),
        }],
        subtitle_languages: vec![],
        subtitle_codecs: vec![],
        subtitle_streams: vec![],
        has_multiaudio: false,
        duration_seconds: Some(7200),
        num_chapters: Some(12),
        container_format: Some("matroska".to_string()),
        scene_name: None,
        release_group: Some("FraMeSToR".to_string()),
        source_type: Some("BluRay".to_string()),
        resolution: Some("2160p".to_string()),
        video_codec_parsed: Some(crate::release_parser::VideoCodec::H264),
        audio_codec_parsed: Some("AAC".to_string()),
        audio_channels_parsed: Some("2.0".to_string()),
        acquisition_score: None,
        scoring_log: None,
        indexer_source: None,
        grabbed_release_title: None,
        grabbed_at: None,
        edition: Some("IMAX Enhanced".to_string()),
        original_file_path: None,
        release_hash: None,
    }
}

#[test]
fn resolve_rename_common_metadata_prefers_analysis_over_parsed_metadata() {
    let media_file = test_media_file("/library/Movie.2024.720p.WEB-DL.AAC2.0.H.264-Parsed.mkv");
    let parsed = parse_release_metadata("Movie.2024.720p.WEB-DL.AAC2.0.H.264-Parsed");

    let resolved =
        resolve_rename_common_metadata(Some(&media_file), &parsed, "Movie", Some("2024"), "mkv");

    assert_eq!(resolved.common.quality, "2160p");
    assert_eq!(resolved.common.source, "BluRay");
    assert_eq!(resolved.common.video_codec, "H.265");
    assert_eq!(resolved.common.audio_codec, "DTS:X");
    assert_eq!(resolved.common.audio_channels, "7.1");
    assert_eq!(resolved.common.group, "FraMeSToR");
    assert_eq!(resolved.edition, "IMAX Enhanced");
}

#[test]
fn resolve_rename_common_metadata_uses_persisted_parsed_backup_when_analysis_missing() {
    let mut media_file = test_media_file("/library/Movie.2024.mkv");
    media_file.video_codec = None;
    media_file.video_width = None;
    media_file.video_height = None;
    media_file.audio_codec = None;
    media_file.audio_profile = None;
    media_file.audio_channels = None;
    media_file.audio_streams.clear();
    media_file.source_type = Some("WEB-DL".to_string());
    media_file.release_group = Some("NTb".to_string());
    media_file.video_codec_parsed = Some(crate::release_parser::VideoCodec::H265);
    media_file.audio_codec_parsed = Some("TrueHD Atmos".to_string());
    media_file.audio_channels_parsed = Some("7.1".to_string());
    media_file.quality_label = Some("1080p".to_string());
    let parsed = parse_release_metadata("Movie");

    let resolved =
        resolve_rename_common_metadata(Some(&media_file), &parsed, "Movie", Some("2024"), "mkv");

    assert_eq!(resolved.common.quality, "1080p");
    assert_eq!(resolved.common.source, "WEB-DL");
    assert_eq!(resolved.common.video_codec, "H.265");
    assert_eq!(resolved.common.audio_codec, "TrueHD Atmos");
    assert_eq!(resolved.common.audio_channels, "7.1");
    assert_eq!(resolved.common.group, "NTb");
}

#[test]
fn movie_rename_items_use_matched_media_file_analysis_instead_of_path_parse() {
    let dir = tempfile::tempdir().expect("tempdir");
    let current_path = dir
        .path()
        .join("Movie.2024.720p.WEB-DL.AAC2.0.H.264-Parsed.mkv");
    std::fs::write(&current_path, b"movie").expect("seed movie file");
    let current_path = current_path.to_string_lossy().to_string();

    let title = test_movie_title("Movie (2024)");
    let collection = test_movie_collection(&current_path);
    let media_file = test_media_file(&current_path);
    let mut planning = RenamePlanningState::default();
    let mut options = MovieRenamePlanOptions {
        media_root: dir.path().to_str().expect("tempdir path"),
        folder_template: "{title} ({year})",
        template: "{title} ({year}) [{quality} {video_codec} {audio_codec} {audio_channels}].{ext}",
        missing_metadata_policy: &RenameMissingMetadataPolicy::FallbackTitle,
        planning: &mut planning,
    };

    let items = build_movie_rename_plan_items(
        &title,
        vec![collection],
        vec![media_file.clone()],
        &mut options,
    );

    assert_eq!(items.len(), 1);
    assert_eq!(
        items[0].media_file_id.as_deref(),
        Some(media_file.id.as_str())
    );
    assert_eq!(
        items[0].normalized_filename.as_deref(),
        Some("Movie (2024) [2160p H.265 DTS X 7.1].mkv")
    );
}

#[test]
fn movie_rename_items_render_external_id_tokens() {
    let dir = tempfile::tempdir().expect("tempdir");
    let current_path = dir.path().join("Movie.2024.1080p.BluRay.x264-GROUP.mkv");
    std::fs::write(&current_path, b"movie").expect("seed movie file");
    let current_path = current_path.to_string_lossy().to_string();

    let mut title = test_movie_title("Movie (2024)");
    title.imdb_id = Some("0468569".to_string());
    title.external_ids = vec![
        ExternalId {
            source: "tmdb".to_string(),
            value: "155".to_string(),
        },
        ExternalId {
            source: "tvdb".to_string(),
            value: "123456".to_string(),
        },
        ExternalId {
            source: "anidb".to_string(),
            value: "69".to_string(),
        },
        ExternalId {
            source: "mal".to_string(),
            value: "21".to_string(),
        },
        ExternalId {
            source: "anilist".to_string(),
            value: "21".to_string(),
        },
    ];
    let collection = test_movie_collection(&current_path);
    let media_file = test_media_file(&current_path);
    let mut planning = RenamePlanningState::default();
    let mut options = MovieRenamePlanOptions {
        media_root: dir.path().to_str().expect("tempdir path"),
        folder_template: "{title} ({year}) [{tmdb_id}]",
        template: "{title} ({year}) [{imdb_id} {tmdb_id} {tvdb_id} {anidb_id} {mal_id} {anilist_id}].{ext}",
        missing_metadata_policy: &RenameMissingMetadataPolicy::FallbackTitle,
        planning: &mut planning,
    };

    let items =
        build_movie_rename_plan_items(&title, vec![collection], vec![media_file], &mut options);

    assert_eq!(items.len(), 1);
    assert_eq!(
        items[0].normalized_filename.as_deref(),
        Some("Movie (2024) [tt0468569 155 123456 69 21 21].mkv")
    );
}

#[test]
fn movie_rename_items_use_saved_hydrated_localized_title_name() {
    let dir = tempfile::tempdir().expect("tempdir");
    let current_path = dir.path().join("Sandline.2021.1080p.BluRay.x264-GROUP.mkv");
    std::fs::write(&current_path, b"movie").expect("seed movie file");
    let current_path = current_path.to_string_lossy().to_string();

    let mut title = test_movie_title("サンドライン");
    title.year = Some(2021);
    title.aliases = vec!["Sandline".to_string()];
    title.metadata_language = Some("jpn".to_string());
    let collection = test_movie_collection(&current_path);
    let media_file = test_media_file(&current_path);
    let mut planning = RenamePlanningState::default();
    let mut options = MovieRenamePlanOptions {
        media_root: dir.path().to_str().expect("tempdir path"),
        folder_template: "{title} ({year})",
        template: "{title}.{ext}",
        missing_metadata_policy: &RenameMissingMetadataPolicy::FallbackTitle,
        planning: &mut planning,
    };

    let items =
        build_movie_rename_plan_items(&title, vec![collection], vec![media_file], &mut options);

    assert_eq!(items.len(), 1);
    assert_eq!(
        items[0].normalized_filename.as_deref(),
        Some("サンドライン.mkv")
    );
    assert!(
        items[0]
            .proposed_path
            .as_deref()
            .is_some_and(|path| path.ends_with("/サンドライン.mkv"))
    );
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

/// An SMB share hands back decomposed names for files written precomposed, so
/// without normalization every accented title plans a rename forever, and each
/// pass does real work over the network.
#[test]
fn rename_planning_key_ignores_unicode_form() {
    let precomposed = "/media/TV/Pok\u{e9}mon/Pok\u{e9}mon - S20E01.mkv";
    let decomposed = "/media/TV/Poke\u{301}mon/Poke\u{301}mon - S20E01.mkv";
    assert_ne!(precomposed, decomposed, "the spellings differ as bytes");
    assert_eq!(
        rename_planning_path_key(precomposed),
        rename_planning_path_key(decomposed)
    );
}

#[test]
fn rename_planning_key_still_separates_different_paths() {
    assert_ne!(
        rename_planning_path_key("/media/TV/Show/one.mkv"),
        rename_planning_path_key("/media/TV/Show/two.mkv")
    );
}
