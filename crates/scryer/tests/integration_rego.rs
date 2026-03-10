mod common;

use serde_json::{json, Value};

use common::TestContext;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Execute a GraphQL query and return the parsed JSON body.
async fn gql(ctx: &TestContext, query: &str, variables: Value) -> Value {
    let client = ctx.http_client();
    let resp = client
        .post(ctx.graphql_url())
        .json(&json!({ "query": query, "variables": variables }))
        .send()
        .await
        .expect("request should succeed");
    assert_eq!(resp.status(), 200);
    resp.json().await.expect("should be valid JSON")
}

fn assert_no_errors(body: &Value) {
    assert!(
        body.get("errors").is_none(),
        "unexpected GraphQL errors: {body}"
    );
}

const SIMPLE_BONUS_REGO: &str = r#"
package scryer.rules.user.placeholder
import rego.v1

score_entry["test_bonus"] := 100 if { true }
"#;

const DUAL_AUDIO_REGO: &str = r#"
package scryer.rules.user.placeholder
import rego.v1

score_entry["dual_audio_bonus"] := 500 if {
    input.release.is_dual_audio
}
"#;

const BLOCK_REGO: &str = r#"
package scryer.rules.user.placeholder
import rego.v1

score_entry["block_it"] := scryer.block_score() if {
    not input.release.is_atmos
}
"#;

const SIZE_GIB_REGO: &str = r#"
package scryer.rules.user.placeholder
import rego.v1

score_entry["large_penalty"] := -200 if {
    scryer.size_gib(input.release.size_bytes) > 50
}
"#;

const LANG_MATCHES_REGO: &str = r#"
package scryer.rules.user.placeholder
import rego.v1

score_entry["eng_bonus"] := 50 if {
    scryer.lang_matches(input.release.languages_audio[0], "eng")
}
"#;

const FLOAT_REGO: &str = r#"
package scryer.rules.user.placeholder
import rego.v1

score_entry["bad"] := 1.5 if { true }
"#;

const INVALID_SYNTAX_REGO: &str = r#"
package scryer.rules.user.placeholder
import rego.v1

score_entry["bad"] := {{{
"#;

/// Create a rule set via GraphQL and return its ID.
async fn create_rule(ctx: &TestContext, name: &str, rego: &str) -> String {
    let body = gql(
        ctx,
        r#"mutation($input: CreateRuleSetInput!) {
            createRuleSet(input: $input) { id name }
        }"#,
        json!({
            "input": {
                "name": name,
                "regoSource": rego,
            }
        }),
    )
    .await;
    assert_no_errors(&body);
    body["data"]["createRuleSet"]["id"]
        .as_str()
        .unwrap()
        .to_string()
}

/// Create a rule set with facet scoping and return its ID.
async fn create_rule_with_facets(
    ctx: &TestContext,
    name: &str,
    rego: &str,
    facets: &[&str],
) -> String {
    let facets_json: Vec<Value> = facets.iter().map(|f| json!(f)).collect();
    let body = gql(
        ctx,
        r#"mutation($input: CreateRuleSetInput!) {
            createRuleSet(input: $input) { id }
        }"#,
        json!({
            "input": {
                "name": name,
                "regoSource": rego,
                "appliedFacets": facets_json,
            }
        }),
    )
    .await;
    assert_no_errors(&body);
    body["data"]["createRuleSet"]["id"]
        .as_str()
        .unwrap()
        .to_string()
}

/// Build a minimal UserRuleInput for direct engine tests.
fn test_input() -> scryer_rules::UserRuleInput {
    scryer_rules::UserRuleInput {
        release: scryer_rules::ReleaseDoc {
            raw_title: "Test.Movie.2024.2160p.WEB-DL.H.265.DDP.5.1-GROUP".to_string(),
            quality: Some("2160P".to_string()),
            source: Some("WEB-DL".to_string()),
            video_codec: Some("H.265".to_string()),
            audio: Some("DDP 5.1".to_string()),
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
            streaming_service: None,
            edition: None,
            anime_version: None,
            release_group: Some("GROUP".to_string()),
            year: Some(2024),
            parse_confidence: 0.95,
            size_bytes: Some(8_000_000_000),
            age_days: Some(2),
            thumbs_up: Some(10),
            thumbs_down: Some(1),
            extra: Default::default(),
        },
        profile: scryer_rules::ProfileDoc {
            id: "test-profile".to_string(),
            name: "Test Profile".to_string(),
            quality_tiers: vec![
                "2160P".to_string(),
                "1080P".to_string(),
                "720P".to_string(),
            ],
            archival_quality: None,
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
        context: scryer_rules::ContextDoc {
            title_id: Some("test-title-id".to_string()),
            media_type: "movie".to_string(),
            category: "movie".to_string(),
            tags: vec![],
            has_existing_file: false,
            existing_score: None,
            search_mode: "interactive".to_string(),
            runtime_minutes: Some(120),
            is_anime: false,
            is_filler: false,
        },
        builtin_score: scryer_rules::BuiltinScoreDoc {
            total: 3200,
            blocked: false,
            codes: vec!["quality_tier_0".to_string(), "source_webdl".to_string()],
        },
        file: None,
    }
}

/// Build a UserRuleInput with overrides.
fn test_input_with(f: impl FnOnce(&mut scryer_rules::UserRuleInput)) -> scryer_rules::UserRuleInput {
    let mut input = test_input();
    f(&mut input);
    input
}

// ===========================================================================
// 1. GraphQL CRUD
// ===========================================================================

#[tokio::test]
async fn rego_create_rule_set() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"mutation($input: CreateRuleSetInput!) {
            createRuleSet(input: $input) {
                id name description regoSource enabled priority appliedFacets createdAt updatedAt
            }
        }"#,
        json!({
            "input": {
                "name": "Test Rule",
                "description": "A test rule",
                "regoSource": SIMPLE_BONUS_REGO,
                "appliedFacets": ["movie"],
                "priority": 5,
            }
        }),
    )
    .await;
    assert_no_errors(&body);
    let rule = &body["data"]["createRuleSet"];
    assert!(rule["id"].is_string(), "should have an id");
    assert_eq!(rule["name"], "Test Rule");
    assert_eq!(rule["description"], "A test rule");
    assert_eq!(rule["enabled"], true);
    assert_eq!(rule["priority"], 5);
    assert_eq!(rule["appliedFacets"].as_array().unwrap().len(), 1);
    assert!(rule["createdAt"].is_string());
    assert!(rule["updatedAt"].is_string());
    // regoSource should have been rewritten with system-assigned package
    assert!(rule["regoSource"].as_str().unwrap().contains("score_entry"));
}

#[tokio::test]
async fn rego_create_with_facets() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"mutation($input: CreateRuleSetInput!) {
            createRuleSet(input: $input) { id appliedFacets }
        }"#,
        json!({
            "input": {
                "name": "Multi-Facet Rule",
                "regoSource": SIMPLE_BONUS_REGO,
                "appliedFacets": ["movie", "anime"],
            }
        }),
    )
    .await;
    assert_no_errors(&body);
    let facets = body["data"]["createRuleSet"]["appliedFacets"]
        .as_array()
        .unwrap();
    assert_eq!(facets.len(), 2);
}

#[tokio::test]
async fn rego_create_with_priority() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"mutation($input: CreateRuleSetInput!) {
            createRuleSet(input: $input) { id priority }
        }"#,
        json!({
            "input": {
                "name": "Priority Rule",
                "regoSource": SIMPLE_BONUS_REGO,
                "priority": 10,
            }
        }),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["createRuleSet"]["priority"], 10);
}

#[tokio::test]
async fn rego_create_minimal_input() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"mutation($input: CreateRuleSetInput!) {
            createRuleSet(input: $input) { id name description enabled priority appliedFacets }
        }"#,
        json!({
            "input": {
                "name": "Minimal Rule",
                "regoSource": SIMPLE_BONUS_REGO,
            }
        }),
    )
    .await;
    assert_no_errors(&body);
    let rule = &body["data"]["createRuleSet"];
    assert_eq!(rule["name"], "Minimal Rule");
    assert_eq!(rule["description"], "");
    assert_eq!(rule["enabled"], true);
    assert_eq!(rule["priority"], 0);
    assert_eq!(rule["appliedFacets"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn rego_list_rule_sets() {
    let ctx = TestContext::new().await;
    create_rule(&ctx, "Rule One", SIMPLE_BONUS_REGO).await;
    create_rule(&ctx, "Rule Two", DUAL_AUDIO_REGO).await;

    let body = gql(&ctx, "{ ruleSets { id name } }", json!({})).await;
    assert_no_errors(&body);
    let rules = body["data"]["ruleSets"].as_array().unwrap();
    assert_eq!(rules.len(), 2);
}

#[tokio::test]
async fn rego_get_by_id() {
    let ctx = TestContext::new().await;
    let id = create_rule(&ctx, "Fetch Me", SIMPLE_BONUS_REGO).await;

    let body = gql(
        &ctx,
        r#"query($id: String!) { ruleSet(id: $id) { id name regoSource } }"#,
        json!({ "id": id }),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["ruleSet"]["name"], "Fetch Me");
    assert!(body["data"]["ruleSet"]["regoSource"]
        .as_str()
        .unwrap()
        .contains("score_entry"));
}

#[tokio::test]
async fn rego_get_nonexistent() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"query($id: String!) { ruleSet(id: $id) { id } }"#,
        json!({ "id": "nonexistent-id" }),
    )
    .await;
    assert_no_errors(&body);
    assert!(body["data"]["ruleSet"].is_null());
}

#[tokio::test]
async fn rego_update_name() {
    let ctx = TestContext::new().await;
    let id = create_rule(&ctx, "Old Name", SIMPLE_BONUS_REGO).await;

    let body = gql(
        &ctx,
        r#"mutation($input: UpdateRuleSetInput!) {
            updateRuleSet(input: $input) { id name regoSource }
        }"#,
        json!({ "input": { "id": id, "name": "New Name" } }),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["updateRuleSet"]["name"], "New Name");
    // regoSource should be unchanged
    assert!(body["data"]["updateRuleSet"]["regoSource"]
        .as_str()
        .unwrap()
        .contains("score_entry"));
}

#[tokio::test]
async fn rego_update_rego_source() {
    let ctx = TestContext::new().await;
    let id = create_rule(&ctx, "Update Source", SIMPLE_BONUS_REGO).await;

    let body = gql(
        &ctx,
        r#"mutation($input: UpdateRuleSetInput!) {
            updateRuleSet(input: $input) { id regoSource }
        }"#,
        json!({ "input": { "id": id, "regoSource": DUAL_AUDIO_REGO } }),
    )
    .await;
    assert_no_errors(&body);
    assert!(body["data"]["updateRuleSet"]["regoSource"]
        .as_str()
        .unwrap()
        .contains("dual_audio_bonus"));
}

#[tokio::test]
async fn rego_delete() {
    let ctx = TestContext::new().await;
    let id = create_rule(&ctx, "To Delete", SIMPLE_BONUS_REGO).await;

    let body = gql(
        &ctx,
        r#"mutation($id: String!) { deleteRuleSet(id: $id) }"#,
        json!({ "id": id }),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["deleteRuleSet"], true);

    // Verify list is empty
    let body = gql(&ctx, "{ ruleSets { id } }", json!({})).await;
    assert_eq!(body["data"]["ruleSets"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn rego_toggle_disable() {
    let ctx = TestContext::new().await;
    let id = create_rule(&ctx, "Toggle Test", SIMPLE_BONUS_REGO).await;

    let body = gql(
        &ctx,
        r#"mutation($input: ToggleRuleSetInput!) {
            toggleRuleSet(input: $input) { id enabled }
        }"#,
        json!({ "input": { "id": id, "enabled": false } }),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["toggleRuleSet"]["enabled"], false);
}

#[tokio::test]
async fn rego_toggle_reenable() {
    let ctx = TestContext::new().await;
    let id = create_rule(&ctx, "Toggle Reenable", SIMPLE_BONUS_REGO).await;

    // Disable
    gql(
        &ctx,
        r#"mutation($input: ToggleRuleSetInput!) {
            toggleRuleSet(input: $input) { id }
        }"#,
        json!({ "input": { "id": id, "enabled": false } }),
    )
    .await;

    // Re-enable
    let body = gql(
        &ctx,
        r#"mutation($input: ToggleRuleSetInput!) {
            toggleRuleSet(input: $input) { id enabled }
        }"#,
        json!({ "input": { "id": id, "enabled": true } }),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["toggleRuleSet"]["enabled"], true);
}

// ===========================================================================
// 2. Validation
// ===========================================================================

#[tokio::test]
async fn rego_validate_valid_rule() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"mutation($input: ValidateRuleSetInput!) {
            validateRuleSet(input: $input) { valid errors }
        }"#,
        json!({ "input": { "regoSource": SIMPLE_BONUS_REGO } }),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["validateRuleSet"]["valid"], true);
    assert_eq!(
        body["data"]["validateRuleSet"]["errors"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
}

#[tokio::test]
async fn rego_validate_syntax_error() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"mutation($input: ValidateRuleSetInput!) {
            validateRuleSet(input: $input) { valid errors }
        }"#,
        json!({ "input": { "regoSource": INVALID_SYNTAX_REGO } }),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["validateRuleSet"]["valid"], false);
    assert!(
        !body["data"]["validateRuleSet"]["errors"]
            .as_array()
            .unwrap()
            .is_empty(),
        "should report syntax errors"
    );
}

#[tokio::test]
async fn rego_validate_empty_source() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"mutation($input: ValidateRuleSetInput!) {
            validateRuleSet(input: $input) { valid errors }
        }"#,
        json!({ "input": { "regoSource": "" } }),
    )
    .await;
    assert_no_errors(&body);
    // Empty source should either fail or produce a valid-but-empty result
    // (it depends on the validation logic, either is acceptable)
    let result = &body["data"]["validateRuleSet"];
    assert!(result["valid"].is_boolean());
}

#[tokio::test]
async fn rego_validate_float_rejected() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"mutation($input: ValidateRuleSetInput!) {
            validateRuleSet(input: $input) { valid errors }
        }"#,
        json!({ "input": { "regoSource": FLOAT_REGO } }),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(
        body["data"]["validateRuleSet"]["valid"], false,
        "float output should be rejected"
    );
    let errors = body["data"]["validateRuleSet"]["errors"]
        .as_array()
        .unwrap();
    assert!(!errors.is_empty());
    // Error message should mention float
    let error_text = errors[0].as_str().unwrap().to_lowercase();
    assert!(
        error_text.contains("float") || error_text.contains("integer"),
        "error should mention float/integer, got: {error_text}"
    );
}

#[tokio::test]
async fn rego_validate_builtins_accepted() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"mutation($input: ValidateRuleSetInput!) {
            validateRuleSet(input: $input) { valid errors }
        }"#,
        json!({ "input": { "regoSource": BLOCK_REGO } }),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(
        body["data"]["validateRuleSet"]["valid"], true,
        "rule using scryer.block_score() should be valid"
    );
}

#[tokio::test]
async fn rego_validate_size_gib_accepted() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"mutation($input: ValidateRuleSetInput!) {
            validateRuleSet(input: $input) { valid errors }
        }"#,
        json!({ "input": { "regoSource": SIZE_GIB_REGO } }),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(
        body["data"]["validateRuleSet"]["valid"], true,
        "rule using scryer.size_gib() should be valid"
    );
}

#[tokio::test]
async fn rego_validate_lang_matches_accepted() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"mutation($input: ValidateRuleSetInput!) {
            validateRuleSet(input: $input) { valid errors }
        }"#,
        json!({ "input": { "regoSource": LANG_MATCHES_REGO } }),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(
        body["data"]["validateRuleSet"]["valid"], true,
        "rule using scryer.lang_matches() should be valid"
    );
}

#[tokio::test]
async fn rego_create_invalid_rego_fails() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"mutation($input: CreateRuleSetInput!) {
            createRuleSet(input: $input) { id }
        }"#,
        json!({
            "input": {
                "name": "Bad Rule",
                "regoSource": INVALID_SYNTAX_REGO,
            }
        }),
    )
    .await;
    assert!(
        body.get("errors").is_some(),
        "creating a rule with invalid Rego should return errors"
    );
}

// ===========================================================================
// 3. Engine evaluation
// ===========================================================================

#[tokio::test]
async fn rego_engine_rebuilds_after_create() {
    let ctx = TestContext::new().await;

    // Before creating any rules, engine should be empty
    {
        let engine = ctx.app.services.user_rules.read().unwrap();
        assert!(engine.is_empty(), "engine should start empty");
    }

    create_rule(&ctx, "Rebuild Test", SIMPLE_BONUS_REGO).await;

    // After creating, engine should have the user rule + any plugin-declared
    // scoring policies (e.g. nzbgeek_vote_penalty, nzbgeek_language_bonus).
    let engine = ctx.app.services.user_rules.read().unwrap();
    assert!(engine.rule_count() >= 1, "engine should have at least 1 rule");
}

#[tokio::test]
async fn rego_engine_simple_bonus() {
    let ctx = TestContext::new().await;
    create_rule(&ctx, "Simple Bonus", SIMPLE_BONUS_REGO).await;

    let engine = ctx.app.services.user_rules.read().unwrap().clone();
    let mut evaluator = engine.evaluator();
    let input = test_input();
    let result = evaluator.evaluate(&input, "movie").unwrap();

    assert_eq!(result.entries.len(), 1);
    assert_eq!(result.entries[0].code, "test_bonus");
    assert_eq!(result.entries[0].delta, 100);
}

#[tokio::test]
async fn rego_engine_conditional_fires() {
    let ctx = TestContext::new().await;
    create_rule(&ctx, "Dual Audio Rule", DUAL_AUDIO_REGO).await;

    let engine = ctx.app.services.user_rules.read().unwrap().clone();
    let mut evaluator = engine.evaluator();
    let input = test_input_with(|i| {
        i.release.is_dual_audio = true;
    });
    let result = evaluator.evaluate(&input, "movie").unwrap();

    assert_eq!(result.entries.len(), 1, "should fire when dual_audio is true");
    assert_eq!(result.entries[0].code, "dual_audio_bonus");
    assert_eq!(result.entries[0].delta, 500);
}

#[tokio::test]
async fn rego_engine_conditional_skips() {
    let ctx = TestContext::new().await;
    create_rule(&ctx, "Dual Audio Rule", DUAL_AUDIO_REGO).await;

    let engine = ctx.app.services.user_rules.read().unwrap().clone();
    let mut evaluator = engine.evaluator();
    let input = test_input_with(|i| {
        i.release.is_dual_audio = false;
    });
    let result = evaluator.evaluate(&input, "movie").unwrap();

    assert_eq!(
        result.entries.len(),
        0,
        "should NOT fire when dual_audio is false"
    );
}

#[tokio::test]
async fn rego_engine_block_score() {
    let ctx = TestContext::new().await;
    create_rule(&ctx, "Block Rule", BLOCK_REGO).await;

    let engine = ctx.app.services.user_rules.read().unwrap().clone();
    let mut evaluator = engine.evaluator();
    let input = test_input_with(|i| {
        i.release.is_atmos = false;
    });
    let result = evaluator.evaluate(&input, "movie").unwrap();

    assert_eq!(result.entries.len(), 1);
    assert_eq!(result.entries[0].code, "block_it");
    assert_eq!(result.entries[0].delta, -10000);
}

#[tokio::test]
async fn rego_engine_facet_skipped() {
    let ctx = TestContext::new().await;
    create_rule_with_facets(&ctx, "Anime Only", SIMPLE_BONUS_REGO, &["anime"]).await;

    let engine = ctx.app.services.user_rules.read().unwrap().clone();
    let mut evaluator = engine.evaluator();
    let input = test_input();
    let result = evaluator.evaluate(&input, "movie").unwrap();

    assert_eq!(
        result.entries.len(),
        0,
        "anime-scoped rule should not fire for movie"
    );
}

#[tokio::test]
async fn rego_engine_facet_matches() {
    let ctx = TestContext::new().await;
    create_rule_with_facets(&ctx, "Anime Only", SIMPLE_BONUS_REGO, &["anime"]).await;

    let engine = ctx.app.services.user_rules.read().unwrap().clone();
    let mut evaluator = engine.evaluator();
    let input = test_input_with(|i| {
        i.context.category = "anime".to_string();
        i.context.is_anime = true;
    });
    let result = evaluator.evaluate(&input, "anime").unwrap();

    assert_eq!(
        result.entries.len(),
        1,
        "anime-scoped rule should fire for anime"
    );
}

#[tokio::test]
async fn rego_engine_disabled_excluded() {
    let ctx = TestContext::new().await;
    let id = create_rule(&ctx, "Disable Test", SIMPLE_BONUS_REGO).await;

    // Verify engine has the rule (plus any plugin-declared scoring policies)
    let count_with_rule = {
        let engine = ctx.app.services.user_rules.read().unwrap();
        assert!(engine.rule_count() >= 1);
        engine.rule_count()
    };

    // Disable it
    gql(
        &ctx,
        r#"mutation($input: ToggleRuleSetInput!) {
            toggleRuleSet(input: $input) { id }
        }"#,
        json!({ "input": { "id": id, "enabled": false } }),
    )
    .await;

    // Engine should have one fewer rule (the disabled user rule)
    let engine = ctx.app.services.user_rules.read().unwrap();
    assert_eq!(
        engine.rule_count(),
        count_with_rule - 1,
        "disabled rule should be excluded from engine"
    );
}

// ===========================================================================
// 4. Error handling
// ===========================================================================

#[tokio::test]
async fn rego_duplicate_name_allowed() {
    let ctx = TestContext::new().await;
    let id1 = create_rule(&ctx, "Same Name", SIMPLE_BONUS_REGO).await;
    let id2 = create_rule(&ctx, "Same Name", DUAL_AUDIO_REGO).await;

    assert_ne!(id1, id2, "should have different IDs");

    let body = gql(&ctx, "{ ruleSets { id } }", json!({})).await;
    assert_eq!(body["data"]["ruleSets"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn rego_delete_nonexistent() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"mutation($id: String!) { deleteRuleSet(id: $id) }"#,
        json!({ "id": "bogus-id" }),
    )
    .await;
    // May return error or succeed silently (delete of nonexistent is often idempotent)
    // Just verify it doesn't crash
    assert!(body["data"].is_object() || body.get("errors").is_some());
}

#[tokio::test]
async fn rego_update_nonexistent() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"mutation($input: UpdateRuleSetInput!) {
            updateRuleSet(input: $input) { id }
        }"#,
        json!({ "input": { "id": "bogus-id", "name": "New Name" } }),
    )
    .await;
    assert!(
        body.get("errors").is_some(),
        "updating nonexistent rule should return error"
    );
}

#[tokio::test]
async fn rego_toggle_nonexistent() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"mutation($input: ToggleRuleSetInput!) {
            toggleRuleSet(input: $input) { id }
        }"#,
        json!({ "input": { "id": "bogus-id", "enabled": false } }),
    )
    .await;
    assert!(
        body.get("errors").is_some(),
        "toggling nonexistent rule should return error"
    );
}
