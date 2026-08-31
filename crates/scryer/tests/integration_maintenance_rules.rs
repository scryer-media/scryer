#![recursion_limit = "256"]

//! End-to-end coverage for the maintenance-rule authoring surface (RFC 137
//! track D): the GraphQL queries and mutations over the authoring service.
//!
//! Everything here is authoring-time. Nothing in this surface schedules
//! evaluation or executes an action, so a rule always persists as disabled and
//! preview persists nothing at all.

mod common;

use async_graphql::Variables;
use common::TestContext;
use scryer_application::{TitleRepository, UserRepository};
use scryer_domain::{Id, MediaFacet, Title, User, UserAuthorization};
use serde_json::{Value, json};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

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

/// Execute against the schema directly so the test can choose the actor.
async fn gql_as(ctx: &TestContext, query: &str, variables: Value, actor: &User) -> Value {
    let request = async_graphql::Request::new(query)
        .variables(Variables::from_json(variables))
        .data(actor.clone());
    let response = ctx.schema.execute(request).await;
    serde_json::to_value(&response).expect("serialize gql response")
}

fn assert_no_errors(body: &Value) {
    assert!(
        body.get("errors").is_none(),
        "unexpected GraphQL errors: {body}"
    );
}

fn error_messages(body: &Value) -> String {
    body["errors"]
        .as_array()
        .map(|errors| {
            errors
                .iter()
                .filter_map(|error| error["message"].as_str())
                .collect::<Vec<_>>()
                .join("; ")
        })
        .unwrap_or_default()
}

fn assert_error_contains(body: &Value, expected: &str) {
    let messages = error_messages(body);
    assert!(
        !messages.is_empty(),
        "expected GraphQL errors containing {expected:?}: {body}"
    );
    assert!(
        messages.contains(expected),
        "expected GraphQL error containing {expected:?}, got {messages}"
    );
}

/// Matches every monitored title. The package line is deliberately absent: the
/// server owns it, and the editor never sees it.
const MONITORED_MATCHER: &str = "match if {\n\
     \tinput.facts.monitored.status == \"known\"\n\
     \tinput.facts.monitored.value\n\
     }\n";

/// Matches nothing: the facet never equals this sentinel.
const NEVER_MATCHER: &str = "match if {\n\
     \tinput.subject.facet == \"not-a-facet\"\n\
     }\n";

/// Reads a fact this wave cannot observe, so every title must be held rather
/// than reported as a no-match.
const ACTIVE_DOWNLOADS_MATCHER: &str = "match if {\n\
     \tinput.facts.active_downloads.value\n\
     }\n\n\
     unknown if {\n\
     \tinput.facts.active_downloads.status != \"known\"\n\
     }\n\n\
     reasons contains \"active_downloads_unavailable\" if {\n\
     \tinput.facts.active_downloads.status != \"known\"\n\
     }\n";

const INVALID_MATCHER: &str = "match if { this is not rego\n";

const DETAIL_FIELDS: &str = r#"
    ruleSet {
        id
        name
        description
        enabled
        evaluationMode
        libraryIds
        subjectKind
        currentRevisionNumber
        graceDays
        actionSpec { kind schemaVersion targetQualityProfileId }
        createdAt
        updatedAt
    }
    revision {
        id
        ruleSetId
        revisionNumber
        regoSource
        graceDays
        matcherContentHash
        createdBy
        createdAt
    }
    actionSpec {
        kind
        schemaVersion
        targetQualityProfileId
    }
"#;

fn create_mutation() -> String {
    format!(
        "mutation($input: CreateMaintenanceRuleSetInput!) {{
            createMaintenanceRuleSet(input: $input) {{{DETAIL_FIELDS}}}
        }}"
    )
}

fn update_matcher_mutation() -> String {
    format!(
        "mutation($input: UpdateMaintenanceRuleMatcherInput!) {{
            updateMaintenanceRuleMatcher(input: $input) {{{DETAIL_FIELDS}}}
        }}"
    )
}

fn get_query() -> String {
    format!(
        "query($id: ID!) {{
            maintenanceRuleSet(id: $id) {{{DETAIL_FIELDS}}}
        }}"
    )
}

const LIST_QUERY: &str = r#"query {
    maintenanceRuleSets {
        id
        name
        enabled
        evaluationMode
        subjectKind
        libraryIds
        currentRevisionNumber
        graceDays
        actionSpec { kind schemaVersion targetQualityProfileId }
    }
}"#;

const REVISIONS_QUERY: &str = r#"query($ruleSetId: ID!) {
    maintenanceRuleRevisions(ruleSetId: $ruleSetId) {
        id
        ruleSetId
        revisionNumber
        regoSource
        graceDays
        matcherContentHash
    }
}"#;

const PREVIEW_MUTATION: &str = r#"mutation($input: PreviewMaintenanceRuleInput!) {
    previewMaintenanceRule(input: $input) {
        ruleSetId
        matcherContentHash
        evaluatedAt
        titles {
            titleId
            titleName
            facet
            libraryId
            outcome
            reasonCodes
            error
        }
    }
}"#;

const VALIDATE_MUTATION: &str = r#"mutation($input: ValidateMaintenanceRuleInput!) {
    validateMaintenanceRule(input: $input) { valid errors }
}"#;

/// Create a rule set and return the whole detail payload.
async fn create_rule(ctx: &TestContext, name: &str, rego: &str, action: Value) -> Value {
    let body = gql(
        ctx,
        &create_mutation(),
        json!({
            "input": {
                "name": name,
                "regoSource": rego,
                "action": action,
            }
        }),
    )
    .await;
    assert_no_errors(&body);
    body["data"]["createMaintenanceRuleSet"].clone()
}

fn delete_action() -> Value {
    json!({ "kind": "DELETE_TITLE_AND_FILES" })
}

fn movie_library_id() -> String {
    scryer_domain::default_library_id_for_facet(&MediaFacet::Movie)
}

async fn seed_title(ctx: &TestContext, id: &str, name: &str, monitored: bool) -> Title {
    let title = Title {
        id: id.to_string(),
        name: name.to_string(),
        facet: MediaFacet::Movie,
        library_id: movie_library_id(),
        monitored,
        tags: vec![],
        canonical_tags: vec![],
        external_ids: vec![],
        root_folder_id: scryer_domain::root_folder_id_for_path("/data/movies"),
        created_by: None,
        created_at: chrono::Utc::now(),
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
    };
    TitleRepository::create(&ctx.titles, title)
        .await
        .expect("seed title")
}

/// A stored user with no application permissions at all.
async fn create_unprivileged_user(ctx: &TestContext, username: &str) -> User {
    ctx.users
        .create(User {
            id: Id::new().0,
            username: username.to_string(),
            password_hash: None,
            password_change_required: false,
            account_kind: Default::default(),
            authorization: UserAuthorization::default(),
        })
        .await
        .expect("unprivileged user should create")
}

fn outcome_for<'a>(preview: &'a Value, title_id: &str) -> &'a Value {
    preview["titles"]
        .as_array()
        .expect("preview titles")
        .iter()
        .find(|title| title["titleId"] == title_id)
        .unwrap_or_else(|| panic!("preview should cover {title_id}: {preview}"))
}

// ===========================================================================
// 1. Authoring round trip
// ===========================================================================

#[tokio::test]
async fn maintenance_rule_create_list_and_get_round_trip() {
    let ctx = TestContext::new().await;

    let body = gql(
        &ctx,
        &create_mutation(),
        json!({
            "input": {
                "name": "Stale movies",
                "description": "Movies nobody monitors",
                "regoSource": MONITORED_MATCHER,
                "action": delete_action(),
                "graceDays": 14,
                "libraryIds": ["lib-movies"],
            }
        }),
    )
    .await;
    assert_no_errors(&body);

    let detail = &body["data"]["createMaintenanceRuleSet"];
    let rule_set = &detail["ruleSet"];
    let id = rule_set["id"].as_str().expect("rule set id").to_string();
    assert_eq!(rule_set["name"], "Stale movies");
    assert_eq!(rule_set["description"], "Movies nobody monitors");
    // Ships dark: nothing evaluates stored rules yet.
    assert_eq!(rule_set["enabled"], false);
    assert_eq!(rule_set["evaluationMode"], "DISABLED");
    assert_eq!(rule_set["subjectKind"], "TITLE");
    assert_eq!(rule_set["libraryIds"], json!(["lib-movies"]));
    assert_eq!(rule_set["currentRevisionNumber"], 1);

    let revision = &detail["revision"];
    assert_eq!(revision["revisionNumber"], 1);
    assert_eq!(revision["ruleSetId"], json!(id));
    assert_eq!(revision["graceDays"], 14);
    assert!(
        revision["matcherContentHash"]
            .as_str()
            .is_some_and(|hash| !hash.is_empty())
    );
    let source = revision["regoSource"].as_str().expect("rego source");
    assert!(
        source.contains("input.facts.monitored"),
        "the editor must get its own source back: {source}"
    );
    assert!(
        !source.contains("package ") && !source.contains("import rego.v1"),
        "server-owned boilerplate must be stripped for the editor: {source}"
    );

    assert_eq!(detail["actionSpec"]["kind"], "DELETE_TITLE_AND_FILES");
    assert_eq!(detail["actionSpec"]["schemaVersion"], 1);
    assert_eq!(detail["actionSpec"]["targetQualityProfileId"], Value::Null);

    let list = gql(&ctx, LIST_QUERY, json!({})).await;
    assert_no_errors(&list);
    let rule_sets = list["data"]["maintenanceRuleSets"]
        .as_array()
        .expect("rule set list");
    assert_eq!(rule_sets.len(), 1);
    assert_eq!(rule_sets[0]["id"], json!(id));
    assert_eq!(rule_sets[0]["currentRevisionNumber"], 1);
    // The list carries what the revision in force does, so rendering a saved
    // rule never costs a second round trip.
    assert_eq!(rule_sets[0]["graceDays"], 14);
    assert_eq!(rule_sets[0]["actionSpec"]["kind"], "DELETE_TITLE_AND_FILES");
    assert_eq!(rule_sets[0]["actionSpec"]["schemaVersion"], 1);
    assert_eq!(
        rule_sets[0]["actionSpec"]["targetQualityProfileId"],
        Value::Null
    );
    assert_eq!(rule_set["graceDays"], 14);
    assert_eq!(rule_set["actionSpec"]["kind"], "DELETE_TITLE_AND_FILES");

    let fetched = gql(&ctx, &get_query(), json!({ "id": id })).await;
    assert_no_errors(&fetched);
    assert_eq!(fetched["data"]["maintenanceRuleSet"], *detail);

    let missing = gql(&ctx, &get_query(), json!({ "id": "no-such-rule" })).await;
    assert_no_errors(&missing);
    assert_eq!(missing["data"]["maintenanceRuleSet"], Value::Null);
}

#[tokio::test]
async fn maintenance_rule_carries_the_quality_profile_target_through_the_action_spec() {
    let ctx = TestContext::new().await;

    let detail = create_rule(
        &ctx,
        "Upgrade to 1080p",
        MONITORED_MATCHER,
        json!({
            "kind": "CHANGE_QUALITY_PROFILE_AND_SEARCH_IF_CHANGED",
            "targetQualityProfileId": "hd-1080p",
        }),
    )
    .await;
    assert_eq!(
        detail["actionSpec"]["kind"],
        "CHANGE_QUALITY_PROFILE_AND_SEARCH_IF_CHANGED"
    );
    assert_eq!(detail["actionSpec"]["targetQualityProfileId"], "hd-1080p");

    let id = detail["ruleSet"]["id"].as_str().expect("rule set id");
    let fetched = gql(&ctx, &get_query(), json!({ "id": id })).await;
    assert_no_errors(&fetched);
    assert_eq!(
        fetched["data"]["maintenanceRuleSet"]["actionSpec"]["targetQualityProfileId"],
        "hd-1080p"
    );
}

#[tokio::test]
async fn maintenance_rule_quality_profile_action_without_a_target_is_rejected() {
    let ctx = TestContext::new().await;

    let body = gql(
        &ctx,
        &create_mutation(),
        json!({
            "input": {
                "name": "Missing target",
                "regoSource": MONITORED_MATCHER,
                "action": { "kind": "CHANGE_QUALITY_PROFILE_AND_SEARCH_IF_CHANGED" },
            }
        }),
    )
    .await;
    assert_error_contains(&body, "validation:");

    let list = gql(&ctx, LIST_QUERY, json!({})).await;
    assert_no_errors(&list);
    assert!(
        list["data"]["maintenanceRuleSets"]
            .as_array()
            .expect("rule set list")
            .is_empty(),
        "a rejected action must not persist a rule set"
    );
}

#[tokio::test]
async fn maintenance_rule_matcher_update_appends_a_revision() {
    let ctx = TestContext::new().await;

    let created = create_rule(&ctx, "Stale movies", MONITORED_MATCHER, delete_action()).await;
    let id = created["ruleSet"]["id"].as_str().expect("id").to_string();
    let first_hash = created["revision"]["matcherContentHash"]
        .as_str()
        .expect("hash")
        .to_string();

    let updated = gql(
        &ctx,
        &update_matcher_mutation(),
        json!({
            "input": {
                "id": id,
                "regoSource": NEVER_MATCHER,
                "action": { "kind": "UNMONITOR_SCOPE_KEEP_FILES" },
                "graceDays": 30,
            }
        }),
    )
    .await;
    assert_no_errors(&updated);

    let detail = &updated["data"]["updateMaintenanceRuleMatcher"];
    assert_eq!(detail["ruleSet"]["currentRevisionNumber"], 2);
    assert_eq!(detail["revision"]["revisionNumber"], 2);
    assert_eq!(detail["revision"]["graceDays"], 30);
    assert_eq!(detail["actionSpec"]["kind"], "UNMONITOR_SCOPE_KEEP_FILES");
    assert_ne!(
        detail["revision"]["matcherContentHash"],
        json!(first_hash),
        "a new matcher must hash differently"
    );

    let revisions = gql(&ctx, REVISIONS_QUERY, json!({ "ruleSetId": id })).await;
    assert_no_errors(&revisions);
    let revisions = revisions["data"]["maintenanceRuleRevisions"]
        .as_array()
        .expect("revision list")
        .clone();
    assert_eq!(revisions.len(), 2);
    assert_eq!(revisions[0]["revisionNumber"], 2);
    assert_eq!(revisions[1]["revisionNumber"], 1);
    // Revision one is immutable: it still carries the source it was written
    // with, so a decision recorded against it stays attributable.
    assert_eq!(revisions[1]["matcherContentHash"], json!(first_hash));
    assert!(
        revisions[1]["regoSource"]
            .as_str()
            .expect("stored source")
            .contains("input.facts.monitored")
    );
}

#[tokio::test]
async fn maintenance_rule_metadata_update_does_not_create_a_revision() {
    let ctx = TestContext::new().await;

    let created = create_rule(&ctx, "Stale movies", MONITORED_MATCHER, delete_action()).await;
    let id = created["ruleSet"]["id"].as_str().expect("id").to_string();

    let renamed = gql(
        &ctx,
        r#"mutation($input: UpdateMaintenanceRuleMetadataInput!) {
            updateMaintenanceRuleMetadata(input: $input) {
                id
                name
                description
                libraryIds
                currentRevisionNumber
                graceDays
                actionSpec { kind targetQualityProfileId }
            }
        }"#,
        json!({
            "input": {
                "id": id,
                "name": "Renamed",
                "description": "Now scoped",
                "libraryIds": ["lib-a", "lib-b"],
            }
        }),
    )
    .await;
    assert_no_errors(&renamed);

    let rule_set = &renamed["data"]["updateMaintenanceRuleMetadata"];
    assert_eq!(rule_set["name"], "Renamed");
    assert_eq!(rule_set["description"], "Now scoped");
    assert_eq!(rule_set["libraryIds"], json!(["lib-a", "lib-b"]));
    assert_eq!(rule_set["currentRevisionNumber"], 1);
    // Renaming leaves the matcher alone, so the action it authorizes is
    // unchanged and still reported on the rule set itself.
    assert_eq!(rule_set["graceDays"], 0);
    assert_eq!(rule_set["actionSpec"]["kind"], "DELETE_TITLE_AND_FILES");

    let revisions = gql(&ctx, REVISIONS_QUERY, json!({ "ruleSetId": id })).await;
    assert_no_errors(&revisions);
    assert_eq!(
        revisions["data"]["maintenanceRuleRevisions"]
            .as_array()
            .expect("revision list")
            .len(),
        1,
        "renaming a rule must not touch its matcher history"
    );
}

// ===========================================================================
// 2. Validation
// ===========================================================================

#[tokio::test]
async fn maintenance_rule_validation_reports_errors_without_persisting() {
    let ctx = TestContext::new().await;

    let invalid = gql(
        &ctx,
        VALIDATE_MUTATION,
        json!({ "input": { "regoSource": INVALID_MATCHER } }),
    )
    .await;
    assert_no_errors(&invalid);
    let result = &invalid["data"]["validateMaintenanceRule"];
    assert_eq!(result["valid"], false);
    assert!(
        !result["errors"]
            .as_array()
            .expect("validation errors")
            .is_empty(),
        "an invalid matcher must say why: {result}"
    );

    let valid = gql(
        &ctx,
        VALIDATE_MUTATION,
        json!({ "input": { "regoSource": MONITORED_MATCHER } }),
    )
    .await;
    assert_no_errors(&valid);
    assert_eq!(valid["data"]["validateMaintenanceRule"]["valid"], true);
    assert_eq!(
        valid["data"]["validateMaintenanceRule"]["errors"],
        json!([])
    );

    let list = gql(&ctx, LIST_QUERY, json!({})).await;
    assert_no_errors(&list);
    assert!(
        list["data"]["maintenanceRuleSets"]
            .as_array()
            .expect("rule set list")
            .is_empty(),
        "validation must never persist a rule set"
    );
}

#[tokio::test]
async fn maintenance_rule_creation_rejects_a_matcher_that_does_not_compile() {
    let ctx = TestContext::new().await;

    let body = gql(
        &ctx,
        &create_mutation(),
        json!({
            "input": {
                "name": "Broken",
                "regoSource": INVALID_MATCHER,
                "action": delete_action(),
            }
        }),
    )
    .await;
    assert_error_contains(&body, "validation:");
}

// ===========================================================================
// 3. Preview
// ===========================================================================

#[tokio::test]
async fn maintenance_rule_preview_separates_match_from_no_match() {
    let ctx = TestContext::new().await;
    seed_title(&ctx, "title-monitored", "Monitored Movie", true).await;
    seed_title(&ctx, "title-unmonitored", "Unmonitored Movie", false).await;

    let body = gql(
        &ctx,
        PREVIEW_MUTATION,
        json!({
            "input": {
                "regoSource": MONITORED_MATCHER,
                "action": delete_action(),
                "titleIds": ["title-monitored", "title-unmonitored"],
            }
        }),
    )
    .await;
    assert_no_errors(&body);

    let preview = &body["data"]["previewMaintenanceRule"];
    assert!(
        preview["matcherContentHash"]
            .as_str()
            .is_some_and(|hash| !hash.is_empty())
    );
    assert!(preview["evaluatedAt"].is_string());
    assert_eq!(preview["titles"].as_array().expect("titles").len(), 2);

    let matched = outcome_for(preview, "title-monitored");
    assert_eq!(matched["outcome"], "MATCH");
    assert_eq!(matched["error"], Value::Null);
    assert_eq!(matched["titleName"], "Monitored Movie");
    assert_eq!(matched["facet"], "movie");
    assert_eq!(matched["libraryId"], json!(movie_library_id()));

    let unmatched = outcome_for(preview, "title-unmonitored");
    assert_eq!(unmatched["outcome"], "NO_MATCH");
    assert_eq!(unmatched["error"], Value::Null);

    // Preview answers "what would this match", never "what happened".
    let list = gql(&ctx, LIST_QUERY, json!({})).await;
    assert_no_errors(&list);
    assert!(
        list["data"]["maintenanceRuleSets"]
            .as_array()
            .expect("rule set list")
            .is_empty()
    );
}

#[tokio::test]
async fn maintenance_rule_preview_holds_a_title_on_an_unobservable_fact() {
    let ctx = TestContext::new().await;
    seed_title(&ctx, "title-monitored", "Monitored Movie", true).await;

    let body = gql(
        &ctx,
        PREVIEW_MUTATION,
        json!({
            "input": {
                "regoSource": ACTIVE_DOWNLOADS_MATCHER,
                "action": delete_action(),
                "titleIds": ["title-monitored"],
            }
        }),
    )
    .await;
    assert_no_errors(&body);

    let preview = &body["data"]["previewMaintenanceRule"];
    let held = outcome_for(preview, "title-monitored");
    // An unobservable fact must hold the rule, never read as a no-match: the
    // action here deletes files.
    assert_eq!(held["outcome"], "UNKNOWN");
    assert_eq!(held["error"], Value::Null);
    assert_eq!(held["reasonCodes"], json!(["active_downloads_unavailable"]));
}

#[tokio::test]
async fn maintenance_rule_preview_runs_a_stored_rule_at_its_current_revision() {
    let ctx = TestContext::new().await;
    seed_title(&ctx, "title-monitored", "Monitored Movie", true).await;

    let created = create_rule(&ctx, "Stale movies", MONITORED_MATCHER, delete_action()).await;
    let id = created["ruleSet"]["id"].as_str().expect("id").to_string();

    let body = gql(
        &ctx,
        PREVIEW_MUTATION,
        json!({
            "input": {
                "ruleSetId": id,
                "libraryId": movie_library_id(),
                "limit": 10,
            }
        }),
    )
    .await;
    assert_no_errors(&body);

    let preview = &body["data"]["previewMaintenanceRule"];
    assert_eq!(preview["ruleSetId"], json!(id));
    assert_eq!(
        preview["matcherContentHash"], created["revision"]["matcherContentHash"],
        "a stored preview must be attributable to the revision in force"
    );
    assert_eq!(outcome_for(preview, "title-monitored")["outcome"], "MATCH");
}

#[tokio::test]
async fn maintenance_rule_preview_rejects_an_oversized_selection() {
    let ctx = TestContext::new().await;

    let title_ids: Vec<String> = (0..51).map(|index| format!("title-{index}")).collect();
    let body = gql(
        &ctx,
        PREVIEW_MUTATION,
        json!({
            "input": {
                "regoSource": MONITORED_MATCHER,
                "action": delete_action(),
                "titleIds": title_ids,
            }
        }),
    )
    .await;
    assert_error_contains(&body, "at most 50 titles");
}

#[tokio::test]
async fn maintenance_rule_preview_requires_exactly_one_matcher_and_one_selection() {
    let ctx = TestContext::new().await;

    let both_matchers = gql(
        &ctx,
        PREVIEW_MUTATION,
        json!({
            "input": {
                "ruleSetId": "rule-1",
                "regoSource": MONITORED_MATCHER,
                "action": delete_action(),
                "titleIds": ["title-1"],
            }
        }),
    )
    .await;
    assert_error_contains(&both_matchers, "not both");

    let no_matcher = gql(
        &ctx,
        PREVIEW_MUTATION,
        json!({ "input": { "titleIds": ["title-1"] } }),
    )
    .await;
    assert_error_contains(&no_matcher, "'ruleSetId' or 'regoSource'");

    let draft_without_action = gql(
        &ctx,
        PREVIEW_MUTATION,
        json!({
            "input": {
                "regoSource": MONITORED_MATCHER,
                "titleIds": ["title-1"],
            }
        }),
    )
    .await;
    assert_error_contains(&draft_without_action, "requires 'action'");

    let both_selections = gql(
        &ctx,
        PREVIEW_MUTATION,
        json!({
            "input": {
                "regoSource": MONITORED_MATCHER,
                "action": delete_action(),
                "titleIds": ["title-1"],
                "libraryId": movie_library_id(),
            }
        }),
    )
    .await;
    assert_error_contains(&both_selections, "'titleIds' or 'libraryId', not both");

    let no_selection = gql(
        &ctx,
        PREVIEW_MUTATION,
        json!({
            "input": {
                "regoSource": MONITORED_MATCHER,
                "action": delete_action(),
            }
        }),
    )
    .await;
    assert_error_contains(&no_selection, "requires either 'titleIds' or 'libraryId'");
}

// ===========================================================================
// 4. Deletion
// ===========================================================================

#[tokio::test]
async fn maintenance_rule_delete_removes_the_rule_set() {
    let ctx = TestContext::new().await;

    let created = create_rule(&ctx, "Stale movies", MONITORED_MATCHER, delete_action()).await;
    let id = created["ruleSet"]["id"].as_str().expect("id").to_string();

    let deleted = gql(
        &ctx,
        r#"mutation($id: ID!) {
            deleteMaintenanceRuleSet(id: $id) { id }
        }"#,
        json!({ "id": id }),
    )
    .await;
    assert_no_errors(&deleted);
    assert_eq!(deleted["data"]["deleteMaintenanceRuleSet"]["id"], json!(id));

    let list = gql(&ctx, LIST_QUERY, json!({})).await;
    assert_no_errors(&list);
    assert!(
        list["data"]["maintenanceRuleSets"]
            .as_array()
            .expect("rule set list")
            .is_empty()
    );

    let fetched = gql(&ctx, &get_query(), json!({ "id": id })).await;
    assert_no_errors(&fetched);
    assert_eq!(fetched["data"]["maintenanceRuleSet"], Value::Null);

    let missing = gql(
        &ctx,
        r#"mutation($id: ID!) {
            deleteMaintenanceRuleSet(id: $id) { id }
        }"#,
        json!({ "id": "no-such-rule" }),
    )
    .await;
    assert!(
        missing.get("errors").is_some(),
        "deleting an unknown rule set must fail: {missing}"
    );
}

// ===========================================================================
// 5. Action catalog
// ===========================================================================

#[tokio::test]
async fn maintenance_action_descriptors_expose_the_static_catalog() {
    let ctx = TestContext::new().await;

    let body = gql(
        &ctx,
        r#"query {
            maintenanceActionDescriptors {
                kind
                supportedSubjects
                riskClass
                effectClasses
                timingMode
                allowedRepeatModes
                requiresTargetQualityProfile
            }
        }"#,
        json!({}),
    )
    .await;
    assert_no_errors(&body);

    let descriptors = body["data"]["maintenanceActionDescriptors"]
        .as_array()
        .expect("descriptor list");
    assert_eq!(descriptors.len(), 9);

    let delete = descriptors
        .iter()
        .find(|descriptor| descriptor["kind"] == "DELETE_TITLE_AND_FILES")
        .expect("delete descriptor");
    assert_eq!(delete["supportedSubjects"], json!(["MOVIE", "SHOW"]));
    assert_eq!(delete["riskClass"], "HIGH");
    assert_eq!(delete["effectClasses"], json!(["destructive_storage"]));
    assert_eq!(delete["timingMode"], "after_grace");
    assert_eq!(delete["allowedRepeatModes"], json!(["once_per_match"]));
    assert_eq!(delete["requiresTargetQualityProfile"], false);

    let requiring: Vec<&Value> = descriptors
        .iter()
        .filter(|descriptor| descriptor["requiresTargetQualityProfile"] == json!(true))
        .collect();
    assert_eq!(requiring.len(), 1);
    assert_eq!(
        requiring[0]["kind"],
        "CHANGE_QUALITY_PROFILE_AND_SEARCH_IF_CHANGED"
    );
}

// ===========================================================================
// 6. Permissions
// ===========================================================================

#[tokio::test]
async fn maintenance_rule_surface_denies_an_actor_without_catalog_settings() {
    let ctx = TestContext::new().await;
    let denied = create_unprivileged_user(&ctx, "maintenance-denied").await;

    let created = gql_as(
        &ctx,
        &create_mutation(),
        json!({
            "input": {
                "name": "Not allowed",
                "regoSource": MONITORED_MATCHER,
                "action": delete_action(),
            }
        }),
        &denied,
    )
    .await;
    assert_error_contains(&created, "unauthorized");

    let listed = gql_as(&ctx, LIST_QUERY, json!({}), &denied).await;
    assert_error_contains(&listed, "unauthorized");

    let descriptors = gql_as(
        &ctx,
        "query { maintenanceActionDescriptors { kind } }",
        json!({}),
        &denied,
    )
    .await;
    assert_error_contains(&descriptors, "unauthorized");

    let previewed = gql_as(
        &ctx,
        PREVIEW_MUTATION,
        json!({
            "input": {
                "regoSource": MONITORED_MATCHER,
                "action": delete_action(),
                "titleIds": ["title-1"],
            }
        }),
        &denied,
    )
    .await;
    assert_error_contains(&previewed, "unauthorized");

    // Nothing the denied actor attempted may have persisted.
    let list = gql(&ctx, LIST_QUERY, json!({})).await;
    assert_no_errors(&list);
    assert!(
        list["data"]["maintenanceRuleSets"]
            .as_array()
            .expect("rule set list")
            .is_empty()
    );
}
