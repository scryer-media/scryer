use crate::{
    builtins, BuiltinScoreDoc, ContextDoc, FileDoc, ProfileDoc, ReleaseDoc, RulesError,
    UserRuleInput,
};
use regorus::{Engine, Value};

/// Result of validating a user-authored Rego rule.
#[derive(Debug, Clone)]
pub struct ValidationResult {
    pub valid: bool,
    pub errors: Vec<String>,
}

impl ValidationResult {
    pub fn valid() -> Self {
        Self {
            valid: true,
            errors: Vec::new(),
        }
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self {
            valid: false,
            errors: vec![message.into()],
        }
    }
}

/// Validate a user-authored Rego rule without persisting it.
///
/// The caller is expected to have already called `rewrite_package_declaration`
/// on the source so the package line matches `rule_set_id`.
///
/// Checks:
/// 1. Package declaration matches `scryer.rules.user.<rule_set_id>`.
/// 2. Source compiles without errors.
/// 3. Dry-run against synthetic input succeeds.
/// 4. Output shape is a map of string keys to integer values.
pub fn validate_user_rule(
    rego_source: &str,
    rule_set_id: &str,
) -> Result<ValidationResult, RulesError> {
    let expected_pkg = format!("package scryer.rules.user.{rule_set_id}");

    // Check package declaration
    let has_pkg = rego_source.lines().any(|line| line.trim() == expected_pkg);
    if !has_pkg {
        return Ok(ValidationResult::invalid(format!(
            "package declaration must be: {expected_pkg}"
        )));
    }

    // Compile in a throwaway engine
    let mut engine = Engine::new();
    builtins::register_builtins(&mut engine);

    let policy_path = format!("user/{rule_set_id}.rego");
    if let Err(e) = engine.add_policy(policy_path, rego_source.to_string()) {
        return Ok(ValidationResult::invalid(format!("compilation error: {e}")));
    }

    // Dry-run against synthetic input
    let test_input = synthetic_test_input();
    let input_value = serde_json::to_value(&test_input).map_err(RulesError::Serialization)?;
    engine.set_input(input_value.into());

    let query = format!("data.scryer.rules.user.{rule_set_id}.score_entry");
    match engine.eval_query(query, false) {
        Ok(results) => {
            let value = results
                .result
                .first()
                .and_then(|r| r.expressions.first())
                .map(|e| &e.value);

            if let Some(v) = value {
                if let Err(e) = validate_score_entry_shape(v) {
                    return Ok(ValidationResult::invalid(format!("output error: {e}")));
                }
            }
            Ok(ValidationResult::valid())
        }
        Err(e) => Ok(ValidationResult::invalid(format!("runtime error: {e}"))),
    }
}

/// Verify that the evaluation result is a map of string → integer.
/// Floats and out-of-range values are rejected.
fn validate_score_entry_shape(value: &Value) -> Result<(), String> {
    // Value::Undefined means the rule conditions weren't met — valid (no entries)
    if matches!(value, Value::Undefined) {
        return Ok(());
    }

    let obj = value.as_object().map_err(|_| {
        "score_entry must produce an object (map), not a scalar or array".to_string()
    })?;

    for (key, val) in obj.iter() {
        if key.as_string().is_err() {
            return Err(format!("score_entry keys must be strings, got: {key:?}"));
        }
        let key_str = key
            .as_string()
            .map(|s| s.to_string())
            .unwrap_or_else(|_| "?".to_string());

        if let Ok(n) = val.as_i64() {
            if i32::try_from(n).is_err() {
                return Err(format!(
                    "score_entry value for {key_str:?} is out of i32 range: {n}"
                ));
            }
        } else if val.as_f64().is_ok() {
            return Err(format!(
                "score_entry values must be integers, got float for key {key_str:?}. \
                 Use round() or ceil() to convert."
            ));
        } else {
            return Err(format!(
                "score_entry values must be integers, got: {val:?} for key {key_str:?}"
            ));
        }
    }

    Ok(())
}

/// Build a representative input for validation dry-runs.
fn synthetic_test_input() -> UserRuleInput {
    UserRuleInput {
        release: ReleaseDoc {
            raw_title: "Test.Movie.2024.2160p.WEB-DL.H.265.DDP.5.1".to_string(),
            quality: Some("2160P".to_string()),
            source: Some("WEB-DL".to_string()),
            video_codec: Some("H.265".to_string()),
            audio: Some("DDP".to_string()),
            audio_codecs: vec!["DDP".to_string()],
            audio_channels: Some("5.1".to_string()),
            languages_audio: vec!["eng".to_string()],
            languages_subtitles: vec!["eng".to_string()],
            is_dual_audio: false,
            is_atmos: false,
            is_dolby_vision: false,
            detected_hdr: false,
            is_remux: false,
            is_bd_disk: false,
            is_proper_upload: false,
            is_repack: false,
            is_ai_enhanced: false,
            is_hardcoded_subs: false,
            is_hdr10plus: false,
            is_hlg: false,
            streaming_service: None,
            edition: None,
            anime_version: None,
            release_group: Some("TestGroup".to_string()),
            year: Some(2024),
            parse_confidence: 0.9,
            size_bytes: Some(8_000_000_000),
            age_days: Some(5),
            thumbs_up: Some(10),
            thumbs_down: Some(0),
            extra: Default::default(),
        },
        profile: ProfileDoc {
            id: "test".to_string(),
            name: "Test".to_string(),
            quality_tiers: vec!["2160P".to_string(), "1080P".to_string(), "720P".to_string()],
            archival_quality: Some("2160P".to_string()),
            allow_unknown_quality: false,
            source_allowlist: vec![],
            source_blocklist: vec![],
            video_codec_allowlist: vec![],
            video_codec_blocklist: vec![],
            audio_codec_allowlist: vec![],
            audio_codec_blocklist: vec![],
            atmos_preferred: false,
            dolby_vision_allowed: true,
            detected_hdr_allowed: true,
            prefer_remux: false,
            allow_bd_disk: false,
            allow_upgrades: true,
            prefer_dual_audio: false,
            required_audio_languages: vec![],
        },
        context: ContextDoc {
            title_id: Some("tt0000000".to_string()),
            media_type: "movie".to_string(),
            category: "movie".to_string(),
            tags: vec![],
            has_existing_file: false,
            existing_score: None,
            search_mode: "auto".to_string(),
            runtime_minutes: Some(120),
            is_anime: false,
            is_filler: false,
        },
        builtin_score: BuiltinScoreDoc {
            total: 3200,
            blocked: false,
            codes: vec!["quality_tier_0".to_string(), "source_webdl".to_string()],
        },
        file: Some(FileDoc {
            video_codec: Some("hevc".to_string()),
            video_width: Some(3840),
            video_height: Some(2160),
            video_bitrate_kbps: Some(40000),
            video_bit_depth: Some(10),
            video_hdr_format: Some("HDR10".to_string()),
            audio_codec: Some("eac3".to_string()),
            audio_channels: Some(6),
            audio_languages: vec!["eng".to_string()],
            subtitle_languages: vec!["eng".to_string()],
            has_multiaudio: false,
            duration_seconds: Some(7200),
            container_format: Some("matroska".to_string()),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_rule_passes_validation() {
        let source = r#"
            package scryer.rules.user.test_rule
            import rego.v1

            score_entry["bonus"] := 100
        "#;
        let result = validate_user_rule(source, "test_rule").unwrap();
        assert!(result.valid, "errors: {:?}", result.errors);
    }

    #[test]
    fn wrong_package_name_rejected() {
        let source = r#"
            package scryer.rules.user.wrong_name
            import rego.v1

            score_entry["bonus"] := 100
        "#;
        let result = validate_user_rule(source, "expected_name").unwrap();
        assert!(!result.valid);
        assert!(result.errors[0].contains("package declaration"));
    }

    #[test]
    fn syntax_error_rejected() {
        let source = r#"
            package scryer.rules.user.bad_syntax
            this is not valid rego at all
        "#;
        let result = validate_user_rule(source, "bad_syntax").unwrap();
        assert!(!result.valid);
        assert!(result.errors[0].contains("compilation error"));
    }

    #[test]
    fn conditional_rule_passes_when_condition_not_met() {
        let source = r#"
            package scryer.rules.user.conditional
            import rego.v1

            score_entry["only_anime"] := 100 if {
                input.context.is_anime
            }
        "#;
        let result = validate_user_rule(source, "conditional").unwrap();
        assert!(result.valid, "errors: {:?}", result.errors);
    }

    #[test]
    fn rule_using_builtin_passes() {
        let source = r#"
            package scryer.rules.user.with_builtin
            import rego.v1

            score_entry["size_block"] := scryer.block_score() if {
                scryer.size_gib(input.release.size_bytes) > 100
            }
        "#;
        let result = validate_user_rule(source, "with_builtin").unwrap();
        assert!(result.valid, "errors: {:?}", result.errors);
    }

    #[test]
    fn float_output_rejected() {
        let source = r#"
            package scryer.rules.user.float_rule
            import rego.v1

            score_entry["bad"] := 3.14
        "#;
        let result = validate_user_rule(source, "float_rule").unwrap();
        assert!(!result.valid);
        assert!(result.errors[0].contains("float"));
    }
}
