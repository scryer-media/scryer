mod builtins;
pub mod validation;

use regorus::{Engine, Value};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::warn;

// ── Errors ──────────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum RulesError {
    #[error("policy compilation failed: {0}")]
    Compilation(String),
    #[error("evaluation failed: {0}")]
    Evaluation(String),
    #[error("invalid rule output: {0}")]
    InvalidOutput(String),
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

// ── Public types ────────────────────────────────────────────────────────────

/// A user-authored Rego policy loaded from the database.
#[derive(Debug, Clone)]
pub struct UserPolicy {
    pub id: String,
    /// Human-readable name shown in the scoring breakdown.
    pub name: String,
    pub rego_source: String,
    /// Facets this rule applies to (e.g. "movie", "tv", "anime").
    /// Empty means the rule applies to all facets.
    pub applied_facets: Vec<String>,
}

/// Score delta at or below this value is treated as a hard block.
/// Matches `scryer.block_score()` builtin which returns -10000.
pub const BLOCK_SCORE_THRESHOLD: i32 = -9000;

/// Input document set per-release for user rule evaluation.
///
/// `file` is `None` during pre-download search scoring (no file on disk yet).
/// It is populated during post-download evaluation after media analysis runs on the
/// actual imported file. Rules that reference `input.file` fields are no-ops
/// during pre-download because `input.file` serializes as `null` and field
/// access on `null` evaluates to `undefined` in Rego.
#[derive(Debug, Clone, Serialize)]
pub struct UserRuleInput {
    pub release: ReleaseDoc,
    pub profile: ProfileDoc,
    pub context: ContextDoc,
    pub builtin_score: BuiltinScoreDoc,
    /// Actual file properties from media analysis. Null during pre-download scoring.
    pub file: Option<FileDoc>,
}

/// Ground-truth file properties from media analysis after download.
/// Available as `input.file` in Rego during post-download evaluation.
#[derive(Debug, Clone, Serialize)]
pub struct FileDoc {
    /// Video stream codec name (e.g. "hevc", "av1", "h264").
    pub video_codec: Option<String>,
    pub video_width: Option<i32>,
    pub video_height: Option<i32>,
    pub video_bitrate_kbps: Option<i32>,
    pub video_bit_depth: Option<i32>,
    /// e.g. "Dolby Vision", "HDR10", "HLG"
    pub video_hdr_format: Option<String>,
    pub dovi_profile: Option<u8>,
    pub dovi_bl_compat_id: Option<u8>,
    pub video_frame_rate: Option<String>,
    pub video_profile: Option<String>,
    /// Primary audio stream codec name.
    pub audio_codec: Option<String>,
    pub audio_channels: Option<i32>,
    pub audio_bitrate_kbps: Option<i32>,
    /// BCP-47/ISO 639-2 codes from all audio streams.
    pub audio_languages: Vec<String>,
    pub audio_streams: Vec<AudioStreamDoc>,
    /// Language codes from all subtitle streams.
    pub subtitle_languages: Vec<String>,
    pub subtitle_codecs: Vec<String>,
    pub subtitle_streams: Vec<SubtitleStreamDoc>,
    pub has_multiaudio: bool,
    pub duration_seconds: Option<i32>,
    pub num_chapters: Option<i32>,
    pub container_format: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AudioStreamDoc {
    pub codec: Option<String>,
    pub channels: Option<i32>,
    pub language: Option<String>,
    pub bitrate_kbps: Option<i32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SubtitleStreamDoc {
    pub codec: Option<String>,
    pub language: Option<String>,
    pub name: Option<String>,
    pub forced: bool,
    pub default: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReleaseDoc {
    pub raw_title: String,
    pub quality: Option<String>,
    pub source: Option<String>,
    pub video_codec: Option<String>,
    pub audio: Option<String>,
    pub audio_codecs: Vec<String>,
    pub audio_channels: Option<String>,
    pub languages_audio: Vec<String>,
    pub languages_subtitles: Vec<String>,
    pub is_dual_audio: bool,
    pub is_atmos: bool,
    pub is_dolby_vision: bool,
    pub detected_hdr: bool,
    pub is_remux: bool,
    pub is_bd_disk: bool,
    pub is_proper_upload: bool,
    pub is_repack: bool,
    pub is_ai_enhanced: bool,
    pub is_hardcoded_subs: bool,
    pub is_hdr10plus: bool,
    pub is_hlg: bool,
    pub is_10bit: bool,
    pub is_uncensored: bool,
    pub is_dubs_only: bool,
    pub has_release_group: bool,
    pub is_obfuscated: bool,
    pub is_retagged: bool,
    pub streaming_service: Option<String>,
    pub edition: Option<String>,
    pub anime_version: Option<u32>,
    pub episode_release_type: Option<String>,
    pub is_season_pack: bool,
    pub is_multi_episode: bool,
    pub release_group: Option<String>,
    pub year: Option<u32>,
    pub parse_confidence: f32,
    pub size_bytes: Option<i64>,
    pub age_days: Option<i64>,
    pub thumbs_up: Option<i32>,
    pub thumbs_down: Option<i32>,
    /// Arbitrary plugin-supplied metadata, accessible as `input.release.extra.*` in Rego.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub extra: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProfileDoc {
    pub id: String,
    pub name: String,
    pub quality_tiers: Vec<String>,
    pub archival_quality: Option<String>,
    pub allow_unknown_quality: bool,
    pub source_allowlist: Vec<String>,
    pub source_blocklist: Vec<String>,
    pub video_codec_allowlist: Vec<String>,
    pub video_codec_blocklist: Vec<String>,
    pub audio_codec_allowlist: Vec<String>,
    pub audio_codec_blocklist: Vec<String>,
    pub atmos_preferred: bool,
    pub dolby_vision_allowed: bool,
    pub detected_hdr_allowed: bool,
    pub prefer_remux: bool,
    pub allow_bd_disk: bool,
    pub allow_upgrades: bool,
    pub prefer_dual_audio: bool,
    pub required_audio_languages: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ContextDoc {
    pub title_id: Option<String>,
    pub media_type: String,
    pub category: String,
    pub tags: Vec<String>,
    pub has_existing_file: bool,
    pub existing_score: Option<i32>,
    pub search_mode: String,
    pub runtime_minutes: Option<i32>,
    pub is_anime: bool,
    pub is_filler: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct BuiltinScoreDoc {
    pub total: i32,
    pub blocked: bool,
    pub codes: Vec<String>,
}

/// A single scoring entry produced by a user rule.
#[derive(Debug, Clone)]
pub struct UserRuleEntry {
    pub code: String,
    pub delta: i32,
    pub rule_set_id: String,
    pub rule_set_name: String,
}

/// A per-rule error encountered during evaluation.
#[derive(Debug, Clone)]
pub struct RuleEvalError {
    pub rule_set_id: String,
    pub rule_set_name: String,
    pub message: String,
}

/// Result of evaluating all user rules for one release.
#[derive(Debug, Clone)]
pub struct EvalResult {
    pub entries: Vec<UserRuleEntry>,
    pub errors: Vec<RuleEvalError>,
}

// ── Package rewriting ───────────────────────────────────────────────────────

/// Rewrite (or insert) the package declaration in user Rego source to match
/// the system-assigned rule ID, and ensure `import rego.v1` is present.
///
/// The editor strips both the package line and the import before showing
/// source to users; this function restores them on every save so the stored
/// source is always a complete, valid Rego module.
pub fn rewrite_package_declaration(rego_source: &str, rule_id: &str) -> String {
    let pkg_line = format!("package scryer.rules.user.{rule_id}");
    let has_import = rego_source.lines().any(|l| l.trim() == "import rego.v1");
    let mut output = String::with_capacity(rego_source.len() + pkg_line.len() + 20);
    let mut found = false;

    for line in rego_source.lines() {
        if !found && line.trim().starts_with("package ") {
            output.push_str(&pkg_line);
            output.push('\n');
            if !has_import {
                output.push_str("import rego.v1\n");
            }
            found = true;
        } else {
            output.push_str(line);
            output.push('\n');
        }
    }

    if !found {
        let mut header = format!("{pkg_line}\n");
        if !has_import {
            header.push_str("import rego.v1\n");
        }
        return format!("{header}{output}");
    }

    output
}

/// Strip boilerplate lines from stored Rego source before displaying in the
/// editor. Removes the package declaration and `import rego.v1`; both are
/// restored automatically by [`rewrite_package_declaration`] on save.
pub fn strip_editor_source(rego_source: &str) -> String {
    let lines: Vec<&str> = rego_source
        .lines()
        .filter(|l| {
            let t = l.trim();
            !t.starts_with("package ") && t != "import rego.v1"
        })
        .collect();

    // Drop leading blank lines left behind after stripping
    let trimmed: Vec<&str> = lines
        .iter()
        .copied()
        .skip_while(|l| l.trim().is_empty())
        .collect();

    if trimmed.is_empty() {
        String::new()
    } else {
        format!("{}\n", trimmed.join("\n"))
    }
}

// ── UserRulesEngine (thread-safe factory) ───────────────────────────────────

/// Pre-compiled Regorus engine holding all active user rules.
///
/// Stored behind `Arc<RwLock<UserRulesEngine>>` in AppServices. When rules
/// change, a new engine is built and swapped in. Evaluators are cheap clones
/// created per search batch.
///
/// `Engine` is `Send + Sync`, so the `Arc` wrapper is safe for sharing across
/// async tasks.
#[derive(Clone)]
pub struct UserRulesEngine {
    template: Arc<Engine>,
    /// (rule_id, rule_name, applied_facets) triples in policy order.
    rules: Vec<(String, String, Vec<String>)>,
}

impl UserRulesEngine {
    /// Build an engine from a set of user-authored policies.
    /// Returns an empty engine if `policies` is empty.
    pub fn build(policies: &[UserPolicy]) -> Result<Self, RulesError> {
        let mut engine = Engine::new();
        builtins::register_builtins(&mut engine);

        let mut rules = Vec::new();

        for policy in policies {
            let path = format!("user/{}.rego", policy.id);
            engine
                .add_policy(path, policy.rego_source.clone())
                .map_err(|e| RulesError::Compilation(format!("{}: {e}", policy.id)))?;
            rules.push((
                policy.id.clone(),
                policy.name.clone(),
                policy.applied_facets.clone(),
            ));
        }

        Ok(Self {
            template: Arc::new(engine),
            rules,
        })
    }

    /// Build an empty engine (no user rules).
    pub fn empty() -> Self {
        let mut engine = Engine::new();
        builtins::register_builtins(&mut engine);
        Self {
            template: Arc::new(engine),
            rules: Vec::new(),
        }
    }

    /// True when no user rules are loaded. Callers should skip evaluation entirely.
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// Number of active user rules.
    pub fn rule_count(&self) -> usize {
        self.rules.len()
    }

    /// Create an evaluator for a single search batch.
    pub fn evaluator(&self) -> UserRulesEvaluator {
        UserRulesEvaluator {
            engine: (*self.template).clone(),
            rules: self.rules.clone(),
        }
    }
}

// ── UserRulesEvaluator (per-batch) ──────────────────────────────────────────

/// Evaluates user rules against release candidates within a single search batch.
///
/// Create one per `search_and_score_releases` call, reuse across all releases
/// in the batch.
pub struct UserRulesEvaluator {
    engine: Engine,
    rules: Vec<(String, String, Vec<String>)>,
}

impl UserRulesEvaluator {
    /// Evaluate all user rules against one release candidate.
    ///
    /// `facet` is the current media facet (e.g. "movie", "tv", "anime").
    /// Rules whose `applied_facets` is non-empty and does not contain `facet`
    /// are skipped.
    ///
    /// Per-rule runtime errors are collected in `EvalResult::errors` rather
    /// than aborting the entire evaluation.
    pub fn evaluate(
        &mut self,
        input: &UserRuleInput,
        facet: &str,
    ) -> Result<EvalResult, RulesError> {
        let mut result = EvalResult {
            entries: Vec::new(),
            errors: Vec::new(),
        };

        if self.rules.is_empty() {
            return Ok(result);
        }

        let input_value = serde_json::to_value(input)?;
        self.engine.set_input(input_value.into());

        for (rule_id, rule_name, applied_facets) in &self.rules {
            // Skip rules that are scoped to other facets.
            if !applied_facets.is_empty() && !applied_facets.iter().any(|f| f == facet) {
                continue;
            }

            let query = format!("data.scryer.rules.user.{rule_id}.score_entry");

            match self.engine.eval_query(query, false) {
                Ok(results) => {
                    if let Some(r) = results.result.first()
                        && let Some(expr) = r.expressions.first()
                    {
                        Self::extract_entries(&expr.value, rule_id, rule_name, &mut result.entries);
                    }
                }
                Err(e) => {
                    warn!(
                        rule_id = rule_id.as_str(),
                        error = %e,
                        "user rule evaluation failed, skipping"
                    );
                    result.errors.push(RuleEvalError {
                        rule_set_id: rule_id.clone(),
                        rule_set_name: rule_name.clone(),
                        message: e.to_string(),
                    });
                }
            }
        }

        Ok(result)
    }

    /// Extract score_entry map from the Rego evaluation result.
    /// Expected shape: `{"code_name": delta_integer, ...}`
    fn extract_entries(
        value: &Value,
        rule_id: &str,
        rule_name: &str,
        entries: &mut Vec<UserRuleEntry>,
    ) {
        // Value::Undefined means the rule conditions weren't met — no entries.
        if matches!(value, Value::Undefined) {
            return;
        }

        let obj = match value.as_object() {
            Ok(obj) => obj,
            Err(_) => return,
        };

        for (key, val) in obj.iter() {
            let code = match key.as_string() {
                Ok(s) => s.to_string(),
                Err(_) => continue,
            };
            let delta = if let Ok(n) = val.as_i64() {
                match i32::try_from(n) {
                    Ok(v) => v,
                    Err(_) => {
                        warn!(
                            rule_id,
                            code = code.as_str(),
                            value = n,
                            "score delta out of i32 range, clamping"
                        );
                        if n > 0 { i32::MAX } else { i32::MIN }
                    }
                }
            } else if let Ok(f) = val.as_f64() {
                if f.is_nan() || f.is_infinite() {
                    warn!(
                        rule_id,
                        code = code.as_str(),
                        "score delta is NaN/Inf, skipping"
                    );
                    continue;
                }
                f.clamp(i32::MIN as f64, i32::MAX as f64) as i32
            } else {
                continue;
            };
            entries.push(UserRuleEntry {
                code,
                delta,
                rule_set_id: rule_id.to_string(),
                rule_set_name: rule_name.to_string(),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_engine_produces_no_entries() {
        let engine = UserRulesEngine::empty();
        assert!(engine.is_empty());
        assert_eq!(engine.rule_count(), 0);

        let mut evaluator = engine.evaluator();
        let input = test_input();
        let result = evaluator.evaluate(&input, "movie").unwrap();
        assert!(result.entries.is_empty());
        assert!(result.errors.is_empty());
    }

    #[test]
    fn single_rule_produces_entries() {
        let policy = UserPolicy {
            id: "test_rule".to_string(),
            name: "Test Rule".to_string(),
            rego_source: r#"
                package scryer.rules.user.test_rule
                import rego.v1

                score_entry["test_bonus"] := 500 if {
                    input.release.is_dual_audio
                }
            "#
            .to_string(),
            applied_facets: vec![],
        };

        let engine = UserRulesEngine::build(&[policy]).unwrap();
        assert!(!engine.is_empty());
        assert_eq!(engine.rule_count(), 1);

        let mut evaluator = engine.evaluator();
        let mut input = test_input();
        input.release.is_dual_audio = true;

        let result = evaluator.evaluate(&input, "movie").unwrap();
        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.entries[0].code, "test_bonus");
        assert_eq!(result.entries[0].delta, 500);
        assert_eq!(result.entries[0].rule_set_id, "test_rule");
    }

    #[test]
    fn rule_does_not_fire_when_condition_unmet() {
        let policy = UserPolicy {
            id: "test_rule".to_string(),
            name: "Test Rule".to_string(),
            rego_source: r#"
                package scryer.rules.user.test_rule
                import rego.v1

                score_entry["test_bonus"] := 500 if {
                    input.release.is_dual_audio
                }
            "#
            .to_string(),
            applied_facets: vec![],
        };

        let engine = UserRulesEngine::build(&[policy]).unwrap();
        let mut evaluator = engine.evaluator();
        let mut input = test_input();
        input.release.is_dual_audio = false;

        let result = evaluator.evaluate(&input, "movie").unwrap();
        assert!(result.entries.is_empty());
    }

    #[test]
    fn multiple_rules_both_produce_entries() {
        let policies = vec![
            UserPolicy {
                id: "rule_a".to_string(),
                name: "Rule A".to_string(),
                rego_source: r#"
                    package scryer.rules.user.rule_a
                    import rego.v1
                    score_entry["bonus_a"] := 100
                "#
                .to_string(),
                applied_facets: vec![],
            },
            UserPolicy {
                id: "rule_b".to_string(),
                name: "Rule B".to_string(),
                rego_source: r#"
                    package scryer.rules.user.rule_b
                    import rego.v1
                    score_entry["bonus_b"] := 200
                "#
                .to_string(),
                applied_facets: vec![],
            },
        ];

        let engine = UserRulesEngine::build(&policies).unwrap();
        let mut evaluator = engine.evaluator();
        let input = test_input();

        let result = evaluator.evaluate(&input, "movie").unwrap();
        assert_eq!(result.entries.len(), 2);
        assert!(
            result
                .entries
                .iter()
                .any(|e| e.code == "bonus_a" && e.delta == 100)
        );
        assert!(
            result
                .entries
                .iter()
                .any(|e| e.code == "bonus_b" && e.delta == 200)
        );
    }

    #[test]
    fn rule_can_read_builtin_score() {
        let policy = UserPolicy {
            id: "score_aware".to_string(),
            name: "Score Aware".to_string(),
            rego_source: r#"
                package scryer.rules.user.score_aware
                import rego.v1

                score_entry["high_score_boost"] := 300 if {
                    input.builtin_score.total > 3000
                }
            "#
            .to_string(),
            applied_facets: vec![],
        };

        let engine = UserRulesEngine::build(&[policy]).unwrap();
        let mut evaluator = engine.evaluator();
        let mut input = test_input();
        input.builtin_score.total = 3500;

        let result = evaluator.evaluate(&input, "movie").unwrap();
        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.entries[0].code, "high_score_boost");
    }

    #[test]
    fn rule_can_use_block_score_builtin() {
        let policy = UserPolicy {
            id: "blocker".to_string(),
            name: "Blocker".to_string(),
            rego_source: r#"
                package scryer.rules.user.blocker
                import rego.v1

                score_entry["custom_block"] := scryer.block_score() if {
                    input.context.is_anime
                    not input.release.is_dual_audio
                }
            "#
            .to_string(),
            applied_facets: vec![],
        };

        let engine = UserRulesEngine::build(&[policy]).unwrap();
        let mut evaluator = engine.evaluator();
        let mut input = test_input();
        input.context.is_anime = true;
        input.release.is_dual_audio = false;

        let result = evaluator.evaluate(&input, "movie").unwrap();
        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.entries[0].delta, -10000);
    }

    #[test]
    fn compilation_error_is_reported() {
        let policy = UserPolicy {
            id: "bad_rule".to_string(),
            name: "Bad Rule".to_string(),
            rego_source: "this is not valid rego".to_string(),
            applied_facets: vec![],
        };

        let result = UserRulesEngine::build(&[policy]);
        assert!(result.is_err());
    }

    #[test]
    fn evaluator_reuse_across_multiple_inputs() {
        let policy = UserPolicy {
            id: "reuse_test".to_string(),
            name: "Reuse Test".to_string(),
            rego_source: r#"
                package scryer.rules.user.reuse_test
                import rego.v1
                score_entry["always"] := 42
            "#
            .to_string(),
            applied_facets: vec![],
        };

        let engine = UserRulesEngine::build(&[policy]).unwrap();
        let mut evaluator = engine.evaluator();

        for _ in 0..5 {
            let result = evaluator.evaluate(&test_input(), "movie").unwrap();
            assert_eq!(result.entries.len(), 1);
            assert_eq!(result.entries[0].delta, 42);
        }
    }

    #[test]
    fn facet_scoped_rule_skipped_for_other_facets() {
        let policy = UserPolicy {
            id: "anime_only".to_string(),
            name: "Anime Only".to_string(),
            rego_source: r#"
                package scryer.rules.user.anime_only
                import rego.v1
                score_entry["anime_boost"] := 500
            "#
            .to_string(),
            applied_facets: vec!["anime".to_string()],
        };

        let engine = UserRulesEngine::build(&[policy]).unwrap();
        let mut evaluator = engine.evaluator();
        let input = test_input();

        // Should fire for anime
        let result = evaluator.evaluate(&input, "anime").unwrap();
        assert_eq!(result.entries.len(), 1);

        // Should be skipped for movie
        let result = evaluator.evaluate(&input, "movie").unwrap();
        assert!(result.entries.is_empty());
    }

    #[test]
    fn global_rule_applies_to_all_facets() {
        let policy = UserPolicy {
            id: "global_rule".to_string(),
            name: "Global Rule".to_string(),
            rego_source: r#"
                package scryer.rules.user.global_rule
                import rego.v1
                score_entry["always"] := 100
            "#
            .to_string(),
            applied_facets: vec![],
        };

        let engine = UserRulesEngine::build(&[policy]).unwrap();
        let mut evaluator = engine.evaluator();
        let input = test_input();

        for facet in &["movie", "tv", "anime"] {
            let result = evaluator.evaluate(&input, facet).unwrap();
            assert_eq!(result.entries.len(), 1, "should apply to {facet}");
        }
    }

    #[test]
    fn rewrite_package_replaces_existing() {
        let source = "package scryer.rules.user.my_rule\nimport rego.v1\nscore_entry[\"x\"] := 1\n";
        let rewritten = rewrite_package_declaration(source, "r1234");
        assert!(rewritten.starts_with("package scryer.rules.user.r1234\n"));
        assert!(rewritten.contains("import rego.v1"));
    }

    #[test]
    fn rewrite_package_inserts_when_missing() {
        let source = "import rego.v1\nscore_entry[\"x\"] := 1\n";
        let rewritten = rewrite_package_declaration(source, "r1234");
        assert!(rewritten.starts_with("package scryer.rules.user.r1234\n"));
        assert!(rewritten.contains("import rego.v1"));
    }

    #[test]
    fn rewrite_injects_import_when_absent() {
        // Editor source has no import — rewrite must add it
        let source = "score_entry[\"x\"] := 1\n";
        let rewritten = rewrite_package_declaration(source, "r1234");
        assert!(rewritten.starts_with("package scryer.rules.user.r1234\n"));
        assert!(rewritten.contains("import rego.v1"));
    }

    #[test]
    fn rewrite_does_not_duplicate_import() {
        let source = "package scryer.rules.user.old\nimport rego.v1\nscore_entry[\"x\"] := 1\n";
        let rewritten = rewrite_package_declaration(source, "r1234");
        let import_count = rewritten
            .lines()
            .filter(|l| l.trim() == "import rego.v1")
            .count();
        assert_eq!(import_count, 1);
    }

    #[test]
    fn strip_editor_source_removes_boilerplate() {
        let stored =
            "package scryer.rules.user.rabc\nimport rego.v1\n\nscore_entry[\"bonus\"] := 100\n";
        let stripped = strip_editor_source(stored);
        assert!(!stripped.contains("package "));
        assert!(!stripped.contains("import rego.v1"));
        assert!(stripped.contains("score_entry"));
    }

    #[test]
    fn strip_then_rewrite_roundtrip() {
        let stored =
            "package scryer.rules.user.rabc\nimport rego.v1\n\nscore_entry[\"bonus\"] := 100\n";
        let stripped = strip_editor_source(stored);
        let restored = rewrite_package_declaration(&stripped, "rabc");
        assert!(restored.contains("package scryer.rules.user.rabc"));
        assert!(restored.contains("import rego.v1"));
        assert!(restored.contains("score_entry[\"bonus\"] := 100"));
    }

    #[test]
    fn i32_overflow_clamped() {
        let policy = UserPolicy {
            id: "big_score".to_string(),
            name: "Big Score".to_string(),
            rego_source: r#"
                package scryer.rules.user.big_score
                import rego.v1
                score_entry["huge"] := 99999999999
            "#
            .to_string(),
            applied_facets: vec![],
        };

        let engine = UserRulesEngine::build(&[policy]).unwrap();
        let mut evaluator = engine.evaluator();
        let result = evaluator.evaluate(&test_input(), "movie").unwrap();
        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.entries[0].delta, i32::MAX);
    }

    #[test]
    fn plugin_scoring_policy_with_rewritten_package() {
        // Simulates the plugin scoring policy flow: the plugin declares its own
        // package name, but the host rewrites it to match the system-assigned ID.
        let rego = r#"package scryer.rules.user.plugin_nzbgeek_vote_penalty
import rego.v1

score_entry["nzbgeek_thumbs_down"] := penalty if {
    td := input.release.extra.thumbs_down
    td > 5
    extra := min([td - 5, 10])
    penalty := -2400 - (extra * 300)
}
"#;
        let id = "plugin_nzbgeek_nzbgeek_vote_penalty";
        let rewritten = rewrite_package_declaration(rego, id);

        let policy = UserPolicy {
            id: id.to_string(),
            name: id.to_string(),
            rego_source: rewritten,
            applied_facets: vec![],
        };

        let engine = UserRulesEngine::build(&[policy]).unwrap();
        let mut evaluator = engine.evaluator();
        let mut input = test_input();
        // thumbs_down = 8 → penalty = -2400 - ((8-5).min(10) * 300) = -2400 - 900 = -3300
        input
            .release
            .extra
            .insert("thumbs_down".to_string(), serde_json::Value::from(8));
        let result = evaluator.evaluate(&input, "movie").unwrap();
        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.entries[0].code, "nzbgeek_thumbs_down");
        assert_eq!(result.entries[0].delta, -3300);
    }

    #[test]
    fn plugin_language_bonus_policy() {
        let rego = r#"package scryer.rules.user.original_doesnt_matter
import rego.v1

score_entry["nzbgeek_english_confirmed"] := 200 if {
    langs := input.release.extra.languages
    count(langs) > 0
    some lang in langs
    lower(lang) == "english"
}
"#;
        let id = "plugin_nzbgeek_nzbgeek_language_bonus";
        let rewritten = rewrite_package_declaration(rego, id);

        let policy = UserPolicy {
            id: id.to_string(),
            name: id.to_string(),
            rego_source: rewritten,
            applied_facets: vec![],
        };

        let engine = UserRulesEngine::build(&[policy]).unwrap();
        let mut evaluator = engine.evaluator();
        let mut input = test_input();
        input.release.extra.insert(
            "languages".to_string(),
            serde_json::json!(["English", "French"]),
        );
        let result = evaluator.evaluate(&input, "movie").unwrap();
        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.entries[0].code, "nzbgeek_english_confirmed");
        assert_eq!(result.entries[0].delta, 200);
    }

    #[test]
    fn post_download_rule_blocks_on_num_chapters() {
        let policy = UserPolicy {
            id: "chapter_gate".to_string(),
            name: "Chapter Gate".to_string(),
            rego_source: rewrite_package_declaration(
                r#"
score_entry["too_few_chapters"] := scryer.block_score() if {
    input.file != null
    input.file.num_chapters < 2
}
"#,
                "chapter_gate",
            ),
            applied_facets: vec!["movie".to_string()],
        };

        let engine = UserRulesEngine::build(&[policy]).unwrap();
        let mut evaluator = engine.evaluator();
        let mut input = test_input();
        input.file = Some(test_file_doc());

        let result = evaluator.evaluate(&input, "movie").unwrap();
        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.entries[0].code, "too_few_chapters");
        assert_eq!(result.entries[0].delta, -10000);
    }

    #[test]
    fn post_download_rule_is_noop_pre_download_when_file_is_null() {
        let policy = UserPolicy {
            id: "chapter_gate".to_string(),
            name: "Chapter Gate".to_string(),
            rego_source: rewrite_package_declaration(
                r#"
score_entry["too_few_chapters"] := scryer.block_score() if {
    input.file != null
    input.file.num_chapters < 2
}
"#,
                "chapter_gate",
            ),
            applied_facets: vec!["movie".to_string()],
        };

        let engine = UserRulesEngine::build(&[policy]).unwrap();
        let mut evaluator = engine.evaluator();
        let result = evaluator.evaluate(&test_input(), "movie").unwrap();
        assert!(result.entries.is_empty());
    }

    fn test_input() -> UserRuleInput {
        UserRuleInput {
            release: ReleaseDoc {
                raw_title: "Test.Movie.2024.2160p.WEB-DL.H.265".to_string(),
                quality: Some("2160P".to_string()),
                source: Some("WEB-DL".to_string()),
                video_codec: Some("H.265".to_string()),
                audio: Some("DDP".to_string()),
                audio_codecs: vec!["DDP".to_string()],
                audio_channels: Some("5.1".to_string()),
                languages_audio: vec!["eng".to_string()],
                languages_subtitles: vec![],
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
                is_10bit: false,
                is_uncensored: false,
                is_dubs_only: false,
                has_release_group: false,
                is_obfuscated: false,
                is_retagged: false,
                streaming_service: None,
                edition: None,
                anime_version: None,
                episode_release_type: None,
                is_season_pack: false,
                is_multi_episode: false,
                release_group: None,
                year: Some(2024),
                parse_confidence: 0.9,
                size_bytes: Some(8_000_000_000),
                age_days: Some(5),
                thumbs_up: None,
                thumbs_down: None,
                extra: Default::default(),
            },
            profile: ProfileDoc {
                id: "4k".to_string(),
                name: "4K".to_string(),
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
                title_id: Some("tt1234567".to_string()),
                media_type: "movie".to_string(),
                category: "movie".to_string(),
                tags: vec![],
                has_existing_file: false,
                existing_score: None,
                search_mode: "auto".to_string(),
                runtime_minutes: None,
                is_anime: false,
                is_filler: false,
            },
            builtin_score: BuiltinScoreDoc {
                total: 0,
                blocked: false,
                codes: vec![],
            },
            file: None,
        }
    }

    fn test_file_doc() -> FileDoc {
        FileDoc {
            video_codec: Some("hevc".to_string()),
            video_width: Some(3840),
            video_height: Some(2160),
            video_bitrate_kbps: Some(40000),
            video_bit_depth: Some(10),
            video_hdr_format: Some("HDR10".to_string()),
            dovi_profile: Some(8),
            dovi_bl_compat_id: Some(1),
            video_frame_rate: Some("23.976".to_string()),
            video_profile: Some("Main 10".to_string()),
            audio_codec: Some("eac3".to_string()),
            audio_channels: Some(6),
            audio_bitrate_kbps: Some(640),
            audio_languages: vec!["eng".to_string()],
            audio_streams: vec![AudioStreamDoc {
                codec: Some("eac3".to_string()),
                channels: Some(6),
                language: Some("eng".to_string()),
                bitrate_kbps: Some(640),
            }],
            subtitle_languages: vec!["eng".to_string()],
            subtitle_codecs: vec!["subrip".to_string()],
            subtitle_streams: vec![SubtitleStreamDoc {
                codec: Some("subrip".to_string()),
                language: Some("eng".to_string()),
                name: Some("English".to_string()),
                forced: false,
                default: true,
            }],
            has_multiaudio: false,
            duration_seconds: Some(7200),
            num_chapters: Some(1),
            container_format: Some("matroska".to_string()),
        }
    }
}
