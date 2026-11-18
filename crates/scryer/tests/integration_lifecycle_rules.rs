#![recursion_limit = "256"]

//! End-to-end coverage for the lifecycle-rule authoring surface: the GraphQL queries and mutations over the authoring service.
//!
//! Everything here is authoring-time. Nothing in this surface schedules
//! evaluation or executes an action, so a rule always persists as disabled and
//! preview persists nothing at all.

// disk-guard: test, example and build-script code may call the filesystem directly.
#![allow(clippy::disallowed_methods)]

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

/// Deletes every monitored title. The package line is deliberately absent: the
/// server owns it, and the editor never sees it. The source declares both when
/// it acts and what it does.
const MONITORED_SOURCE: &str = "delete_title if {\n\
     \tinput.facts.monitored\n\
     }\n";

/// Unmonitors every monitored title, keeping its files.
const MONITORED_UNMONITOR: &str = "unmonitor if {\n\
     \tinput.facts.monitored\n\
     }\n";

/// Acts on nothing: the facet never equals this sentinel.
const NEVER_SOURCE: &str = "unmonitor if {\n\
     \tinput.subject.facet == \"not-a-facet\"\n\
     }\n";

/// Reads a fact Scryer cannot observe, so every title must be held rather
/// than reported as a no-match. Nothing in the source says so: the server
/// derives the hold from the fact the matcher references, which is exactly the
/// mistake an author can no longer make.
const ACTIVE_DOWNLOADS_SOURCE: &str = "delete_title if {\n\
     \tnot input.facts.active_downloads\n\
     }\n";

/// Compiles, but defines no action, so it could never do anything.
const NO_ACTION_SOURCE: &str = "match if {\n\
     \tinput.facts.monitored\n\
     }\n";

const INVALID_SOURCE: &str = "delete_title if { this is not rego\n";

/// Adds `tag` to every monitored title.
fn add_tag_source(tag: &str) -> String {
    format!("add_tags := [\"{tag}\"] if {{\n\tinput.facts.monitored\n}}\n")
}

/// Removes `tag` from every monitored title.
fn remove_tag_source(tag: &str) -> String {
    format!("remove_tags := [\"{tag}\"] if {{\n\tinput.facts.monitored\n}}\n")
}

const DETAIL_FIELDS: &str = r#"
    ruleSet {
        id
        name
        description
        state
        editedSinceEnabled
        libraryIds
        currentRevisionNumber
        graceDays
        declaredActions { heads destructive qualityProfiles addTags removeTags }
        loadRefusal
        createdAt
        updatedAt
    }
    revision {
        id
        ruleSetId
        revisionNumber
        regoSource
        graceDays
        sourceContentHash
        createdBy
        createdAt
    }
    declaredActions { heads destructive qualityProfiles addTags removeTags }
"#;

/// The stable reason code of the first GraphQL error.
fn error_reason(body: &Value) -> &Value {
    &body["errors"][0]["extensions"]["reason"]
}

fn create_mutation() -> String {
    format!(
        "mutation($input: CreateLifecycleRuleSetInput!) {{
            createLifecycleRuleSet(input: $input) {{{DETAIL_FIELDS}}}
        }}"
    )
}

fn update_source_mutation() -> String {
    format!(
        "mutation($input: UpdateLifecycleRuleSourceInput!) {{
            updateLifecycleRuleSource(input: $input) {{{DETAIL_FIELDS}}}
        }}"
    )
}

fn get_query() -> String {
    format!(
        "query($id: ID!) {{
            lifecycleRuleSet(id: $id) {{{DETAIL_FIELDS}}}
        }}"
    )
}

const LIST_QUERY: &str = r#"query {
    lifecycleRuleSets {
        id
        name
        state
        libraryIds
        currentRevisionNumber
        graceDays
        declaredActions { heads destructive }
        loadRefusal
    }
}"#;

const REVISIONS_QUERY: &str = r#"query($ruleSetId: ID!) {
    lifecycleRuleRevisions(ruleSetId: $ruleSetId) {
        id
        ruleSetId
        revisionNumber
        regoSource
        graceDays
        sourceContentHash
    }
}"#;

const PREVIEW_MUTATION: &str = r#"mutation($input: PreviewLifecycleRuleInput!) {
    previewLifecycleRule(input: $input) {
        ruleSetId
        sourceContentHash
        evaluatedAt
        titles {
            titleId
            titleName
            facet
            libraryId
            outcome
            reasonCodes
            error
            errorCode
            actions { heads }
        }
    }
}"#;

const VALIDATE_MUTATION: &str = r#"mutation($input: ValidateLifecycleRuleInput!) {
    validateLifecycleRule(input: $input) {
        valid
        errors
        diagnostics { code }
        warnings { code }
        declaredActions { heads destructive }
    }
}"#;

/// The per-run maximum the tests give every rule; a rule that deletes must
/// carry one, and this is far above any fixture's title count.
const TEST_MAX_TITLES_PER_RUN: u32 = 100;

/// Create a rule set and return the whole detail payload.
async fn create_rule(ctx: &TestContext, name: &str, rego: &str) -> Value {
    let body = gql(
        ctx,
        &create_mutation(),
        json!({
            "input": {
                "name": name,
                "regoSource": rego,
                "maxTitlesPerRun": TEST_MAX_TITLES_PER_RUN,
            }
        }),
    )
    .await;
    assert_no_errors(&body);
    body["data"]["createLifecycleRuleSet"].clone()
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
async fn lifecycle_rule_create_list_and_get_round_trip() {
    let ctx = TestContext::new().await;

    let body = gql(
        &ctx,
        &create_mutation(),
        json!({
            "input": {
                "name": "Stale movies",
                "description": "Movies nobody monitors",
                "regoSource": MONITORED_SOURCE,
                "graceDays": 14,
                "libraryIds": [movie_library_id()],
                "maxTitlesPerRun": TEST_MAX_TITLES_PER_RUN,
            }
        }),
    )
    .await;
    assert_no_errors(&body);

    let detail = &body["data"]["createLifecycleRuleSet"];
    let rule_set = &detail["ruleSet"];
    let id = rule_set["id"].as_str().expect("rule set id").to_string();
    assert_eq!(rule_set["name"], "Stale movies");
    assert_eq!(rule_set["description"], "Movies nobody monitors");
    // Ships dark: nothing evaluates stored rules yet.
    assert_eq!(rule_set["state"], "DISABLED");
    assert_eq!(rule_set["declaredActions"]["destructive"], true);
    assert_eq!(rule_set["editedSinceEnabled"], false);
    assert_eq!(rule_set["libraryIds"], json!([movie_library_id()]));
    assert_eq!(rule_set["currentRevisionNumber"], 1);

    let revision = &detail["revision"];
    assert_eq!(revision["revisionNumber"], 1);
    assert_eq!(revision["ruleSetId"], json!(id));
    assert_eq!(revision["graceDays"], 14);
    assert!(
        revision["sourceContentHash"]
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

    // What the rule does is read from its own source.
    assert_eq!(detail["declaredActions"]["heads"], json!(["DELETE_TITLE"]));
    assert_eq!(detail["declaredActions"]["destructive"], true);
    assert_eq!(rule_set["declaredActions"], detail["declaredActions"]);
    assert_eq!(rule_set["loadRefusal"], Value::Null);

    let list = gql(&ctx, LIST_QUERY, json!({})).await;
    assert_no_errors(&list);
    let rule_sets = list["data"]["lifecycleRuleSets"]
        .as_array()
        .expect("rule set list");
    assert_eq!(rule_sets.len(), 1);
    assert_eq!(rule_sets[0]["id"], json!(id));
    assert_eq!(rule_sets[0]["currentRevisionNumber"], 1);
    // The list carries what the revision in force does, so rendering a saved
    // rule never costs a second round trip.
    assert_eq!(rule_sets[0]["graceDays"], 14);
    assert_eq!(
        rule_sets[0]["declaredActions"],
        json!({ "heads": ["DELETE_TITLE"], "destructive": true })
    );
    assert_eq!(rule_sets[0]["loadRefusal"], Value::Null);
    assert_eq!(rule_set["graceDays"], 14);

    let fetched = gql(&ctx, &get_query(), json!({ "id": id })).await;
    assert_no_errors(&fetched);
    assert_eq!(fetched["data"]["lifecycleRuleSet"], *detail);

    let missing = gql(&ctx, &get_query(), json!({ "id": "no-such-rule" })).await;
    assert_no_errors(&missing);
    assert_eq!(missing["data"]["lifecycleRuleSet"], Value::Null);
}

#[tokio::test]
async fn lifecycle_rule_declares_several_actions_in_source_order() {
    let ctx = TestContext::new().await;

    let detail = create_rule(
        &ctx,
        "Unmonitor and search",
        "unmonitor if {\n\tinput.facts.monitored\n}\n\n\
         search if {\n\tinput.facts.monitored\n}\n",
    )
    .await;
    assert_eq!(
        detail["declaredActions"]["heads"],
        json!(["UNMONITOR", "SEARCH"])
    );
    assert_eq!(detail["declaredActions"]["destructive"], false);
    assert_eq!(detail["ruleSet"]["state"], "DISABLED");

    let id = detail["ruleSet"]["id"].as_str().expect("rule set id");
    let fetched = gql(&ctx, &get_query(), json!({ "id": id })).await;
    assert_no_errors(&fetched);
    assert_eq!(
        fetched["data"]["lifecycleRuleSet"]["declaredActions"],
        detail["declaredActions"]
    );
}

#[tokio::test]
async fn lifecycle_rule_naming_an_unknown_quality_profile_is_rejected() {
    let ctx = TestContext::new().await;

    let body = gql(
        &ctx,
        &create_mutation(),
        json!({
            "input": {
                "name": "Missing target",
                "regoSource": "set_quality_profile := \"No Such Profile\" if {\n\
                               \tinput.facts.monitored\n\
                               }\n",
            }
        }),
    )
    .await;
    assert_eq!(
        *error_reason(&body),
        json!("LIFECYCLE_QUALITY_PROFILE_NOT_FOUND"),
        "{body}"
    );

    let list = gql(&ctx, LIST_QUERY, json!({})).await;
    assert_no_errors(&list);
    assert!(
        list["data"]["lifecycleRuleSets"]
            .as_array()
            .expect("rule set list")
            .is_empty(),
        "a rejected action must not persist a rule set"
    );
}

#[tokio::test]
async fn lifecycle_rule_source_update_appends_a_revision() {
    let ctx = TestContext::new().await;

    let created = create_rule(&ctx, "Stale movies", MONITORED_SOURCE).await;
    let id = created["ruleSet"]["id"].as_str().expect("id").to_string();
    let first_hash = created["revision"]["sourceContentHash"]
        .as_str()
        .expect("hash")
        .to_string();

    let updated = gql(
        &ctx,
        &update_source_mutation(),
        json!({
            "input": {
                "id": id,
                "regoSource": NEVER_SOURCE,
                "graceDays": 30,
            }
        }),
    )
    .await;
    assert_no_errors(&updated);

    let detail = &updated["data"]["updateLifecycleRuleSource"];
    assert_eq!(detail["ruleSet"]["currentRevisionNumber"], 2);
    assert_eq!(detail["revision"]["revisionNumber"], 2);
    assert_eq!(detail["revision"]["graceDays"], 30);
    assert_eq!(detail["declaredActions"]["heads"], json!(["UNMONITOR"]));
    assert_ne!(
        detail["revision"]["sourceContentHash"],
        json!(first_hash),
        "a new matcher must hash differently"
    );

    let revisions = gql(&ctx, REVISIONS_QUERY, json!({ "ruleSetId": id })).await;
    assert_no_errors(&revisions);
    let revisions = revisions["data"]["lifecycleRuleRevisions"]
        .as_array()
        .expect("revision list")
        .clone();
    assert_eq!(revisions.len(), 2);
    assert_eq!(revisions[0]["revisionNumber"], 2);
    assert_eq!(revisions[1]["revisionNumber"], 1);
    // Revision one is immutable: it still carries the source it was written
    // with, so a decision recorded against it stays attributable.
    assert_eq!(revisions[1]["sourceContentHash"], json!(first_hash));
    assert!(
        revisions[1]["regoSource"]
            .as_str()
            .expect("stored source")
            .contains("input.facts.monitored")
    );
}

/// Omitting `builderModel` keeps the stored document, `null` clears it, and a
/// value replaces it, for lifecycle and scoring rules alike.
#[tokio::test]
async fn builder_models_are_kept_cleared_and_replaced_through_graphql() {
    let ctx = TestContext::new().await;
    let model = json!({
        "version": 2,
        "family": "lifecycle",
        "join": "ALL",
        "conditions": [],
        "outputs": {"UNMONITOR": true}
    });
    let builder_query = "query($id: ID!) {
        lifecycleRuleSet(id: $id) { revision { revisionNumber } builderModel }
    }";

    let created = gql(
        &ctx,
        &create_mutation(),
        json!({"input": {
            "name": "Built rule",
            "regoSource": MONITORED_UNMONITOR,
            "builderModel": model,
        }}),
    )
    .await;
    assert_no_errors(&created);
    let id = created["data"]["createLifecycleRuleSet"]["ruleSet"]["id"]
        .as_str()
        .expect("id")
        .to_string();
    let read = gql(&ctx, builder_query, json!({"id": id})).await;
    assert_no_errors(&read);
    assert_eq!(read["data"]["lifecycleRuleSet"]["builderModel"], model);

    // Omitted: the new revision keeps the document.
    let kept = gql(
        &ctx,
        &update_source_mutation(),
        json!({"input": {"id": id, "regoSource": MONITORED_UNMONITOR, "graceDays": 3}}),
    )
    .await;
    assert_no_errors(&kept);
    let read = gql(&ctx, builder_query, json!({"id": id})).await;
    assert_eq!(
        read["data"]["lifecycleRuleSet"]["revision"]["revisionNumber"],
        2
    );
    assert_eq!(read["data"]["lifecycleRuleSet"]["builderModel"], model);

    // Omitted with a changed source: the hand edit drops the document.
    let edited = gql(
        &ctx,
        &update_source_mutation(),
        json!({"input": {"id": id, "regoSource": NEVER_SOURCE}}),
    )
    .await;
    assert_no_errors(&edited);
    let read = gql(&ctx, builder_query, json!({"id": id})).await;
    assert_eq!(
        read["data"]["lifecycleRuleSet"]["builderModel"],
        Value::Null
    );
    let rebuilt = gql(
        &ctx,
        &update_source_mutation(),
        json!({"input": {"id": id, "regoSource": MONITORED_UNMONITOR, "builderModel": model}}),
    )
    .await;
    assert_no_errors(&rebuilt);

    // A document for another family is refused with its code.
    let refused = gql(
        &ctx,
        &update_source_mutation(),
        json!({"input": {
            "id": id,
            "regoSource": MONITORED_UNMONITOR,
            "builderModel": {"version": 2, "family": "scoring"},
        }}),
    )
    .await;
    assert_eq!(error_reason(&refused), "RULE_BUILDER_MODEL_INVALID");

    // Null: cleared.
    let cleared = gql(
        &ctx,
        &update_source_mutation(),
        json!({"input": {"id": id, "regoSource": MONITORED_UNMONITOR, "builderModel": null}}),
    )
    .await;
    assert_no_errors(&cleared);
    let read = gql(&ctx, builder_query, json!({"id": id})).await;
    assert_eq!(
        read["data"]["lifecycleRuleSet"]["builderModel"],
        Value::Null
    );

    // Scoring: read only by `ruleSet(id)`.
    let scoring_model = json!({
        "version": 2,
        "family": "scoring",
        "join": "ALL",
        "conditions": [],
        "outputs": {}
    });
    let created = gql(
        &ctx,
        "mutation($input: CreateRuleSetInput!) { createRuleSet(input: $input) { id } }",
        json!({"input": {
            "name": "Built scoring rule",
            "regoSource": "score_entry[\"built\"] := 5 if { true }",
            "appliedFacets": ["movie"],
            "builderModel": scoring_model,
        }}),
    )
    .await;
    assert_no_errors(&created);
    let scoring_id = created["data"]["createRuleSet"]["id"]
        .as_str()
        .expect("scoring id")
        .to_string();
    let scoring_query = "query($id: ID!) { ruleSet(id: $id) { name builderModel } }";
    let read = gql(&ctx, scoring_query, json!({"id": scoring_id})).await;
    assert_no_errors(&read);
    assert_eq!(read["data"]["ruleSet"]["builderModel"], scoring_model);

    let renamed = gql(
        &ctx,
        "mutation($input: UpdateRuleSetInput!) { updateRuleSet(input: $input) { id } }",
        json!({"input": {"id": scoring_id, "name": "Renamed scoring rule"}}),
    )
    .await;
    assert_no_errors(&renamed);
    let read = gql(&ctx, scoring_query, json!({"id": scoring_id})).await;
    assert_eq!(read["data"]["ruleSet"]["builderModel"], scoring_model);

    let cleared = gql(
        &ctx,
        "mutation($input: UpdateRuleSetInput!) { updateRuleSet(input: $input) { id } }",
        json!({"input": {"id": scoring_id, "builderModel": null}}),
    )
    .await;
    assert_no_errors(&cleared);
    let read = gql(&ctx, scoring_query, json!({"id": scoring_id})).await;
    assert_eq!(read["data"]["ruleSet"]["builderModel"], Value::Null);
    assert_eq!(read["data"]["ruleSet"]["name"], "Renamed scoring rule");

    // Omitted with a changed source: the document is dropped, the rule kept.
    let rebuilt = gql(
        &ctx,
        "mutation($input: UpdateRuleSetInput!) { updateRuleSet(input: $input) { id } }",
        json!({"input": {"id": scoring_id, "builderModel": scoring_model}}),
    )
    .await;
    assert_no_errors(&rebuilt);
    let edited = gql(
        &ctx,
        "mutation($input: UpdateRuleSetInput!) { updateRuleSet(input: $input) { id } }",
        json!({"input": {"id": scoring_id, "regoSource": "score_entry[\"built\"] := 7 if { true }"}}),
    )
    .await;
    assert_no_errors(&edited);
    let read = gql(&ctx, scoring_query, json!({"id": scoring_id})).await;
    assert_eq!(read["data"]["ruleSet"]["builderModel"], Value::Null);
    assert_eq!(read["data"]["ruleSet"]["name"], "Renamed scoring rule");
}

#[tokio::test]
async fn lifecycle_rule_metadata_update_does_not_create_a_revision() {
    let ctx = TestContext::new().await;
    let library_ids = vec![
        movie_library_id(),
        scryer_domain::default_library_id_for_facet(&MediaFacet::Series),
    ];

    let created = create_rule(&ctx, "Stale movies", MONITORED_SOURCE).await;
    let id = created["ruleSet"]["id"].as_str().expect("id").to_string();

    let renamed = gql(
        &ctx,
        r#"mutation($input: UpdateLifecycleRuleMetadataInput!) {
            updateLifecycleRuleMetadata(input: $input) {
                id
                name
                description
                libraryIds
                currentRevisionNumber
                graceDays
                declaredActions { heads }
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

    let rule_set = &renamed["data"]["updateLifecycleRuleMetadata"];
    assert_eq!(rule_set["name"], "Renamed");
    assert_eq!(rule_set["description"], "Now scoped");
    assert_eq!(rule_set["libraryIds"], json!(library_ids));
    assert_eq!(rule_set["currentRevisionNumber"], 1);
    // Renaming leaves the source alone, so the actions it declares are
    // unchanged and still reported on the rule set itself.
    assert_eq!(rule_set["graceDays"], 0);
    assert_eq!(
        rule_set["declaredActions"]["heads"],
        json!(["DELETE_TITLE"])
    );

    let revisions = gql(&ctx, REVISIONS_QUERY, json!({ "ruleSetId": id })).await;
    assert_no_errors(&revisions);
    assert_eq!(
        revisions["data"]["lifecycleRuleRevisions"]
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
async fn lifecycle_rule_validation_reports_errors_without_persisting() {
    let ctx = TestContext::new().await;

    let invalid = gql(
        &ctx,
        VALIDATE_MUTATION,
        json!({ "input": { "regoSource": INVALID_SOURCE } }),
    )
    .await;
    assert_no_errors(&invalid);
    let result = &invalid["data"]["validateLifecycleRule"];
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
        json!({ "input": { "regoSource": MONITORED_SOURCE } }),
    )
    .await;
    assert_no_errors(&valid);
    assert_eq!(valid["data"]["validateLifecycleRule"]["valid"], true);
    assert_eq!(valid["data"]["validateLifecycleRule"]["errors"], json!([]));
    // Validation reports what the source would do before it is saved.
    assert_eq!(
        valid["data"]["validateLifecycleRule"]["declaredActions"],
        json!({ "heads": ["DELETE_TITLE"], "destructive": true })
    );

    // A source that defines no action could never do anything.
    let no_action = gql(
        &ctx,
        VALIDATE_MUTATION,
        json!({ "input": { "regoSource": NO_ACTION_SOURCE } }),
    )
    .await;
    assert_no_errors(&no_action);
    let result = &no_action["data"]["validateLifecycleRule"];
    assert_eq!(result["valid"], false);
    assert_eq!(
        result["diagnostics"],
        json!([{ "code": "RULE_ACTION_REQUIRED" }])
    );

    let list = gql(&ctx, LIST_QUERY, json!({})).await;
    assert_no_errors(&list);
    assert!(
        list["data"]["lifecycleRuleSets"]
            .as_array()
            .expect("rule set list")
            .is_empty(),
        "validation must never persist a rule set"
    );
}

#[tokio::test]
async fn lifecycle_rule_creation_rejects_a_source_that_does_not_compile() {
    let ctx = TestContext::new().await;

    let body = gql(
        &ctx,
        &create_mutation(),
        json!({
            "input": {
                "name": "Broken",
                "regoSource": INVALID_SOURCE,
            }
        }),
    )
    .await;
    assert_error_contains(&body, "validation:");

    let no_action = gql(
        &ctx,
        &create_mutation(),
        json!({
            "input": {
                "name": "Does nothing",
                "regoSource": NO_ACTION_SOURCE,
            }
        }),
    )
    .await;
    assert!(no_action.get("errors").is_some(), "{no_action}");
    let list = gql(&ctx, LIST_QUERY, json!({})).await;
    assert_no_errors(&list);
    assert!(
        list["data"]["lifecycleRuleSets"]
            .as_array()
            .expect("rule set list")
            .is_empty(),
        "a source without an action must not persist a rule set"
    );
}

// ===========================================================================
// 3. Preview
// ===========================================================================

#[tokio::test]
async fn lifecycle_rule_preview_separates_match_from_no_match() {
    let ctx = TestContext::new().await;
    seed_title(&ctx, "title-monitored", "Monitored Movie", true).await;
    seed_title(&ctx, "title-unmonitored", "Unmonitored Movie", false).await;

    let body = gql(
        &ctx,
        PREVIEW_MUTATION,
        json!({
            "input": {
                "regoSource": MONITORED_SOURCE,
                "titleIds": ["title-monitored", "title-unmonitored"],
            }
        }),
    )
    .await;
    assert_no_errors(&body);

    let preview = &body["data"]["previewLifecycleRule"];
    assert!(
        preview["sourceContentHash"]
            .as_str()
            .is_some_and(|hash| !hash.is_empty())
    );
    assert!(preview["evaluatedAt"].is_string());
    assert_eq!(preview["titles"].as_array().expect("titles").len(), 2);

    let matched = outcome_for(preview, "title-monitored");
    assert_eq!(matched["outcome"], "MATCH");
    assert_eq!(matched["error"], Value::Null);
    assert_eq!(matched["errorCode"], Value::Null);
    assert_eq!(matched["actions"]["heads"], json!(["DELETE_TITLE"]));
    assert_eq!(matched["titleName"], "Monitored Movie");
    assert_eq!(matched["facet"], "movie");
    assert_eq!(matched["libraryId"], json!(movie_library_id()));

    let unmatched = outcome_for(preview, "title-unmonitored");
    assert_eq!(unmatched["outcome"], "NO_MATCH");
    assert_eq!(unmatched["error"], Value::Null);
    assert_eq!(unmatched["actions"], Value::Null);

    // Preview answers "what would this match", never "what happened".
    let list = gql(&ctx, LIST_QUERY, json!({})).await;
    assert_no_errors(&list);
    assert!(
        list["data"]["lifecycleRuleSets"]
            .as_array()
            .expect("rule set list")
            .is_empty()
    );
}

#[tokio::test]
async fn lifecycle_rule_preview_holds_a_title_on_an_unobservable_fact() {
    let ctx = TestContext::new().await;
    seed_title(&ctx, "title-monitored", "Monitored Movie", true).await;

    let body = gql(
        &ctx,
        PREVIEW_MUTATION,
        json!({
            "input": {
                "regoSource": ACTIVE_DOWNLOADS_SOURCE,
                "titleIds": ["title-monitored"],
            }
        }),
    )
    .await;
    assert_no_errors(&body);

    let preview = &body["data"]["previewLifecycleRule"];
    let held = outcome_for(preview, "title-monitored");
    // An unobservable fact must hold the rule, never read as a no-match: the
    // action here deletes files.
    assert_eq!(held["outcome"], "UNKNOWN");
    assert_eq!(held["error"], Value::Null);
    assert_eq!(held["reasonCodes"], json!(["not_yet_collected"]));
}

#[tokio::test]
async fn lifecycle_rule_preview_runs_a_stored_rule_at_its_current_revision() {
    let ctx = TestContext::new().await;
    seed_title(&ctx, "title-monitored", "Monitored Movie", true).await;

    let created = create_rule(&ctx, "Stale movies", MONITORED_SOURCE).await;
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

    let preview = &body["data"]["previewLifecycleRule"];
    assert_eq!(preview["ruleSetId"], json!(id));
    assert_eq!(
        preview["sourceContentHash"], created["revision"]["sourceContentHash"],
        "a stored preview must be attributable to the revision in force"
    );
    assert_eq!(outcome_for(preview, "title-monitored")["outcome"], "MATCH");
}

#[tokio::test]
async fn lifecycle_rule_preview_rejects_an_oversized_selection() {
    let ctx = TestContext::new().await;

    let title_ids: Vec<String> = (0..51).map(|index| format!("title-{index}")).collect();
    let body = gql(
        &ctx,
        PREVIEW_MUTATION,
        json!({
            "input": {
                "regoSource": MONITORED_SOURCE,
                "titleIds": title_ids,
            }
        }),
    )
    .await;
    assert_error_contains(&body, "at most 50 titles");
}

#[tokio::test]
async fn lifecycle_rule_preview_requires_exactly_one_source_and_one_selection() {
    let ctx = TestContext::new().await;

    let both_sources = gql(
        &ctx,
        PREVIEW_MUTATION,
        json!({
            "input": {
                "ruleSetId": "rule-1",
                "regoSource": MONITORED_SOURCE,
                "titleIds": ["title-1"],
            }
        }),
    )
    .await;
    assert_error_contains(&both_sources, "not both");

    let no_source = gql(
        &ctx,
        PREVIEW_MUTATION,
        json!({ "input": { "titleIds": ["title-1"] } }),
    )
    .await;
    assert_error_contains(&no_source, "'ruleSetId' or 'regoSource'");

    let draft_without_action = gql(
        &ctx,
        PREVIEW_MUTATION,
        json!({
            "input": {
                "regoSource": NO_ACTION_SOURCE,
                "titleIds": ["title-1"],
            }
        }),
    )
    .await;
    assert!(
        draft_without_action.get("errors").is_some(),
        "a draft that defines no action cannot be previewed: {draft_without_action}"
    );

    let both_selections = gql(
        &ctx,
        PREVIEW_MUTATION,
        json!({
            "input": {
                "regoSource": MONITORED_SOURCE,
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
                "regoSource": MONITORED_SOURCE,
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
async fn lifecycle_rule_delete_removes_the_rule_set() {
    let ctx = TestContext::new().await;

    let created = create_rule(&ctx, "Stale movies", MONITORED_SOURCE).await;
    let id = created["ruleSet"]["id"].as_str().expect("id").to_string();

    let deleted = gql(
        &ctx,
        r#"mutation($id: ID!) {
            deleteLifecycleRuleSet(id: $id) { id }
        }"#,
        json!({ "id": id }),
    )
    .await;
    assert_no_errors(&deleted);
    assert_eq!(deleted["data"]["deleteLifecycleRuleSet"]["id"], json!(id));

    let list = gql(&ctx, LIST_QUERY, json!({})).await;
    assert_no_errors(&list);
    assert!(
        list["data"]["lifecycleRuleSets"]
            .as_array()
            .expect("rule set list")
            .is_empty()
    );

    let fetched = gql(&ctx, &get_query(), json!({ "id": id })).await;
    assert_no_errors(&fetched);
    assert_eq!(fetched["data"]["lifecycleRuleSet"], Value::Null);

    let missing = gql(
        &ctx,
        r#"mutation($id: ID!) {
            deleteLifecycleRuleSet(id: $id) { id }
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
// 5. Declared actions
// ===========================================================================

/// The source is the only place a rule's actions come from: the separate
/// action input and the action catalog that fed it are gone from the schema.
#[tokio::test]
async fn the_action_input_and_catalog_are_no_longer_part_of_the_api() {
    let ctx = TestContext::new().await;

    let with_action = gql(
        &ctx,
        &create_mutation(),
        json!({
            "input": {
                "name": "Old shape",
                "regoSource": MONITORED_SOURCE,
                "action": { "kind": "DELETE_TITLE_AND_FILES" },
            }
        }),
    )
    .await;
    assert!(with_action.get("errors").is_some(), "{with_action}");

    let catalog = gql(
        &ctx,
        "query { lifecycleActionDescriptors { kind } }",
        json!({}),
    )
    .await;
    assert!(catalog.get("errors").is_some(), "{catalog}");

    let list = gql(&ctx, LIST_QUERY, json!({})).await;
    assert_no_errors(&list);
    assert!(
        list["data"]["lifecycleRuleSets"]
            .as_array()
            .expect("rule set list")
            .is_empty()
    );
}

// ===========================================================================
// 6. Permissions
// ===========================================================================

#[tokio::test]
async fn lifecycle_rule_surface_denies_an_actor_without_catalog_settings() {
    let ctx = TestContext::new().await;
    let denied = create_unprivileged_user(&ctx, "lifecycle-denied").await;

    let created = gql_as(
        &ctx,
        &create_mutation(),
        json!({
            "input": {
                "name": "Not allowed",
                "regoSource": MONITORED_SOURCE,
            }
        }),
        &denied,
    )
    .await;
    assert_error_contains(&created, "unauthorized");

    let listed = gql_as(&ctx, LIST_QUERY, json!({}), &denied).await;
    assert_error_contains(&listed, "unauthorized");

    let validated = gql_as(
        &ctx,
        VALIDATE_MUTATION,
        json!({ "input": { "regoSource": MONITORED_SOURCE } }),
        &denied,
    )
    .await;
    assert_error_contains(&validated, "unauthorized");

    let previewed = gql_as(
        &ctx,
        PREVIEW_MUTATION,
        json!({
            "input": {
                "regoSource": MONITORED_SOURCE,
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
        list["data"]["lifecycleRuleSets"]
            .as_array()
            .expect("rule set list")
            .is_empty()
    );
}

// ===========================================================================
// 6. Scheduled evaluation
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
    actions { heads }
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
        "query($ruleSetId: ID, $states: [LifecycleCandidateState!]) {{
            lifecycleCandidates(
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
    format!("query {{ lifecycleInstanceGates {{{GATES_FIELDS}}} }}")
}

fn set_gates_mutation() -> String {
    format!(
        "mutation($input: SetLifecycleInstanceGatesInput!) {{
            setLifecycleInstanceGates(input: $input) {{{GATES_FIELDS}}}
        }}"
    )
}

const SET_STATE_MUTATION: &str = r#"mutation(
    $ruleSetId: ID!
    $state: LifecycleRuleState!
    $confirmation: LifecycleRuleArmingConfirmationInput
) {
    setLifecycleRuleState(ruleSetId: $ruleSetId, state: $state, confirmation: $confirmation) {
        id
        state
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
    body["data"]["setLifecycleRuleState"].clone()
}

const RUN_NOW_MUTATION: &str = r#"mutation($ruleSetId: ID) {
    runLifecycleEvaluationNow(ruleSetId: $ruleSetId) { started message }
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
        "mutation($input: ExcludeLifecycleSubjectInput!) {{
            excludeLifecycleSubject(input: $input) {{{EXCLUSION_FIELDS}}}
        }}"
    )
}

fn exclusions_query() -> String {
    format!(
        "query($ruleSetId: ID) {{
            lifecycleExclusions(ruleSetId: $ruleSetId) {{{EXCLUSION_FIELDS}}}
        }}"
    )
}

const REMOVE_EXCLUSION_MUTATION: &str = r#"mutation($id: ID!) {
    removeLifecycleExclusion(id: $id) { id }
}"#;

const EVALUATION_RUNS_QUERY: &str = r#"query($ruleSetId: ID) {
    lifecycleEvaluationRuns(ruleSetId: $ruleSetId, limit: 10) {
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
async fn seed_lifecycle_gate_definitions(ctx: &TestContext) {
    let seeds = [
        "lifecycle.gate.evaluation",
        "lifecycle.gate.result_display",
        "lifecycle.gate.reversible_effects",
        "lifecycle.gate.destructive_effects",
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
        .expect("seed lifecycle gate definitions");
}

/// Put one rule in dry run and switch lifecycle on: both are always
/// deliberate, separate steps, which is the whole point of shipping dark.
async fn arm_rule_and_gate(ctx: &TestContext, rule_set_id: &str) {
    seed_lifecycle_gate_definitions(ctx).await;
    let dry_run = set_rule_state(ctx, rule_set_id, "DRY_RUN", Value::Null).await;
    assert_eq!(dry_run["state"], "DRY_RUN");

    let gates = gql(
        ctx,
        &set_gates_mutation(),
        json!({ "input": { "enabled": true } }),
    )
    .await;
    assert_no_errors(&gates);
    let switched = &gates["data"]["setLifecycleInstanceGates"];
    assert_eq!(switched["enabled"], true);
    assert_eq!(
        switched["actionsPaused"], true,
        "enabling never unpauses actions"
    );
}

#[tokio::test]
async fn lifecycle_evaluation_is_dark_until_both_the_rule_and_the_gate_are_armed() {
    let ctx = TestContext::new().await;
    seed_title(&ctx, "title-dark-1", "Monitored Movie", true).await;
    let detail = create_rule(&ctx, "Stale movies", MONITORED_SOURCE).await;
    let rule_set_id = detail["ruleSet"]["id"]
        .as_str()
        .expect("rule id")
        .to_string();
    seed_lifecycle_gate_definitions(&ctx).await;

    // Gate off: the run is refused and says so rather than silently doing
    // nothing.
    let gated = gql(&ctx, RUN_NOW_MUTATION, json!({ "ruleSetId": rule_set_id })).await;
    assert_no_errors(&gated);
    assert_eq!(gated["data"]["runLifecycleEvaluationNow"]["started"], false);
    assert!(
        gated["data"]["runLifecycleEvaluationNow"]["message"]
            .as_str()
            .expect("message")
            .contains("gate is off")
    );

    // Lifecycle on, rule still disabled: still nothing.
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
        disabled["data"]["runLifecycleEvaluationNow"]["started"],
        false
    );

    let candidates = gql(&ctx, &candidates_query(), json!({})).await;
    assert_no_errors(&candidates);
    assert!(
        candidates["data"]["lifecycleCandidates"]
            .as_array()
            .expect("candidates")
            .is_empty(),
        "nothing may be recorded while either switch is off"
    );
}

#[tokio::test]
async fn a_dry_run_rule_records_a_candidate_shown_only_while_lifecycle_is_on() {
    let ctx = TestContext::new().await;
    seed_title(&ctx, "title-shadow-1", "Monitored Movie", true).await;
    seed_title(&ctx, "title-shadow-2", "Unmonitored Movie", false).await;
    let detail = create_rule(&ctx, "Stale movies", MONITORED_SOURCE).await;
    let rule_set_id = detail["ruleSet"]["id"]
        .as_str()
        .expect("rule id")
        .to_string();
    arm_rule_and_gate(&ctx, &rule_set_id).await;

    let started = gql(&ctx, RUN_NOW_MUTATION, json!({ "ruleSetId": rule_set_id })).await;
    assert_no_errors(&started);
    assert_eq!(
        started["data"]["runLifecycleEvaluationNow"]["started"],
        true
    );

    let shown = gql(
        &ctx,
        &candidates_query(),
        json!({ "ruleSetId": rule_set_id }),
    )
    .await;
    assert_no_errors(&shown);
    let candidates = shown["data"]["lifecycleCandidates"]
        .as_array()
        .expect("candidates");
    assert_eq!(candidates.len(), 1, "{shown}");
    let candidate = &candidates[0];
    assert_eq!(candidate["titleId"], "title-shadow-1");
    assert_eq!(candidate["titleName"], "Monitored Movie");
    assert_eq!(candidate["ruleName"], "Stale movies");
    assert_eq!(candidate["state"], "OBSERVING");
    assert_eq!(candidate["stateReason"], "first_match");
    assert_eq!(candidate["actionKind"], "ACTION_SEQUENCE");
    assert_eq!(candidate["actions"]["heads"], json!(["DELETE_TITLE"]));
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
    let after_candidates = after["data"]["lifecycleCandidates"]
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
        by_state["data"]["lifecycleCandidates"]
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
    let run_rows = runs["data"]["lifecycleEvaluationRuns"]
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

    // Switching lifecycle off hides the results even from a caller that may
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
        hidden["data"]["lifecycleCandidates"]
            .as_array()
            .expect("candidates")
            .is_empty()
    );
}

#[tokio::test]
async fn an_exclusion_round_trips_and_closes_the_candidate_it_covers() {
    let ctx = TestContext::new().await;
    seed_title(&ctx, "title-excl-1", "Hands Off", true).await;
    let detail = create_rule(&ctx, "Stale movies", MONITORED_SOURCE).await;
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
    let exclusion = &excluded["data"]["excludeLifecycleSubject"];
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
        listed["data"]["lifecycleExclusions"]
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
    let rows = candidates["data"]["lifecycleCandidates"]
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
        removed["data"]["removeLifecycleExclusion"]["id"],
        exclusion_id.as_str()
    );
    let after = gql(&ctx, &exclusions_query(), json!({})).await;
    assert_no_errors(&after);
    assert!(
        after["data"]["lifecycleExclusions"]
            .as_array()
            .expect("exclusions")
            .is_empty()
    );
}

#[tokio::test]
async fn the_instance_switches_round_trip_and_default_to_off() {
    let ctx = TestContext::new().await;
    seed_lifecycle_gate_definitions(&ctx).await;

    let initial = gql(&ctx, &gates_query(), json!({})).await;
    assert_no_errors(&initial);
    let gates = &initial["data"]["lifecycleInstanceGates"];
    assert_eq!(gates["enabled"], false, "lifecycle must ship off");
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
    let updated = &enabled["data"]["setLifecycleInstanceGates"];
    assert_eq!(updated["enabled"], true);
    assert_eq!(updated["actionsPaused"], false);

    let reread = gql(&ctx, &gates_query(), json!({})).await;
    assert_no_errors(&reread);
    assert_eq!(
        reread["data"]["lifecycleInstanceGates"],
        enabled["data"]["setLifecycleInstanceGates"]
    );
}

#[tokio::test]
async fn the_evaluation_surface_is_permission_gated() {
    let ctx = TestContext::new().await;
    let denied = create_unprivileged_user(&ctx, "lifecycle-evaluation-denied").await;

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

// ── Action execution ────────────────────────────────────────────────────────

const RUN_HANDLER_MUTATION: &str = r#"mutation($ruleSetId: ID) {
    runLifecycleActionHandlerNow(ruleSetId: $ruleSetId) { started message }
}"#;

const ACTION_RUNS_QUERY: &str = r#"query($ruleSetId: ID) {
    lifecycleActionRuns(ruleSetId: $ruleSetId, limit: 10) {
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

/// Put a rule in dry run, switch lifecycle on and unpause actions. Every
/// step is its own deliberate mutation, mirroring how an operator arms the
/// real thing.
async fn observe_rule_with_gates(ctx: &TestContext, rule_set_id: &str) {
    seed_lifecycle_gate_definitions(ctx).await;
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
        let rows = shown["data"]["lifecycleCandidates"]
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
    let detail = create_rule(&ctx, "Review deletion", MONITORED_SOURCE).await;
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
        assert_eq!(
            error_reason(&response),
            "LIFECYCLE_ARM_CONFIRMATION_STALE",
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
    assert_eq!(enabled["editedSinceEnabled"], false);
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
        &update_source_mutation(),
        json!({ "input": {
            "id": id,
            "regoSource": MONITORED_SOURCE,
            "graceDays": 3,
        }}),
    )
    .await;
    assert_no_errors(&edited);
    let rule = &edited["data"]["updateLifecycleRuleSource"]["ruleSet"];
    assert_eq!(rule["state"], "DRY_RUN");
    assert_eq!(rule["editedSinceEnabled"], true);
}

#[tokio::test]
async fn the_full_destructive_journey_removes_a_matching_title() {
    let ctx = TestContext::new().await;
    seed_title(&ctx, "title-doomed", "Doomed Movie", true).await;
    let root = tempfile::tempdir().expect("lifecycle fixture root");
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
    .expect("configure isolated lifecycle root");
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
    let detail = create_rule(&ctx, "Retire watched", MONITORED_SOURCE).await;
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
    assert_eq!(
        error_reason(&mismatch),
        "LIFECYCLE_ARM_COUNT_MISMATCH",
        "{mismatch}"
    );
    assert_eq!(
        mismatch["errors"][0]["extensions"]["currentCount"], 1,
        "{mismatch}"
    );

    let armed = set_rule_state(
        &ctx,
        &rule_set_id,
        "ENABLED",
        json!({ "revisionNumber": 1, "libraryIds": [], "candidateCount": 1 }),
    )
    .await;
    assert_eq!(armed["state"], "ENABLED");

    let handled = gql(
        &ctx,
        RUN_HANDLER_MUTATION,
        json!({ "ruleSetId": rule_set_id }),
    )
    .await;
    assert_no_errors(&handled);
    assert_eq!(
        handled["data"]["runLifecycleActionHandlerNow"]["started"],
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
    let rows = runs["data"]["lifecycleActionRuns"]
        .as_array()
        .expect("runs");
    assert_eq!(rows.len(), 1, "{runs}");
    assert_eq!(rows[0]["status"], "succeeded");
    assert_eq!(rows[0]["actionKind"], "ACTION_SEQUENCE");
    assert_eq!(rows[0]["attempt"], 1);
    assert_ne!(rows[0]["finishedAt"], Value::Null);
}

#[tokio::test]
async fn a_reversible_unmonitor_journey_needs_no_confirmation() {
    let ctx = TestContext::new().await;
    seed_title(&ctx, "title-unmon", "Stale Movie", true).await;
    let detail = create_rule(&ctx, "Unmonitor stale", MONITORED_UNMONITOR).await;
    let rule_set_id = detail["ruleSet"]["id"]
        .as_str()
        .expect("rule id")
        .to_string();
    observe_rule_with_gates(&ctx, &rule_set_id).await;

    let armed = set_rule_state(&ctx, &rule_set_id, "ENABLED", Value::Null).await;
    assert_eq!(armed["state"], "ENABLED");

    let started = gql(&ctx, RUN_NOW_MUTATION, json!({ "ruleSetId": rule_set_id })).await;
    assert_no_errors(&started);
    let handled = gql(
        &ctx,
        RUN_HANDLER_MUTATION,
        json!({ "ruleSetId": rule_set_id }),
    )
    .await;
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
async fn a_subject_that_stopped_matching_is_canceled_before_it_is_acted_on() {
    let ctx = TestContext::new().await;
    seed_title(&ctx, "title-kept", "Rewatched Movie", true).await;
    let detail = create_rule(&ctx, "Retire watched", MONITORED_SOURCE).await;
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

    // The subject stops matching after the first evaluation. Acting now
    // evaluates the rule again first, and that evaluation cancels the
    // candidate rather than letting it delete.
    ctx.titles
        .update_monitored("title-kept", false)
        .await
        .expect("unmonitor title");

    let handled = gql(
        &ctx,
        RUN_HANDLER_MUTATION,
        json!({ "ruleSetId": rule_set_id }),
    )
    .await;
    assert_no_errors(&handled);

    let candidate = wait_for_candidate_state(&ctx, &rule_set_id, "CANCELED").await;
    assert_eq!(candidate["stateReason"], "no_match");

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
    let detail = create_rule(&ctx, "Flag for review", &add_tag_source("needs review")).await;
    let rule_set_id = detail["ruleSet"]["id"]
        .as_str()
        .expect("rule id")
        .to_string();
    assert_eq!(detail["declaredActions"]["heads"], json!(["ADD_TAGS"]));
    assert_eq!(
        detail["declaredActions"]["addTags"],
        json!(["needs review"])
    );

    run_tag_rule(&ctx, &rule_set_id).await;
    let handled = gql(
        &ctx,
        RUN_HANDLER_MUTATION,
        json!({ "ruleSetId": rule_set_id }),
    )
    .await;
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
    let detail = create_rule(&ctx, "Flag for review", &add_tag_source("needs review")).await;
    let rule_set_id = detail["ruleSet"]["id"]
        .as_str()
        .expect("rule id")
        .to_string();

    run_tag_rule(&ctx, &rule_set_id).await;
    let handled = gql(
        &ctx,
        RUN_HANDLER_MUTATION,
        json!({ "ruleSetId": rule_set_id }),
    )
    .await;
    assert_no_errors(&handled);

    // The sequence as a whole succeeds; its one step records that the label
    // was already there, so nothing was written.
    wait_for_candidate_state(&ctx, &rule_set_id, "SUCCEEDED").await;
    let runs = gql(
        &ctx,
        r#"query($ruleSetId: ID) {
            lifecycleActionRuns(ruleSetId: $ruleSetId, limit: 10) {
                actionKind
                sequenceSteps { step { kind } run { state } }
            }
        }"#,
        json!({ "ruleSetId": rule_set_id }),
    )
    .await;
    assert_no_errors(&runs);
    let run = &runs["data"]["lifecycleActionRuns"][0];
    assert_eq!(run["actionKind"], "ACTION_SEQUENCE", "{runs}");
    let steps = run["sequenceSteps"].as_array().expect("steps");
    assert_eq!(steps.len(), 1, "{runs}");
    assert_eq!(steps[0]["run"]["state"], "already_satisfied", "{runs}");
    let title = ctx
        .titles
        .get_by_id("title-tagged")
        .await
        .expect("read title")
        .expect("title survives");
    assert_eq!(title.tags, vec!["needs review".to_string()]);
}

#[tokio::test]
async fn a_tag_rule_holds_when_its_label_was_deleted_after_authoring() {
    let ctx = TestContext::new().await;
    let tag_id = define_tag(&ctx, "needs review").await;
    seed_title(&ctx, "title-orphaned", "Orphaned Movie", true).await;
    let detail = create_rule(&ctx, "Flag for review", &add_tag_source("needs review")).await;
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

    let handled = gql(
        &ctx,
        RUN_HANDLER_MUTATION,
        json!({ "ruleSetId": rule_set_id }),
    )
    .await;
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

    let adder = create_rule(&ctx, "Flag for review", &add_tag_source("needs review")).await;
    let adder_id = adder["ruleSet"]["id"]
        .as_str()
        .expect("rule id")
        .to_string();
    let remover = create_rule(
        &ctx,
        "Clear review flag",
        &remove_tag_source("needs review"),
    )
    .await;
    let remover_id = remover["ruleSet"]["id"]
        .as_str()
        .expect("rule id")
        .to_string();

    run_tag_rule(&ctx, &adder_id).await;
    run_tag_rule(&ctx, &remover_id).await;

    for rule_set_id in [&adder_id, &remover_id] {
        let handled = gql(
            &ctx,
            RUN_HANDLER_MUTATION,
            json!({ "ruleSetId": rule_set_id }),
        )
        .await;
        assert_no_errors(&handled);
    }

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
                "regoSource": add_tag_source("never defined"),
            }
        }),
    )
    .await;
    assert_eq!(
        error_reason(&refused),
        "CATALOG_NOT_DEFINED_TAG",
        "{refused}"
    );
}
