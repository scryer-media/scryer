use crate::{
    AudioStreamDoc, BuiltinScoreDoc, ContextDoc, FileDoc, ProfileDoc, ReleaseDoc, RulesError,
    SubtitleStreamDoc, UserRuleInput, builtins, score_entry_wrapper_policy_path,
    score_entry_wrapper_rule_path, score_entry_wrapper_source,
};
use regorus::{
    Engine, Value,
    unstable::{Expr, Literal, Module, Parser, Query, Rule, RuleBody, RuleHead, Source},
};
use std::{
    collections::{BTreeSet, HashSet},
    sync::OnceLock,
};

#[derive(Debug, serde::Deserialize)]
struct RuleInputContract {
    sections: Vec<RuleInputContractSection>,
}

#[derive(Debug, serde::Deserialize)]
struct RuleInputContractSection {
    path: String,
    fields: Vec<RuleInputContractField>,
}

#[derive(Debug, serde::Deserialize)]
struct RuleInputContractField {
    field: String,
    #[serde(rename = "type")]
    field_type: String,
}

#[derive(Debug)]
struct RuleInputCatalog {
    known_paths: HashSet<String>,
    array_container_paths: HashSet<String>,
}

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

#[derive(Debug, Clone, PartialEq, Eq)]
enum InputPathComponent {
    Field(String),
    ArrayItem,
    Dynamic,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InputReferencePath {
    components: Vec<InputPathComponent>,
    display: String,
}

impl InputReferencePath {
    fn normalized(&self) -> String {
        let mut out = String::new();
        for component in &self.components {
            match component {
                InputPathComponent::Field(field) => {
                    if !out.is_empty() {
                        out.push('.');
                    }
                    out.push_str(field);
                }
                InputPathComponent::ArrayItem => out.push_str("[]"),
                InputPathComponent::Dynamic => out.push_str("[*]"),
            }
        }
        out
    }

    fn is_dynamic_extra_path(&self) -> bool {
        matches!(
            self.components.as_slice(),
            [
                InputPathComponent::Field(input),
                InputPathComponent::Field(release),
                InputPathComponent::Field(extra),
                ..,
            ] if input == "input" && release == "release" && extra == "extra"
        )
    }

    fn has_dynamic_component(&self) -> bool {
        self.components
            .iter()
            .any(|component| matches!(component, InputPathComponent::Dynamic))
    }
}

fn rule_input_catalog() -> &'static RuleInputCatalog {
    static CATALOG: OnceLock<RuleInputCatalog> = OnceLock::new();
    CATALOG.get_or_init(|| {
        let contract: RuleInputContract =
            serde_json::from_str(include_str!("../rule-input-contract.json"))
                .expect("rule-input-contract.json should be valid");

        let mut known_paths = HashSet::new();
        let mut array_container_paths = HashSet::new();
        known_paths.insert("input".to_string());

        for section in contract.sections {
            known_paths.insert(section.path.clone());
            if let Some(base_path) = section.path.strip_suffix("[]") {
                array_container_paths.insert(base_path.to_string());
            }

            for field in section.fields {
                let field_path = format!("{}.{}", section.path, field.field);
                known_paths.insert(field_path.clone());
                if field.field_type.ends_with("[]") {
                    array_container_paths.insert(field_path.clone());
                    known_paths.insert(format!("{field_path}[]"));
                }
            }
        }

        RuleInputCatalog {
            known_paths,
            array_container_paths,
        }
    })
}

fn unknown_rule_input_path_message(path: &str) -> String {
    match path {
        "input.release.password_protected" => format!(
            "Unknown rule input path '{path}'. Use a documented field from the Rules Context Reference. For password-protected releases, use 'input.release.is_password_protected'."
        ),
        _ => format!(
            "Unknown rule input path '{path}'. Use one of the documented input fields from the Rules Context Reference."
        ),
    }
}

fn unsupported_dynamic_input_path_message(path: &str) -> String {
    format!(
        "Unsupported dynamic rule input path '{path}'. Use documented field access, documented array indexing, or input.release.extra.<key>."
    )
}

fn collect_unknown_input_path_errors(
    rego_source: &str,
    policy_path: &str,
) -> Result<Vec<String>, String> {
    let source = Source::from_contents(policy_path.to_string(), rego_source.to_string())
        .map_err(|e| e.to_string())?;
    let mut parser = Parser::new(&source).map_err(|e| e.to_string())?;
    parser.enable_rego_v1().map_err(|e| e.to_string())?;
    let module: Module = parser.parse().map_err(|e| e.to_string())?;

    let mut errors = BTreeSet::new();
    for rule in &module.policy {
        visit_rule(rule, &mut errors);
    }

    Ok(errors.into_iter().collect())
}

fn visit_rule(rule: &Rule, errors: &mut BTreeSet<String>) {
    match rule {
        Rule::Spec { head, bodies, .. } => {
            visit_rule_head(head, errors);
            for body in bodies {
                visit_rule_body(body, errors);
            }
        }
        Rule::Default {
            refr, args, value, ..
        } => {
            visit_expr(refr, errors);
            for arg in args {
                visit_expr(arg, errors);
            }
            visit_expr(value, errors);
        }
    }
}

fn visit_rule_head(head: &RuleHead, errors: &mut BTreeSet<String>) {
    match head {
        RuleHead::Compr { refr, assign, .. } => {
            visit_expr(refr, errors);
            if let Some(assign) = assign {
                visit_expr(&assign.value, errors);
            }
        }
        RuleHead::Set { refr, key, .. } => {
            visit_expr(refr, errors);
            if let Some(key) = key {
                visit_expr(key, errors);
            }
        }
        RuleHead::Func {
            refr, args, assign, ..
        } => {
            visit_expr(refr, errors);
            for arg in args {
                visit_expr(arg, errors);
            }
            if let Some(assign) = assign {
                visit_expr(&assign.value, errors);
            }
        }
    }
}

fn visit_rule_body(body: &RuleBody, errors: &mut BTreeSet<String>) {
    if let Some(assign) = &body.assign {
        visit_expr(&assign.value, errors);
    }
    visit_query(&body.query, errors);
}

fn visit_query(query: &Query, errors: &mut BTreeSet<String>) {
    for stmt in &query.stmts {
        visit_literal(&stmt.literal, errors);
        for with_mod in &stmt.with_mods {
            visit_expr(&with_mod.refr, errors);
            visit_expr(&with_mod.r#as, errors);
        }
    }
}

fn visit_literal(literal: &Literal, errors: &mut BTreeSet<String>) {
    match literal {
        Literal::SomeVars { .. } => {}
        Literal::SomeIn {
            key,
            value,
            collection,
            ..
        } => {
            if let Some(key) = key {
                visit_expr(key, errors);
            }
            visit_expr(value, errors);
            visit_expr(collection, errors);
        }
        Literal::Expr { expr, .. } | Literal::NotExpr { expr, .. } => visit_expr(expr, errors),
        Literal::Every { domain, query, .. } => {
            visit_expr(domain, errors);
            visit_query(query, errors);
        }
    }
}

fn visit_expr(expr: &Expr, errors: &mut BTreeSet<String>) {
    if let Some(path) = extract_input_reference_path(expr)
        && let Some(error) = validate_input_reference_path(&path)
    {
        errors.insert(error);
    }

    match expr {
        Expr::Array { items, .. } | Expr::Set { items, .. } => {
            for item in items {
                visit_expr(item, errors);
            }
        }
        Expr::Object { fields, .. } => {
            for (_, key, value) in fields {
                visit_expr(key, errors);
                visit_expr(value, errors);
            }
        }
        Expr::ArrayCompr { term, query, .. } | Expr::SetCompr { term, query, .. } => {
            visit_expr(term, errors);
            visit_query(query, errors);
        }
        Expr::ObjectCompr {
            key, value, query, ..
        } => {
            visit_expr(key, errors);
            visit_expr(value, errors);
            visit_query(query, errors);
        }
        Expr::Call { fcn, params, .. } => {
            visit_expr(fcn, errors);
            for param in params {
                visit_expr(param, errors);
            }
        }
        Expr::UnaryExpr { expr, .. } => visit_expr(expr, errors),
        Expr::RefDot { refr, .. } => visit_expr(refr, errors),
        Expr::RefBrack { refr, index, .. } => {
            visit_expr(refr, errors);
            visit_expr(index, errors);
        }
        Expr::BinExpr { lhs, rhs, .. }
        | Expr::BoolExpr { lhs, rhs, .. }
        | Expr::ArithExpr { lhs, rhs, .. }
        | Expr::AssignExpr { lhs, rhs, .. } => {
            visit_expr(lhs, errors);
            visit_expr(rhs, errors);
        }
        Expr::Membership {
            key,
            value,
            collection,
            ..
        } => {
            if let Some(key) = key {
                visit_expr(key, errors);
            }
            visit_expr(value, errors);
            visit_expr(collection, errors);
        }
        Expr::String { .. }
        | Expr::RawString { .. }
        | Expr::Number { .. }
        | Expr::Bool { .. }
        | Expr::Null { .. }
        | Expr::Var { .. } => {}
        Expr::OrExpr { lhs, rhs, .. } => {
            visit_expr(lhs, errors);
            visit_expr(rhs, errors);
        }
    }
}

fn extract_input_reference_path(expr: &Expr) -> Option<InputReferencePath> {
    match expr {
        Expr::Var { span, .. } if span.text() == "input" => Some(InputReferencePath {
            components: vec![InputPathComponent::Field("input".to_string())],
            display: "input".to_string(),
        }),
        Expr::RefDot { refr, field, .. } => {
            let mut path = extract_input_reference_path(refr)?;
            let field_name = field.1.as_string().ok()?.to_string();
            path.display.push('.');
            path.display.push_str(&field_name);
            path.components.push(InputPathComponent::Field(field_name));
            Some(path)
        }
        Expr::RefBrack { refr, index, .. } => {
            let mut path = extract_input_reference_path(refr)?;
            path.display.push('[');
            path.display.push_str(index.span().text());
            path.display.push(']');

            let current_path = path.normalized();
            if rule_input_catalog()
                .array_container_paths
                .contains(current_path.as_str())
            {
                path.components.push(InputPathComponent::ArrayItem);
            } else {
                match index.as_ref() {
                    Expr::String { value, .. } | Expr::RawString { value, .. } => {
                        let field_name = value.as_string().ok()?.to_string();
                        path.components.push(InputPathComponent::Field(field_name));
                    }
                    Expr::Number { .. } => path.components.push(InputPathComponent::ArrayItem),
                    Expr::Var { span, .. } if span.text() == "_" => {
                        path.components.push(InputPathComponent::ArrayItem);
                    }
                    _ => path.components.push(InputPathComponent::Dynamic),
                }
            }
            Some(path)
        }
        _ => None,
    }
}

fn validate_input_reference_path(path: &InputReferencePath) -> Option<String> {
    if path.is_dynamic_extra_path() {
        return None;
    }

    if path.has_dynamic_component() {
        return Some(unsupported_dynamic_input_path_message(&path.display));
    }

    let normalized = path.normalized();
    if rule_input_catalog()
        .known_paths
        .contains(normalized.as_str())
    {
        None
    } else {
        Some(unknown_rule_input_path_message(&normalized))
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
/// 5. The generated runtime `eval_rule` wrapper evaluates successfully.
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
    if let Err(e) = engine.add_policy(policy_path.clone(), rego_source.to_string()) {
        return Ok(ValidationResult::invalid(format!("compilation error: {e}")));
    }
    if let Err(e) = engine.add_policy(
        score_entry_wrapper_policy_path(rule_set_id),
        score_entry_wrapper_source(rule_set_id),
    ) {
        return Ok(ValidationResult::invalid(format!("compilation error: {e}")));
    }

    let input_path_errors = collect_unknown_input_path_errors(rego_source, &policy_path)
        .map_err(RulesError::Compilation)?;
    if !input_path_errors.is_empty() {
        return Ok(ValidationResult {
            valid: false,
            errors: input_path_errors,
        });
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

            if let Some(v) = value
                && let Err(e) = validate_score_entry_shape(v)
            {
                return Ok(ValidationResult::invalid(format!("output error: {e}")));
            }
            match engine.eval_rule(score_entry_wrapper_rule_path(rule_set_id)) {
                Ok(value) => {
                    if let Err(e) = validate_score_entry_shape(&value) {
                        return Ok(ValidationResult::invalid(format!("output error: {e}")));
                    }
                    Ok(ValidationResult::valid())
                }
                Err(e) => Ok(ValidationResult::invalid(format!("runtime error: {e}"))),
            }
        }
        Err(e) => Ok(ValidationResult::invalid(format!("runtime error: {e}"))),
    }
}

/// Validate that a persisted user rule can execute through the generated
/// runtime `eval_rule` wrapper.
///
/// This deliberately checks only the runtime path. New edits continue to use
/// [`validate_user_rule`] so the editor keeps its richer `eval_query`
/// diagnostics; migrations use this narrower check to protect existing rules
/// when the runtime entry point changes.
pub fn validate_runtime_wrapper(
    rego_source: &str,
    rule_set_id: &str,
) -> Result<ValidationResult, RulesError> {
    let expected_pkg = format!("package scryer.rules.user.{rule_set_id}");
    let has_pkg = rego_source.lines().any(|line| line.trim() == expected_pkg);
    if !has_pkg {
        return Ok(ValidationResult::invalid(format!(
            "package declaration must be: {expected_pkg}"
        )));
    }

    let mut engine = Engine::new();
    builtins::register_builtins(&mut engine);

    let policy_path = format!("user/{rule_set_id}.rego");
    if let Err(e) = engine.add_policy(policy_path, rego_source.to_string()) {
        return Ok(ValidationResult::invalid(format!("compilation error: {e}")));
    }
    if let Err(e) = engine.add_policy(
        score_entry_wrapper_policy_path(rule_set_id),
        score_entry_wrapper_source(rule_set_id),
    ) {
        return Ok(ValidationResult::invalid(format!("compilation error: {e}")));
    }

    let input_value =
        serde_json::to_value(synthetic_test_input()).map_err(RulesError::Serialization)?;
    engine.set_input(input_value.into());

    match engine.eval_rule(score_entry_wrapper_rule_path(rule_set_id)) {
        Ok(value) => match validate_score_entry_shape(&value) {
            Ok(()) => Ok(ValidationResult::valid()),
            Err(e) => Ok(ValidationResult::invalid(format!("output error: {e}"))),
        },
        Err(e) => Ok(ValidationResult::invalid(format!("runtime error: {e}"))),
    }
}

/// Validate a system-managed score-only policy.
///
/// Managed packs are opt-in, so the source is no longer inspected
/// for `scryer.block_score()`. That check only restricted the *spelling* of a
/// veto — a pack could emit the sentinel as a literal and block identically —
/// and the property it was reaching for now lives in `validate_managed_entries`
/// at evaluation time, where it cannot be bypassed.
pub fn validate_managed_rule(
    rego_source: &str,
    rule_set_id: &str,
) -> Result<ValidationResult, RulesError> {
    validate_user_rule(rego_source, rule_set_id)
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
            is_password_protected: Some(false),
            is_hdr10plus: false,
            is_hlg: false,
            is_10bit: false,
            is_uncensored: false,
            is_dubs_only: false,
            has_release_group: true,
            is_obfuscated: false,
            is_retagged: false,
            streaming_service: None,
            edition: None,
            anime_version: None,
            episode_release_type: Some("single_episode".to_string()),
            is_season_pack: false,
            is_multi_episode: false,
            release_group: Some("TestGroup".to_string()),
            year: Some(2024),
            parse_confidence: 0.9,
            size_bytes: Some(8_000_000_000),
            age_days: Some(5),
            thumbs_up: Some(10),
            thumbs_down: Some(0),
            guide_facts: vec![],
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
            library_name: Some("Movies".to_string()),
            media_type: "movie".to_string(),
            category: "movie".to_string(),
            original_language: Some("eng".to_string()),
            original_country: Some("US".to_string()),
            inferred_original_audio_language: "eng".to_string(),
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
            dovi_profile: Some(8),
            dovi_bl_compat_id: Some(1),
            video_frame_rate: Some("23.976".to_string()),
            video_profile: Some("Main 10".to_string()),
            audio_codec: Some("eac3".to_string()),
            audio_profile: Some("Dolby Digital Plus + Dolby Atmos".to_string()),
            audio_channels: Some(6),
            audio_bitrate_kbps: Some(640),
            audio_languages: vec!["eng".to_string()],
            audio_streams: vec![AudioStreamDoc {
                codec: Some("eac3".to_string()),
                profile: Some("Dolby Digital Plus + Dolby Atmos".to_string()),
                channels: Some(6),
                language: Some("eng".to_string()),
                name: None,
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
            num_chapters: Some(12),
            container_format: Some("matroska".to_string()),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, path::PathBuf};

    fn built_in_templates() -> Vec<(String, String)> {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("apps")
            .join("scryer-web")
            .join("lib")
            .join("constants")
            .join("rule-templates.ts");
        let source = fs::read_to_string(path).expect("rule templates file should be readable");
        let mut templates = Vec::new();
        let mut cursor = source.as_str();

        while let Some(id_start) = cursor.find("id: \"") {
            cursor = &cursor[id_start + 5..];
            let id_end = cursor
                .find('"')
                .expect("template id should be terminated by a quote");
            let id = cursor[..id_end].to_string();
            cursor = &cursor[id_end..];

            let rego_marker = "regoSource: `";
            let rego_start = cursor
                .find(rego_marker)
                .expect("template should define regoSource");
            cursor = &cursor[rego_start + rego_marker.len()..];
            let rego_end = cursor
                .find("`,")
                .expect("template regoSource should terminate with backtick-comma");
            let rego_source = cursor[..rego_end].to_string();
            templates.push((id, rego_source));
            cursor = &cursor[rego_end + 2..];
        }

        assert!(!templates.is_empty(), "should parse built-in templates");
        templates
    }

    fn built_in_template_source(template_id: &str) -> String {
        built_in_templates()
            .into_iter()
            .find(|(id, _)| id == template_id)
            .map(|(_, rego_source)| rego_source)
            .unwrap_or_else(|| panic!("missing built-in template {template_id}"))
    }

    fn evaluate_template(
        template_id: &str,
        input: UserRuleInput,
        facet: &str,
    ) -> crate::EvalResult {
        let policy_id = format!("builtin_{}", template_id.replace('-', "_"));
        let rego_source =
            crate::rewrite_package_declaration(&built_in_template_source(template_id), &policy_id);
        let engine = crate::UserRulesEngine::build(&[crate::UserPolicy {
            id: policy_id,
            name: template_id.to_string(),
            rego_source,
            origin: crate::PolicyOrigin::User,
            applied_facets: Vec::new(),
        }])
        .expect("template policy should compile");
        let mut evaluator = engine.evaluator();
        evaluator
            .evaluate(&input, facet)
            .expect("template evaluation should succeed")
    }

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
    fn runtime_wrapper_validation_accepts_persisted_rule() {
        let source = r#"
            package scryer.rules.user.persisted_rule
            import rego.v1

            score_entry["bonus"] := 100
        "#;

        let result = validate_runtime_wrapper(source, "persisted_rule").unwrap();

        assert!(result.valid, "errors: {:?}", result.errors);
    }

    #[test]
    fn runtime_wrapper_validation_rejects_persisted_rule_that_fails_at_runtime() {
        let source = r#"
            package scryer.rules.user.runtime_failure
            import rego.v1

            score_entry["bonus"] := lower(input.release.year)
        "#;

        let result = validate_runtime_wrapper(source, "runtime_failure").unwrap();

        assert!(!result.valid);
        assert!(result.errors[0].contains("runtime error"));
    }

    #[test]
    fn rule_input_contract_copies_are_byte_identical() {
        assert_eq!(
            include_str!("../rule-input-contract.json"),
            include_str!("../../../apps/scryer-web/lib/contracts/rule-input-contract.json"),
            "crates/scryer-rules/rule-input-contract.json and \
             apps/scryer-web/lib/contracts/rule-input-contract.json must stay byte-identical"
        );
    }

    /// Opt-in managed packs may veto, so the builtin is accepted
    /// in managed source. The bound that still applies is evaluation-time.
    #[test]
    fn managed_rule_accepts_block_score_builtin() {
        let source = r#"
            package scryer.rules.user.managed_rule
            import rego.v1
            score_entry["blocked"] := scryer.block_score()
        "#;

        let result = validate_managed_rule(source, "managed_rule").unwrap();
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

    #[test]
    fn unknown_release_field_rejected() {
        let source = r#"
            package scryer.rules.user.unknown_release_field
            import rego.v1

            score_entry["bad"] := 100 if {
                input.release.password_protected != null
            }
        "#;
        let result = validate_user_rule(source, "unknown_release_field").unwrap();
        assert!(!result.valid);
        assert!(
            result
                .errors
                .iter()
                .any(|error| error.contains("input.release.password_protected"))
        );
        assert!(result.errors.iter().any(|error| {
            error.contains("input.release.is_password_protected")
                && error.contains("Rules Context Reference")
        }));
    }

    #[test]
    fn unknown_context_field_rejected() {
        let source = r#"
            package scryer.rules.user.unknown_context_field
            import rego.v1

            score_entry["bad"] := 100 if {
                input.context.missing_flag
            }
        "#;
        let result = validate_user_rule(source, "unknown_context_field").unwrap();
        assert!(!result.valid);
        assert!(
            result
                .errors
                .iter()
                .any(|error| error.contains("input.context.missing_flag"))
        );
        assert!(
            result
                .errors
                .iter()
                .any(|error| error.contains("Rules Context Reference"))
        );
    }

    #[test]
    fn release_extra_dot_access_is_allowed() {
        let source = r#"
            package scryer.rules.user.release_extra_field
            import rego.v1

            score_entry["bonus"] := 100 if {
                input.release.extra.freeleech == true
            }
        "#;
        let result = validate_user_rule(source, "release_extra_field").unwrap();
        assert!(result.valid, "errors: {:?}", result.errors);
    }

    #[test]
    fn array_variable_index_access_is_allowed() {
        let source = r#"
            package scryer.rules.user.array_variable_index
            import rego.v1

            score_entry["eng_bonus"] := 100 if {
                some i
                input.file.audio_languages[i] == "eng"
            }
        "#;
        let result = validate_user_rule(source, "array_variable_index").unwrap();
        assert!(result.valid, "errors: {:?}", result.errors);
    }

    #[test]
    fn unsupported_dynamic_non_extra_path_rejected_with_guidance() {
        let source = r#"
            package scryer.rules.user.dynamic_context_lookup
            import rego.v1

            score_entry["bad"] := 100 if {
                some key
                input.context[key]
            }
        "#;
        let result = validate_user_rule(source, "dynamic_context_lookup").unwrap();
        assert!(!result.valid);
        assert!(result.errors.iter().any(|error| {
            error.contains("Unsupported dynamic rule input path")
                && error.contains("documented array indexing")
                && error.contains("input.release.extra.<key>")
        }));
    }

    #[test]
    fn all_built_in_templates_validate_after_rewrite() {
        for (index, (template_id, rego_source)) in built_in_templates().into_iter().enumerate() {
            let rule_id = format!("builtin_{index}");
            let rewritten = crate::rewrite_package_declaration(&rego_source, &rule_id);
            let result = validate_user_rule(&rewritten, &rule_id).unwrap();
            assert!(result.valid, "{template_id}: {:?}", result.errors);
        }
    }

    #[test]
    fn canonical_quality_templates_fire() {
        let webdl_result = evaluate_template("prefer-web-dl", synthetic_test_input(), "movie");
        assert!(
            webdl_result
                .entries
                .iter()
                .any(|entry| entry.code == "prefer_webdl" && entry.delta == 100),
            "prefer-web-dl should match canonical WEB-DL input"
        );

        let x265_result = evaluate_template("prefer-x265", synthetic_test_input(), "movie");
        assert!(
            x265_result
                .entries
                .iter()
                .any(|entry| entry.code == "x265_bonus" && entry.delta == 100),
            "prefer-x265 should match canonical H.265 input"
        );

        let mut x264_input = synthetic_test_input();
        x264_input.release.video_codec = Some("H.264".to_string());
        let x264_result = evaluate_template("penalize-x264-4k", x264_input, "movie");
        assert!(
            x264_result
                .entries
                .iter()
                .any(|entry| entry.code == "x264_4k_penalty" && entry.delta == -200),
            "penalize-x264-4k should match canonical 2160P H.264 input"
        );
    }

    #[test]
    fn group_templates_noop_when_release_group_is_missing() {
        for template_id in [
            "anime-group-preference",
            "block-mini-encodes",
            "block-low-quality-groups",
        ] {
            let mut input = synthetic_test_input();
            input.release.release_group = None;
            let result = evaluate_template(template_id, input, "anime");
            assert!(
                result.errors.is_empty(),
                "{template_id} should not raise runtime errors when release_group is null"
            );
            assert!(
                result.entries.is_empty(),
                "{template_id} should no-op when release_group is null"
            );
        }
    }

    #[test]
    fn password_protected_template_matches_injected_signal() {
        let mut input = synthetic_test_input();
        input.release.is_password_protected = Some(true);
        let result = evaluate_template("block-password-protected", input, "movie");
        assert!(
            result
                .entries
                .iter()
                .any(|entry| entry.code == "password_protected"),
            "password-protected template should block when the signal is injected"
        );
    }
}
