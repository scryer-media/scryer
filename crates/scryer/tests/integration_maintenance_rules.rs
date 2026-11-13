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
     \tinput.facts.monitored\n\
     }\n";

/// Matches nothing: the facet never equals this sentinel.
const NEVER_MATCHER: &str = "match if {\n\
     \tinput.subject.facet == \"not-a-facet\"\n\
     }\n";

/// Reads a fact this wave cannot observe, so every title must be held rather
/// than reported as a no-match. Nothing in the source says so: the server
/// derives the hold from the fact the matcher references, which is exactly the
/// mistake an author can no longer make.
const ACTIVE_DOWNLOADS_MATCHER: &str = "match if {\n\
     \tnot input.facts.active_downloads\n\
     }\n";

const INVALID_MATCHER: &str = "match if { this is not rego\n";

const DETAIL_FIELDS: &str = r#"
    ruleSet {
        id
        name
        description
        state
        enabledArming
        editedSinceEnabled
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
        state
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
        movie_release_dates: None,
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
                "libraryIds": [movie_library_id()],
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
    assert_eq!(rule_set["state"], "DISABLED");
    assert_eq!(rule_set["enabledArming"], "DESTRUCTIVE");
    assert_eq!(rule_set["editedSinceEnabled"], false);
    assert_eq!(rule_set["subjectKind"], "TITLE");
    assert_eq!(rule_set["libraryIds"], json!([movie_library_id()]));
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
    let library_ids = vec![
        movie_library_id(),
        scryer_domain::default_library_id_for_facet(&MediaFacet::Series),
    ];

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
                "libraryIds": library_ids,
            }
        }),
    )
    .await;
    assert_no_errors(&renamed);

    let rule_set = &renamed["data"]["updateMaintenanceRuleMetadata"];
    assert_eq!(rule_set["name"], "Renamed");
    assert_eq!(rule_set["description"], "Now scoped");
    assert_eq!(rule_set["libraryIds"], json!(library_ids));
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
    assert_eq!(held["reasonCodes"], json!(["not_yet_collected"]));
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
                requiresTags
            }
        }"#,
        json!({}),
    )
    .await;
    assert_no_errors(&body);

    let descriptors = body["data"]["maintenanceActionDescriptors"]
        .as_array()
        .expect("descriptor list");
    assert_eq!(descriptors.len(), 11);

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

    // The tag actions are the second parameterized pair, and the editor learns
    // that from the descriptor rather than from a hardcoded list of kinds.
    let tagging: Vec<&Value> = descriptors
        .iter()
        .filter(|descriptor| descriptor["requiresTags"] == json!(true))
        .collect();
    assert_eq!(tagging.len(), 2);
    let tagging_kinds: Vec<&Value> = tagging
        .iter()
        .map(|descriptor| &descriptor["kind"])
        .collect();
    assert_eq!(
        tagging_kinds,
        vec![&json!("ADD_TAGS"), &json!("REMOVE_TAGS")]
    );
    let add = tagging[0];
    assert_eq!(add["supportedSubjects"], json!(["MOVIE", "SHOW"]));
    assert_eq!(add["riskClass"], "LOW");
    assert_eq!(add["effectClasses"], json!(["catalog_intent"]));
    assert_eq!(add["timingMode"], "after_grace");
    assert_eq!(add["allowedRepeatModes"], json!(["ensure_state"]));
    assert_eq!(add["requiresTargetQualityProfile"], false);
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

// ===========================================================================
// 6. Scheduled dark evaluation (RFC 137 tracks C1/C2)
// ===========================================================================

const CANDIDATE_FIELDS: &str = r#"
    id
    ruleSetId
    ruleName
    revisionNumber
    titleId
    titleName
    libraryId
    facet
    state
    stateReason
    reasonCodes
    actionKind
    graceDays
    matchGeneration
    firstMatchedAt
    lastMatchedAt
    dueAt
    heldSince
    updatedAt
"#;

fn candidates_query() -> String {
    format!(
        "query($ruleSetId: ID, $states: [MaintenanceCandidateState!]) {{
            maintenanceCandidates(
                ruleSetId: $ruleSetId
                states: $states
            ) {{{CANDIDATE_FIELDS}}}
        }}"
    )
}

const GATES_FIELDS: &str = r#"
    enabled
    actionsPaused
"#;

fn gates_query() -> String {
    format!("query {{ maintenanceInstanceGates {{{GATES_FIELDS}}} }}")
}

fn set_gates_mutation() -> String {
    format!(
        "mutation($input: SetMaintenanceInstanceGatesInput!) {{
            setMaintenanceInstanceGates(input: $input) {{{GATES_FIELDS}}}
        }}"
    )
}

const SET_STATE_MUTATION: &str = r#"mutation(
    $ruleSetId: ID!
    $state: MaintenanceRuleState!
    $confirmation: MaintenanceRuleArmingConfirmationInput
) {
    setMaintenanceRuleState(ruleSetId: $ruleSetId, state: $state, confirmation: $confirmation) {
        id
        state
        enabledArming
        editedSinceEnabled
        currentRevisionNumber
    }
}"#;

/// Set one rule's state, checking the response carries no errors.
async fn set_rule_state(
    ctx: &TestContext,
    rule_set_id: &str,
    state: &str,
    confirmation: Value,
) -> Value {
    let body = gql(
        ctx,
        SET_STATE_MUTATION,
        json!({ "ruleSetId": rule_set_id, "state": state, "confirmation": confirmation }),
    )
    .await;
    assert_no_errors(&body);
    body["data"]["setMaintenanceRuleState"].clone()
}

const RUN_NOW_MUTATION: &str = r#"mutation($ruleSetId: ID) {
    runMaintenanceEvaluationNow(ruleSetId: $ruleSetId) { started message }
}"#;

const EXCLUSION_FIELDS: &str = r#"
    id
    ruleSetId
    titleId
    titleName
    reason
    createdBy
    createdAt
"#;

fn exclude_mutation() -> String {
    format!(
        "mutation($input: ExcludeMaintenanceSubjectInput!) {{
            excludeMaintenanceSubject(input: $input) {{{EXCLUSION_FIELDS}}}
        }}"
    )
}

fn exclusions_query() -> String {
    format!(
        "query($ruleSetId: ID) {{
            maintenanceExclusions(ruleSetId: $ruleSetId) {{{EXCLUSION_FIELDS}}}
        }}"
    )
}

const REMOVE_EXCLUSION_MUTATION: &str = r#"mutation($id: ID!) {
    removeMaintenanceExclusion(id: $id) { id }
}"#;

const EVALUATION_RUNS_QUERY: &str = r#"query($ruleSetId: ID) {
    maintenanceEvaluationRuns(ruleSetId: $ruleSetId, limit: 10) {
        id
        ruleSetId
        revisionNumber
        status
        startedAt
        finishedAt
        evaluatedCount
        matchedCount
        noMatchCount
        unknownCount
        errorCount
        durationMs
        error
    }
}"#;

/// Register the four gate definitions the way service bootstrap does. The test
/// harness builds its stores directly rather than running bootstrap, and a
/// settings write against an unregistered key is refused by the store.
async fn seed_maintenance_gate_definitions(ctx: &TestContext) {
    let seeds = [
        "maintenance.gate.evaluation",
        "maintenance.gate.result_display",
        "maintenance.gate.reversible_effects",
        "maintenance.gate.destructive_effects",
    ]
    .into_iter()
    .map(
        |key_name| scryer_infrastructure_sql::types::SettingDefinitionSeed {
            category: "general".into(),
            scope: "system".into(),
            key_name: key_name.into(),
            data_type: "boolean".into(),
            default_value_json: "false".into(),
            is_sensitive: false,
            validation_json: None,
        },
    )
    .collect();

    ctx.settings_store
        .batch_ensure_setting_definitions(seeds)
        .await
        .expect("seed maintenance gate definitions");
}

/// Put one rule in dry run and switch maintenance on: both are always
/// deliberate, separate steps, which is the whole point of shipping dark.
async fn arm_rule_and_gate(ctx: &TestContext, rule_set_id: &str) {
    seed_maintenance_gate_definitions(ctx).await;
    let dry_run = set_rule_state(ctx, rule_set_id, "DRY_RUN", Value::Null).await;
    assert_eq!(dry_run["state"], "DRY_RUN");

    let gates = gql(
        ctx,
        &set_gates_mutation(),
        json!({ "input": { "enabled": true } }),
    )
    .await;
    assert_no_errors(&gates);
    let switched = &gates["data"]["setMaintenanceInstanceGates"];
    assert_eq!(switched["enabled"], true);
    assert_eq!(
        switched["actionsPaused"], true,
        "enabling never unpauses actions"
    );
}

#[tokio::test]
async fn maintenance_evaluation_is_dark_until_both_the_rule_and_the_gate_are_armed() {
    let ctx = TestContext::new().await;
    seed_title(&ctx, "title-dark-1", "Monitored Movie", true).await;
    let detail = create_rule(&ctx, "Stale movies", MONITORED_MATCHER, delete_action()).await;
    let rule_set_id = detail["ruleSet"]["id"]
        .as_str()
        .expect("rule id")
        .to_string();
    seed_maintenance_gate_definitions(&ctx).await;

    // Gate off: the run is refused and says so rather than silently doing
    // nothing.
    let gated = gql(&ctx, RUN_NOW_MUTATION, json!({ "ruleSetId": rule_set_id })).await;
    assert_no_errors(&gated);
    assert_eq!(
        gated["data"]["runMaintenanceEvaluationNow"]["started"],
        false
    );
    assert!(
        gated["data"]["runMaintenanceEvaluationNow"]["message"]
            .as_str()
            .expect("message")
            .contains("gate is off")
    );

    // Maintenance on, rule still disabled: still nothing.
    let gates = gql(
        &ctx,
        &set_gates_mutation(),
        json!({ "input": { "enabled": true } }),
    )
    .await;
    assert_no_errors(&gates);
    let disabled = gql(&ctx, RUN_NOW_MUTATION, json!({ "ruleSetId": rule_set_id })).await;
    assert_no_errors(&disabled);
    assert_eq!(
        disabled["data"]["runMaintenanceEvaluationNow"]["started"],
        false
    );

    let candidates = gql(&ctx, &candidates_query(), json!({})).await;
    assert_no_errors(&candidates);
    assert!(
        candidates["data"]["maintenanceCandidates"]
            .as_array()
            .expect("candidates")
            .is_empty(),
        "nothing may be recorded while either switch is off"
    );
}

#[tokio::test]
async fn a_dry_run_rule_records_a_candidate_shown_only_while_maintenance_is_on() {
    let ctx = TestContext::new().await;
    seed_title(&ctx, "title-shadow-1", "Monitored Movie", true).await;
    seed_title(&ctx, "title-shadow-2", "Unmonitored Movie", false).await;
    let detail = create_rule(&ctx, "Stale movies", MONITORED_MATCHER, delete_action()).await;
    let rule_set_id = detail["ruleSet"]["id"]
        .as_str()
        .expect("rule id")
        .to_string();
    arm_rule_and_gate(&ctx, &rule_set_id).await;

    let started = gql(&ctx, RUN_NOW_MUTATION, json!({ "ruleSetId": rule_set_id })).await;
    assert_no_errors(&started);
    assert_eq!(
        started["data"]["runMaintenanceEvaluationNow"]["started"],
        true
    );

    let shown = gql(
        &ctx,
        &candidates_query(),
        json!({ "ruleSetId": rule_set_id }),
    )
    .await;
    assert_no_errors(&shown);
    let candidates = shown["data"]["maintenanceCandidates"]
        .as_array()
        .expect("candidates");
    assert_eq!(candidates.len(), 1, "{shown}");
    let candidate = &candidates[0];
    assert_eq!(candidate["titleId"], "title-shadow-1");
    assert_eq!(candidate["titleName"], "Monitored Movie");
    assert_eq!(candidate["ruleName"], "Stale movies");
    assert_eq!(candidate["state"], "OBSERVING");
    assert_eq!(candidate["stateReason"], "first_match");
    assert_eq!(candidate["actionKind"], "DELETE_TITLE_AND_FILES");
    assert_eq!(candidate["matchGeneration"], 1);
    assert_eq!(candidate["revisionNumber"], 1);
    assert_eq!(candidate["graceDays"], 0);
    assert_eq!(candidate["heldSince"], Value::Null);
    assert_eq!(
        candidate["dueAt"], candidate["firstMatchedAt"],
        "a zero-day grace period is due the moment it matches"
    );
    assert_eq!(candidate["libraryId"], movie_library_id());

    // Re-running is idempotent: the same candidate is reused, not duplicated.
    let again = gql(&ctx, RUN_NOW_MUTATION, json!({ "ruleSetId": rule_set_id })).await;
    assert_no_errors(&again);
    let after = gql(
        &ctx,
        &candidates_query(),
        json!({ "ruleSetId": rule_set_id }),
    )
    .await;
    assert_no_errors(&after);
    let after_candidates = after["data"]["maintenanceCandidates"]
        .as_array()
        .expect("candidates");
    assert_eq!(after_candidates.len(), 1);
    assert_eq!(after_candidates[0]["id"], candidate["id"]);
    assert_eq!(
        after_candidates[0]["firstMatchedAt"], candidate["firstMatchedAt"],
        "a repeat match must never restart the grace clock"
    );

    // Filtering by state reaches the same row through the pinned enum.
    let by_state = gql(
        &ctx,
        &candidates_query(),
        json!({ "states": ["OBSERVING"] }),
    )
    .await;
    assert_no_errors(&by_state);
    assert_eq!(
        by_state["data"]["maintenanceCandidates"]
            .as_array()
            .expect("candidates")
            .len(),
        1
    );

    let runs = gql(
        &ctx,
        EVALUATION_RUNS_QUERY,
        json!({ "ruleSetId": rule_set_id }),
    )
    .await;
    assert_no_errors(&runs);
    let run_rows = runs["data"]["maintenanceEvaluationRuns"]
        .as_array()
        .expect("runs");
    assert_eq!(run_rows.len(), 2, "one row per rule per pass");
    let newest = &run_rows[0];
    assert_eq!(newest["ruleSetId"], rule_set_id.as_str());
    assert_eq!(newest["status"], "succeeded");
    assert_eq!(newest["evaluatedCount"], 2);
    assert_eq!(newest["matchedCount"], 1);
    assert_eq!(newest["noMatchCount"], 1);
    assert_eq!(newest["unknownCount"], 0);
    assert_eq!(newest["errorCount"], 0);
    assert_eq!(newest["error"], Value::Null);
    assert_ne!(newest["finishedAt"], Value::Null);

    // Switching maintenance off hides the results even from a caller that may
    // manage catalog settings.
    let off = gql(
        &ctx,
        &set_gates_mutation(),
        json!({ "input": { "enabled": false } }),
    )
    .await;
    assert_no_errors(&off);
    let hidden = gql(&ctx, &candidates_query(), json!({})).await;
    assert_no_errors(&hidden);
    assert!(
        hidden["data"]["maintenanceCandidates"]
            .as_array()
            .expect("candidates")
            .is_empty()
    );
}

#[tokio::test]
async fn an_exclusion_round_trips_and_closes_the_candidate_it_covers() {
    let ctx = TestContext::new().await;
    seed_title(&ctx, "title-excl-1", "Hands Off", true).await;
    let detail = create_rule(&ctx, "Stale movies", MONITORED_MATCHER, delete_action()).await;
    let rule_set_id = detail["ruleSet"]["id"]
        .as_str()
        .expect("rule id")
        .to_string();
    arm_rule_and_gate(&ctx, &rule_set_id).await;
    gql(&ctx, RUN_NOW_MUTATION, json!({ "ruleSetId": rule_set_id })).await;

    let excluded = gql(
        &ctx,
        &exclude_mutation(),
        json!({
            "input": {
                "titleId": "title-excl-1",
                "reason": "operator pinned",
            }
        }),
    )
    .await;
    assert_no_errors(&excluded);
    let exclusion = &excluded["data"]["excludeMaintenanceSubject"];
    assert_eq!(exclusion["titleId"], "title-excl-1");
    assert_eq!(exclusion["titleName"], "Hands Off");
    assert_eq!(exclusion["reason"], "operator pinned");
    assert_eq!(
        exclusion["ruleSetId"],
        Value::Null,
        "an exclusion with no rule is global"
    );
    let exclusion_id = exclusion["id"].as_str().expect("exclusion id").to_string();

    // A global exclusion is part of what applies to every rule.
    let listed = gql(
        &ctx,
        &exclusions_query(),
        json!({ "ruleSetId": rule_set_id }),
    )
    .await;
    assert_no_errors(&listed);
    assert_eq!(
        listed["data"]["maintenanceExclusions"]
            .as_array()
            .expect("exclusions")
            .len(),
        1
    );

    gql(&ctx, RUN_NOW_MUTATION, json!({ "ruleSetId": rule_set_id })).await;
    let candidates = gql(
        &ctx,
        &candidates_query(),
        json!({ "ruleSetId": rule_set_id }),
    )
    .await;
    assert_no_errors(&candidates);
    let rows = candidates["data"]["maintenanceCandidates"]
        .as_array()
        .expect("candidates");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["state"], "EXCLUDED");
    assert_eq!(rows[0]["stateReason"], "excluded");

    let removed = gql(
        &ctx,
        REMOVE_EXCLUSION_MUTATION,
        json!({ "id": exclusion_id }),
    )
    .await;
    assert_no_errors(&removed);
    assert_eq!(
        removed["data"]["removeMaintenanceExclusion"]["id"],
        exclusion_id.as_str()
    );
    let after = gql(&ctx, &exclusions_query(), json!({})).await;
    assert_no_errors(&after);
    assert!(
        after["data"]["maintenanceExclusions"]
            .as_array()
            .expect("exclusions")
            .is_empty()
    );
}

#[tokio::test]
async fn the_instance_switches_round_trip_and_default_to_off() {
    let ctx = TestContext::new().await;
    seed_maintenance_gate_definitions(&ctx).await;

    let initial = gql(&ctx, &gates_query(), json!({})).await;
    assert_no_errors(&initial);
    let gates = &initial["data"]["maintenanceInstanceGates"];
    assert_eq!(gates["enabled"], false, "maintenance must ship off");
    assert_eq!(gates["actionsPaused"], true, "actions must ship paused");

    let unpaused = gql(
        &ctx,
        &set_gates_mutation(),
        json!({ "input": { "actionsPaused": false } }),
    )
    .await;
    assert_no_errors(&unpaused);

    // An omitted switch stays exactly as stored.
    let enabled = gql(
        &ctx,
        &set_gates_mutation(),
        json!({ "input": { "enabled": true } }),
    )
    .await;
    assert_no_errors(&enabled);
    let updated = &enabled["data"]["setMaintenanceInstanceGates"];
    assert_eq!(updated["enabled"], true);
    assert_eq!(updated["actionsPaused"], false);

    let reread = gql(&ctx, &gates_query(), json!({})).await;
    assert_no_errors(&reread);
    assert_eq!(
        reread["data"]["maintenanceInstanceGates"],
        enabled["data"]["setMaintenanceInstanceGates"]
    );
}

#[tokio::test]
async fn the_evaluation_surface_is_permission_gated() {
    let ctx = TestContext::new().await;
    let denied = create_unprivileged_user(&ctx, "maintenance-evaluation-denied").await;

    for (query, variables) in [
        (candidates_query(), json!({})),
        (exclusions_query(), json!({})),
        (gates_query(), json!({})),
    ] {
        let body = gql_as(&ctx, &query, variables, &denied).await;
        assert_error_contains(&body, "unauthorized");
    }

    let runs = gql_as(&ctx, EVALUATION_RUNS_QUERY, json!({}), &denied).await;
    assert_error_contains(&runs, "unauthorized");

    let triggered = gql_as(&ctx, RUN_NOW_MUTATION, json!({}), &denied).await;
    assert_error_contains(&triggered, "unauthorized");

    let excluded = gql_as(
        &ctx,
        &exclude_mutation(),
        json!({ "input": { "titleId": "title-1" } }),
        &denied,
    )
    .await;
    assert_error_contains(&excluded, "unauthorized");
}

// ── Action execution (WP-H) ─────────────────────────────────────────────────

fn unmonitor_action() -> Value {
    json!({ "kind": "UNMONITOR_SCOPE_KEEP_FILES" })
}

const RUN_HANDLER_MUTATION: &str = r#"mutation {
    runMaintenanceActionHandlerNow { started message }
}"#;

const ACTION_RUNS_QUERY: &str = r#"query($ruleSetId: ID) {
    maintenanceActionRuns(ruleSetId: $ruleSetId, limit: 10) {
        id
        ruleSetId
        candidateId
        titleId
        titleName
        actionKind
        attempt
        status
        holdReason
        error
        startedAt
        finishedAt
    }
}"#;

/// Put a rule in dry run, switch maintenance on and unpause actions. Every
/// step is its own deliberate mutation, mirroring how an operator arms the
/// real thing.
async fn observe_rule_with_gates(ctx: &TestContext, rule_set_id: &str) {
    seed_maintenance_gate_definitions(ctx).await;
    set_rule_state(ctx, rule_set_id, "DRY_RUN", Value::Null).await;
    let gates = gql(
        ctx,
        &set_gates_mutation(),
        json!({ "input": { "enabled": true, "actionsPaused": false } }),
    )
    .await;
    assert_no_errors(&gates);
}

/// The handler trigger runs in the background; poll the candidate until it
/// reaches `state`. The deadline is a hang guard only.
async fn wait_for_candidate_state(ctx: &TestContext, rule_set_id: &str, state: &str) -> Value {
    let deadline = tokio::time::Instant::now() + common::WAIT_UNTIL_TIMEOUT;
    while tokio::time::Instant::now() < deadline {
        let shown = gql(
            ctx,
            &candidates_query(),
            json!({ "ruleSetId": rule_set_id, "states": [state] }),
        )
        .await;
        assert_no_errors(&shown);
        let rows = shown["data"]["maintenanceCandidates"]
            .as_array()
            .expect("candidates")
            .clone();
        if let Some(row) = rows.first() {
            return row.clone();
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let last = gql(
        ctx,
        &candidates_query(),
        json!({ "ruleSetId": rule_set_id }),
    )
    .await;
    let runs = gql(ctx, ACTION_RUNS_QUERY, json!({"ruleSetId": rule_set_id})).await;
    panic!("candidate never reached state {state} for rule {rule_set_id}: {last}; runs={runs}");
}

#[tokio::test]
async fn enabling_a_delete_rule_requires_the_reviewed_revision_and_explicit_library_scope() {
    let ctx = TestContext::new().await;
    let detail = create_rule(&ctx, "Review deletion", MONITORED_MATCHER, delete_action()).await;
    let id = detail["ruleSet"]["id"].as_str().unwrap();
    for confirmation in [
        Value::Null,
        json!({ "revisionNumber": 2, "libraryIds": [], "candidateCount": 0 }),
        json!({ "revisionNumber": 1, "libraryIds": ["different-library"], "candidateCount": 0 }),
    ] {
        let response = gql(
            &ctx,
            SET_STATE_MUTATION,
            json!({ "ruleSetId": id, "state": "ENABLED", "confirmation": confirmation }),
        )
        .await;
        assert!(
            response["errors"][0]["message"]
                .as_str()
                .unwrap()
                .contains("reviewed again"),
            "{response}"
        );
    }
    let enabled = set_rule_state(
        &ctx,
        id,
        "ENABLED",
        json!({ "revisionNumber": 1, "libraryIds": [], "candidateCount": 0 }),
    )
    .await;
    assert_eq!(enabled["state"], "ENABLED");
    assert_eq!(enabled["enabledArming"], "DESTRUCTIVE");
    let dry_run = set_rule_state(&ctx, id, "DRY_RUN", Value::Null).await;
    assert_eq!(dry_run["state"], "DRY_RUN");

    // Editing an enabled rule drops it back to dry run and says why.
    set_rule_state(
        &ctx,
        id,
        "ENABLED",
        json!({ "revisionNumber": 1, "libraryIds": [], "candidateCount": 0 }),
    )
    .await;
    let edited = gql(
        &ctx,
        &update_matcher_mutation(),
        json!({ "input": {
            "id": id,
            "regoSource": MONITORED_MATCHER,
            "action": delete_action(),
            "graceDays": 3,
        }}),
    )
    .await;
    assert_no_errors(&edited);
    let rule = &edited["data"]["updateMaintenanceRuleMatcher"]["ruleSet"];
    assert_eq!(rule["state"], "DRY_RUN");
    assert_eq!(rule["editedSinceEnabled"], true);
}

#[tokio::test]
async fn the_full_destructive_journey_removes_a_matching_title() {
    let ctx = TestContext::new().await;
    seed_title(&ctx, "title-doomed", "Doomed Movie", true).await;
    let root = tempfile::tempdir().expect("maintenance fixture root");
    let library =
        scryer_application::LibraryRepository::get_by_id(&ctx.libraries, &movie_library_id())
            .await
            .unwrap()
            .unwrap();
    scryer_application::LibraryRepository::update(
        &ctx.libraries,
        &library.id,
        library.name,
        library.slug,
        vec![scryer_application::LibraryRootDraft {
            id: None,
            path: root.path().to_string_lossy().into_owned(),
            is_default: true,
        }],
    )
    .await
    .expect("configure isolated maintenance root");
    let folder = root.path().join("Doomed Movie");
    std::fs::create_dir(&folder).expect("create title folder");
    ctx.titles
        .set_folder_path("title-doomed", folder.to_str().unwrap())
        .await
        .expect("record title folder ownership");
    let media_path = folder.join("Doomed.Movie.mkv");
    std::fs::write(&media_path, b"video").expect("write owned media");
    scryer_application::MediaFileRepository::insert_media_file(
        &ctx.media_files,
        &scryer_application::InsertMediaFileInput {
            title_id: "title-doomed".into(),
            file_path: media_path.to_string_lossy().into_owned(),
            size_bytes: 5,
            ..Default::default()
        },
    )
    .await
    .expect("record owned media");
    let unrelated = root.path().join("unrelated.txt");
    std::fs::write(&unrelated, b"preserved").expect("write unrelated file");
    let detail = create_rule(&ctx, "Retire watched", MONITORED_MATCHER, delete_action()).await;
    let rule_set_id = detail["ruleSet"]["id"]
        .as_str()
        .expect("rule id")
        .to_string();
    observe_rule_with_gates(&ctx, &rule_set_id).await;

    let started = gql(&ctx, RUN_NOW_MUTATION, json!({ "ruleSetId": rule_set_id })).await;
    assert_no_errors(&started);

    // Enabling a delete rule demands the acknowledged candidate count, and a
    // mismatch names the real count so the client can re-present it.
    let mismatch = gql(
        &ctx,
        SET_STATE_MUTATION,
        json!({ "ruleSetId": rule_set_id, "state": "ENABLED",
            "confirmation": { "revisionNumber": 1, "libraryIds": [], "candidateCount": 7 } }),
    )
    .await;
    let message = mismatch["errors"][0]["message"]
        .as_str()
        .expect("mismatch error");
    assert!(
        message.contains("acknowledging the current candidate count (1)"),
        "{message}"
    );

    let armed = set_rule_state(
        &ctx,
        &rule_set_id,
        "ENABLED",
        json!({ "revisionNumber": 1, "libraryIds": [], "candidateCount": 1 }),
    )
    .await;
    assert_eq!(armed["state"], "ENABLED");

    let handled = gql(&ctx, RUN_HANDLER_MUTATION, json!({})).await;
    assert_no_errors(&handled);
    assert_eq!(
        handled["data"]["runMaintenanceActionHandlerNow"]["started"],
        true
    );

    let candidate = wait_for_candidate_state(&ctx, &rule_set_id, "SUCCEEDED").await;
    assert_eq!(candidate["stateReason"], "action_succeeded");

    let gone = ctx
        .titles
        .get_by_id("title-doomed")
        .await
        .expect("read title");
    assert!(gone.is_none(), "the title must be removed by the action");
    assert!(!media_path.exists(), "the action removes the owned media");
    assert_eq!(std::fs::read(&unrelated).unwrap(), b"preserved");

    let runs = gql(&ctx, ACTION_RUNS_QUERY, json!({ "ruleSetId": rule_set_id })).await;
    assert_no_errors(&runs);
    let rows = runs["data"]["maintenanceActionRuns"]
        .as_array()
        .expect("runs");
    assert_eq!(rows.len(), 1, "{runs}");
    assert_eq!(rows[0]["status"], "succeeded");
    assert_eq!(rows[0]["actionKind"], "DELETE_TITLE_AND_FILES");
    assert_eq!(rows[0]["attempt"], 1);
    assert_ne!(rows[0]["finishedAt"], Value::Null);
}

#[tokio::test]
async fn a_reversible_unmonitor_journey_needs_no_confirmation() {
    let ctx = TestContext::new().await;
    seed_title(&ctx, "title-unmon", "Stale Movie", true).await;
    let detail = create_rule(
        &ctx,
        "Unmonitor stale",
        MONITORED_MATCHER,
        unmonitor_action(),
    )
    .await;
    let rule_set_id = detail["ruleSet"]["id"]
        .as_str()
        .expect("rule id")
        .to_string();
    observe_rule_with_gates(&ctx, &rule_set_id).await;

    let armed = set_rule_state(&ctx, &rule_set_id, "ENABLED", Value::Null).await;
    assert_eq!(armed["enabledArming"], "REVERSIBLE");

    let started = gql(&ctx, RUN_NOW_MUTATION, json!({ "ruleSetId": rule_set_id })).await;
    assert_no_errors(&started);
    let handled = gql(&ctx, RUN_HANDLER_MUTATION, json!({})).await;
    assert_no_errors(&handled);

    let candidate = wait_for_candidate_state(&ctx, &rule_set_id, "SUCCEEDED").await;
    assert_eq!(candidate["stateReason"], "action_succeeded");

    let title = ctx
        .titles
        .get_by_id("title-unmon")
        .await
        .expect("read title")
        .expect("title survives an unmonitor");
    assert!(!title.monitored, "the action must unmonitor the title");
}

#[tokio::test]
async fn a_subject_that_stopped_matching_is_canceled_at_execution_time() {
    let ctx = TestContext::new().await;
    seed_title(&ctx, "title-kept", "Rewatched Movie", true).await;
    let detail = create_rule(&ctx, "Retire watched", MONITORED_MATCHER, delete_action()).await;
    let rule_set_id = detail["ruleSet"]["id"]
        .as_str()
        .expect("rule id")
        .to_string();
    observe_rule_with_gates(&ctx, &rule_set_id).await;
    let started = gql(&ctx, RUN_NOW_MUTATION, json!({ "ruleSetId": rule_set_id })).await;
    assert_no_errors(&started);
    set_rule_state(
        &ctx,
        &rule_set_id,
        "ENABLED",
        json!({ "revisionNumber": 1, "libraryIds": [], "candidateCount": 1 }),
    )
    .await;

    // The subject stops matching between evaluation and handling; the fresh
    // re-evaluation at execution time must cancel rather than delete.
    ctx.titles
        .update_monitored("title-kept", false)
        .await
        .expect("unmonitor title");

    let handled = gql(&ctx, RUN_HANDLER_MUTATION, json!({})).await;
    assert_no_errors(&handled);

    let candidate = wait_for_candidate_state(&ctx, &rule_set_id, "CANCELED").await;
    assert_eq!(candidate["stateReason"], "no_match_at_execution");

    let survivor = ctx
        .titles
        .get_by_id("title-kept")
        .await
        .expect("read title");
    assert!(
        survivor.is_some(),
        "a canceled candidate must not delete the title"
    );
}

// ── Tag actions ─────────────────────────────────────────────────────────────

const DEFINE_TAG_MUTATION: &str = r#"mutation($input: CreateTitleTagDefinitionInput!) {
    createTitleTagDefinition(input: $input) { definition { id label } }
}"#;

const DELETE_TAG_MUTATION: &str = r#"mutation($id: ID!) {
    deleteTitleTagDefinition(id: $id) { id label }
}"#;

/// Define a tag in the registry and return its id. Every tag write, by a rule
/// or by a person, is refused unless the label is defined here first.
async fn define_tag(ctx: &TestContext, label: &str) -> String {
    let body = gql(
        ctx,
        DEFINE_TAG_MUTATION,
        json!({ "input": { "label": label } }),
    )
    .await;
    assert_no_errors(&body);
    body["data"]["createTitleTagDefinition"]["definition"]["id"]
        .as_str()
        .expect("tag id")
        .to_string()
}

fn tag_action(kind: &str, tags: &[&str]) -> Value {
    json!({ "kind": kind, "tags": tags })
}

/// Seed a title that already carries `tags`, through the repository, so the
/// fixture does not depend on the assignment path it is testing around.
async fn seed_tagged_title(ctx: &TestContext, id: &str, name: &str, tags: &[&str]) -> Title {
    let title = seed_title(ctx, id, name, true).await;
    TitleRepository::update_metadata(
        &ctx.titles,
        id,
        None,
        None,
        Some(tags.iter().map(|tag| (*tag).to_string()).collect()),
        None,
    )
    .await
    .expect("seed title tags");
    title
}

/// Arm one tag rule and run a full evaluate-then-handle pass over it.
async fn run_tag_rule(ctx: &TestContext, rule_set_id: &str) {
    observe_rule_with_gates(ctx, rule_set_id).await;
    let started = gql(ctx, RUN_NOW_MUTATION, json!({ "ruleSetId": rule_set_id })).await;
    assert_no_errors(&started);
    set_rule_state(ctx, rule_set_id, "ENABLED", Value::Null).await;
}

#[tokio::test]
async fn a_tag_rule_writes_the_label_through_the_normal_assignment_path() {
    let ctx = TestContext::new().await;
    define_tag(&ctx, "needs review").await;
    seed_title(&ctx, "title-untagged", "Untagged Movie", true).await;
    let detail = create_rule(
        &ctx,
        "Flag for review",
        MONITORED_MATCHER,
        tag_action("ADD_TAGS", &["needs review"]),
    )
    .await;
    let rule_set_id = detail["ruleSet"]["id"]
        .as_str()
        .expect("rule id")
        .to_string();
    assert_eq!(detail["actionSpec"]["kind"], "ADD_TAGS");

    run_tag_rule(&ctx, &rule_set_id).await;
    let handled = gql(&ctx, RUN_HANDLER_MUTATION, json!({})).await;
    assert_no_errors(&handled);

    let candidate = wait_for_candidate_state(&ctx, &rule_set_id, "SUCCEEDED").await;
    assert_eq!(candidate["stateReason"], "action_succeeded");

    let title = ctx
        .titles
        .get_by_id("title-untagged")
        .await
        .expect("read title")
        .expect("a tag write never removes the title");
    assert!(
        title.tags.iter().any(|tag| tag == "needs review"),
        "{:?}",
        title.tags
    );
}

#[tokio::test]
async fn a_tag_rule_whose_postcondition_already_holds_reports_already_satisfied() {
    let ctx = TestContext::new().await;
    define_tag(&ctx, "needs review").await;
    seed_tagged_title(&ctx, "title-tagged", "Tagged Movie", &["needs review"]).await;
    let detail = create_rule(
        &ctx,
        "Flag for review",
        MONITORED_MATCHER,
        tag_action("ADD_TAGS", &["needs review"]),
    )
    .await;
    let rule_set_id = detail["ruleSet"]["id"]
        .as_str()
        .expect("rule id")
        .to_string();

    run_tag_rule(&ctx, &rule_set_id).await;
    let handled = gql(&ctx, RUN_HANDLER_MUTATION, json!({})).await;
    assert_no_errors(&handled);

    let candidate = wait_for_candidate_state(&ctx, &rule_set_id, "SUCCEEDED").await;
    assert_eq!(
        candidate["stateReason"], "already_satisfied",
        "the label was already there, so nothing was written"
    );
    let runs = gql(&ctx, ACTION_RUNS_QUERY, json!({ "ruleSetId": rule_set_id })).await;
    assert_no_errors(&runs);
    assert_eq!(
        runs["data"]["maintenanceActionRuns"][0]["status"],
        "already_satisfied"
    );
}

#[tokio::test]
async fn a_tag_rule_holds_when_its_label_was_deleted_after_authoring() {
    let ctx = TestContext::new().await;
    let tag_id = define_tag(&ctx, "needs review").await;
    seed_title(&ctx, "title-orphaned", "Orphaned Movie", true).await;
    let detail = create_rule(
        &ctx,
        "Flag for review",
        MONITORED_MATCHER,
        tag_action("ADD_TAGS", &["needs review"]),
    )
    .await;
    let rule_set_id = detail["ruleSet"]["id"]
        .as_str()
        .expect("rule id")
        .to_string();
    run_tag_rule(&ctx, &rule_set_id).await;

    // The administrator retires the tag between authoring and execution. The
    // rule is still stored, still armed, and now names a vocabulary the
    // instance no longer has.
    let deleted = gql(&ctx, DELETE_TAG_MUTATION, json!({ "id": tag_id })).await;
    assert_no_errors(&deleted);

    let handled = gql(&ctx, RUN_HANDLER_MUTATION, json!({})).await;
    assert_no_errors(&handled);

    let candidate = wait_for_candidate_state(&ctx, &rule_set_id, "BLOCKED").await;
    assert_eq!(candidate["stateReason"], "tag_not_defined");
    let title = ctx
        .titles
        .get_by_id("title-orphaned")
        .await
        .expect("read title")
        .expect("title survives");
    assert!(
        title.tags.is_empty(),
        "a held candidate must write nothing: {:?}",
        title.tags
    );
}

#[tokio::test]
async fn two_rules_writing_opposite_tag_patches_hold_both_candidates() {
    let ctx = TestContext::new().await;
    define_tag(&ctx, "needs review").await;
    seed_tagged_title(
        &ctx,
        "title-contested",
        "Contested Movie",
        &["needs review"],
    )
    .await;

    let adder = create_rule(
        &ctx,
        "Flag for review",
        MONITORED_MATCHER,
        tag_action("ADD_TAGS", &["needs review"]),
    )
    .await;
    let adder_id = adder["ruleSet"]["id"]
        .as_str()
        .expect("rule id")
        .to_string();
    let remover = create_rule(
        &ctx,
        "Clear review flag",
        MONITORED_MATCHER,
        tag_action("REMOVE_TAGS", &["needs review"]),
    )
    .await;
    let remover_id = remover["ruleSet"]["id"]
        .as_str()
        .expect("rule id")
        .to_string();

    run_tag_rule(&ctx, &adder_id).await;
    run_tag_rule(&ctx, &remover_id).await;

    let handled = gql(&ctx, RUN_HANDLER_MUTATION, json!({})).await;
    assert_no_errors(&handled);

    for rule_set_id in [&adder_id, &remover_id] {
        let candidate = wait_for_candidate_state(&ctx, rule_set_id, "BLOCKED").await;
        assert_eq!(
            candidate["stateReason"], "tag_patch_conflict",
            "both sides hold rather than letting rule order decide"
        );
    }

    let title = ctx
        .titles
        .get_by_id("title-contested")
        .await
        .expect("read title")
        .expect("title survives");
    assert_eq!(
        title.tags,
        vec!["needs review".to_string()],
        "neither side wrote anything"
    );
}

#[tokio::test]
async fn a_tag_action_naming_an_undefined_label_is_refused_at_authoring_time() {
    let ctx = TestContext::new().await;
    let refused = gql(
        &ctx,
        &create_mutation(),
        json!({
            "input": {
                "name": "Flag for review",
                "regoSource": MONITORED_MATCHER,
                "action": tag_action("ADD_TAGS", &["never defined"]),
            }
        }),
    )
    .await;
    assert_error_contains(&refused, "is not a defined tag");
}
