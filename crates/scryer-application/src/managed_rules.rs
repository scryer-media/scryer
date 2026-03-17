//! Rego source generation for managed convenience rules.
//!
//! Each function produces a complete Rego module that the rules engine
//! evaluates identically to user-authored rules. The generated code is
//! stored in the `rego_source` column of `rule_sets` with `is_managed = true`.

use std::fmt::Write;

// ─── Required audio language ─────────────────────────────────────────────────

/// Generate a facet-level "required audio language" Rego rule.
///
/// - `languages`: ISO 639-2/3 codes the user requires (e.g. `["eng"]`).
/// - `excepted_title_ids`: titles that have their own override and should
///   be skipped by this facet rule.
///
/// Pre-download: only blocks when the release parser detected languages
/// AND the required one is missing. When no languages are detected in the
/// release name, the rule passes (fail-open).
///
/// Post-download: ffprobe always populates `audio_languages`, so the check
/// is unconditional.
pub fn generate_required_audio_rego(languages: &[String], excepted_title_ids: &[String]) -> String {
    assert!(!languages.is_empty(), "languages must not be empty");
    let mut out = String::with_capacity(512);
    out.push_str("import rego.v1\n\n");

    // Exception set for titles with their own override
    if !excepted_title_ids.is_empty() {
        out.push_str("_excepted_titles := {");
        for (i, id) in excepted_title_ids.iter().enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            write!(out, "\"{}\"", id).unwrap();
        }
        out.push_str("}\n\n");
        out.push_str("_is_excepted if {\n");
        out.push_str("    input.context.title_id == _excepted_titles[_]\n");
        out.push_str("}\n\n");
    }

    let exception_guard = if excepted_title_ids.is_empty() {
        ""
    } else {
        "    not _is_excepted\n"
    };

    // Build the language match set
    let lang_set = languages
        .iter()
        .map(|l| format!("\"{}\"", l.trim().to_ascii_lowercase()))
        .collect::<Vec<_>>()
        .join(", ");

    out.push_str("_required_langs := {");
    out.push_str(&lang_set);
    out.push_str("}\n\n");

    // Pre-download: check release parser languages (fail-open when none detected)
    out.push_str("score_entry[\"managed_required_audio_missing\"] := scryer.block_score() if {\n");
    out.push_str(exception_guard);
    out.push_str("    count(input.release.languages_audio) > 0\n");
    out.push_str("    _missing_pre := _required_langs - {lower(lang) | some lang in input.release.languages_audio}\n");
    out.push_str("    count(_missing_pre) > 0\n");
    out.push_str("}\n\n");

    // Post-download: ffprobe data is always available
    out.push_str(
        "score_entry[\"managed_required_audio_missing_file\"] := scryer.block_score() if {\n",
    );
    out.push_str(exception_guard);
    out.push_str("    input.file != null\n");
    out.push_str("    _missing_post := _required_langs - {lower(lang) | some lang in input.file.audio_languages}\n");
    out.push_str("    count(_missing_post) > 0\n");
    out.push_str("}\n");

    out
}

/// Generate a title-scoped "required audio language" Rego rule.
///
/// Only fires for the specific title. Used when a title overrides the
/// facet default with its own language requirement.
pub fn generate_title_required_audio_rego(title_id: &str, languages: &[String]) -> String {
    assert!(!languages.is_empty(), "languages must not be empty");
    let mut out = String::with_capacity(512);
    out.push_str("import rego.v1\n\n");

    let lang_set = languages
        .iter()
        .map(|l| format!("\"{}\"", l.trim().to_ascii_lowercase()))
        .collect::<Vec<_>>()
        .join(", ");

    out.push_str("_required_langs := {");
    out.push_str(&lang_set);
    out.push_str("}\n\n");

    // Pre-download
    out.push_str("score_entry[\"managed_required_audio_missing\"] := scryer.block_score() if {\n");
    writeln!(out, "    input.context.title_id == \"{}\"", title_id).unwrap();
    out.push_str("    count(input.release.languages_audio) > 0\n");
    out.push_str("    _missing_pre := _required_langs - {lower(lang) | some lang in input.release.languages_audio}\n");
    out.push_str("    count(_missing_pre) > 0\n");
    out.push_str("}\n\n");

    // Post-download
    out.push_str(
        "score_entry[\"managed_required_audio_missing_file\"] := scryer.block_score() if {\n",
    );
    writeln!(out, "    input.context.title_id == \"{}\"", title_id).unwrap();
    out.push_str("    input.file != null\n");
    out.push_str("    _missing_post := _required_langs - {lower(lang) | some lang in input.file.audio_languages}\n");
    out.push_str("    count(_missing_post) > 0\n");
    out.push_str("}\n");

    out
}

// ─── Prefer dual audio ───────────────────────────────────────────────────────

/// Generate a "prefer dual audio" scoring rule.
///
/// Awards a bonus when `is_dual_audio` is true. No penalty when false —
/// this is a preference, not a requirement.
pub fn generate_prefer_dual_audio_rego() -> String {
    "\
import rego.v1

score_entry[\"managed_dual_audio_preferred\"] := 200 if {
    input.release.is_dual_audio
}
"
    .to_string()
}

// ─── Managed key helpers ─────────────────────────────────────────────────────

pub fn managed_key_required_audio(scope: &str) -> String {
    format!("convenience:required-audio:{scope}")
}

pub fn managed_key_required_audio_title(title_id: &str) -> String {
    format!("convenience:required-audio:title:{title_id}")
}

pub fn managed_key_prefer_dual_audio(scope: &str) -> String {
    format!("convenience:prefer-dual-audio:{scope}")
}

/// Prefix for finding all title-level overrides under a facet's required audio.
pub const MANAGED_KEY_REQUIRED_AUDIO_TITLE_PREFIX: &str = "convenience:required-audio:title:";

// ─── Display names for UI ────────────────────────────────────────────────────

/// Human-readable name for a managed rule, derived from its managed_key.
pub fn managed_rule_display_name(managed_key: &str) -> String {
    if let Some(rest) = managed_key.strip_prefix("convenience:required-audio:title:") {
        format!(
            "Required Audio (title override: {})",
            &rest[..8.min(rest.len())]
        )
    } else if let Some(scope) = managed_key.strip_prefix("convenience:required-audio:") {
        let scope_label = match scope {
            "global" => "All",
            "movie" => "Movies",
            "series" => "Series",
            "anime" => "Anime",
            other => other,
        };
        format!("Required Audio ({scope_label})")
    } else if let Some(scope) = managed_key.strip_prefix("convenience:prefer-dual-audio:") {
        let scope_label = match scope {
            "global" => "All",
            "movie" => "Movies",
            "series" => "Series",
            "anime" => "Anime",
            other => other,
        };
        format!("Prefer Dual Audio ({scope_label})")
    } else {
        format!("Managed Rule ({managed_key})")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn required_audio_single_language_no_exceptions() {
        let rego = generate_required_audio_rego(&["eng".into()], &[]);
        assert!(rego.contains("import rego.v1"));
        assert!(rego.contains("_required_langs := {\"eng\"}"));
        assert!(rego.contains("managed_required_audio_missing"));
        assert!(rego.contains("managed_required_audio_missing_file"));
        assert!(!rego.contains("_excepted_titles"));
        assert!(!rego.contains("_is_excepted"));
    }

    #[test]
    fn required_audio_multiple_languages() {
        let rego = generate_required_audio_rego(&["eng".into(), "jpn".into()], &[]);
        assert!(rego.contains("\"eng\", \"jpn\"") || rego.contains("\"jpn\", \"eng\""));
    }

    #[test]
    fn required_audio_with_exceptions() {
        let rego =
            generate_required_audio_rego(&["eng".into()], &["abc-123".into(), "def-456".into()]);
        assert!(rego.contains("_excepted_titles := {\"abc-123\", \"def-456\"}"));
        assert!(rego.contains("not _is_excepted"));
    }

    #[test]
    fn title_required_audio() {
        let rego = generate_title_required_audio_rego("my-title-id", &["jpn".into()]);
        assert!(rego.contains("input.context.title_id == \"my-title-id\""));
        assert!(rego.contains("_required_langs := {\"jpn\"}"));
    }

    #[test]
    fn prefer_dual_audio() {
        let rego = generate_prefer_dual_audio_rego();
        assert!(rego.contains("managed_dual_audio_preferred"));
        assert!(rego.contains("input.release.is_dual_audio"));
    }

    #[test]
    fn managed_key_formats() {
        assert_eq!(
            managed_key_required_audio("anime"),
            "convenience:required-audio:anime"
        );
        assert_eq!(
            managed_key_required_audio_title("abc"),
            "convenience:required-audio:title:abc"
        );
        assert_eq!(
            managed_key_prefer_dual_audio("global"),
            "convenience:prefer-dual-audio:global"
        );
    }

    #[test]
    fn display_names() {
        assert_eq!(
            managed_rule_display_name("convenience:required-audio:anime"),
            "Required Audio (Anime)"
        );
        assert_eq!(
            managed_rule_display_name("convenience:required-audio:title:abc-123-def"),
            "Required Audio (title override: abc-123-)"
        );
        assert_eq!(
            managed_rule_display_name("convenience:prefer-dual-audio:global"),
            "Prefer Dual Audio (All)"
        );
    }
}
